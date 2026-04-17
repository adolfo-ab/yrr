use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use yrr_bus::bus::SignalBus;
use yrr_core::error::Result;
use yrr_core::message::SignalMessage;
use yrr_core::schema::{DispatchConfig, DispatchRule, DispatchStrategy, SignalList};

use crate::events::{self, EventSender, SwarmEvent};

/// Tracks the state of a replica for dispatch purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplicaStatus {
    Idle,
    Busy,
}

/// A replica known to the dispatcher.
#[derive(Debug)]
struct ReplicaState {
    agent_id: String,
    status: ReplicaStatus,
}

/// The Dispatcher routes signals to replicas in a pool.
///
/// Instead of all replicas subscribing to a signal directly (broadcast),
/// the dispatcher subscribes and routes each signal to a subset of idle
/// replicas based on the configured strategy and concurrency.
///
/// When all replicas are busy, incoming signals are queued and dispatched
/// as replicas become idle.
pub struct Dispatcher {
    /// The signal bus.
    bus: Arc<dyn SignalBus>,
    /// Dispatch rules per signal name.
    rules: HashMap<String, DispatchRule>,
    /// Registered replicas and their current status.
    replicas: Vec<ReplicaState>,
    /// Round-robin index (per signal, for round-robin strategy).
    rr_index: HashMap<String, usize>,
    /// Queue of (signal_name, message) waiting for idle replicas.
    queue: VecDeque<(String, SignalMessage)>,
    /// Optional event sender for TUI/observer.
    event_tx: EventSender,
    /// Messages to process immediately (for lazy-spawned dispatchers).
    initial_messages: Vec<SignalMessage>,
}

impl Dispatcher {
    /// Create a new dispatcher.
    ///
    /// - `bus`: the signal bus for subscribing and dispatching.
    /// - `dispatch_config`: the dispatch configuration from the swarm.
    /// - `agent_ids`: the agent IDs of all replicas in this group.
    /// - `subscribed_signals`: all signals this agent group subscribes to.
    pub fn new(
        bus: Arc<dyn SignalBus>,
        dispatch_config: &DispatchConfig,
        agent_ids: Vec<String>,
        subscribed_signals: &SignalList,
        event_tx: EventSender,
        initial_messages: Vec<SignalMessage>,
    ) -> Self {
        // Build per-signal rules for pooled signals only.
        let mut rules = HashMap::new();
        for signal in subscribed_signals.names() {
            if let Some(rule) = dispatch_config.rule_for(signal) {
                rules.insert(signal.to_string(), rule.clone());
            }
        }

        let replicas = agent_ids
            .into_iter()
            .map(|id| ReplicaState {
                agent_id: id,
                status: ReplicaStatus::Idle,
            })
            .collect();

        Self {
            bus,
            rules,
            replicas,
            rr_index: HashMap::new(),
            queue: VecDeque::new(),
            event_tx,
            initial_messages,
        }
    }

    /// Returns the signal names that this dispatcher handles (pool mode only).
    pub fn pooled_signals(&self) -> Vec<String> {
        self.rules.keys().cloned().collect()
    }

    /// Run the dispatcher loop. Blocks until all channels close.
    pub async fn run(mut self) -> Result<()> {
        let pooled_signals = self.pooled_signals();
        if pooled_signals.is_empty() {
            return Ok(()); // Nothing to dispatch.
        }

        info!(
            signals = ?pooled_signals,
            replicas = self.replicas.len(),
            "dispatcher starting"
        );

        // Subscribe to each pooled signal.
        let (signal_tx, mut signal_rx) = mpsc::channel::<SignalMessage>(128);
        for signal in &pooled_signals {
            let mut rx = self.bus.subscribe(signal).await?;
            let tx = signal_tx.clone();
            let sig = signal.clone();

            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if tx.send(msg).await.is_err() {
                        break;
                    }
                }
                debug!(signal = %sig, "dispatcher signal subscription closed");
            });
        }
        for msg in std::mem::take(&mut self.initial_messages) {
            let _ = signal_tx.send(msg).await;
        }
        drop(signal_tx);

        // Subscribe to status updates from all replicas.
        let mut status_rx = self.bus.subscribe_status().await?;

        loop {
            tokio::select! {
                msg = signal_rx.recv() => {
                    match msg {
                        Some(msg) => self.handle_signal(msg).await?,
                        None => break, // All signal subscriptions closed.
                    }
                }
                status = status_rx.recv() => {
                    match status {
                        Some((agent_id, status)) => {
                            self.handle_status_update(&agent_id, &status).await?;
                        }
                        None => break, // Status subscription closed.
                    }
                }
            }
        }

        info!("dispatcher stopped");
        Ok(())
    }

    /// Handle an incoming signal: dispatch to idle replicas or queue.
    async fn handle_signal(&mut self, msg: SignalMessage) -> Result<()> {
        let signal = msg.signal.clone();
        let rule = match self.rules.get(&signal) {
            Some(r) => r.clone(),
            None => return Ok(()), // Not a pooled signal (shouldn't happen).
        };

        let concurrency = rule.concurrency.max(1);
        let selected = self.select_replicas(&signal, &rule.strategy, concurrency as usize);

        if selected.is_empty() {
            // All replicas busy — queue the signal.
            info!(
                signal = %signal,
                queue_len = self.queue.len() + 1,
                "all replicas busy, queuing signal"
            );
            self.queue.push_back((signal.clone(), msg));
            events::emit(
                &self.event_tx,
                SwarmEvent::SignalQueued {
                    signal,
                    queue_len: self.queue.len(),
                },
            );
            return Ok(());
        }

        // Dispatch to selected replicas.
        for agent_id in &selected {
            info!(
                signal = %signal,
                agent_id = %agent_id,
                "dispatching signal to replica"
            );
            self.bus.dispatch_to(agent_id, &msg).await?;
            events::emit(
                &self.event_tx,
                SwarmEvent::SignalDispatched {
                    signal: signal.clone(),
                    target_agent_id: agent_id.clone(),
                },
            );
        }

        // Mark selected replicas as busy.
        for agent_id in &selected {
            if let Some(replica) = self.replicas.iter_mut().find(|r| r.agent_id == *agent_id) {
                replica.status = ReplicaStatus::Busy;
            }
        }

        // If fewer replicas were available than concurrency, queue for the rest.
        let shortfall = concurrency as usize - selected.len();
        if shortfall > 0 {
            debug!(
                signal = %signal,
                shortfall,
                "not enough idle replicas, queuing for remaining"
            );
            for _ in 0..shortfall {
                self.queue.push_back((signal.clone(), msg.clone()));
            }
        }

        Ok(())
    }

    /// Handle a status update from a replica.
    async fn handle_status_update(&mut self, agent_id: &str, status: &str) -> Result<()> {
        // Update replica status.
        let is_known = self.replicas.iter().any(|r| r.agent_id == agent_id);
        if !is_known {
            return Ok(()); // Not our replica.
        }

        let new_status = match status {
            "idle" => ReplicaStatus::Idle,
            "busy" => ReplicaStatus::Busy,
            _ => {
                warn!(agent_id, status, "unknown status value");
                return Ok(());
            }
        };

        if let Some(replica) = self.replicas.iter_mut().find(|r| r.agent_id == agent_id) {
            replica.status = new_status.clone();
        }

        // If a replica became idle, try to drain the queue.
        if new_status == ReplicaStatus::Idle {
            self.drain_queue().await?;
        }

        Ok(())
    }

    /// Try to dispatch queued signals to newly idle replicas.
    async fn drain_queue(&mut self) -> Result<()> {
        while !self.queue.is_empty() {
            let (signal, _) = self.queue.front().unwrap();
            let rule = match self.rules.get(signal) {
                Some(r) => r.clone(),
                None => {
                    self.queue.pop_front();
                    continue;
                }
            };

            // Try to find one idle replica for this queued item.
            let selected = self.select_replicas(&signal.clone(), &rule.strategy, 1);
            if selected.is_empty() {
                break; // No idle replicas available.
            }

            let (signal, msg) = self.queue.pop_front().unwrap();
            let agent_id = &selected[0];

            info!(
                signal = %signal,
                agent_id = %agent_id,
                queue_remaining = self.queue.len(),
                "dispatching queued signal to replica"
            );

            self.bus.dispatch_to(agent_id, &msg).await?;
            events::emit(
                &self.event_tx,
                SwarmEvent::SignalDispatched {
                    signal,
                    target_agent_id: agent_id.clone(),
                },
            );

            if let Some(replica) = self.replicas.iter_mut().find(|r| r.agent_id == *agent_id) {
                replica.status = ReplicaStatus::Busy;
            }
        }

        Ok(())
    }

    /// Select up to `count` idle replicas using the given strategy.
    fn select_replicas(
        &mut self,
        signal: &str,
        strategy: &DispatchStrategy,
        count: usize,
    ) -> Vec<String> {
        let idle_ids: Vec<usize> = self
            .replicas
            .iter()
            .enumerate()
            .filter(|(_, r)| r.status == ReplicaStatus::Idle)
            .map(|(i, _)| i)
            .collect();

        if idle_ids.is_empty() {
            return Vec::new();
        }

        let to_select = count.min(idle_ids.len());

        match strategy {
            DispatchStrategy::RoundRobin => {
                let start = self.rr_index.entry(signal.to_string()).or_insert(0);
                let mut selected = Vec::with_capacity(to_select);

                for i in 0..to_select {
                    let idx = (*start + i) % idle_ids.len();
                    selected.push(self.replicas[idle_ids[idx]].agent_id.clone());
                }

                *self.rr_index.get_mut(signal).unwrap() = (*start + to_select) % idle_ids.len();

                selected
            }
            DispatchStrategy::Random => {
                use rand::seq::SliceRandom;
                let mut rng = rand::rng();
                let mut shuffled = idle_ids;
                shuffled.shuffle(&mut rng);
                shuffled
                    .into_iter()
                    .take(to_select)
                    .map(|i| self.replicas[i].agent_id.clone())
                    .collect()
            }
            DispatchStrategy::LeastBusy => {
                // For least-busy, just pick the first N idle replicas.
                // In the future this could track activation counts for
                // more sophisticated load balancing.
                idle_ids
                    .into_iter()
                    .take(to_select)
                    .map(|i| self.replicas[i].agent_id.clone())
                    .collect()
            }
        }
    }
}
