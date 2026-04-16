# System Architecture Overview

> **This is a living document.** Crate boundaries, data flow, abstractions, and technology choices can all change. If you find a better structure — change it, update this doc, log your reasoning in `docs/development/`.

## Crate Structure

yrr is a Rust workspace with four crates:

```
crates/
├── yrr-core/        # Schema, types, traits — no I/O, no runtime deps
├── yrr-bus/         # Signal bus abstraction + Zenoh implementation
├── yrr-runtime/     # Agent lifecycle, sidecar, runtime backends
└── yrr-cli/         # CLI binary — ties everything together
```

### yrr-core

Pure types. No async, no I/O. This crate defines:

- **Schema types** — `AgentDef`, `SwarmDef`, `AgentRef`, `SwarmInclude`, `Permissions`, `Lifecycle`. Serde-deserializable from YAML.
- **Message types** — `SignalMessage` (the envelope that flows between agents via the bus), `TraceEntry`, `AgentOutput`, `EmittedSignal`.
- **Traits** — `AgentRuntime` (the abstraction over AI backends).
- **Error types** — shared error enum.
- **Validation** — schema validation, signal graph analysis (dead signals, orphan triggers, unreachable agents).

Dependencies: `serde`, `serde_json`, `serde_yaml_ng`, `uuid`, `chrono`, `thiserror`, `async-trait`.

### yrr-bus

The communication layer. Abstracts over the messaging system so the rest of yrr never touches Zenoh directly.

- **`SignalBus` trait** — `publish(signal, message)`, `subscribe(signal) -> Receiver`.
- **`ZenohBus`** — implementation that maps signal names to Zenoh key expressions: `yrr/{namespace}/{signal}`.
- **`SignalMapper`** — handles signal-to-key-expression mapping, including signal remapping for nested swarms.

Dependencies: `zenoh`, `tokio`, `yrr-core`, `tracing`.

### yrr-runtime

Agent lifecycle management. The heart of the system.

- **`AgentSidecar`** — the core loop for one agent: subscribe to `listens` signals → on message, spawn agent via runtime → stream output, parse `<<SIGNAL:...>>` markers → publish emitted signals. Handles lifecycle (ephemeral vs persistent, death conditions).
- **`ClaudeCodeRuntime`** — `AgentRuntime` implementation that spawns the `claude` CLI as a subprocess, passes prompt + context, streams output.
- **Process utilities** — async subprocess management, output streaming.

Dependencies: `tokio`, `yrr-core`, `yrr-bus`, `tracing`.

### yrr-cli

The user-facing binary.

- `yrr run <swarm.yaml> [--seed "message"]` — load, validate, run
- `yrr validate <swarm.yaml>` — schema + signal graph validation
- `yrr inject <signal> <message>` — inject into running swarm
- `yrr list` — list available agents/swarms
- `yrr graph <swarm.yaml>` — visualize signal flow
- `yrr schedule` — manage cron swarms

Dependencies: `clap`, `tokio`, `tracing-subscriber`, `yrr-core`, `yrr-bus`, `yrr-runtime`.

## Data Flow

```
User: yrr run dev-pipeline.yaml --seed "Add login"
                    │
                    ▼
            ┌──────────────┐
            │   CLI Layer   │  Parse YAML, resolve refs, validate
            └──────┬───────┘
                   │
                   ▼
            ┌──────────────┐
            │ Orchestrator  │  Create ZenohBus, spawn sidecars
            └──────┬───────┘
                   │
          ┌────────┼────────┐
          ▼        ▼        ▼
     ┌─────────┐ ┌──────┐ ┌──────────┐
     │Sidecar:  │ │Side: │ │Sidecar:  │    Each sidecar subscribes
     │planner   │ │impl  │ │reviewer  │    to its agent's `listens`
     └────┬────┘ └──┬───┘ └────┬─────┘    signals via ZenohBus
          │         │          │
          ▼         ▼          ▼
     ┌─────────┐ ┌──────┐ ┌──────────┐
     │Claude   │ │Claude│ │Claude    │    Sidecar spawns agent via
     │Code CLI │ │Code  │ │Code CLI  │    AgentRuntime, streams output
     └─────────┘ └──────┘ └──────────┘
```

### Signal flow for dev-pipeline

```
1. CLI publishes seed → yrr/dev-pipeline/task_received

2. Planner sidecar receives task_received
   → spawns Claude Code with planner prompt + seed payload
   → streams output, finds <<SIGNAL:plan_ready>>
   → publishes to yrr/dev-pipeline/plan_ready

3. Implementer sidecar receives plan_ready
   → spawns Claude Code with implementer prompt + payload
   → streams output, finds <<SIGNAL:code_ready>>
   → publishes to yrr/dev-pipeline/code_ready

4. Reviewer sidecar receives code_ready
   → spawns Claude Code with reviewer prompt + payload
   → finds <<SIGNAL:review_failed>>
   → publishes to yrr/dev-pipeline/review_failed

5. Planner sidecar receives review_failed (feedback loop!)
   → re-plans with review feedback
   → cycle continues...

6. Eventually reviewer emits <<SIGNAL:review_passed>>
   → matches swarm's `done` → orchestrator shuts down
```

### Signal-to-Zenoh mapping

Internal mapping, invisible to users:

```
Signal name: "plan_ready"
Swarm namespace: "dev-pipeline"
Zenoh key expression: "yrr/dev-pipeline/plan_ready"
```

For nested swarms with signal remapping:
```
Parent swarm: "full-sdlc"
Sub-swarm: "dev-pipeline"
Remap: { task_received: design_ready }

Parent signal "design_ready" publishes to:
  yrr/full-sdlc/design_ready

Sub-swarm's planner subscribes to:
  yrr/full-sdlc/design_ready  (remapped from task_received)
```

## Key Abstractions

### SignalBus trait

```rust
#[async_trait]
pub trait SignalBus: Send + Sync {
    async fn publish(&self, signal: &str, message: &SignalMessage) -> Result<()>;
    async fn subscribe(&self, signal: &str) -> Result<Receiver<SignalMessage>>;
    async fn close(&self) -> Result<()>;
}
```

### AgentRuntime trait

```rust
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    async fn run(&self, agent: &AgentDef, input: &SignalMessage) -> Result<AgentOutput>;
    async fn health_check(&self) -> Result<()>;
    fn name(&self) -> &str;
}
```

### AgentSidecar (pseudocode)

```
loop {
    message = await any_subscribed_signal()

    if lifecycle.should_die() { break }

    output = runtime.run(agent_def, message).await
    // streaming: signals are published as they appear in output

    for signal in output.emitted_signals {
        bus.publish(signal.name, build_message(signal)).await
    }
}
```

## Technology Choices

| Component | Choice | Version | Why |
|-----------|--------|---------|-----|
| Language | Rust | stable | Performance, safety, great async ecosystem |
| Async | Tokio | 1.51 | Industry standard, process spawning support |
| Messaging | Zenoh | 1.8 | Zero-overhead pub/sub + query/reply, peer-to-peer capable, no broker needed |
| YAML | serde_yaml_ng | 0.9 | Maintained fork of serde_yaml (deprecated) |
| CLI | clap | 4.6 | Derive-based, completion generation |
| Logging | tracing | 0.1 | Structured, async-aware, span-based |
| Errors | thiserror | 2 | Derive Error implementations |
