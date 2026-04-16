use std::path::{Path, PathBuf};

use crate::error::{YrrError, Result};
use crate::schema::{
    AgentDef, AgentFile, AgentInlineDef, AgentRef, DispatchConfig, YrrFile, SwarmDef,
    SwarmFile,
};

/// Load a YAML file and parse it as either an agent or swarm.
pub fn load_file(path: &Path) -> Result<YrrFile> {
    let content = std::fs::read_to_string(path)?;
    let file: YrrFile = serde_yaml_ng::from_str(&content)?;
    Ok(file)
}

/// Load an agent definition from a YAML file.
pub fn load_agent(path: &Path) -> Result<AgentDef> {
    let content = std::fs::read_to_string(path)?;
    let file: AgentFile = serde_yaml_ng::from_str(&content)?;
    Ok(file.agent)
}

/// Load a swarm definition from a YAML file.
pub fn load_swarm(path: &Path) -> Result<SwarmDef> {
    let content = std::fs::read_to_string(path)?;
    let file: SwarmFile = serde_yaml_ng::from_str(&content)?;
    Ok(file.swarm)
}

/// A resolved swarm — all `use:` references loaded into full agent definitions,
/// with overrides applied.
#[derive(Debug, Clone)]
pub struct ResolvedSwarm {
    pub name: String,
    pub description: Option<String>,
    pub agents: Vec<ResolvedAgent>,
    pub entry: Vec<String>,
    pub done: Vec<String>,
    pub output: Vec<String>,
    /// Default seed message from the swarm definition.
    pub seed_message: Option<String>,
}

/// A fully resolved agent within a swarm — definition + orchestration.
#[derive(Debug, Clone)]
pub struct ResolvedAgent {
    /// The name of this agent within the swarm.
    pub swarm_key: String,
    /// The fully resolved agent definition (after overrides applied).
    pub def: AgentDef,
    /// Number of replicas to spawn.
    pub replicas: u32,
    /// Collect configuration: signal → count.
    pub collect: std::collections::HashMap<String, u32>,
    /// Lifecycle configuration.
    pub lifecycle: Option<crate::schema::Lifecycle>,
    /// Dispatch configuration.
    pub dispatch: Option<DispatchConfig>,
    /// Dynamic spawn configuration.
    pub spawn: Option<crate::schema::SpawnConfig>,
    /// Path to the source file this agent was loaded from (None for inline agents).
    pub source_path: Option<PathBuf>,
}

/// Resolve a swarm — load all `use:` references and apply overrides.
///
/// `base_dir` is the directory to resolve relative paths from (typically
/// the directory containing the swarm YAML file).
pub fn resolve_swarm(swarm: &SwarmDef, base_dir: &Path) -> Result<ResolvedSwarm> {
    let mut agents = Vec::new();

    for (key, agent_ref) in &swarm.agents {
        let mut resolved = resolve_agent_ref(key, agent_ref, base_dir)?;

        // Append swarm-level project prompt to every agent's prompt.
        if let Some(project_prompt) = &swarm.project_prompt {
            resolved.def.prompt.push_str("\n\n--- Project Prompt ---\n");
            resolved.def.prompt.push_str(project_prompt);
            resolved.def.prompt.push('\n');
        }

        agents.push(resolved);
    }

    Ok(ResolvedSwarm {
        name: swarm.name.clone(),
        description: swarm.description.clone(),
        agents,
        entry: swarm.entry.clone().into_vec(),
        done: swarm.done.clone().unwrap_or_default(),
        output: swarm.output.clone().unwrap_or_default(),
        seed_message: swarm.seed.clone(),
    })
}

fn resolve_agent_ref(key: &str, agent_ref: &AgentRef, base_dir: &Path) -> Result<ResolvedAgent> {
    match agent_ref {
        AgentRef::Use(use_ref) => {
            let agent_path = resolve_agent_path(&use_ref.r#use, base_dir)?;
            let mut def = load_agent(&agent_path)?;

            // Apply overrides
            if let Some(ovr) = &use_ref.r#override {
                apply_override(&mut def, ovr);
            }

            let lifecycle = use_ref.lifecycle.clone().or(def.lifecycle.clone());
            Ok(ResolvedAgent {
                swarm_key: key.to_string(),
                def,
                replicas: use_ref.replicas.unwrap_or(1),
                collect: use_ref.collect.clone().unwrap_or_default(),
                lifecycle,
                dispatch: use_ref.dispatch.clone(),
                spawn: use_ref.spawn.clone(),
                source_path: Some(agent_path),
            })
        }
        AgentRef::Inline(inline) => {
            let def = inline_to_agent_def(key, inline);
            Ok(ResolvedAgent {
                swarm_key: key.to_string(),
                def,
                replicas: inline.replicas.unwrap_or(1),
                collect: inline.collect.clone().unwrap_or_default(),
                lifecycle: inline.lifecycle.clone(),
                dispatch: inline.dispatch.clone(),
                spawn: inline.spawn.clone(),
                source_path: None,
            })
        }
    }
}

/// Resolve an agent path from a `use:` reference.
/// Tries: `{name}.yaml`, `{name}`, `agents/{name}.yaml`, `examples/agents/{name}.yaml`
fn resolve_agent_path(reference: &str, base_dir: &Path) -> Result<PathBuf> {
    let candidates = [
        base_dir.join(format!("{reference}.yaml")),
        base_dir.join(reference),
        base_dir.join(format!("agents/{reference}.yaml")),
        base_dir.join(format!("examples/agents/{reference}.yaml")),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    Err(YrrError::AgentNotFound(format!(
        "could not resolve agent '{reference}' from {}, tried: {}",
        base_dir.display(),
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn apply_override(def: &mut AgentDef, ovr: &crate::schema::AgentOverride) {
    if let Some(name) = &ovr.name {
        def.name = name.clone();
    }
    if let Some(desc) = &ovr.description {
        def.description = Some(desc.clone());
    }
    if let Some(runtime) = &ovr.runtime {
        def.runtime = runtime.clone();
    }
    if let Some(config) = &ovr.config {
        def.config = Some(config.clone());
    }
    if let Some(prompt) = &ovr.prompt {
        def.prompt = prompt.clone();
    }
    if let Some(subscribe) = &ovr.subscribe {
        def.subscribe = subscribe.clone();
    }
    if let Some(publish) = &ovr.publish {
        def.publish = publish.clone();
    }
    if let Some(queryable) = &ovr.queryable {
        def.queryable = queryable.clone();
    }
    if let Some(query) = &ovr.query {
        def.query = query.clone();
    }
    if let Some(context) = &ovr.context {
        def.context = Some(context.clone());
    }
    if let Some(permissions) = &ovr.permissions {
        def.permissions = Some(permissions.clone());
    }
    if let Some(steer) = &ovr.steer {
        def.steer = Some(steer.clone());
    }
}

fn inline_to_agent_def(key: &str, inline: &AgentInlineDef) -> AgentDef {
    AgentDef {
        name: key.to_string(),
        description: inline.description.clone(),
        runtime: inline.runtime.clone(),
        config: inline.config.clone(),
        prompt: inline.prompt.clone(),
        subscribe: inline.subscribe.clone(),
        publish: inline.publish.clone(),
        queryable: inline.queryable.clone(),
        query: inline.query.clone(),
        context: inline.context.clone(),
        permissions: inline.permissions.clone(),
        lifecycle: None,
        steer: inline.steer.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_yaml(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn load_and_resolve_swarm() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        write_yaml(
            base,
            "planner.yaml",
            r#"
agent:
  name: planner
  runtime: claude-code
  prompt: "Plan things"
  subscribe: [task_received]
  publish: [plan_ready]
"#,
        );

        write_yaml(
            base,
            "reviewer.yaml",
            r#"
agent:
  name: reviewer
  runtime: claude-code
  prompt: "Review things"
  subscribe: [code_ready]
  publish: [review_passed, review_failed]
"#,
        );

        let wf_path = write_yaml(
            base,
            "pipeline.yaml",
            r#"
swarm:
  name: test-pipeline
  agents:
    planner:
      use: planner
    reviewer:
      use: reviewer
      override:
        prompt: "Be strict"
      replicas: 2
      collect:
        code_ready: 2
  entry: task_received
  done: [review_passed]
"#,
        );

        let wf = load_swarm(&wf_path).unwrap();
        let resolved = resolve_swarm(&wf, base).unwrap();

        assert_eq!(resolved.name, "test-pipeline");
        assert_eq!(resolved.agents.len(), 2);

        let planner = resolved
            .agents
            .iter()
            .find(|a| a.swarm_key == "planner")
            .unwrap();
        assert_eq!(planner.def.prompt, "Plan things");
        assert_eq!(planner.replicas, 1);

        let reviewer = resolved
            .agents
            .iter()
            .find(|a| a.swarm_key == "reviewer")
            .unwrap();
        assert_eq!(reviewer.def.prompt, "Be strict"); // overridden
        assert_eq!(reviewer.replicas, 2);
        assert_eq!(reviewer.collect.get("code_ready"), Some(&2));
    }

    #[test]
    fn project_prompt_appended_to_all_agents() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        write_yaml(
            base,
            "planner.yaml",
            r#"
agent:
  name: planner
  runtime: claude-code
  prompt: "Plan things"
  subscribe: [task_received]
  publish: [plan_ready]
"#,
        );

        let wf_path = write_yaml(
            base,
            "pipeline.yaml",
            r#"
swarm:
  name: test-project-prompt
  project-prompt: "Always read the README before doing anything."
  agents:
    planner:
      use: planner
    inline-worker:
      runtime: claude-code
      prompt: "Do work"
      subscribe: [plan_ready]
      publish: [done]
  entry: task_received
  done: [done]
"#,
        );

        let wf = load_swarm(&wf_path).unwrap();
        let resolved = resolve_swarm(&wf, base).unwrap();

        // Both agents should have the project prompt appended.
        for agent in &resolved.agents {
            assert!(
                agent.def.prompt.contains("--- Project Prompt ---"),
                "agent '{}' missing project prompt section",
                agent.swarm_key,
            );
            assert!(
                agent.def.prompt.contains("Always read the README before doing anything."),
                "agent '{}' missing project prompt content",
                agent.swarm_key,
            );
        }

        // The original prompt content should still be there.
        let planner = resolved
            .agents
            .iter()
            .find(|a| a.swarm_key == "planner")
            .unwrap();
        assert!(planner.def.prompt.starts_with("Plan things"));

        let worker = resolved
            .agents
            .iter()
            .find(|a| a.swarm_key == "inline-worker")
            .unwrap();
        assert!(worker.def.prompt.starts_with("Do work"));
    }

    #[test]
    fn no_project_prompt_leaves_prompts_unchanged() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        write_yaml(
            base,
            "worker.yaml",
            r#"
agent:
  name: worker
  runtime: claude-code
  prompt: "Do work"
  subscribe: [start]
  publish: [done]
"#,
        );

        let wf_path = write_yaml(
            base,
            "pipeline.yaml",
            r#"
swarm:
  name: no-project-prompt
  agents:
    worker:
      use: worker
  entry: start
  done: [done]
"#,
        );

        let wf = load_swarm(&wf_path).unwrap();
        let resolved = resolve_swarm(&wf, base).unwrap();

        let worker = resolved
            .agents
            .iter()
            .find(|a| a.swarm_key == "worker")
            .unwrap();
        assert_eq!(worker.def.prompt, "Do work");
    }

    #[test]
    fn lifecycle_from_agent_file_used_when_swarm_omits_it() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        write_yaml(
            base,
            "coordinator.yaml",
            r#"
agent:
  name: coordinator
  runtime: claude-code
  prompt: "Coordinate"
  subscribe: [scaffold_ready, sub_task_ready]
  publish: [sub_task, coding_complete]
  lifecycle:
    mode: persistent
    max_activations: 30
"#,
        );

        let wf_path = write_yaml(
            base,
            "pipeline.yaml",
            r#"
swarm:
  name: test-lifecycle
  agents:
    coordinator:
      use: coordinator
  entry: scaffold_ready
  done: [coding_complete]
"#,
        );

        let wf = load_swarm(&wf_path).unwrap();
        let resolved = resolve_swarm(&wf, base).unwrap();

        let coord = resolved
            .agents
            .iter()
            .find(|a| a.swarm_key == "coordinator")
            .unwrap();

        let lc = coord
            .lifecycle
            .as_ref()
            .expect("lifecycle from agent file should be used when swarm omits it");
        assert_eq!(
            lc.mode,
            crate::schema::LifecycleMode::Persistent,
            "mode should be persistent from agent file"
        );
        assert_eq!(lc.max_activations, Some(30));
    }

    #[test]
    fn swarm_lifecycle_overrides_agent_file_lifecycle() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        write_yaml(
            base,
            "worker.yaml",
            r#"
agent:
  name: worker
  runtime: claude-code
  prompt: "Work"
  subscribe: [start]
  publish: [done]
  lifecycle:
    mode: persistent
    max_activations: 99
"#,
        );

        let wf_path = write_yaml(
            base,
            "pipeline.yaml",
            r#"
swarm:
  name: test-override
  agents:
    worker:
      use: worker
      lifecycle:
        mode: persistent
        max_activations: 5
  entry: start
  done: [done]
"#,
        );

        let wf = load_swarm(&wf_path).unwrap();
        let resolved = resolve_swarm(&wf, base).unwrap();

        let worker = resolved
            .agents
            .iter()
            .find(|a| a.swarm_key == "worker")
            .unwrap();

        let lc = worker.lifecycle.as_ref().unwrap();
        assert_eq!(
            lc.max_activations,
            Some(5),
            "swarm-level lifecycle should override agent file lifecycle"
        );
    }
}
