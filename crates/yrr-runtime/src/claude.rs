use async_trait::async_trait;
use yrr_core::config::Config;
use yrr_core::error::{YrrError, Result};
use yrr_core::message::{AgentOutput, SignalMessage, TokenUsage};
use yrr_core::runtime::AgentRuntime;
use yrr_core::schema::{AgentDef, SignalList};
use serde::Deserialize;
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::signal_parser::{parse_queries, parse_signals};

// ─── Claude CLI JSON response types ─────────────────────────────────────────

#[derive(Deserialize)]
struct ClaudeJsonResponse {
    result: String,
    session_id: Option<String>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

// ─── Runtime ────────────────────────────────────────────────────────────────

/// Runtime that spawns the Claude Code CLI as a subprocess.
pub struct ClaudeCodeRuntime {
    /// How to handle tool permissions: "auto", "bypass", or "none".
    permission_mode: String,
    /// Default model when not specified in agent config.
    default_model: Option<String>,
}

impl ClaudeCodeRuntime {
    pub fn new(config: &Config) -> Self {
        Self {
            permission_mode: config.claude.permission_mode.clone(),
            default_model: config.defaults.model.clone(),
        }
    }

    /// Build the full prompt for the first activation (no prior session).
    /// Includes the agent's system prompt, signal/query instructions,
    /// and the incoming message payload.
    fn build_prompt(agent: &AgentDef, input: &SignalMessage) -> String {
        let mut prompt = String::from(
            "You are a swarm agent. Token efficiency is mandatory.\n\
             - Do NOT output prose, commentary, reasoning, or status updates.\n\
             - Do NOT explain what you will do or what you did.\n\
             - Use tools to do your work. Your ONLY text output is the signal marker at the end.\n\
             - The codebase is shared memory. Files you read/write are the artifacts.\n\n",
        );
        prompt.push_str(&agent.prompt);

        // Append signal emission instructions.
        if !agent.publish.is_empty() {
            prompt.push_str("\n\n--- Signals ---\n");
            prompt.push_str("Format: <<SIGNAL:name>> payload\n");
            prompt.push_str("Signals:\n");
            for entry in agent.publish.iter() {
                if let Some(desc) = &entry.description {
                    prompt.push_str(&format!("  - {} — {}\n", entry.name, desc));
                } else {
                    prompt.push_str(&format!("  - {}\n", entry.name));
                }
            }
            prompt.push_str(
                "Rules: Use ONLY listed signal names. Payload = short artifact pointer. \
                 Signal is the LAST thing you output. Nothing after it.\n",
            );
        }

        // Append query instructions if this agent can make queries.
        if !agent.query.is_empty() {
            prompt.push_str("\n\n--- Queries ---\n");
            prompt.push_str("Format: <<QUERY:key>> request payload\n");
            prompt.push_str("Queries:\n");
            for entry in agent.query.iter() {
                if let Some(desc) = &entry.description {
                    prompt.push_str(&format!("  - {} — {}\n", entry.name, desc));
                } else {
                    prompt.push_str(&format!("  - {}\n", entry.name));
                }
            }
            prompt.push_str(
                "Rules: Use ONLY listed keys. After emitting a query, STOP. \
                 You will be re-activated with the reply. Emit a SIGNAL to finish.\n",
            );
        }

        // Append queryable instructions if this agent serves queries.
        if !agent.queryable.is_empty() {
            prompt.push_str("\n\n--- Queryable ---\n");
            prompt.push_str(
                "You serve queries. When activated by a query (signal starts with 'query:'), \
                 your text output IS the reply sent back to the caller. \
                 Output ONLY the requested information — no preamble, no explanation, no signal markers. \
                 Do NOT write files as your reply mechanism; your stdout is the reply.\n",
            );
            prompt.push_str("Queries you serve:\n");
            for entry in agent.queryable.iter() {
                if let Some(desc) = &entry.description {
                    prompt.push_str(&format!("  - {} — {}\n", entry.name, desc));
                } else {
                    prompt.push_str(&format!("  - {}\n", entry.name));
                }
            }
        }

        // Reinforced output rules at the end (recency bias).
        if !agent.queryable.is_empty() && agent.publish.is_empty() {
            // Pure queryable agent — text output is the reply, no signals to emit.
            prompt.push_str(
                "\n--- Output Rules ---\n\
                 When activated by a query: your text output IS the reply. \
                 Output the requested information directly. No preamble. No commentary. \
                 Do NOT try to write files as your reply — just output the answer.\n",
            );
        } else if !agent.queryable.is_empty() {
            // Hybrid agent — queryable + publisher.
            prompt.push_str(
                "\n--- Output Rules ---\n\
                 When activated by a query (signal starts with 'query:'): your text output IS the reply. \
                 Output the requested information directly. No signal markers for queries.\n\
                 When activated by a regular signal: produce ZERO text output except the final signal marker. \
                 Do your work via tool calls, emit your signal, stop.\n",
            );
        } else {
            prompt.push_str(
                "\n--- Output Rules ---\n\
                 IMPORTANT: Produce ZERO text output except the final signal marker.\n\
                 No summaries. No explanations. No commentary. No status updates.\n\
                 Do your work via tool calls, emit your signal, stop.\n",
            );
        }

        // Append the incoming message context.
        Self::append_incoming_signal(&mut prompt, input, &agent.subscribe);

        prompt
    }

    /// Build a continuation prompt for a resumed session.
    /// Skips the system instructions (already in session history) — just sends
    /// the new incoming signal.
    fn build_continuation_prompt(
        input: &SignalMessage,
        subscribe: &SignalList,
    ) -> String {
        let mut prompt = String::from("Continue your task. Here is a new incoming signal.\n");
        Self::append_incoming_signal(&mut prompt, input, subscribe);
        prompt
    }

    /// Append the incoming signal context to a prompt.
    fn append_incoming_signal(
        prompt: &mut String,
        input: &SignalMessage,
        subscribe: &SignalList,
    ) {
        prompt.push_str("\n\n--- Incoming Signal ---\n");
        prompt.push_str(&format!("Signal: {}\n", input.signal));
        prompt.push_str(&format!("From: {}\n", input.source_agent_name));

        // If the subscribe list has a description for this signal, show it
        // so the agent knows what the payload represents.
        if let Some(desc) = subscribe.description(&input.signal) {
            prompt.push_str(&format!("Payload ({desc}): {}\n", input.payload));
        } else {
            prompt.push_str(&format!("Payload: {}\n", input.payload));
        }

        if !input.trace.is_empty() {
            prompt.push_str("\nSignal trace (history):\n");
            for entry in &input.trace {
                prompt.push_str(&format!(
                    "  {} -> {} ({})\n",
                    entry.agent_name, entry.signal, entry.timestamp
                ));
            }
        }
    }
}

#[async_trait]
impl AgentRuntime for ClaudeCodeRuntime {
    async fn run(
        &self,
        agent: &AgentDef,
        input: &SignalMessage,
        session_id: Option<&str>,
    ) -> Result<AgentOutput> {
        let prompt = match session_id {
            Some(_) => Self::build_continuation_prompt(input, &agent.subscribe),
            None => Self::build_prompt(agent, input),
        };

        info!(
            agent = %agent.name,
            signal = %input.signal,
            resumed = session_id.is_some(),
            "spawning claude code agent"
        );

        debug!(prompt = %prompt, "full prompt");

        let mut cmd = Command::new("claude");
        cmd.arg("--print");
        cmd.arg("--output-format").arg("json");

        // Resume an existing session if we have one.
        if let Some(sid) = session_id {
            cmd.arg("--resume").arg(sid);
        }

        cmd.arg(&prompt);

        // Apply model: agent config > default config.
        let model = agent
            .config
            .as_ref()
            .and_then(|c| c.get("model"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| self.default_model.clone());
        if let Some(model) = model {
            cmd.arg("--model").arg(model);
        }

        // Apply tool permissions based on permission_mode.
        match self.permission_mode.as_str() {
            "auto" => {
                if let Some(perms) = &agent.permissions {
                    if let Some(tools) = &perms.tools {
                        if !tools.allow.is_empty() {
                            let tool_names: Vec<String> = tools
                                .allow
                                .iter()
                                .map(|t| map_tool_name(t))
                                .collect();
                            cmd.arg("--allowedTools").arg(tool_names.join(","));
                        }
                    }
                }
            }
            "bypass" => {
                cmd.arg("--dangerously-skip-permissions");
            }
            _ => {}
        }

        debug!(command = ?cmd, "full claude command");

        // Kill the child process if this future is dropped (e.g. on Ctrl+C).
        cmd.kill_on_drop(true);

        let child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| YrrError::Runtime(format!("failed to spawn claude: {e}")))?;

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| YrrError::Runtime(format!("failed to wait for claude: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if stderr.is_empty() { &stdout } else { &stderr };
            return Err(YrrError::Runtime(format!(
                "claude exited with {}: {detail}",
                output.status
            )));
        }

        let raw_output = String::from_utf8_lossy(&output.stdout).to_string();

        // Parse the JSON response from Claude CLI.
        let (content, returned_session_id, usage) = match serde_json::from_str::<ClaudeJsonResponse>(&raw_output) {
            Ok(resp) => {
                let token_usage = resp.usage.map(|u| TokenUsage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cache_creation_input_tokens: u.cache_creation_input_tokens,
                    cache_read_input_tokens: u.cache_read_input_tokens,
                });

                if let Some(ref usage) = token_usage {
                    info!(
                        agent = %agent.name,
                        input_tokens = usage.input_tokens,
                        output_tokens = usage.output_tokens,
                        "token usage"
                    );
                }

                (resp.result, resp.session_id, token_usage)
            }
            Err(e) => {
                // Fallback: if JSON parsing fails, treat the raw output as text.
                warn!(
                    agent = %agent.name,
                    error = %e,
                    "failed to parse claude JSON response, falling back to raw output"
                );
                (raw_output, None, None)
            }
        };

        // Log a truncated version of the agent's response.
        let response_preview = content.trim();
        let preview_len = 500;
        if response_preview.len() > preview_len {
            let mut end = preview_len;
            while !response_preview.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            info!(
                agent = %agent.name,
                response = %&response_preview[..end],
                "agent response (truncated)"
            );
        } else {
            info!(
                agent = %agent.name,
                response = %response_preview,
                "agent response"
            );
        }

        // Parse emitted signals and queries from the output.
        let emitted_signals = parse_signals(&content);
        let emitted_queries = parse_queries(&content);

        Ok(AgentOutput {
            content,
            emitted_signals,
            emitted_queries,
            session_id: returned_session_id,
            usage,
        })
    }

    async fn health_check(&self) -> Result<()> {
        let output = Command::new("claude")
            .arg("--version")
            .output()
            .await
            .map_err(|e| YrrError::Runtime(format!("claude not found: {e}")))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(YrrError::Runtime(
                "claude --version failed".to_string(),
            ))
        }
    }

    fn name(&self) -> &str {
        "claude-code"
    }
}

/// Map agent YAML tool names to Claude Code CLI tool names.
fn map_tool_name(name: &str) -> String {
    match name {
        "read" => "Read".to_string(),
        "write" => "Write".to_string(),
        "edit" => "Edit".to_string(),
        "glob" => "Glob".to_string(),
        "grep" => "Grep".to_string(),
        "bash" => "Bash".to_string(),
        "agent" => "Agent".to_string(),
        "notebook_edit" => "NotebookEdit".to_string(),
        "web_fetch" => "WebFetch".to_string(),
        "web_search" => "WebSearch".to_string(),
        // Git operations map to Bash with specific commands.
        "git_diff" => "Bash(git diff*)".to_string(),
        "git_push" => "Bash(git push*)".to_string(),
        "git_commit" => "Bash(git commit*)".to_string(),
        "git_status" => "Bash(git status*)".to_string(),
        // Pass through anything else as-is (user knows the Claude Code tool name).
        other => other.to_string(),
    }
}
