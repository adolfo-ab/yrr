use std::collections::{HashMap, HashSet};

use crate::loader::ResolvedSwarm;

/// Validation warning — not an error, since decoupled systems can intentionally
/// have dead signals or orphan triggers.
#[derive(Debug, Clone)]
pub struct ValidationWarning {
    pub kind: WarningKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningKind {
    DeadSignal,
    OrphanTrigger,
    UnreachableAgent,
    CollectMismatch,
    MissingDone,
    UnmatchedQuery,
    UnservedQueryable,
    DispatchConcurrencyExceedsReplicas,
    DispatchSignalNotSubscribed,
    DispatchRequiresReplicas,
    SpawnSignalNotPublished,
}

impl std::fmt::Display for ValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = match self.kind {
            WarningKind::DeadSignal => "dead signal",
            WarningKind::OrphanTrigger => "orphan trigger",
            WarningKind::UnreachableAgent => "unreachable agent",
            WarningKind::CollectMismatch => "collect mismatch",
            WarningKind::MissingDone => "missing done",
            WarningKind::UnmatchedQuery => "unmatched query",
            WarningKind::UnservedQueryable => "unserved queryable",
            WarningKind::DispatchConcurrencyExceedsReplicas => "dispatch concurrency > replicas",
            WarningKind::DispatchSignalNotSubscribed => "dispatch signal not subscribed",
            WarningKind::DispatchRequiresReplicas => "dispatch requires replicas",
            WarningKind::SpawnSignalNotPublished => "spawn signal not published",
        };
        write!(f, "{prefix}: {}", self.message)
    }
}

/// Result of validating a resolved swarm.
#[derive(Debug)]
pub struct ValidationResult {
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationResult {
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }
}

/// Validate a resolved swarm's signal graph.
pub fn validate_swarm(swarm: &ResolvedSwarm) -> ValidationResult {
    let mut warnings = Vec::new();

    // Collect all emitted and listened signals across agents.
    let mut all_emitted: HashSet<&str> = HashSet::new();
    let mut all_listened: HashSet<&str> = HashSet::new();
    let mut emitted_by: HashMap<&str, Vec<&str>> = HashMap::new(); // signal → [agent keys]
    let mut listened_by: HashMap<&str, Vec<&str>> = HashMap::new();

    // Entry signals are "emitted" by the orchestrator.
    for entry in &swarm.entry {
        all_emitted.insert(entry.as_str());
        emitted_by
            .entry(entry.as_str())
            .or_default()
            .push("(entry)");
    }

    for agent in &swarm.agents {
        for signal in agent.def.publish.names() {
            all_emitted.insert(signal);
            emitted_by
                .entry(signal)
                .or_default()
                .push(&agent.swarm_key);
        }
        for signal in agent.def.subscribe.names() {
            all_listened.insert(signal);
            listened_by
                .entry(signal)
                .or_default()
                .push(&agent.swarm_key);
        }
        // die_on signals are also listeners (the sidecar subscribes to them).
        if let Some(lifecycle) = &agent.lifecycle {
            if let Some(die_on) = &lifecycle.die_on {
                for signal in die_on {
                    all_listened.insert(signal.as_str());
                    listened_by
                        .entry(signal.as_str())
                        .or_default()
                        .push(&agent.swarm_key);
                }
            }
        }
        // spawn.on signals are listened (the orchestrator subscribes for spawn triggers).
        if let Some(spawn) = &agent.spawn {
            all_listened.insert(spawn.on.as_str());
            listened_by
                .entry(spawn.on.as_str())
                .or_default()
                .push(&agent.swarm_key);
        }
    }

    // Dead signals: emitted but nobody listens.
    // Skip entry signals, done signals, and output signals.
    let done_signals: HashSet<&str> = swarm.done.iter().map(|s| s.as_str()).collect();
    let output_signals: HashSet<&str> = swarm.output.iter().map(|s| s.as_str()).collect();
    for signal in &all_emitted {
        if !all_listened.contains(signal)
            && !done_signals.contains(signal)
            && !output_signals.contains(signal)
        {
            warnings.push(ValidationWarning {
                kind: WarningKind::DeadSignal,
                message: format!(
                    "signal \"{signal}\" is emitted by {} but nobody listens",
                    emitted_by[signal].join(", ")
                ),
            });
        }
    }

    // Orphan triggers: listened but nobody emits.
    for signal in &all_listened {
        if !all_emitted.contains(signal) {
            warnings.push(ValidationWarning {
                kind: WarningKind::OrphanTrigger,
                message: format!(
                    "signal \"{signal}\" is listened by {} but nobody emits it",
                    listened_by[signal].join(", ")
                ),
            });
        }
    }

    // Unreachable agents: no path from entry signals to this agent's listens.
    let reachable = compute_reachable(&swarm.entry, &swarm.agents);
    for agent in &swarm.agents {
        if !reachable.contains(agent.swarm_key.as_str()) {
            warnings.push(ValidationWarning {
                kind: WarningKind::UnreachableAgent,
                message: format!(
                    "agent \"{}\" is unreachable from entry signals",
                    agent.swarm_key
                ),
            });
        }
    }

    // Collect mismatch: collect expects N but fewer emitters exist.
    for agent in &swarm.agents {
        for (signal, expected_count) in &agent.collect {
            let _emitter_count = emitted_by
                .get(signal.as_str())
                .map(|v| v.len())
                .unwrap_or(0);
            // Count replicas too.
            let total_emitters: u32 = swarm
                .agents
                .iter()
                .filter(|a| a.def.publish.contains(signal.as_str()))
                .map(|a| a.replicas)
                .sum();
            if total_emitters < *expected_count {
                warnings.push(ValidationWarning {
                    kind: WarningKind::CollectMismatch,
                    message: format!(
                        "agent \"{}\" collects {expected_count} \"{signal}\" signals but only {total_emitters} emitter(s) exist",
                        agent.swarm_key
                    ),
                });
            }
        }
    }

    // Missing done.
    if swarm.done.is_empty() {
        warnings.push(ValidationWarning {
            kind: WarningKind::MissingDone,
            message: "swarm has no 'done' signals defined — it will run indefinitely".into(),
        });
    }

    // Queryable validation.
    let mut all_queryables: HashSet<&str> = HashSet::new();
    let mut all_queries: HashSet<&str> = HashSet::new();

    for agent in &swarm.agents {
        for key in agent.def.queryable.names() {
            all_queryables.insert(key);
        }
        for key in agent.def.query.names() {
            all_queries.insert(key);
        }
    }

    // Unmatched query: agent queries a key but no agent serves it.
    for key in &all_queries {
        if !all_queryables.contains(key) {
            let queriers: Vec<&str> = swarm
                .agents
                .iter()
                .filter(|a| a.def.query.contains(key))
                .map(|a| a.swarm_key.as_str())
                .collect();
            warnings.push(ValidationWarning {
                kind: WarningKind::UnmatchedQuery,
                message: format!(
                    "query key \"{key}\" is used by {} but no agent declares it as queryable",
                    queriers.join(", ")
                ),
            });
        }
    }

    // Unserved queryable: agent declares queryable but nobody queries it.
    for key in &all_queryables {
        if !all_queries.contains(key) {
            let servers: Vec<&str> = swarm
                .agents
                .iter()
                .filter(|a| a.def.queryable.contains(key))
                .map(|a| a.swarm_key.as_str())
                .collect();
            warnings.push(ValidationWarning {
                kind: WarningKind::UnservedQueryable,
                message: format!(
                    "queryable \"{key}\" is declared by {} but no agent queries it",
                    servers.join(", ")
                ),
            });
        }
    }

    // Dispatch validation.
    for agent in &swarm.agents {
        if let Some(dispatch) = &agent.dispatch {
            // Pool dispatch requires replicas > 1.
            if agent.replicas <= 1 {
                warnings.push(ValidationWarning {
                    kind: WarningKind::DispatchRequiresReplicas,
                    message: format!(
                        "agent \"{}\" has dispatch config but only {} replica(s) — \
                         pool dispatch is only useful with replicas > 1",
                        agent.swarm_key, agent.replicas
                    ),
                });
            }

            let subscribed: HashSet<&str> =
                agent.def.subscribe.names().collect();

            match dispatch {
                crate::schema::DispatchConfig::Uniform(rule) => {
                    if rule.mode == crate::schema::DispatchMode::Pool
                        && rule.concurrency > agent.replicas
                    {
                        warnings.push(ValidationWarning {
                            kind: WarningKind::DispatchConcurrencyExceedsReplicas,
                            message: format!(
                                "agent \"{}\" dispatch concurrency ({}) exceeds replicas ({})",
                                agent.swarm_key, rule.concurrency, agent.replicas
                            ),
                        });
                    }
                }
                crate::schema::DispatchConfig::PerSignal(map) => {
                    for (signal, rule) in map {
                        // Check signal is in subscribe list.
                        if !subscribed.contains(signal.as_str()) {
                            warnings.push(ValidationWarning {
                                kind: WarningKind::DispatchSignalNotSubscribed,
                                message: format!(
                                    "agent \"{}\" dispatch references signal \"{signal}\" \
                                     which is not in its subscribe list",
                                    agent.swarm_key
                                ),
                            });
                        }
                        // Check concurrency <= replicas for pool signals.
                        if rule.mode == crate::schema::DispatchMode::Pool
                            && rule.concurrency > agent.replicas
                        {
                            warnings.push(ValidationWarning {
                                kind: WarningKind::DispatchConcurrencyExceedsReplicas,
                                message: format!(
                                    "agent \"{}\" dispatch concurrency ({}) for signal \
                                     \"{signal}\" exceeds replicas ({})",
                                    agent.swarm_key, rule.concurrency, agent.replicas
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    // Spawn validation.
    for agent in &swarm.agents {
        if let Some(spawn) = &agent.spawn {
            if !all_emitted.contains(spawn.on.as_str()) {
                warnings.push(ValidationWarning {
                    kind: WarningKind::SpawnSignalNotPublished,
                    message: format!(
                        "agent \"{}\" spawn trigger signal \"{}\" is not published by any agent",
                        agent.swarm_key, spawn.on
                    ),
                });
            }
        }
    }

    ValidationResult { warnings }
}

/// BFS from entry signals to find all reachable agents.
fn compute_reachable<'a>(
    entry: &'a [String],
    agents: &'a [crate::loader::ResolvedAgent],
) -> HashSet<&'a str> {
    let mut reachable: HashSet<&str> = HashSet::new();
    let mut signal_queue: Vec<&str> = entry.iter().map(|s| s.as_str()).collect();
    let mut visited_signals: HashSet<&str> = HashSet::new();

    while let Some(signal) = signal_queue.pop() {
        if visited_signals.contains(signal) {
            continue;
        }
        visited_signals.insert(signal);

        for agent in agents {
            let subscribes = agent.def.subscribe.contains(signal);
            let spawned_by = agent
                .spawn
                .as_ref()
                .map(|s| s.on.as_str() == signal)
                .unwrap_or(false);

            if subscribes || spawned_by {
                if reachable.insert(&agent.swarm_key) {
                    for emit in agent.def.publish.names() {
                        signal_queue.push(emit);
                    }
                }
            }
        }
    }

    reachable
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{ResolvedAgent, ResolvedSwarm};
    use crate::schema::{AgentDef, SignalList};

    fn make_agent(key: &str, subscribe: &[&str], publish: &[&str]) -> ResolvedAgent {
        ResolvedAgent {
            swarm_key: key.to_string(),
            def: AgentDef {
                name: key.to_string(),
                description: None,
                runtime: "test".to_string(),
                config: None,
                prompt: "test".to_string(),
                subscribe: subscribe.iter().map(|s| s.to_string()).collect(),
                publish: publish.iter().map(|s| s.to_string()).collect(),
                queryable: SignalList::default(),
                query: SignalList::default(),
                context: None,
                permissions: None,
                lifecycle: None,
                steer: None,
            },
            replicas: 1,
            collect: Default::default(),
            lifecycle: None,
            dispatch: None,
            spawn: None,
            source_path: None,
        }
    }

    #[test]
    fn clean_pipeline_validates() {
        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![
                make_agent("planner", &["task_received"], &["plan_ready"]),
                make_agent("implementer", &["plan_ready"], &["code_ready"]),
                make_agent("reviewer", &["code_ready"], &["review_passed"]),
            ],
            entry: vec!["task_received".into()],
            done: vec!["review_passed".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(result.is_clean(), "warnings: {:?}", result.warnings);
    }

    #[test]
    fn detects_dead_signal() {
        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![
                make_agent("a", &["start"], &["output", "unused_signal"]),
            ],
            entry: vec!["start".into()],
            done: vec!["output".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(result.warnings.iter().any(|w| w.kind == WarningKind::DeadSignal
            && w.message.contains("unused_signal")));
    }

    #[test]
    fn detects_orphan_trigger() {
        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![
                make_agent("a", &["start", "nobody_emits_this"], &["done"]),
            ],
            entry: vec!["start".into()],
            done: vec!["done".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(result.warnings.iter().any(|w| w.kind == WarningKind::OrphanTrigger
            && w.message.contains("nobody_emits_this")));
    }

    #[test]
    fn detects_unreachable_agent() {
        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![
                make_agent("reachable", &["start"], &["middle"]),
                make_agent("unreachable", &["totally_disconnected"], &["nowhere"]),
            ],
            entry: vec!["start".into()],
            done: vec!["middle".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(result.warnings.iter().any(|w| w.kind == WarningKind::UnreachableAgent
            && w.message.contains("unreachable")));
    }

    #[test]
    fn detects_missing_done() {
        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![make_agent("a", &["start"], &["out"])],
            entry: vec!["start".into()],
            done: vec![],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::MissingDone));
    }

    #[test]
    fn detects_dispatch_concurrency_exceeds_replicas() {
        use crate::schema::{DispatchConfig, DispatchMode, DispatchRule, DispatchStrategy};

        let mut agent = make_agent("coder", &["plan_ready"], &["code_ready"]);
        agent.replicas = 3;
        agent.dispatch = Some(DispatchConfig::Uniform(DispatchRule {
            mode: DispatchMode::Pool,
            concurrency: 5,
            strategy: DispatchStrategy::RoundRobin,
        }));

        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![agent],
            entry: vec!["plan_ready".into()],
            done: vec!["code_ready".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(result.warnings.iter().any(|w| {
            w.kind == WarningKind::DispatchConcurrencyExceedsReplicas
        }));
    }

    #[test]
    fn detects_dispatch_signal_not_subscribed() {
        use crate::schema::{DispatchConfig, DispatchMode, DispatchRule, DispatchStrategy};
        use std::collections::HashMap;

        let mut agent = make_agent("coder", &["plan_ready"], &["code_ready"]);
        agent.replicas = 3;
        let mut map = HashMap::new();
        map.insert(
            "unknown_signal".to_string(),
            DispatchRule {
                mode: DispatchMode::Pool,
                concurrency: 1,
                strategy: DispatchStrategy::RoundRobin,
            },
        );
        agent.dispatch = Some(DispatchConfig::PerSignal(map));

        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![agent],
            entry: vec!["plan_ready".into()],
            done: vec!["code_ready".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(result.warnings.iter().any(|w| {
            w.kind == WarningKind::DispatchSignalNotSubscribed
                && w.message.contains("unknown_signal")
        }));
    }

    #[test]
    fn detects_dispatch_requires_replicas() {
        use crate::schema::{DispatchConfig, DispatchMode, DispatchRule, DispatchStrategy};

        let mut agent = make_agent("coder", &["plan_ready"], &["code_ready"]);
        agent.replicas = 1; // Only 1 replica — dispatch is useless.
        agent.dispatch = Some(DispatchConfig::Uniform(DispatchRule {
            mode: DispatchMode::Pool,
            concurrency: 1,
            strategy: DispatchStrategy::RoundRobin,
        }));

        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![agent],
            entry: vec!["plan_ready".into()],
            done: vec!["code_ready".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(result.warnings.iter().any(|w| {
            w.kind == WarningKind::DispatchRequiresReplicas
        }));
    }

    #[test]
    fn die_on_signal_is_not_dead() {
        use crate::schema::{Lifecycle, LifecycleMode};

        let coordinator = make_agent("coordinator", &["start"], &["sub_task", "coding_complete"]);
        let mut coder = make_agent("coder", &["sub_task"], &["sub_task_ready"]);
        coder.lifecycle = Some(Lifecycle {
            mode: LifecycleMode::Persistent,
            max_activations: Some(10),
            max_turns: None,
            idle_timeout: None,
            max_uptime: None,
            die_on: Some(vec!["coding_complete".to_string()]),
        });

        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![coordinator, coder],
            entry: vec!["start".into()],
            done: vec!["sub_task_ready".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        // coding_complete should NOT be flagged as dead because coder listens via die_on
        assert!(
            !result.warnings.iter().any(|w| {
                w.kind == WarningKind::DeadSignal && w.message.contains("coding_complete")
            }),
            "coding_complete should not be a dead signal when used in die_on. warnings: {:?}",
            result.warnings
        );
    }

    #[test]
    fn valid_dispatch_no_warnings() {
        use crate::schema::{DispatchConfig, DispatchMode, DispatchRule, DispatchStrategy};

        let mut agent = make_agent("coder", &["plan_ready"], &["code_ready"]);
        agent.replicas = 5;
        agent.dispatch = Some(DispatchConfig::Uniform(DispatchRule {
            mode: DispatchMode::Pool,
            concurrency: 2,
            strategy: DispatchStrategy::RoundRobin,
        }));

        let wf = ResolvedSwarm {
            name: "test".into(),
            description: None,
            agents: vec![agent],
            entry: vec!["plan_ready".into()],
            done: vec!["code_ready".into()],
            output: vec![],
            seed_message: None,
        };

        let result = validate_swarm(&wf);
        assert!(
            result.is_clean(),
            "unexpected warnings: {:?}",
            result.warnings
        );
    }
}
