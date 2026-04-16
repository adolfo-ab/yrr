use async_trait::async_trait;

use crate::error::Result;
use crate::message::{AgentOutput, SignalMessage};
use crate::schema::AgentDef;

/// Abstraction over AI agent backends.
///
/// Implementations: ClaudeCodeRuntime, and future runtimes (Ollama, OpenAI, script, etc.)
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    /// Run an agent to completion with the given input signal.
    /// Returns the agent's output with parsed emitted signals.
    ///
    /// `session_id` — if provided, the runtime should resume an existing
    /// conversation session (e.g. `claude --resume <id>`).
    async fn run(
        &self,
        agent: &AgentDef,
        input: &SignalMessage,
        session_id: Option<&str>,
    ) -> Result<AgentOutput>;

    /// Check if this runtime backend is available and healthy.
    async fn health_check(&self) -> Result<()>;

    /// Name of this runtime, for matching against the `runtime` field in agent definitions.
    fn name(&self) -> &str;
}
