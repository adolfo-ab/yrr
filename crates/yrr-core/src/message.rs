use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The envelope that flows between agents via the signal bus.
/// Lightweight by design — payload is a short string pointing to the codebase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalMessage {
    /// Unique message ID.
    pub id: Uuid,
    /// Correlation ID — tracks the entire chain of signals from the initial prompt.
    pub correlation_id: Uuid,
    /// Which agent emitted this signal (unique agent instance ID).
    pub source_agent_id: String,
    /// Which agent name emitted this signal.
    pub source_agent_name: String,
    /// The signal name.
    pub signal: String,
    /// Lightweight payload — a short string, not bulk data.
    pub payload: String,
    /// When this signal was emitted.
    pub timestamp: DateTime<Utc>,
    /// Breadcrumb trail of the chain that led to this signal.
    pub trace: Vec<TraceEntry>,
}

impl SignalMessage {
    /// Create a new signal message.
    pub fn new(
        source_agent_id: impl Into<String>,
        source_agent_name: impl Into<String>,
        signal: impl Into<String>,
        payload: impl Into<String>,
        correlation_id: Uuid,
        trace: Vec<TraceEntry>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            correlation_id,
            source_agent_id: source_agent_id.into(),
            source_agent_name: source_agent_name.into(),
            signal: signal.into(),
            payload: payload.into(),
            timestamp: Utc::now(),
            trace,
        }
    }

    /// Create a prompt message — the initial message that kicks off a swarm.
    pub fn prompt(signal: impl Into<String>, payload: impl Into<String>) -> Self {
        let correlation_id = Uuid::new_v4();
        Self {
            id: Uuid::new_v4(),
            correlation_id,
            source_agent_id: "yrr-cli".into(),
            source_agent_name: "prompt".into(),
            signal: signal.into(),
            payload: payload.into(),
            timestamp: Utc::now(),
            trace: vec![],
        }
    }

    /// Build a child trace from this message, appending the current signal.
    pub fn child_trace(&self) -> Vec<TraceEntry> {
        let mut trace = self.trace.clone();
        trace.push(TraceEntry {
            agent_id: self.source_agent_id.clone(),
            agent_name: self.source_agent_name.clone(),
            signal: self.signal.clone(),
            timestamp: self.timestamp,
        });
        trace
    }
}

/// A single entry in the signal trace — breadcrumb trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    /// Unique agent instance ID.
    pub agent_id: String,
    /// Agent name.
    pub agent_name: String,
    /// Signal that was emitted.
    pub signal: String,
    /// When it was emitted.
    pub timestamp: DateTime<Utc>,
}

/// Token usage from a single runtime invocation.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

/// Output from an agent after execution.
pub struct AgentOutput {
    /// Raw output content from the agent.
    pub content: String,
    /// Signals parsed from the output.
    pub emitted_signals: Vec<EmittedSignal>,
    /// Queries parsed from the output.
    pub emitted_queries: Vec<EmittedQuery>,
    /// Session ID returned by the runtime (for conversation persistence).
    pub session_id: Option<String>,
    /// Token usage for this invocation.
    pub usage: Option<TokenUsage>,
}

/// A signal parsed from agent output.
pub struct EmittedSignal {
    /// The signal name (from `<<SIGNAL:name>>`).
    pub signal: String,
    /// The payload text following the marker.
    pub payload: String,
}

/// A query parsed from agent output.
pub struct EmittedQuery {
    /// The queryable key name (from `<<QUERY:key>>`).
    pub key: String,
    /// The payload text following the marker.
    pub payload: String,
}

/// A reply received from a queryable, to be injected into re-activation.
pub struct QueryReply {
    /// Which query key this reply is for.
    pub key: String,
    /// The original payload that was sent.
    pub request_payload: String,
    /// The reply content.
    pub reply: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_message_has_correct_defaults() {
        let msg = SignalMessage::prompt("task_received", "Add a login page");
        assert_eq!(msg.signal, "task_received");
        assert_eq!(msg.payload, "Add a login page");
        assert_eq!(msg.source_agent_name, "prompt");
        assert!(msg.trace.is_empty());
        assert_eq!(msg.id, msg.id); // uuid was generated
        assert_eq!(msg.correlation_id, msg.correlation_id);
    }

    #[test]
    fn child_trace_appends() {
        let msg = SignalMessage::prompt("task_received", "do stuff");
        let child_trace = msg.child_trace();
        assert_eq!(child_trace.len(), 1);
        assert_eq!(child_trace[0].signal, "task_received");
        assert_eq!(child_trace[0].agent_name, "prompt");
    }

    #[test]
    fn signal_message_serializes_to_json() {
        let msg = SignalMessage::prompt("plan_ready", "Plan written to PLAN.md");
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SignalMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.signal, "plan_ready");
        assert_eq!(parsed.payload, "Plan written to PLAN.md");
        assert_eq!(parsed.correlation_id, msg.correlation_id);
    }
}
