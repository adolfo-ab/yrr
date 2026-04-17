# Declarative Format Specification

> **This is a living document.** It describes the current design thinking, not a frozen spec. If you find a better approach — change it, update this doc, and log your reasoning in `docs/development/`. Only the [core vision](../README.md#philosophy) is fixed.

This document specifies the YAML format for defining yrr agents and swarms.

## File Conventions

- Agent definitions: `planner.yaml`, `reviewer.yaml` — just `.yaml`
- Swarm definitions: `dev-pipeline.yaml` — just `.yaml`
- Differentiated by top-level key: `agent:` vs `swarm:`
- No special extensions, no ceremony

## Agent Definition

An agent is the fundamental building block. It knows nothing about orchestration — only its prompt, what it listens for, what it can emit, and what it's allowed to do.

### Minimal example

```yaml
agent:
  name: planner
  description: "Breaks down tasks into implementation plans"
  runtime: claude-code
  config:
    model: claude-opus-4-6
    max_turns: 3
  prompt: |
    You are a planning agent. Break down the given task
    into concrete implementation steps. Write your plan
    to PLAN.md in the repo root.
  listens:
    - task_received
    - review_failed
  emit:
    - plan_ready
```

### With permissions

Security lives on the agent — you reason about what each building block should be allowed to do at the building block level.

```yaml
agent:
  name: reviewer
  runtime: claude-code
  prompt: "Review the recent changes for correctness and style"
  listens: [code_ready]
  emit: [review_passed, review_failed]
  permissions:
    tools:
      allow: [read, grep, glob, git_diff]
      deny: [write, edit, bash]
    paths:
      allow: ["src/**", "tests/**"]
      deny: [".env", "secrets/**", "*.pem"]
    network: false
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Unique identifier |
| `description` | no | Human-readable purpose |
| `runtime` | yes | Backend: `claude-code`, `ollama`, `script`, etc. |
| `config` | no | Runtime-specific: model, max_turns, timeout, etc. |
| `prompt` | yes | System prompt / instructions |
| `subscribe` | yes | Signal names this agent reacts to |
| `publish` | yes | Signal names this agent can produce |
| `query` | no | Query keys this agent can issue to other agents |
| `queryable` | no | Query keys this agent serves responses for |
| `permissions` | no | ACLs: tools, paths, network access |

### Permissions

```yaml
permissions:
  tools:
    allow: [read, write, edit, glob, grep]    # whitelist
    deny: [git_push, bash]                     # blacklist
  paths:
    allow: ["src/**", "tests/**"]              # visible directories
    deny: ["infra/**", ".env", "*.key"]        # hidden from agent
  network: false                               # network access
```

When `permissions` is omitted, the agent inherits the default permissions of its runtime.

## Swarm Definition

A swarm declares which agents participate and applies orchestration primitives. Connections emerge implicitly from signal name matching — no explicit wiring.

### Basic swarm

```yaml
swarm:
  name: dev-pipeline
  description: "Plan, implement, review with feedback loop"

  agents:
    planner:
      use: planner            # reference to planner.yaml
    implementer:
      use: implementer
    reviewer:
      use: reviewer

  entry: task_received        # signal injected at startup
  done: [review_passed]       # signals that mean completion
```

Data flow emerges automatically:
- `entry: task_received` triggers planner (`listens: [task_received]`)
- planner emits `plan_ready` → triggers implementer
- implementer emits `code_ready` → triggers reviewer
- reviewer emits `review_failed` → loops back to planner
- reviewer emits `review_passed` → swarm done

### Inline agents

For one-off agents, define them inline within the swarm:

```yaml
swarm:
  name: quick-fix
  agents:
    fixer:
      runtime: claude-code
      prompt: "Fix the described bug"
      listens: [bug_report]
      emit: [fix_ready]
    verifier:
      runtime: claude-code
      prompt: "Verify the fix passes tests"
      listens: [fix_ready]
      emit: [verified, fix_rejected]
  entry: bug_report
  done: [verified]
```

## Orchestration Primitives

All orchestration is swarm-level. Agents stay simple.

### Replicas

Spawn N copies of an agent on the same input:

```yaml
agents:
  dev:
    use: implementer
    replicas: 3
```

Each replica runs independently. They all emit to the same signal name.

### Collect

Buffer N signals before triggering an agent:

```yaml
agents:
  judge:
    use: comparator
    collect:
      code_ready: 3           # wait for 3 code_ready signals
```

Without `collect`, agents fire once per signal (default pub/sub). With `collect`, they wait for N to accumulate.

### Lifecycle

Controls when agents live and die. Default is `ephemeral` (spawn per signal, die on completion).

```yaml
agents:
  debater:
    use: advocate
    lifecycle:
      mode: persistent        # stays alive across signals
      max_activations: 5      # die after firing 5 times
      max_turns: 30           # die after 30 LLM turns total
      idle_timeout: 10m       # die after 10 minutes of inactivity
      max_uptime: 2h          # hard cap on total lifetime
      die_on:                 # die when any of these signals fire
        - consensus
        - cancelled
```

**Modes:**
- `ephemeral` (default) — spawn on signal, die on completion. Stateless between activations.
- `persistent` — stays alive across multiple signals. Maintains context.

**Death conditions** (combine any — agent dies when **any** triggers):
- `max_activations` — cap on total number of times the agent fires
- `max_turns` — cap on total LLM turns across all activations
- `idle_timeout` — die after N duration of inactivity
- `max_uptime` — hard cap on total lifetime
- `die_on` — die when specific signals fire anywhere in the swarm

`die_on` enables signal-driven lifecycle: speculative agents die when `best_chosen` fires, debate agents die when `consensus` fires, all workers die when `deadline_reached` fires.

### Dispatch

Controls how signals are routed to replicas. By default, all replicas receive every signal (broadcast). With `dispatch`, you can configure replicas as a worker pool where only N replicas activate per signal.

**Uniform dispatch** — same rule for all subscribed signals:

```yaml
agents:
  coder:
    use: implementer
    replicas: 5
    dispatch:
      mode: pool              # "broadcast" (default) | "pool"
      concurrency: 2          # how many replicas per signal
      strategy: round-robin   # "round-robin" | "random" | "least-busy"
```

**Per-signal dispatch** — different rules for different signals:

```yaml
agents:
  coder:
    use: implementer
    replicas: 5
    dispatch:
      plan-ready:
        mode: pool
        concurrency: 2
      urgent-fix:
        mode: broadcast
```

Signals not listed in per-signal dispatch default to broadcast.

**Modes:**
- `broadcast` (default) — all replicas receive every signal. Use for best-of-N patterns.
- `pool` — replicas form a worker pool. Each signal is dispatched to `concurrency` replicas. Others stay idle, ready for the next signal.

**Strategies** (pool mode only):
- `round-robin` (default) — cycle through replicas in order.
- `random` — pick replicas at random.
- `least-busy` — pick replicas that have been idle the longest.

When all replicas are busy, incoming signals are queued and dispatched as replicas become idle.

### Override

Any agent field can be overridden at the swarm level. The agent definition is the default; the swarm is the final word:

```yaml
agents:
  reviewer:
    use: reviewer
    override:
      config:
        max_turns: 10
      prompt: |
        You are a STRICT reviewer. Reject anything
        without 100% test coverage.
      permissions:
        tools:
          deny: [bash, write, edit]
      listens: [code_ready, hotfix_ready]
      emit: [review_passed, review_failed, needs_discussion]
```

Any field defined in the agent (`name`, `description`, `runtime`, `config`, `prompt`, `listens`, `emit`, `permissions`) can appear under `override`.

### Swarm-wide defaults

Set defaults that apply to all agents in a swarm:

```yaml
swarm:
  name: strict-pipeline
  defaults:
    permissions:
      paths:
        deny: [".env", "secrets/**", "*.pem", "*.key"]
  agents:
    # all agents inherit these permission defaults
```

## Swarm Composition

Swarms can include other swarms as building blocks. Signal remapping at boundaries connects the two namespaces:

```yaml
swarm:
  name: full-sdlc

  agents:
    architect:
      runtime: claude-code
      prompt: "Design the architecture. Write to ARCHITECTURE.md"
      listens: [feature_request]
      emit: [design_ready]
    tester:
      runtime: claude-code
      prompt: "Write and run tests"
      listens: [code_verified]
      emit: [tests_passed, tests_failed]

  include:
    - use: dev-pipeline
      signals:
        task_received: design_ready    # parent's design_ready → sub's task_received
        review_passed: code_verified   # sub's review_passed → parent's code_verified

  entry: feature_request
  done: [tests_passed]
```

The `signals` map is the **only** place where relationships are explicitly described, and only at composition boundaries for renaming.

## Cron Swarms

Swarms can be triggered on a schedule:

```yaml
swarm:
  name: nightly-audit
  cron: "0 2 * * *"
  prompt: "Run nightly audit"

  agents:
    auditor:
      use: auditor
  entry: audit_requested
  done: [report_ready]
```

Multiple schedules:

```yaml
cron:
  - schedule: "0 2 * * 1-5"
    prompt: "Run weekday maintenance"
  - schedule: "0 4 * * 0"
    prompt: "Run deep weekly audit"
```

## Signal Payloads

Signals carry lightweight payloads — short strings that tell the next agent where to look, not bulk data:

```
<<SIGNAL:plan_ready>> Plan written to PLAN.md
<<SIGNAL:code_ready>> Implemented auth module. See commit a1b2c3, files: src/auth.rs, src/db.rs
<<SIGNAL:review_failed>> XSS vulnerability in src/auth.rs:47, missing input sanitization
<<SIGNAL:progress>> Finished database layer, starting API endpoints
```

### Payload descriptions

Signal declarations can include descriptions that explain what the payload represents. This creates an explicit contract between agents — the producer knows what to emit, and the consumer knows what to expect.

Both `subscribe`/`publish` and `query`/`queryable` support two formats:

```yaml
# Bare list (no descriptions)
subscribe:
  - task_received
  - review_failed

# Map with payload descriptions
subscribe:
  task_received: "the task description to break down into a plan"
  review_failed: "review feedback explaining what needs to change"
```

The description is injected into the agent's prompt at runtime, giving the LLM clear guidance about what each signal payload should contain (for publish) or what it will receive (for subscribe). You can mix formats across fields — bare list for simple signals, described map where the contract matters:

```yaml
agent:
  name: implementer
  subscribe:
    plan_ready: "filepath to the plan to implement"
  publish:
    code_ready: "summary of changes made and affected files"
    stuck: "what blocked you and what you need to proceed"
  query:
    review: "request a code review on specific files"
```

Descriptions are optional and backward compatible — existing bare-list definitions work unchanged.

### Mid-task signals

Agents can emit signals **while still running**. The sidecar streams the agent's output in real-time, publishing signals as `<<SIGNAL:name>>` markers appear. This enables progress reporting, early warnings, and streaming pipelines.

### Signal determination

1. **Convention-based**: Agent output contains `<<SIGNAL:signal_name>>` markers, parsed by the sidecar
2. **Default fallback**: If the agent defines exactly one `emit` signal and no markers are found, emit that signal with the full output as payload

## Observers

Agents with `emit: []` are pure observers — they listen but never produce signals:

```yaml
agents:
  logger:
    runtime: script
    config:
      command: "tee -a swarm.log"
    listens: [plan_ready, code_ready, review_passed, review_failed]
    emit: []
```

## Architecture Examples

### Best-of-N Selection

```yaml
swarm:
  name: best-of-three
  agents:
    dev:
      use: implementer
      replicas: 3
      lifecycle:
        mode: persistent
        die_on: [best_chosen]
    judge:
      runtime: claude-code
      prompt: "Compare implementations. Pick the best and merge to main."
      listens: [code_ready]
      emit: [best_chosen]
      collect:
        code_ready: 3
  entry: plan_ready
  done: [best_chosen]
```

### Debate

```yaml
swarm:
  name: design-debate
  agents:
    advocate:
      runtime: claude-code
      prompt: "Advocate for simple solutions. Write to DISCUSSION.md."
      listens: [topic, skeptic_responds]
      emit: [advocate_responds, consensus]
      lifecycle:
        mode: persistent
        max_activations: 5
        die_on: [consensus]
    skeptic:
      runtime: claude-code
      prompt: "Challenge assumptions. Write to DISCUSSION.md."
      listens: [advocate_responds]
      emit: [skeptic_responds, consensus]
      lifecycle:
        mode: persistent
        max_activations: 5
        die_on: [consensus]
  entry: topic
  done: [consensus]
```

### Parallel Review with Aggregation

```yaml
swarm:
  name: thorough-review
  agents:
    security:
      runtime: claude-code
      prompt: "Review for security. Write to REVIEW-security.md"
      listens: [code_ready]
      emit: [review_done]
    performance:
      runtime: claude-code
      prompt: "Review for performance. Write to REVIEW-perf.md"
      listens: [code_ready]
      emit: [review_done]
    style:
      runtime: claude-code
      prompt: "Review for style. Write to REVIEW-style.md"
      listens: [code_ready]
      emit: [review_done]
    aggregator:
      runtime: claude-code
      prompt: "Read REVIEW-*.md. Synthesize final verdict."
      listens: [review_done]
      emit: [all_clear, needs_work]
      collect:
        review_done: 3
  entry: code_ready
  done: [all_clear]
```

### Worker Pool

```yaml
swarm:
  name: task-queue
  description: "Planner breaks work into subtasks, coders pick them up from a pool"
  agents:
    planner:
      use: planner
    coder:
      use: implementer
      replicas: 5
      dispatch:
        mode: pool
        concurrency: 1
      lifecycle:
        mode: persistent
        die_on: [all_done]
    aggregator:
      use: aggregator
      collect:
        code_ready: 5
  entry: task_received
  done: [all_done]
```

Planner emits multiple `plan_ready` signals. Each goes to one idle coder. If all 5 are busy, signals queue until a coder finishes.

### MapReduce

```yaml
swarm:
  name: codebase-audit
  agents:
    analyzer:
      use: file-analyzer
      replicas: 5
    synthesizer:
      runtime: claude-code
      prompt: "Read ANALYSIS-*.md. Produce unified report."
      listens: [analysis_done]
      emit: [report_ready]
      collect:
        analysis_done: 5
  entry: audit_requested
  done: [report_ready]
```
