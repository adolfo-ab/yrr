use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

use yrr_bus::bus::SignalBus;
use yrr_bus::zenoh_bus::ZenohBus;
use yrr_core::config::Config;
use yrr_core::loader::{ResolvedAgent, ResolvedSwarm};
use yrr_core::message::SignalMessage;
use yrr_core::runtime::AgentRuntime;

use crate::claude::ClaudeCodeRuntime;
use crate::dispatcher::Dispatcher;
use crate::events::{self, EventSender, SwarmEvent};
use crate::sidecar::AgentSidecar;

/// Outcome of a swarm run.
#[derive(Debug, Clone)]
pub enum SwarmOutcome {
    Done { signal: String },
    AllSidecarsFinished,
    Timeout,
    Interrupted,
}

/// Runs a resolved swarm to completion.
pub struct SwarmRunner {
    pub resolved: ResolvedSwarm,
    pub config: Config,
    pub seed: Option<String>,
    pub timeout: Option<Duration>,
    pub event_tx: EventSender,
}

impl SwarmRunner {
    /// Run the swarm. Blocks until completion, timeout, or shutdown signal.
    pub async fn run(
        self,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<SwarmOutcome> {
        let SwarmRunner {
            resolved,
            config,
            seed,
            timeout,
            event_tx,
        } = self;

        info!(
            swarm = %resolved.name,
            agents = resolved.agents.len(),
            entry = ?resolved.entry,
            done = ?resolved.done,
            "starting swarm"
        );

        let bus: Arc<dyn SignalBus> = Arc::new(
            ZenohBus::new(&resolved.name)
                .await
                .context("failed to open zenoh bus")?,
        );

        let runtime: Arc<dyn AgentRuntime> = Arc::new(ClaudeCodeRuntime::new(&config));

        // Subscribe to done signals.
        let (done_tx, mut done_rx) = mpsc::channel::<SignalMessage>(1);
        for signal in &resolved.done {
            let mut rx = bus.subscribe(signal).await?;
            let tx = done_tx.clone();
            tokio::spawn(async move {
                if let Some(msg) = rx.recv().await {
                    let _ = tx.send(msg).await;
                }
            });
        }
        drop(done_tx);

        // ── Dependency maps ─────────────────────────────────────────────
        let mut signal_to_agents: HashMap<String, Vec<usize>> = HashMap::new();
        let mut queryable_to_agents: HashMap<String, Vec<usize>> = HashMap::new();
        // Spawn triggers: signal → [(agent_index, max, active_counter)].
        let mut spawn_triggers: HashMap<String, Vec<(usize, u32, Arc<AtomicU32>)>> =
            HashMap::new();

        for (idx, agent) in resolved.agents.iter().enumerate() {
            for signal in agent.def.subscribe.names() {
                signal_to_agents
                    .entry(signal.to_string())
                    .or_default()
                    .push(idx);
            }
            for key in agent.def.queryable.names() {
                queryable_to_agents
                    .entry(key.to_string())
                    .or_default()
                    .push(idx);
            }
            if let Some(spawn) = &agent.spawn {
                spawn_triggers
                    .entry(spawn.on.clone())
                    .or_default()
                    .push((idx, spawn.max, Arc::new(AtomicU32::new(0))));
            }
        }

        let mut spawned: HashSet<usize> = HashSet::new();
        let mut join_set: JoinSet<()> = JoinSet::new();

        // ── Spawn entry agents and their queryable providers ─────────
        let mut pending_spawns: Vec<usize> = Vec::new();
        for entry_signal in &resolved.entry {
            if let Some(indices) = signal_to_agents.get(entry_signal) {
                for &idx in indices {
                    if spawned.insert(idx) {
                        pending_spawns.push(idx);
                    }
                }
            }
        }

        // Transitively spawn queryable providers.
        let mut i = 0;
        while i < pending_spawns.len() {
            let idx = pending_spawns[i];
            for query_key in resolved.agents[idx].def.query.names() {
                if let Some(providers) = queryable_to_agents.get(query_key) {
                    for &p in providers {
                        if spawned.insert(p) {
                            pending_spawns.push(p);
                        }
                    }
                }
            }
            i += 1;
        }

        for idx in pending_spawns {
            spawn_agent_group(
                &mut join_set,
                &resolved.agents[idx],
                &runtime,
                &bus,
                &config,
                &event_tx,
                Vec::new(),
            );
        }

        // Give sidecars a moment to subscribe before injecting seed.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // ── Inject seed ─────────────────────────────────────────────────
        let seed_text = seed.unwrap_or_else(|| "Start".to_string());
        for entry_signal in &resolved.entry {
            let msg = SignalMessage::seed(entry_signal, &seed_text);
            info!(signal = %entry_signal, "injecting seed");
            bus.publish(entry_signal, &msg).await?;
            events::emit(
                &event_tx,
                SwarmEvent::SeedInjected {
                    signal: entry_signal.clone(),
                },
            );
        }

        // ── Subscribe to all signals for lazy spawning ──────────────────
        let lazy_signals: HashSet<String> = resolved
            .agents
            .iter()
            .flat_map(|a| a.def.subscribe.names())
            .chain(spawn_triggers.keys().map(|s| s.as_str()))
            .map(|s| s.to_string())
            .collect();

        let (lazy_tx, mut lazy_rx) = mpsc::channel::<SignalMessage>(256);
        for signal in &lazy_signals {
            let mut rx = bus.subscribe(signal).await?;
            let tx = lazy_tx.clone();
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if tx.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        }
        drop(lazy_tx);

        // ── Main event loop ─────────────────────────────────────────────
        let timeout_fut = async {
            match timeout {
                Some(d) => tokio::time::sleep(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(timeout_fut);
        tokio::pin!(shutdown_rx);

        // Helper: spawn pending agents triggered by a signal.
        let try_spawn_for_signal = |msg: &SignalMessage,
                                         spawned: &mut HashSet<usize>,
                                         join_set: &mut JoinSet<()>|
         -> bool {
            let mut new_spawns: Vec<usize> = Vec::new();
            if let Some(indices) = signal_to_agents.get(&msg.signal) {
                for &idx in indices {
                    if spawned.insert(idx) {
                        new_spawns.push(idx);
                    }
                }
            }

            // Transitively spawn queryable providers.
            let mut j = 0;
            while j < new_spawns.len() {
                let idx = new_spawns[j];
                for query_key in resolved.agents[idx].def.query.names() {
                    if let Some(providers) = queryable_to_agents.get(query_key) {
                        for &p in providers {
                            if spawned.insert(p) {
                                new_spawns.push(p);
                            }
                        }
                    }
                }
                j += 1;
            }

            let did_spawn = !new_spawns.is_empty();
            for idx in &new_spawns {
                spawn_agent_group(
                    join_set,
                    &resolved.agents[*idx],
                    &runtime,
                    &bus,
                    &config,
                    &event_tx,
                    vec![msg.clone()],
                );
            }
            did_spawn
        };

        let outcome = loop {
            tokio::select! {
                // Lazy spawn + dynamic spawn: when a signal fires, spawn pending
                // subscribers and/or dynamically spawn new instances.
                Some(msg) = lazy_rx.recv() => {
                    if try_spawn_for_signal(&msg, &mut spawned, &mut join_set) {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    if let Some(triggers) = spawn_triggers.get(&msg.signal) {
                        for (idx, max, active) in triggers {
                            let current = active.load(Ordering::SeqCst);
                            if current >= *max {
                                warn!(
                                    agent = %resolved.agents[*idx].swarm_key,
                                    current,
                                    max,
                                    "spawn cap reached, skipping"
                                );
                                continue;
                            }
                            active.fetch_add(1, Ordering::SeqCst);
                            let counter = Arc::clone(active);
                            spawn_single_instance(
                                &mut join_set,
                                &resolved.agents[*idx],
                                &runtime,
                                &bus,
                                &config,
                                &event_tx,
                                vec![msg.clone()],
                                counter,
                            );
                        }
                    }
                }

                // Done signal — stop spawning but let active agents finish.
                Some(done_msg) = done_rx.recv() => {
                    let signal = done_msg.signal;
                    info!(signal = %signal, "swarm done signal received, draining active agents");
                    events::emit(&event_tx, SwarmEvent::DoneReceived { signal: signal.clone() });
                    while join_set.join_next().await.is_some() {}
                    break SwarmOutcome::Done { signal };
                }

                // A sidecar/dispatcher task finished.
                Some(_) = join_set.join_next() => {
                    // When all running tasks finish, wait for in-flight signals
                    // before declaring the swarm done. An ephemeral agent may
                    // publish a signal and exit almost simultaneously — the
                    // signal needs time to propagate through the bus.
                    while join_set.is_empty() {
                        match tokio::time::timeout(
                            Duration::from_secs(1),
                            lazy_rx.recv(),
                        ).await {
                            Ok(Some(msg)) => {
                                try_spawn_for_signal(&msg, &mut spawned, &mut join_set);
                                if let Some(triggers) = spawn_triggers.get(&msg.signal) {
                                    for (idx, max, active) in triggers {
                                        let current = active.load(Ordering::SeqCst);
                                        if current >= *max {
                                            continue;
                                        }
                                        active.fetch_add(1, Ordering::SeqCst);
                                        let counter = Arc::clone(active);
                                        spawn_single_instance(
                                            &mut join_set,
                                            &resolved.agents[*idx],
                                            &runtime,
                                            &bus,
                                            &config,
                                            &event_tx,
                                            vec![msg.clone()],
                                            counter,
                                        );
                                    }
                                }
                            }
                            _ => break,
                        }
                    }
                    if join_set.is_empty() {
                        break SwarmOutcome::AllSidecarsFinished;
                    }
                }

                // Timeout.
                _ = &mut timeout_fut => {
                    info!(timeout_secs = timeout.map(|d| d.as_secs()).unwrap_or(0), "swarm timed out");
                    events::emit(&event_tx, SwarmEvent::SwarmTimeout);
                    break SwarmOutcome::Timeout;
                }

                // Shutdown.
                _ = &mut shutdown_rx => {
                    info!("shutdown signal received");
                    events::emit(&event_tx, SwarmEvent::SwarmInterrupted);
                    break SwarmOutcome::Interrupted;
                }
            }
        };

        join_set.abort_all();
        bus.close().await?;
        info!(outcome = ?outcome, "swarm finished");

        Ok(outcome)
    }
}

/// Spawn an agent group (replicas + optional dispatcher) into the JoinSet.
fn spawn_agent_group(
    join_set: &mut JoinSet<()>,
    agent: &ResolvedAgent,
    runtime: &Arc<dyn AgentRuntime>,
    bus: &Arc<dyn SignalBus>,
    config: &Config,
    event_tx: &EventSender,
    initial_messages: Vec<SignalMessage>,
) {
    let pooled_signals: HashSet<String> = agent
        .dispatch
        .as_ref()
        .map(|d| d.pooled_signals(&agent.def.subscribe).into_iter().collect())
        .unwrap_or_default();

    let has_dispatch = agent.dispatch.is_some() && !pooled_signals.is_empty();

    // Split initial messages: pooled signals go to the dispatcher, rest to sidecars.
    let (dispatcher_msgs, sidecar_msgs) = if has_dispatch {
        let mut d_msgs = Vec::new();
        let mut s_msgs = Vec::new();
        for msg in initial_messages {
            if pooled_signals.contains(&msg.signal) {
                d_msgs.push(msg);
            } else {
                s_msgs.push(msg);
            }
        }
        (d_msgs, s_msgs)
    } else {
        (Vec::new(), initial_messages)
    };

    let mut agent_ids = Vec::new();
    for replica_idx in 0..agent.replicas {
        let sidecar = AgentSidecar::new(
            agent.def.clone(),
            Arc::clone(runtime),
            Arc::clone(bus),
            agent.collect.clone(),
            agent.lifecycle.clone(),
            pooled_signals.clone(),
            config,
            event_tx.clone(),
            sidecar_msgs.clone(),
        );

        agent_ids.push(sidecar.agent_id.clone());

        let model = agent
            .def
            .config
            .as_ref()
            .and_then(|c| c.get("model"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        info!(
            agent = %agent.swarm_key,
            replica = replica_idx,
            agent_id = %sidecar.agent_id,
            pooled = ?pooled_signals,
            "spawning sidecar"
        );

        events::emit(
            event_tx,
            SwarmEvent::AgentSpawned {
                agent_name: agent.swarm_key.clone(),
                agent_id: sidecar.agent_id.clone(),
                replica_idx,
                model,
            },
        );

        join_set.spawn(async move {
            if let Err(e) = sidecar.run().await {
                error!(error = %e, "sidecar failed");
            }
        });
    }

    if let Some(dispatch_config) = &agent.dispatch {
        if !pooled_signals.is_empty() {
            let dispatcher = Dispatcher::new(
                Arc::clone(bus),
                dispatch_config,
                agent_ids,
                &agent.def.subscribe,
                event_tx.clone(),
                dispatcher_msgs,
            );

            info!(
                agent = %agent.swarm_key,
                pooled = ?dispatcher.pooled_signals(),
                replicas = agent.replicas,
                "spawning dispatcher"
            );

            join_set.spawn(async move {
                if let Err(e) = dispatcher.run().await {
                    error!(error = %e, "dispatcher failed");
                }
            });
        }
    }
}

/// Spawn a single dynamically-triggered agent instance into the JoinSet.
/// Decrements `active_counter` when the sidecar finishes.
fn spawn_single_instance(
    join_set: &mut JoinSet<()>,
    agent: &ResolvedAgent,
    runtime: &Arc<dyn AgentRuntime>,
    bus: &Arc<dyn SignalBus>,
    config: &Config,
    event_tx: &EventSender,
    initial_messages: Vec<SignalMessage>,
    active_counter: Arc<AtomicU32>,
) {
    let sidecar = AgentSidecar::new(
        agent.def.clone(),
        Arc::clone(runtime),
        Arc::clone(bus),
        agent.collect.clone(),
        agent.lifecycle.clone(),
        HashSet::new(),
        config,
        event_tx.clone(),
        initial_messages,
    );

    let model = agent
        .def
        .config
        .as_ref()
        .and_then(|c| c.get("model"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    info!(
        agent = %agent.swarm_key,
        agent_id = %sidecar.agent_id,
        active = active_counter.load(Ordering::SeqCst),
        "spawning dynamic instance"
    );

    events::emit(
        event_tx,
        SwarmEvent::AgentSpawned {
            agent_name: agent.swarm_key.clone(),
            agent_id: sidecar.agent_id.clone(),
            replica_idx: 0,
            model,
        },
    );

    join_set.spawn(async move {
        if let Err(e) = sidecar.run().await {
            error!(error = %e, "dynamic sidecar failed");
        }
        active_counter.fetch_sub(1, Ordering::SeqCst);
    });
}
