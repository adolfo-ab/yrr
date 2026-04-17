use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Sender for swarm events. `None` means no observer (CLI mode).
pub type EventSender = Option<tokio::sync::mpsc::UnboundedSender<SwarmEvent>>;

/// Events emitted by the swarm runtime for TUI/observer consumption.
#[derive(Debug, Clone)]
pub enum SwarmEvent {
    AgentSpawned {
        agent_name: String,
        agent_id: String,
        replica_idx: u32,
        model: Option<String>,
    },
    AgentStopped {
        agent_id: String,
        agent_name: String,
        reason: String,
    },
    SignalReceived {
        agent_id: String,
        agent_name: String,
        signal: String,
        from_agent: String,
        payload: String,
        correlation_id: Uuid,
        timestamp: DateTime<Utc>,
    },
    SignalEmitted {
        agent_id: String,
        agent_name: String,
        signal: String,
        payload: String,
        correlation_id: Uuid,
        timestamp: DateTime<Utc>,
    },
    ActivationStarted {
        agent_id: String,
        agent_name: String,
        trigger_signal: String,
    },
    ActivationCompleted {
        agent_id: String,
        agent_name: String,
        duration_ms: u64,
    },
    ActivationFailed {
        agent_id: String,
        agent_name: String,
        error: String,
    },
    SignalDispatched {
        signal: String,
        target_agent_id: String,
    },
    SignalQueued {
        signal: String,
        queue_len: usize,
    },
    PromptInjected {
        signal: String,
    },
    DoneReceived {
        signal: String,
    },
    SteerReceived {
        agent_id: String,
        agent_name: String,
        payload: String,
    },
    SteerSent {
        agent_name: String,
        payload: String,
    },
    SwarmTimeout,
    SwarmInterrupted,
}

/// Emit a swarm event if a sender is present. Never blocks or panics.
pub fn emit(tx: &EventSender, event: SwarmEvent) {
    if let Some(tx) = tx {
        let _ = tx.send(event);
    }
}
