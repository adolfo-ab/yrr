use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── SignalList ─────────────────────────────────────────────────────────────────

/// A single signal/query declaration — name plus optional payload description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEntry {
    pub name: String,
    /// Describes what the signal payload contains (e.g., "filepath to the plan").
    /// Used to instruct agents about what to produce or expect.
    pub description: Option<String>,
}

/// A list of signal or query declarations.
///
/// Accepts two YAML formats:
///
/// ```yaml
/// # Bare list (backward compatible)
/// subscribe:
///   - task_received
///   - review_failed
///
/// # Map with payload descriptions
/// subscribe:
///   task_received: "the task description to break down"
///   review_failed: "review feedback with issues found"
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct SignalList(Vec<SignalEntry>);

impl SignalList {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if a signal name is in the list.
    pub fn contains(&self, name: &str) -> bool {
        self.0.iter().any(|e| e.name == name)
    }

    /// Iterate over signal names only.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|e| e.name.as_str())
    }

    /// Get the payload description for a signal, if one was provided.
    pub fn description(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|e| e.name == name)
            .and_then(|e| e.description.as_deref())
    }

    /// Iterate over all entries (name + description).
    pub fn iter(&self) -> impl Iterator<Item = &SignalEntry> {
        self.0.iter()
    }

    /// Get the first entry, if any.
    pub fn first(&self) -> Option<&SignalEntry> {
        self.0.first()
    }
}

impl Default for SignalList {
    fn default() -> Self {
        SignalList(Vec::new())
    }
}

/// Allows `names.iter().map(|s| s.to_string()).collect::<SignalList>()`.
impl FromIterator<String> for SignalList {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        SignalList(
            iter.into_iter()
                .map(|name| SignalEntry {
                    name,
                    description: None,
                })
                .collect(),
        )
    }
}

impl<'de> Deserialize<'de> for SignalList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SignalListVisitor;

        impl<'de> serde::de::Visitor<'de> for SignalListVisitor {
            type Value = SignalList;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(
                    "a list of signal names or a map of signal name → payload description",
                )
            }

            /// Bare list: `[task_received, review_failed]`
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some(name) = seq.next_element::<String>()? {
                    entries.push(SignalEntry {
                        name,
                        description: None,
                    });
                }
                Ok(SignalList(entries))
            }

            /// Map: `{task_received: "the task prompt", review_failed: "feedback"}`
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some((name, description)) = map.next_entry::<String, String>()? {
                    entries.push(SignalEntry {
                        name,
                        description: Some(description),
                    });
                }
                Ok(SignalList(entries))
            }
        }

        deserializer.deserialize_any(SignalListVisitor)
    }
}

/// Top-level YAML file — differentiated by which key is present.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum YrrFile {
    Agent(AgentFile),
    Swarm(SwarmFile),
}

impl YrrFile {
    /// Returns the name of the definition.
    pub fn name(&self) -> &str {
        match self {
            YrrFile::Agent(f) => &f.agent.name,
            YrrFile::Swarm(f) => &f.swarm.name,
        }
    }
}

// ─── Agent ───────────────────────────────────────────────────────────────────

/// A standalone agent definition file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentFile {
    pub agent: AgentDef,
}

/// Human steering configuration — allows mid-execution human guidance.
///
/// Accepts two YAML formats:
///
/// ```yaml
/// # Simple boolean
/// steer: true
///
/// # With description (shown as prompt hint in TUI)
/// steer: "provide architectural guidance when trade-offs arise"
/// ```
#[derive(Debug, Clone, Serialize)]
pub enum Steer {
    Enabled,
    Described(String),
}

impl Steer {
    pub fn description(&self) -> Option<&str> {
        match self {
            Steer::Enabled => None,
            Steer::Described(d) => Some(d),
        }
    }
}

impl<'de> Deserialize<'de> for Steer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SteerVisitor;

        impl<'de> serde::de::Visitor<'de> for SteerVisitor {
            type Value = Steer;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("true or a description string")
            }

            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                if v {
                    Ok(Steer::Enabled)
                } else {
                    Err(E::custom("steer: false is not valid, omit the field instead"))
                }
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(Steer::Described(v.to_string()))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(Steer::Described(v))
            }
        }

        deserializer.deserialize_any(SteerVisitor)
    }
}

/// An agent — the fundamental building block.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub runtime: String,
    #[serde(default)]
    pub config: Option<HashMap<String, serde_json::Value>>,
    pub prompt: String,
    pub subscribe: SignalList,
    pub publish: SignalList,
    #[serde(default)]
    pub queryable: SignalList,
    #[serde(default)]
    pub query: SignalList,
    #[serde(default)]
    pub context: Option<ContextConfig>,
    #[serde(default)]
    pub permissions: Option<Permissions>,
    #[serde(default)]
    pub lifecycle: Option<Lifecycle>,
    #[serde(default)]
    pub steer: Option<Steer>,
}

/// Context window configuration for an agent.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContextConfig {
    /// Maximum context size in tokens.
    pub max_tokens: u64,
    /// Action to take when the context limit is reached.
    pub on_limit: ContextLimitAction,
}

/// What to do when an agent's context window fills up.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContextLimitAction {
    /// Compress/summarize the context and continue.
    Compress,
    /// Restart the agent with a fresh context.
    Restart,
    /// Kill the agent entirely.
    Kill,
}

/// Agent permissions — ACLs for tools, paths, and network.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Permissions {
    #[serde(default)]
    pub tools: Option<ToolPermissions>,
    #[serde(default)]
    pub paths: Option<PathPermissions>,
    #[serde(default)]
    pub network: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ToolPermissions {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PathPermissions {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

// ─── Swarm ────────────────────────────────────────────────────────────────

/// A standalone swarm definition file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SwarmFile {
    pub swarm: SwarmDef,
}

/// A swarm — composes agents via shared signal namespace.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SwarmDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "project-prompt")]
    pub project_prompt: Option<String>,
    #[serde(default)]
    pub agents: HashMap<String, AgentRef>,
    #[serde(default)]
    pub include: Option<Vec<SwarmInclude>>,
    pub entry: StringOrVec,
    #[serde(default)]
    pub done: Option<Vec<String>>,
    #[serde(default)]
    pub output: Option<Vec<String>>,
    #[serde(default)]
    pub defaults: Option<SwarmDefaults>,
    #[serde(default)]
    pub cron: Option<CronConfig>,
    #[serde(default)]
    pub seed: Option<String>,
}

/// Reference to an agent — either `use: path` or inline definition.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AgentRef {
    /// Reference to an external agent file: `use: planner`
    Use(AgentUseRef),
    /// Inline agent definition within the swarm.
    Inline(AgentInlineDef),
}

/// A `use:` reference to an external agent, with optional orchestration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentUseRef {
    pub r#use: String,
    #[serde(default)]
    pub r#override: Option<AgentOverride>,
    #[serde(default)]
    pub replicas: Option<u32>,
    #[serde(default)]
    pub collect: Option<HashMap<String, u32>>,
    #[serde(default)]
    pub lifecycle: Option<Lifecycle>,
    #[serde(default)]
    pub dispatch: Option<DispatchConfig>,
    #[serde(default)]
    pub spawn: Option<SpawnConfig>,
}

/// An inline agent definition with optional orchestration fields.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentInlineDef {
    pub runtime: String,
    pub prompt: String,
    pub subscribe: SignalList,
    pub publish: SignalList,
    #[serde(default)]
    pub queryable: SignalList,
    #[serde(default)]
    pub query: SignalList,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub config: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub context: Option<ContextConfig>,
    #[serde(default)]
    pub permissions: Option<Permissions>,
    #[serde(default)]
    pub steer: Option<Steer>,
    // Orchestration (swarm-level)
    #[serde(default)]
    pub replicas: Option<u32>,
    #[serde(default)]
    pub collect: Option<HashMap<String, u32>>,
    #[serde(default)]
    pub lifecycle: Option<Lifecycle>,
    #[serde(default)]
    pub dispatch: Option<DispatchConfig>,
    #[serde(default)]
    pub spawn: Option<SpawnConfig>,
}

/// Override block — any agent field can be overridden.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AgentOverride {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub config: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub subscribe: Option<SignalList>,
    #[serde(default)]
    pub publish: Option<SignalList>,
    #[serde(default)]
    pub queryable: Option<SignalList>,
    #[serde(default)]
    pub query: Option<SignalList>,
    #[serde(default)]
    pub context: Option<ContextConfig>,
    #[serde(default)]
    pub permissions: Option<Permissions>,
    #[serde(default)]
    pub steer: Option<Steer>,
}

// ─── Dispatch ───────────────────────────────────────────────────────────────

/// Dispatch configuration — controls how signals are routed to replicas.
///
/// Uniform: all subscribed signals use the same dispatch rule.
/// PerSignal: each signal can have its own dispatch rule (signals not listed
/// default to broadcast).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum DispatchConfig {
    /// Same dispatch rule for all subscribed signals.
    Uniform(DispatchRule),
    /// Per-signal dispatch rules. Signals not listed default to broadcast.
    PerSignal(HashMap<String, DispatchRule>),
}

impl DispatchConfig {
    /// Get the dispatch rule for a specific signal.
    /// Returns `None` if the signal should use broadcast (default).
    pub fn rule_for(&self, signal: &str) -> Option<&DispatchRule> {
        match self {
            DispatchConfig::Uniform(rule) => {
                if rule.mode == DispatchMode::Broadcast {
                    None
                } else {
                    Some(rule)
                }
            }
            DispatchConfig::PerSignal(map) => {
                map.get(signal).and_then(|rule| {
                    if rule.mode == DispatchMode::Broadcast {
                        None
                    } else {
                        Some(rule)
                    }
                })
            }
        }
    }

    /// Returns the set of signal names that use pool dispatch.
    pub fn pooled_signals(&self, subscribe: &SignalList) -> Vec<String> {
        subscribe
            .names()
            .filter(|s| self.rule_for(s).is_some())
            .map(|s| s.to_string())
            .collect()
    }
}

/// A single dispatch rule — mode + pool settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DispatchRule {
    pub mode: DispatchMode,
    /// How many replicas activate per incoming signal (pool mode only).
    /// Defaults to 1.
    #[serde(default = "default_concurrency")]
    pub concurrency: u32,
    /// How to pick which replicas activate (pool mode only).
    /// Defaults to round-robin.
    #[serde(default)]
    pub strategy: DispatchStrategy,
}

fn default_concurrency() -> u32 {
    1
}

/// Dispatch mode — broadcast (all replicas) or pool (N replicas).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DispatchMode {
    /// All replicas receive every signal (default, current behavior).
    Broadcast,
    /// Only N replicas activate per signal (worker pool).
    Pool,
}

/// Strategy for picking which replicas handle a pooled signal.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DispatchStrategy {
    /// Cycle through replicas in order.
    #[default]
    RoundRobin,
    /// Pick replicas at random.
    Random,
    /// Pick the replicas that have been idle the longest.
    LeastBusy,
}

/// Lifecycle configuration — when agents live and die.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Lifecycle {
    #[serde(default = "default_lifecycle_mode")]
    pub mode: LifecycleMode,
    #[serde(default)]
    pub max_activations: Option<u32>,
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub idle_timeout: Option<String>,
    #[serde(default)]
    pub max_uptime: Option<String>,
    #[serde(default)]
    pub die_on: Option<Vec<String>>,
}

fn default_lifecycle_mode() -> LifecycleMode {
    LifecycleMode::Ephemeral
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleMode {
    Ephemeral,
    Persistent,
}

/// Dynamic spawn configuration — each occurrence of a signal creates a new agent instance.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SpawnConfig {
    /// Signal that triggers creation of a new instance.
    pub on: String,
    /// Maximum number of concurrent instances.
    pub max: u32,
}

/// Including a sub-swarm with signal remapping.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SwarmInclude {
    pub r#use: String,
    #[serde(default)]
    pub signals: Option<HashMap<String, String>>,
}

/// Swarm-wide defaults.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SwarmDefaults {
    #[serde(default)]
    pub permissions: Option<Permissions>,
}

/// Cron configuration — single schedule or multiple.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum CronConfig {
    Single(String),
    Multiple(Vec<CronEntry>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CronEntry {
    pub schedule: String,
    #[serde(default)]
    pub seed: Option<String>,
}

/// A string or a list of strings (for `entry` and similar fields).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StringOrVec {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrVec {
    pub fn as_vec(&self) -> Vec<&str> {
        match self {
            StringOrVec::Single(s) => vec![s.as_str()],
            StringOrVec::Multiple(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }

    pub fn into_vec(self) -> Vec<String> {
        match self {
            StringOrVec::Single(s) => vec![s],
            StringOrVec::Multiple(v) => v,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_with_lifecycle() {
        let yaml = r#"
agent:
  name: coordinator
  runtime: claude-code
  prompt: "Coordinate work"
  subscribe: [scaffold_ready, sub_task_ready]
  publish: [sub_task, coding_complete]
  lifecycle:
    mode: persistent
    max_activations: 30
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        let lc = file
            .agent
            .lifecycle
            .as_ref()
            .expect("lifecycle should be parsed from agent file");
        assert_eq!(lc.mode, LifecycleMode::Persistent);
        assert_eq!(lc.max_activations, Some(30));
        assert!(lc.die_on.is_none());
    }

    #[test]
    fn parse_agent_file() {
        let yaml = r#"
agent:
  name: planner
  description: "Breaks down tasks into plans"
  runtime: claude-code
  config:
    model: opus
    max_turns: 3
  prompt: |
    You are a planning agent.
  subscribe:
    - task_received
    - review_failed
  publish:
    - plan_ready
  permissions:
    tools:
      allow: [read, grep, glob]
      deny: [bash]
    paths:
      allow: ["src/**"]
      deny: [".env"]
    network: false
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(file.agent.name, "planner");
        let sub_names: Vec<&str> = file.agent.subscribe.names().collect();
        assert_eq!(sub_names, vec!["task_received", "review_failed"]);
        let pub_names: Vec<&str> = file.agent.publish.names().collect();
        assert_eq!(pub_names, vec!["plan_ready"]);
        let perms = file.agent.permissions.unwrap();
        assert_eq!(perms.network, Some(false));
        let tools = perms.tools.unwrap();
        assert_eq!(tools.allow, vec!["read", "grep", "glob"]);
        assert_eq!(tools.deny, vec!["bash"]);
    }

    #[test]
    fn parse_swarm_file() {
        let yaml = r#"
swarm:
  name: dev-pipeline
  description: "Plan, implement, review with feedback loop"
  agents:
    planner:
      use: planner
    implementer:
      use: implementer
      replicas: 3
      lifecycle:
        mode: persistent
        max_activations: 5
        die_on:
          - review_passed
    reviewer:
      use: reviewer
      collect:
        code_ready: 3
  entry: task_received
  done:
    - review_passed
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let wf = &file.swarm;
        assert_eq!(wf.name, "dev-pipeline");
        assert_eq!(wf.entry.as_vec(), vec!["task_received"]);
        assert_eq!(wf.done, Some(vec!["review_passed".into()]));
        assert_eq!(wf.agents.len(), 3);

        // Check implementer orchestration
        let imp = wf.agents.get("implementer").unwrap();
        match imp {
            AgentRef::Use(u) => {
                assert_eq!(u.r#use, "implementer");
                assert_eq!(u.replicas, Some(3));
                let lc = u.lifecycle.as_ref().unwrap();
                assert_eq!(lc.mode, LifecycleMode::Persistent);
                assert_eq!(lc.max_activations, Some(5));
                assert_eq!(lc.die_on, Some(vec!["review_passed".into()]));
            }
            _ => panic!("expected AgentRef::Use"),
        }
    }

    #[test]
    fn parse_inline_agent() {
        let yaml = r#"
swarm:
  name: quick-fix
  agents:
    fixer:
      runtime: claude-code
      prompt: "Fix the bug"
      subscribe: [bug_report]
      publish: [fix_ready]
  entry: bug_report
  done: [fix_ready]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let fixer = file.swarm.agents.get("fixer").unwrap();
        match fixer {
            AgentRef::Inline(def) => {
                assert_eq!(def.runtime, "claude-code");
                assert!(def.subscribe.contains("bug_report"));
                assert!(def.publish.contains("fix_ready"));
            }
            _ => panic!("expected inline agent"),
        }
    }

    #[test]
    fn parse_swarm_with_override() {
        let yaml = r#"
swarm:
  name: strict
  agents:
    reviewer:
      use: reviewer
      override:
        prompt: "Be strict"
        permissions:
          tools:
            deny: [bash, write]
        subscribe: [code_ready, hotfix_ready]
  entry: code_ready
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let rev = file.swarm.agents.get("reviewer").unwrap();
        match rev {
            AgentRef::Use(u) => {
                let ovr = u.r#override.as_ref().unwrap();
                assert_eq!(ovr.prompt, Some("Be strict".into()));
                let ovr_sub = ovr.subscribe.as_ref().unwrap();
                assert!(ovr_sub.contains("code_ready"));
                assert!(ovr_sub.contains("hotfix_ready"));
            }
            _ => panic!("expected AgentRef::Use"),
        }
    }

    #[test]
    fn parse_cron_swarm() {
        let yaml = r#"
swarm:
  name: nightly
  cron: "0 2 * * *"
  seed: "Run audit"
  agents:
    auditor:
      runtime: script
      prompt: "audit"
      subscribe: [audit_requested]
      publish: [report_ready]
  entry: audit_requested
  done: [report_ready]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        match &file.swarm.cron {
            Some(CronConfig::Single(s)) => assert_eq!(s, "0 2 * * *"),
            _ => panic!("expected single cron"),
        }
        assert_eq!(file.swarm.seed, Some("Run audit".into()));
    }

    #[test]
    fn parse_agent_with_context() {
        let yaml = r#"
agent:
  name: worker
  runtime: claude-code
  prompt: "Do work"
  subscribe: [start]
  publish: [done]
  context:
    max_tokens: 100000
    on_limit: compress
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        let ctx = file.agent.context.unwrap();
        assert_eq!(ctx.max_tokens, 100_000);
        assert_eq!(ctx.on_limit, ContextLimitAction::Compress);
    }

    #[test]
    fn parse_agent_context_actions() {
        for (action_str, expected) in [
            ("compress", ContextLimitAction::Compress),
            ("restart", ContextLimitAction::Restart),
            ("kill", ContextLimitAction::Kill),
        ] {
            let yaml = format!(
                r#"
agent:
  name: test
  runtime: claude-code
  prompt: "test"
  subscribe: [s]
  publish: [p]
  context:
    max_tokens: 50000
    on_limit: {action_str}
"#
            );
            let file: AgentFile = serde_yaml_ng::from_str(&yaml).unwrap();
            let ctx = file.agent.context.unwrap();
            assert_eq!(ctx.on_limit, expected, "failed for action: {action_str}");
        }
    }

    #[test]
    fn parse_agent_without_context() {
        let yaml = r#"
agent:
  name: simple
  runtime: claude-code
  prompt: "Simple agent"
  subscribe: [start]
  publish: [done]
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(file.agent.context.is_none());
    }

    #[test]
    fn parse_uniform_dispatch() {
        let yaml = r#"
swarm:
  name: pool-test
  agents:
    coder:
      use: coder
      replicas: 5
      dispatch:
        mode: pool
        concurrency: 2
        strategy: round-robin
  entry: task
  done: [done]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let coder = file.swarm.agents.get("coder").unwrap();
        match coder {
            AgentRef::Use(u) => {
                assert_eq!(u.replicas, Some(5));
                let dispatch = u.dispatch.as_ref().unwrap();
                match dispatch {
                    DispatchConfig::Uniform(rule) => {
                        assert_eq!(rule.mode, DispatchMode::Pool);
                        assert_eq!(rule.concurrency, 2);
                        assert_eq!(rule.strategy, DispatchStrategy::RoundRobin);
                    }
                    _ => panic!("expected uniform dispatch"),
                }
            }
            _ => panic!("expected AgentRef::Use"),
        }
    }

    #[test]
    fn parse_per_signal_dispatch() {
        let yaml = r#"
swarm:
  name: mixed-test
  agents:
    coder:
      use: coder
      replicas: 5
      dispatch:
        plan-ready:
          mode: pool
          concurrency: 2
        urgent-fix:
          mode: broadcast
  entry: task
  done: [done]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let coder = file.swarm.agents.get("coder").unwrap();
        match coder {
            AgentRef::Use(u) => {
                let dispatch = u.dispatch.as_ref().unwrap();
                match dispatch {
                    DispatchConfig::PerSignal(map) => {
                        assert_eq!(map.len(), 2);
                        let plan = map.get("plan-ready").unwrap();
                        assert_eq!(plan.mode, DispatchMode::Pool);
                        assert_eq!(plan.concurrency, 2);
                        let urgent = map.get("urgent-fix").unwrap();
                        assert_eq!(urgent.mode, DispatchMode::Broadcast);
                    }
                    _ => panic!("expected per-signal dispatch"),
                }
            }
            _ => panic!("expected AgentRef::Use"),
        }
    }

    #[test]
    fn parse_dispatch_defaults() {
        let yaml = r#"
swarm:
  name: defaults-test
  agents:
    coder:
      use: coder
      replicas: 3
      dispatch:
        mode: pool
  entry: task
  done: [done]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let coder = file.swarm.agents.get("coder").unwrap();
        match coder {
            AgentRef::Use(u) => {
                let dispatch = u.dispatch.as_ref().unwrap();
                match dispatch {
                    DispatchConfig::Uniform(rule) => {
                        assert_eq!(rule.mode, DispatchMode::Pool);
                        assert_eq!(rule.concurrency, 1); // default
                        assert_eq!(rule.strategy, DispatchStrategy::RoundRobin); // default
                    }
                    _ => panic!("expected uniform dispatch"),
                }
            }
            _ => panic!("expected AgentRef::Use"),
        }
    }

    #[test]
    fn parse_dispatch_least_busy() {
        let yaml = r#"
swarm:
  name: lb-test
  agents:
    coder:
      use: coder
      replicas: 5
      dispatch:
        mode: pool
        concurrency: 3
        strategy: least-busy
  entry: task
  done: [done]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let coder = file.swarm.agents.get("coder").unwrap();
        match coder {
            AgentRef::Use(u) => {
                let dispatch = u.dispatch.as_ref().unwrap();
                match dispatch {
                    DispatchConfig::Uniform(rule) => {
                        assert_eq!(rule.strategy, DispatchStrategy::LeastBusy);
                    }
                    _ => panic!("expected uniform dispatch"),
                }
            }
            _ => panic!("expected AgentRef::Use"),
        }
    }

    #[test]
    fn dispatch_rule_for_uniform() {
        let config = DispatchConfig::Uniform(DispatchRule {
            mode: DispatchMode::Pool,
            concurrency: 2,
            strategy: DispatchStrategy::RoundRobin,
        });
        assert!(config.rule_for("any_signal").is_some());
        assert_eq!(config.rule_for("any_signal").unwrap().concurrency, 2);
    }

    #[test]
    fn dispatch_rule_for_per_signal() {
        let mut map = HashMap::new();
        map.insert(
            "plan-ready".to_string(),
            DispatchRule {
                mode: DispatchMode::Pool,
                concurrency: 2,
                strategy: DispatchStrategy::RoundRobin,
            },
        );
        map.insert(
            "urgent".to_string(),
            DispatchRule {
                mode: DispatchMode::Broadcast,
                concurrency: 1,
                strategy: DispatchStrategy::RoundRobin,
            },
        );
        let config = DispatchConfig::PerSignal(map);

        // Pool signal returns the rule.
        assert!(config.rule_for("plan-ready").is_some());
        // Broadcast signal returns None.
        assert!(config.rule_for("urgent").is_none());
        // Unknown signal returns None (default broadcast).
        assert!(config.rule_for("unknown").is_none());
    }

    #[test]
    fn dispatch_pooled_signals() {
        let mut map = HashMap::new();
        map.insert(
            "plan-ready".to_string(),
            DispatchRule {
                mode: DispatchMode::Pool,
                concurrency: 2,
                strategy: DispatchStrategy::RoundRobin,
            },
        );
        map.insert(
            "urgent".to_string(),
            DispatchRule {
                mode: DispatchMode::Broadcast,
                concurrency: 1,
                strategy: DispatchStrategy::RoundRobin,
            },
        );
        let config = DispatchConfig::PerSignal(map);

        let subscribe: SignalList = vec![
            "plan-ready".to_string(),
            "urgent".to_string(),
            "other".to_string(),
        ]
        .into_iter()
        .collect();
        let pooled = config.pooled_signals(&subscribe);
        assert_eq!(pooled, vec!["plan-ready".to_string()]);
    }

    #[test]
    fn no_dispatch_field() {
        let yaml = r#"
swarm:
  name: no-dispatch
  agents:
    coder:
      use: coder
      replicas: 3
  entry: task
  done: [done]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let coder = file.swarm.agents.get("coder").unwrap();
        match coder {
            AgentRef::Use(u) => {
                assert!(u.dispatch.is_none());
            }
            _ => panic!("expected AgentRef::Use"),
        }
    }

    #[test]
    fn parse_swarm_with_project_prompt() {
        let yaml = r#"
swarm:
  name: guided
  project-prompt: |
    Always read the README and docs before doing anything.
    Be concise in your outputs.
  agents:
    worker:
      runtime: claude-code
      prompt: "Do work"
      subscribe: [start]
      publish: [done]
  entry: start
  done: [done]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let project_prompt = file.swarm.project_prompt.unwrap();
        assert!(project_prompt.contains("Always read the README"));
        assert!(project_prompt.contains("Be concise"));
    }

    #[test]
    fn parse_swarm_without_project_prompt() {
        let yaml = r#"
swarm:
  name: plain
  agents:
    worker:
      runtime: claude-code
      prompt: "Do work"
      subscribe: [start]
      publish: [done]
  entry: start
  done: [done]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(file.swarm.project_prompt.is_none());
    }

    // ─── SignalList tests ───────────────────────────────────────────────────

    #[test]
    fn signal_list_from_bare_list() {
        let yaml = r#"
agent:
  name: test
  runtime: claude-code
  prompt: "test"
  subscribe:
    - task_received
    - review_failed
  publish:
    - plan_ready
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(file.agent.subscribe.len(), 2);
        assert!(file.agent.subscribe.contains("task_received"));
        assert!(file.agent.subscribe.contains("review_failed"));
        assert!(file.agent.subscribe.description("task_received").is_none());
    }

    #[test]
    fn signal_list_from_described_map() {
        let yaml = r#"
agent:
  name: test
  runtime: claude-code
  prompt: "test"
  subscribe:
    task_received: "the task description to break down"
    review_failed: "review feedback with issues found"
  publish:
    plan_ready: "filepath to the completed plan"
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(file.agent.subscribe.len(), 2);
        assert!(file.agent.subscribe.contains("task_received"));
        assert_eq!(
            file.agent.subscribe.description("task_received"),
            Some("the task description to break down")
        );
        assert_eq!(
            file.agent.subscribe.description("review_failed"),
            Some("review feedback with issues found")
        );
        assert_eq!(
            file.agent.publish.description("plan_ready"),
            Some("filepath to the completed plan")
        );
    }

    #[test]
    fn signal_list_mixed_formats_in_agent() {
        let yaml = r#"
agent:
  name: test
  runtime: claude-code
  prompt: "test"
  subscribe: [task_received]
  publish:
    plan_ready: "filepath to plan"
    stuck: "what blocked you"
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        // subscribe is bare list
        assert!(file.agent.subscribe.description("task_received").is_none());
        // publish is described map
        assert_eq!(
            file.agent.publish.description("plan_ready"),
            Some("filepath to plan")
        );
    }

    #[test]
    fn signal_list_collect_from_strings() {
        let list: SignalList = vec!["a".to_string(), "b".to_string()]
            .into_iter()
            .collect();
        assert_eq!(list.len(), 2);
        assert!(list.contains("a"));
        assert!(list.contains("b"));
        assert!(!list.contains("c"));
        assert!(list.description("a").is_none());
    }

    #[test]
    fn signal_list_names_iterator() {
        let yaml = r#"
agent:
  name: test
  runtime: claude-code
  prompt: "test"
  subscribe:
    task_received: "task prompt"
    review_failed: "feedback"
  publish: [done]
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        let names: Vec<&str> = file.agent.subscribe.names().collect();
        assert_eq!(names, vec!["task_received", "review_failed"]);
    }

    #[test]
    fn parse_inline_agent_with_dispatch() {
        let yaml = r#"
swarm:
  name: inline-dispatch
  agents:
    coder:
      runtime: claude-code
      prompt: "Code it"
      subscribe: [plan-ready, urgent]
      publish: [code-ready]
      replicas: 4
      dispatch:
        plan-ready:
          mode: pool
          concurrency: 2
        urgent:
          mode: broadcast
  entry: plan-ready
  done: [code-ready]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let coder = file.swarm.agents.get("coder").unwrap();
        match coder {
            AgentRef::Inline(def) => {
                assert_eq!(def.replicas, Some(4));
                let dispatch = def.dispatch.as_ref().unwrap();
                match dispatch {
                    DispatchConfig::PerSignal(map) => {
                        assert_eq!(map.len(), 2);
                        assert_eq!(map.get("plan-ready").unwrap().mode, DispatchMode::Pool);
                        assert_eq!(map.get("urgent").unwrap().mode, DispatchMode::Broadcast);
                    }
                    _ => panic!("expected per-signal dispatch"),
                }
            }
            _ => panic!("expected inline agent"),
        }
    }

    #[test]
    fn parse_agent_steer_bool() {
        let yaml = r#"
agent:
  name: planner
  runtime: claude-code
  prompt: "Plan"
  subscribe: [task]
  publish: [plan]
  steer: true
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        let steer = file.agent.steer.unwrap();
        assert!(matches!(steer, Steer::Enabled));
        assert!(steer.description().is_none());
    }

    #[test]
    fn parse_agent_steer_description() {
        let yaml = r#"
agent:
  name: planner
  runtime: claude-code
  prompt: "Plan"
  subscribe: [task]
  publish: [plan]
  steer: "provide guidance on architecture"
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        let steer = file.agent.steer.unwrap();
        assert_eq!(steer.description(), Some("provide guidance on architecture"));
    }

    #[test]
    fn parse_agent_without_steer() {
        let yaml = r#"
agent:
  name: worker
  runtime: claude-code
  prompt: "Work"
  subscribe: [start]
  publish: [done]
"#;
        let file: AgentFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert!(file.agent.steer.is_none());
    }

    #[test]
    fn parse_inline_agent_with_steer() {
        let yaml = r#"
swarm:
  name: steered
  agents:
    planner:
      runtime: claude-code
      prompt: "Plan"
      subscribe: [task]
      publish: [plan]
      steer: "guide the planning"
  entry: task
  done: [plan]
"#;
        let file: SwarmFile = serde_yaml_ng::from_str(yaml).unwrap();
        let planner = file.swarm.agents.get("planner").unwrap();
        match planner {
            AgentRef::Inline(def) => {
                let steer = def.steer.as_ref().unwrap();
                assert_eq!(steer.description(), Some("guide the planning"));
            }
            _ => panic!("expected inline agent"),
        }
    }
}
