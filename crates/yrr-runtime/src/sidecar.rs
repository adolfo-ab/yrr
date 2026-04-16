use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use yrr_core::config::Config;
use yrr_core::error::Result;
use yrr_core::message::{AgentOutput, QueryReply, SignalMessage, TokenUsage};
use yrr_core::runtime::AgentRuntime;
use yrr_core::schema::{AgentDef, ContextLimitAction, Lifecycle, LifecycleMode};
use yrr_bus::bus::{BusQuery, SignalBus};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::events::{self, EventSender, SwarmEvent};

/// What triggered this activation.
enum Activation {
    Signal(SignalMessage),
    Query(BusQuery),
    Steer(String),
}

/// Result of an activation — carries session and usage info back to the run loop.
struct ActivationResult {
    session_id: Option<String>,
    usage: Option<TokenUsage>,
}

/// The AgentSidecar is the bridge between the signal bus and the agent runtime.
/// One sidecar per agent instance.
///
/// Core loop: subscribe to signals and register queryables → on message or query,
/// spawn agent via runtime → parse emitted signals/queries → publish or reply.
pub struct AgentSidecar {
    /// Unique instance ID for this agent.
    pub agent_id: String,
    /// The agent definition (after overrides applied).
    pub agent_def: AgentDef,
    /// The runtime to execute the agent with.
    pub runtime: Arc<dyn AgentRuntime>,
    /// The signal bus.
    pub bus: Arc<dyn SignalBus>,
    /// Collect configuration: signal → count needed before triggering.
    pub collect: HashMap<String, u32>,
    /// Lifecycle configuration.
    pub lifecycle: Option<Lifecycle>,
    /// Signal names that are dispatched via pool (not subscribed directly).
    pub pooled_signals: HashSet<String>,
    /// Hard safety cap on activations (from config).
    pub max_activations_cap: u32,
    /// Max query loop iterations per activation.
    pub max_query_iterations: u32,
    /// Default query timeout.
    pub query_timeout: Duration,
    /// Optional event sender for TUI/observer.
    event_tx: EventSender,
    /// Messages to process immediately (for lazy-spawned agents that missed the bus signal).
    initial_messages: Vec<SignalMessage>,
}

impl AgentSidecar {
    pub fn new(
        agent_def: AgentDef,
        runtime: Arc<dyn AgentRuntime>,
        bus: Arc<dyn SignalBus>,
        collect: HashMap<String, u32>,
        lifecycle: Option<Lifecycle>,
        pooled_signals: HashSet<String>,
        config: &Config,
        event_tx: EventSender,
        initial_messages: Vec<SignalMessage>,
    ) -> Self {
        let agent_id = format!("{}-{}", agent_def.name, Uuid::new_v4());
        Self {
            agent_id,
            agent_def,
            runtime,
            bus,
            collect,
            lifecycle,
            pooled_signals,
            max_activations_cap: config.safety.max_activations,
            max_query_iterations: config.safety.max_query_iterations,
            query_timeout: Duration::from_secs(config.safety.default_query_timeout_secs),
            event_tx,
            initial_messages,
        }
    }

    /// Run the sidecar loop. Blocks until the agent's lifecycle ends.
    pub async fn run(mut self) -> Result<()> {
        info!(
            agent_id = %self.agent_id,
            agent = %self.agent_def.name,
            subscribe = ?self.agent_def.subscribe,
            publish = ?self.agent_def.publish,
            queryable = ?self.agent_def.queryable,
            query = ?self.agent_def.query,
            context = ?self.agent_def.context,
            lifecycle = ?self.lifecycle,
            "starting agent sidecar"
        );

        let has_pooled = !self.pooled_signals.is_empty();

        // Subscribe to all listened signals.
        let (merged_tx, mut merged_rx) = mpsc::channel::<SignalMessage>(128);

        // Subscribe to broadcast signals (those not in pooled_signals).
        for signal in self.agent_def.subscribe.names() {
            if self.pooled_signals.contains(signal) {
                continue; // Pooled signals come via dispatch channel.
            }
            let mut rx = self.bus.subscribe(signal).await?;
            let tx = merged_tx.clone();
            let signal_name = signal.to_string();

            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if tx.send(msg).await.is_err() {
                        break;
                    }
                }
                tracing::debug!(signal = %signal_name, "subscription channel closed");
            });
        }

        // Subscribe to dispatch channel for pooled signals.
        if has_pooled {
            let mut rx = self.bus.subscribe_dispatch(&self.agent_id).await?;
            let tx = merged_tx.clone();
            let agent_id = self.agent_id.clone();

            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if tx.send(msg).await.is_err() {
                        break;
                    }
                }
                tracing::debug!(agent_id = %agent_id, "dispatch channel closed");
            });

            // Publish initial idle status so the dispatcher knows we're ready.
            self.bus.publish_status(&self.agent_id, "idle").await?;
        }

        // Subscribe to steer channel if this agent is steerable.
        let mut steer_rx: Option<mpsc::Receiver<String>> = None;
        if self.agent_def.steer.is_some() {
            steer_rx = Some(self.bus.subscribe_steer(&self.agent_def.name).await?);
            info!(
                agent_id = %self.agent_id,
                agent = %self.agent_def.name,
                "agent is steerable, subscribed to steer channel"
            );
        }

        // Feed initial messages (from lazy spawning) into the merged channel.
        for msg in std::mem::take(&mut self.initial_messages) {
            let _ = merged_tx.send(msg).await;
        }

        // Drop our copy of the sender so merged_rx completes when all subs drop.
        drop(merged_tx);

        // Declare queryables and merge into a single channel.
        let (query_tx, mut query_rx) = mpsc::channel::<BusQuery>(64);
        let has_queryables = !self.agent_def.queryable.is_empty();

        for key in self.agent_def.queryable.names() {
            let mut rx = self.bus.declare_queryable(key).await?;
            let tx = query_tx.clone();
            let key_name = key.to_string();

            tokio::spawn(async move {
                while let Some(q) = rx.recv().await {
                    if tx.send(q).await.is_err() {
                        break;
                    }
                }
                tracing::debug!(queryable = %key_name, "queryable channel closed");
            });
        }

        drop(query_tx);

        let mut activation_count: u32 = 0;
        let mut collect_buffers: HashMap<String, Vec<SignalMessage>> = HashMap::new();
        let is_persistent = self
            .lifecycle
            .as_ref()
            .map(|l| l.mode == LifecycleMode::Persistent)
            .unwrap_or(false);

        // Session and context tracking (mutable state across activations).
        let mut session_id: Option<String> = None;
        let mut last_input_tokens: u64 = 0;

        // Warn if persistent agent has no activation limit.
        if is_persistent {
            let has_limit = self
                .lifecycle
                .as_ref()
                .map(|l| l.max_activations.is_some() || l.die_on.is_some())
                .unwrap_or(false);
            if !has_limit {
                let cap = self.max_activations_cap;
                warn!(
                    agent_id = %self.agent_id,
                    hard_cap = cap,
                    "persistent agent has no max_activations or die_on — \
                     hard safety cap of {cap} activations applies"
                );
            }
        }

        // Subscribe to die_on signals if configured.
        let mut die_rx = None;
        if let Some(lifecycle) = &self.lifecycle {
            if let Some(die_on) = &lifecycle.die_on {
                let (die_tx, rx) = mpsc::channel::<()>(1);
                for signal in die_on {
                    let mut sub = self.bus.subscribe(signal).await?;
                    let tx = die_tx.clone();
                    tokio::spawn(async move {
                        if sub.recv().await.is_some() {
                            let _ = tx.send(()).await;
                        }
                    });
                }
                drop(die_tx);
                die_rx = Some(rx);
            }
        }

        #[allow(unused_assignments)]
        let mut exit_reason = "unknown";

        loop {
            // Hard safety cap — always applies.
            if activation_count >= self.max_activations_cap {
                error!(
                    agent_id = %self.agent_id,
                    cap = self.max_activations_cap,
                    "SAFETY: hard activation cap reached, forcing shutdown"
                );
                exit_reason = "hard_activation_cap";
                break;
            }

            // Check user-configured limits.
            if let Some(lifecycle) = &self.lifecycle {
                if let Some(max) = lifecycle.max_activations {
                    if activation_count >= max {
                        info!(
                            agent_id = %self.agent_id,
                            "max activations reached ({max}), shutting down"
                        );
                        exit_reason = "max_activations";
                        break;
                    }
                }
                if let Some(max) = lifecycle.max_turns {
                    if activation_count >= max {
                        info!(
                            agent_id = %self.agent_id,
                            "max turns reached ({max}), shutting down"
                        );
                        exit_reason = "max_turns";
                        break;
                    }
                }
            }

            // Wait for a signal, a query, a steer message, or a die signal.
            let activation = tokio::select! {
                msg = merged_rx.recv() => {
                    match msg {
                        Some(m) => Activation::Signal(m),
                        None if has_queryables => {
                            // No more subscriptions but we have queryables — keep going.
                            // Wait on queries or die.
                            tokio::select! {
                                q = query_rx.recv() => {
                                    match q {
                                        Some(q) => Activation::Query(q),
                                        None => {
                                            exit_reason = "all_channels_closed";
                                            break;
                                        }
                                    }
                                }
                                _ = async {
                                    if let Some(rx) = die_rx.as_mut() {
                                        rx.recv().await;
                                    } else {
                                        std::future::pending::<()>().await;
                                    }
                                } => {
                                    info!(agent_id = %self.agent_id, "die_on signal received, shutting down");
                                    exit_reason = "die_on";
                                    break;
                                }
                            }
                        }
                        None => {
                            warn!(
                                agent_id = %self.agent_id,
                                agent = %self.agent_def.name,
                                is_persistent,
                                activations = activation_count,
                                "all subscription channels closed"
                            );
                            exit_reason = "subscriptions_closed";
                            break;
                        }
                    }
                }
                q = query_rx.recv(), if has_queryables => {
                    match q {
                        Some(q) => Activation::Query(q),
                        None => continue, // Queryable channel closed, fall through to signal wait.
                    }
                }
                payload = async {
                    if let Some(rx) = steer_rx.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<String>>().await
                    }
                } => {
                    match payload {
                        Some(p) => Activation::Steer(p),
                        None => continue,
                    }
                }
                _ = async {
                    if let Some(rx) = die_rx.as_mut() {
                        rx.recv().await;
                    } else {
                        // Never resolves.
                        std::future::pending::<()>().await;
                    }
                } => {
                    info!(agent_id = %self.agent_id, "die_on signal received, shutting down");
                    exit_reason = "die_on";
                    break;
                }
            };

            match activation {
                Activation::Signal(msg) => {
                    info!(
                        agent = %self.agent_def.name,
                        agent_id = %self.agent_id,
                        signal = %msg.signal,
                        from = %msg.source_agent_name,
                        payload = %truncate_payload(&msg.payload, 200),
                        "signal received"
                    );

                    events::emit(&self.event_tx, SwarmEvent::SignalReceived {
                        agent_id: self.agent_id.clone(),
                        agent_name: self.agent_def.name.clone(),
                        signal: msg.signal.clone(),
                        from_agent: msg.source_agent_name.clone(),
                        payload: msg.payload.clone(),
                        correlation_id: msg.correlation_id,
                        timestamp: msg.timestamp,
                    });

                    // Check if this signal needs collecting.
                    if let Some(&needed) = self.collect.get(&msg.signal) {
                        let buffer = collect_buffers.entry(msg.signal.clone()).or_default();
                        buffer.push(msg);
                        if (buffer.len() as u32) < needed {
                            continue; // Not enough yet.
                        }
                        // Enough collected — combine payloads and proceed.
                        let combined_payload: String = buffer
                            .iter()
                            .map(|m| format!("[{}] {}", m.source_agent_name, m.payload))
                            .collect::<Vec<_>>()
                            .join("\n");

                        let combined_msg = SignalMessage::new(
                            "collected",
                            "collector",
                            &buffer[0].signal,
                            combined_payload,
                            buffer[0].correlation_id,
                            buffer[0].child_trace(),
                        );
                        collect_buffers.remove(&combined_msg.signal);

                        if has_pooled {
                            self.bus.publish_status(&self.agent_id, "busy").await?;
                        }
                        events::emit(&self.event_tx, SwarmEvent::ActivationStarted {
                            agent_id: self.agent_id.clone(),
                            agent_name: self.agent_def.name.clone(),
                            trigger_signal: combined_msg.signal.clone(),
                        });
                        let activation_start = Instant::now();
                        match self.activate(&combined_msg, session_id.as_deref()).await {
                            Ok(result) => {
                                events::emit(&self.event_tx, SwarmEvent::ActivationCompleted {
                                    agent_id: self.agent_id.clone(),
                                    agent_name: self.agent_def.name.clone(),
                                    duration_ms: activation_start.elapsed().as_millis() as u64,
                                });
                                session_id = result.session_id;
                                if let Some(ref usage) = result.usage {
                                    last_input_tokens = usage.input_tokens;
                                }
                            }
                            Err(e) => {
                                events::emit(&self.event_tx, SwarmEvent::ActivationFailed {
                                    agent_id: self.agent_id.clone(),
                                    agent_name: self.agent_def.name.clone(),
                                    error: e.to_string(),
                                });
                                if has_pooled {
                                    self.bus.publish_status(&self.agent_id, "idle").await?;
                                }
                                return Err(e);
                            }
                        }
                        if has_pooled {
                            self.bus.publish_status(&self.agent_id, "idle").await?;
                        }
                    } else {
                        if has_pooled {
                            self.bus.publish_status(&self.agent_id, "busy").await?;
                        }
                        events::emit(&self.event_tx, SwarmEvent::ActivationStarted {
                            agent_id: self.agent_id.clone(),
                            agent_name: self.agent_def.name.clone(),
                            trigger_signal: msg.signal.clone(),
                        });
                        let activation_start = Instant::now();
                        match self.activate(&msg, session_id.as_deref()).await {
                            Ok(result) => {
                                events::emit(&self.event_tx, SwarmEvent::ActivationCompleted {
                                    agent_id: self.agent_id.clone(),
                                    agent_name: self.agent_def.name.clone(),
                                    duration_ms: activation_start.elapsed().as_millis() as u64,
                                });
                                session_id = result.session_id;
                                if let Some(ref usage) = result.usage {
                                    last_input_tokens = usage.input_tokens;
                                }
                            }
                            Err(e) => {
                                events::emit(&self.event_tx, SwarmEvent::ActivationFailed {
                                    agent_id: self.agent_id.clone(),
                                    agent_name: self.agent_def.name.clone(),
                                    error: e.to_string(),
                                });
                                if has_pooled {
                                    self.bus.publish_status(&self.agent_id, "idle").await?;
                                }
                                return Err(e);
                            }
                        }
                        if has_pooled {
                            self.bus.publish_status(&self.agent_id, "idle").await?;
                        }
                    }
                }
                Activation::Query(bus_query) => {
                    self.handle_query(bus_query).await;
                }
                Activation::Steer(payload) => {
                    info!(
                        agent = %self.agent_def.name,
                        agent_id = %self.agent_id,
                        payload = %truncate_payload(&payload, 200),
                        "steer message received from human"
                    );

                    events::emit(&self.event_tx, SwarmEvent::SteerReceived {
                        agent_id: self.agent_id.clone(),
                        agent_name: self.agent_def.name.clone(),
                        payload: payload.clone(),
                    });

                    let steer_input = format!(
                        "--- Human Steering ---\n{payload}\n--- End Steering ---"
                    );

                    let msg = SignalMessage::new(
                        "human",
                        "human",
                        "__steer__",
                        steer_input,
                        Uuid::new_v4(),
                        vec![],
                    );

                    if has_pooled {
                        self.bus.publish_status(&self.agent_id, "busy").await?;
                    }
                    events::emit(&self.event_tx, SwarmEvent::ActivationStarted {
                        agent_id: self.agent_id.clone(),
                        agent_name: self.agent_def.name.clone(),
                        trigger_signal: "__steer__".into(),
                    });
                    let activation_start = Instant::now();
                    match self.activate(&msg, session_id.as_deref()).await {
                        Ok(result) => {
                            events::emit(&self.event_tx, SwarmEvent::ActivationCompleted {
                                agent_id: self.agent_id.clone(),
                                agent_name: self.agent_def.name.clone(),
                                duration_ms: activation_start.elapsed().as_millis() as u64,
                            });
                            session_id = result.session_id;
                            if let Some(ref usage) = result.usage {
                                last_input_tokens = usage.input_tokens;
                            }
                        }
                        Err(e) => {
                            events::emit(&self.event_tx, SwarmEvent::ActivationFailed {
                                agent_id: self.agent_id.clone(),
                                agent_name: self.agent_def.name.clone(),
                                error: e.to_string(),
                            });
                            if has_pooled {
                                self.bus.publish_status(&self.agent_id, "idle").await?;
                            }
                            return Err(e);
                        }
                    }
                    if has_pooled {
                        self.bus.publish_status(&self.agent_id, "idle").await?;
                    }
                }
            }

            activation_count += 1;

            // ── Context limit check ─────────────────────────────────────
            if let Some(context) = &self.agent_def.context {
                if last_input_tokens >= context.max_tokens {
                    match context.on_limit {
                        ContextLimitAction::Kill => {
                            info!(
                                agent_id = %self.agent_id,
                                input_tokens = last_input_tokens,
                                max_tokens = context.max_tokens,
                                "context limit reached — killing agent"
                            );
                            exit_reason = "context_limit_kill";
                            break;
                        }
                        ContextLimitAction::Restart => {
                            info!(
                                agent_id = %self.agent_id,
                                input_tokens = last_input_tokens,
                                max_tokens = context.max_tokens,
                                "context limit reached — restarting with fresh session"
                            );
                            session_id = None;
                            last_input_tokens = 0;
                            // Continue the loop — next activation starts fresh.
                        }
                        ContextLimitAction::Compress => {
                            info!(
                                agent_id = %self.agent_id,
                                input_tokens = last_input_tokens,
                                max_tokens = context.max_tokens,
                                "context limit reached — continuing (runtime handles compression)"
                            );
                            // Keep going with the same session. Claude Code
                            // auto-compresses when its own context fills up.
                        }
                    }
                }
            }

            // If ephemeral, exit after one activation.
            if !is_persistent {
                exit_reason = "ephemeral";
                break;
            }
        }

        info!(
            agent_id = %self.agent_id,
            agent = %self.agent_def.name,
            activations = activation_count,
            exit_reason,
            is_persistent,
            "agent sidecar stopped"
        );

        events::emit(&self.event_tx, SwarmEvent::AgentStopped {
            agent_id: self.agent_id.clone(),
            agent_name: self.agent_def.name.clone(),
            reason: format!("{exit_reason} (activations={activation_count})"),
        });

        Ok(())
    }

    /// Handle an incoming query: activate the agent and reply with its output.
    async fn handle_query(&self, bus_query: BusQuery) {
        info!(
            agent = %self.agent_def.name,
            agent_id = %self.agent_id,
            queryable = %bus_query.key,
            payload = %truncate_payload(&bus_query.payload, 200),
            "query received, activating agent"
        );

        // Build a synthetic SignalMessage for the runtime.
        let input = SignalMessage::new(
            "query-client",
            "query-client",
            &format!("query:{}", bus_query.key),
            &bus_query.payload,
            Uuid::new_v4(),
            vec![],
        );

        // Queries don't participate in session tracking — always fresh.
        match self.runtime.run(&self.agent_def, &input, None).await {
            Ok(output) => {
                info!(
                    agent = %self.agent_def.name,
                    agent_id = %self.agent_id,
                    queryable = %bus_query.key,
                    reply_len = output.content.len(),
                    reply = %truncate_payload(&output.content, 500),
                    "replying to query"
                );
                if let Err(e) = bus_query.reply(output.content) {
                    error!(error = %e, "failed to send query reply");
                }
            }
            Err(e) => {
                error!(
                    agent = %self.agent_def.name,
                    agent_id = %self.agent_id,
                    error = %e,
                    "agent failed during query handling"
                );
                let _ = bus_query.reply_err(e.to_string());
            }
        }
    }

    /// Activate the agent: run it, resolve any queries, then publish emitted signals.
    /// Returns session and usage info from the final invocation.
    async fn activate(
        &self,
        input: &SignalMessage,
        session_id: Option<&str>,
    ) -> Result<ActivationResult> {
        info!(
            agent_id = %self.agent_id,
            signal = %input.signal,
            correlation_id = %input.correlation_id,
            resumed = session_id.is_some(),
            "activating agent"
        );

        let mut current_input = input.clone();
        let mut current_session_id = session_id.map(|s| s.to_string());

        for iteration in 0..=self.max_query_iterations {
            let output = match self
                .runtime
                .run(&self.agent_def, &current_input, current_session_id.as_deref())
                .await
            {
                Ok(out) => out,
                Err(e) => {
                    error!(
                        agent_id = %self.agent_id,
                        error = %e,
                        "agent execution failed"
                    );
                    return Err(e);
                }
            };

            // Track the session_id from this invocation.
            let result_session_id = output.session_id.clone().or(current_session_id.clone());
            let result_usage = output.usage.clone();

            // Update session for next iteration in the query loop.
            if let Some(ref sid) = output.session_id {
                current_session_id = Some(sid.clone());
            }

            // If no queries emitted, publish signals and return.
            if output.emitted_queries.is_empty() {
                self.publish_signals(&output, input).await?;
                return Ok(ActivationResult {
                    session_id: result_session_id,
                    usage: result_usage,
                });
            }

            // Validate and resolve queries.
            let valid_queries: Vec<_> = output
                .emitted_queries
                .iter()
                .filter(|q| {
                    if !self.agent_def.query.contains(&q.key) {
                        warn!(
                            agent = %self.agent_def.name,
                            agent_id = %self.agent_id,
                            query_key = %q.key,
                            "agent emitted undeclared query, skipping"
                        );
                        false
                    } else {
                        true
                    }
                })
                .collect();

            if valid_queries.is_empty() {
                // All queries were invalid — treat as final output.
                self.publish_signals(&output, input).await?;
                return Ok(ActivationResult {
                    session_id: result_session_id,
                    usage: result_usage,
                });
            }

            if iteration == self.max_query_iterations {
                warn!(
                    agent = %self.agent_def.name,
                    agent_id = %self.agent_id,
                    max = self.max_query_iterations,
                    "max query iterations reached, forcing completion"
                );
                self.publish_signals(&output, input).await?;
                return Ok(ActivationResult {
                    session_id: result_session_id,
                    usage: result_usage,
                });
            }

            // Issue all queries and collect replies.
            let mut replies = Vec::new();
            let timeout = self
                .agent_def
                .config
                .as_ref()
                .and_then(|c| c.get("query_timeout"))
                .and_then(|v| v.as_u64())
                .map(Duration::from_secs)
                .unwrap_or(self.query_timeout);

            for q in &valid_queries {
                info!(
                    agent = %self.agent_def.name,
                    agent_id = %self.agent_id,
                    query_key = %q.key,
                    payload = %truncate_payload(&q.payload, 200),
                    "issuing query"
                );

                match self.bus.query(&q.key, &q.payload, timeout).await {
                    Ok(reply) => {
                        info!(
                            agent = %self.agent_def.name,
                            agent_id = %self.agent_id,
                            query_key = %q.key,
                            reply_len = reply.len(),
                            reply = %truncate_payload(&reply, 500),
                            "query reply received"
                        );
                        replies.push(QueryReply {
                            key: q.key.clone(),
                            request_payload: q.payload.clone(),
                            reply,
                        });
                    }
                    Err(e) => {
                        error!(
                            agent = %self.agent_def.name,
                            agent_id = %self.agent_id,
                            query_key = %q.key,
                            error = %e,
                            "query failed"
                        );
                        replies.push(QueryReply {
                            key: q.key.clone(),
                            request_payload: q.payload.clone(),
                            reply: format!("ERROR: query failed: {e}"),
                        });
                    }
                }
            }

            // Build a new input with the replies injected.
            let mut reply_payload = String::new();
            reply_payload.push_str("--- Query Replies ---\n");
            for r in &replies {
                reply_payload.push_str(&format!(
                    "Query to '{}': {}\nReply:\n{}\n\n",
                    r.key, r.request_payload, r.reply
                ));
            }
            reply_payload.push_str("--- Original Task ---\n");
            reply_payload.push_str(&input.payload);

            current_input = SignalMessage::new(
                &input.source_agent_id,
                &input.source_agent_name,
                &input.signal,
                reply_payload,
                input.correlation_id,
                input.trace.clone(),
            );
        }

        Ok(ActivationResult {
            session_id: current_session_id,
            usage: None,
        })
    }

    /// Publish emitted signals from agent output.
    async fn publish_signals(&self, output: &AgentOutput, input: &SignalMessage) -> Result<()> {
        let child_trace = input.child_trace();
        let mut published_count = 0;

        for emitted in &output.emitted_signals {
            // Verify the agent is allowed to emit this signal.
            if !self.agent_def.publish.contains(&emitted.signal) {
                warn!(
                    agent = %self.agent_def.name,
                    agent_id = %self.agent_id,
                    signal = %emitted.signal,
                    "agent emitted undeclared signal, skipping"
                );
                continue;
            }

            let payload = truncate_payload(&emitted.payload, 300);

            info!(
                agent = %self.agent_def.name,
                agent_id = %self.agent_id,
                signal = %emitted.signal,
                payload = %payload,
                "signal emitted"
            );

            let msg = SignalMessage::new(
                &self.agent_id,
                &self.agent_def.name,
                &emitted.signal,
                &payload,
                input.correlation_id,
                child_trace.clone(),
            );

            self.bus.publish(&emitted.signal, &msg).await?;
            published_count += 1;

            events::emit(&self.event_tx, SwarmEvent::SignalEmitted {
                agent_id: self.agent_id.clone(),
                agent_name: self.agent_def.name.clone(),
                signal: emitted.signal.clone(),
                payload: emitted.payload.clone(),
                correlation_id: input.correlation_id,
                timestamp: Utc::now(),
            });
        }

        // Fallback: if no valid signals were published and the agent has
        // exactly one declared publish signal, auto-emit it.
        if published_count == 0 && self.agent_def.publish.len() == 1 {
            let entry = self.agent_def.publish.first().unwrap();
            let payload = truncate_payload(&output.content, 200);

            warn!(
                agent = %self.agent_def.name,
                agent_id = %self.agent_id,
                signal = %entry.name,
                "no valid signals emitted, auto-emitting sole declared signal"
            );

            let msg = SignalMessage::new(
                &self.agent_id,
                &self.agent_def.name,
                &entry.name,
                &payload,
                input.correlation_id,
                child_trace.clone(),
            );

            self.bus.publish(&entry.name, &msg).await?;

            events::emit(&self.event_tx, SwarmEvent::SignalEmitted {
                agent_id: self.agent_id.clone(),
                agent_name: self.agent_def.name.clone(),
                signal: entry.name.clone(),
                payload: output.content.clone(),
                correlation_id: input.correlation_id,
                timestamp: Utc::now(),
            });
        } else if published_count == 0 {
            warn!(
                agent = %self.agent_def.name,
                agent_id = %self.agent_id,
                "agent finished without emitting any valid signals"
            );
        }

        Ok(())
    }
}

/// Truncate a payload string to a maximum length, respecting UTF-8 boundaries.
fn truncate_payload(s: &str, max_len: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max_len {
        return trimmed.to_string();
    }
    // Find a safe char boundary.
    let mut end = max_len;
    while !trimmed.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}...", &trimmed[..end])
}
