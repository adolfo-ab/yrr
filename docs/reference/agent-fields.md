# Agent Fields Reference

Complete reference for every field in a yrr agent definition.

## File Structure

Agent files use a top-level `agent:` key:

```yaml
agent:
  name: planner
  runtime: claude-code
  prompt: "Break down the task into steps"
  subscribe: [task_received]
  publish: [plan_ready]
```

## Fields

### `name`

| | |
|---|---|
| **Type** | string |
| **Required** | yes |

Unique identifier for the agent. Used to reference the agent in swarm definitions and in signal routing.

```yaml
name: architect
```

### `description`

| | |
|---|---|
| **Type** | string |
| **Required** | no |

Human-readable description of the agent's purpose. Displayed in the TUI and useful for documentation.

```yaml
description: "Designs the technical architecture and implementation plan"
```

### `runtime`

| | |
|---|---|
| **Type** | string |
| **Required** | yes |

Which backend runs this agent. The runtime determines how the agent's prompt is executed and what tools are available.

```yaml
runtime: claude-code
```

Built-in runtimes: `claude-code`, `ollama`, `script`. New runtimes can be added by implementing the `AgentRuntime` trait.

### `config`

| | |
|---|---|
| **Type** | map (string keys, arbitrary values) |
| **Required** | no |

Runtime-specific configuration. The contents depend on the runtime — yrr passes this map through without interpretation.

```yaml
config:
  model: claude-sonnet-4-6
  max_turns: 3
  timeout: 300
```

### `prompt`

| | |
|---|---|
| **Type** | string |
| **Required** | yes |

System prompt / instructions for the agent. This is what tells the agent what to do. Use YAML `|` for multiline.

```yaml
prompt: |
  You are a senior technical architect. Given a product plan
  with features, you design the complete implementation plan.
```

### `subscribe`

| | |
|---|---|
| **Type** | signal list |
| **Required** | yes |

Signal names this agent reacts to. When any of these signals fire in the swarm, this agent activates and receives the signal payload.

Accepts two formats:

```yaml
# Bare list
subscribe:
  - task_received
  - review_failed

# Map with payload descriptions
subscribe:
  task_received: "the task description to break down"
  review_failed: "review feedback explaining what needs to change"
```

Payload descriptions are injected into the agent's prompt at runtime, giving the LLM clear guidance about what each signal payload will contain.

### `publish`

| | |
|---|---|
| **Type** | signal list |
| **Required** | yes |

Signal names this agent can produce. The agent emits signals by including `<<SIGNAL:name>> payload` markers in its output.

Same two formats as `subscribe`:

```yaml
# Bare list
publish:
  - plan_ready
  - stuck

# Map with payload descriptions
publish:
  plan_ready: "filepath to the architecture doc"
  stuck: "what blocked progress and what is needed"
```

If an agent defines exactly one publish signal and no markers appear in its output, yrr emits that signal with the full output as payload.

Set to an empty list (`publish: []`) for observer agents that listen but never produce signals.

### `query`

| | |
|---|---|
| **Type** | signal list |
| **Required** | no |
| **Default** | empty |

Query keys this agent can issue to other agents. Queries are synchronous request/response exchanges, unlike the async fire-and-forget nature of signals.

```yaml
query:
  review: "request a code review on specific files"
```

### `queryable`

| | |
|---|---|
| **Type** | signal list |
| **Required** | no |
| **Default** | empty |

Query keys this agent serves responses for. When another agent issues a query matching one of these keys, this agent receives the request and sends back a response.

```yaml
queryable:
  design_review: "request review on design decisions"
```

### `context`

| | |
|---|---|
| **Type** | object |
| **Required** | no |

Context window configuration. Controls how the agent handles running out of context space.

```yaml
context:
  max_tokens: 256000
  on_limit: compress
```

| Subfield | Type | Description |
|---|---|---|
| `max_tokens` | integer | Maximum context size in tokens |
| `on_limit` | string | Action when the limit is reached |

**`on_limit` values:**

| Value | Behavior |
|---|---|
| `compress` | Compress/summarize the context and continue |
| `restart` | Restart the agent with a fresh context |
| `kill` | Kill the agent entirely |

### `permissions`

| | |
|---|---|
| **Type** | object |
| **Required** | no |

Security ACLs for tools, filesystem paths, and network access. When omitted, the agent inherits the default permissions of its runtime.

```yaml
permissions:
  tools:
    allow: [read, write, edit, glob, grep]
    deny: [bash, git_push]
  paths:
    allow: ["src/**", "docs/**"]
    deny: [".env", "secrets/**", "*.pem"]
  network: false
```

| Subfield | Type | Description |
|---|---|---|
| `tools.allow` | list of strings | Tool whitelist |
| `tools.deny` | list of strings | Tool blacklist |
| `paths.allow` | list of strings (glob patterns) | Visible directories/files |
| `paths.deny` | list of strings (glob patterns) | Hidden from the agent |
| `network` | boolean | Whether the agent can access the network |

### `lifecycle`

| | |
|---|---|
| **Type** | object |
| **Required** | no |

Controls when agents live and die. Default behavior is ephemeral (spawn on signal, die on completion).

```yaml
lifecycle:
  mode: persistent
  max_activations: 5
  max_turns: 30
  idle_timeout: 10m
  max_uptime: 2h
  die_on:
    - consensus
    - cancelled
```

| Subfield | Type | Default | Description |
|---|---|---|---|
| `mode` | string | `ephemeral` | `ephemeral` or `persistent` |
| `max_activations` | integer | none | Die after firing N times |
| `max_turns` | integer | none | Die after N LLM turns total |
| `idle_timeout` | string (duration) | none | Die after this duration of inactivity |
| `max_uptime` | string (duration) | none | Hard cap on total lifetime |
| `die_on` | list of strings | none | Die when any of these signals fire |

**Modes:**

- **`ephemeral`** (default) -- spawn on signal, die on completion. Stateless between activations.
- **`persistent`** -- stays alive across multiple signals. Maintains context between activations.

**Death conditions** combine with OR logic -- the agent dies when **any** condition triggers. `die_on` enables signal-driven lifecycle: speculative agents die when `best_chosen` fires, debate agents die when `consensus` fires.

### `steer`

| | |
|---|---|
| **Type** | boolean or string |
| **Required** | no |

Enables human steering -- the ability for a human to inject guidance into the agent mid-execution via the TUI. When set to a string, the description is shown as a prompt hint in the TUI.

```yaml
# Simple toggle
steer: true

# With description (shown in TUI as a hint)
steer: "reprioritize phases, adjust scope, or redirect the build"
```

Setting `steer: false` is not valid -- omit the field instead if steering is not needed.

## Full Example

```yaml
agent:
  name: architect
  description: "Designs the technical architecture and implementation plan"
  runtime: claude-code
  config:
    model: claude-sonnet-4-6
  context:
    max_tokens: 256000
    on_limit: compress
  prompt: |
    You are a senior technical architect. Given a product plan
    with features, you design the complete technical implementation plan.

    PROCESS:
    1. Read docs/product-plan.md
    2. Design the architecture
    3. Write to docs/architecture.md
    4. Emit: <<SIGNAL:plan_ready>> docs/architecture.md
  subscribe:
    features_ready: "filepath to the product plan with features"
  publish:
    plan_ready: "filepath to the technical architecture and implementation plan"
  permissions:
    tools:
      allow: [read, write, edit, glob, grep]
      deny: [bash, git_push]
    paths:
      allow: ["docs/**"]
      deny: [".env", "secrets/**"]
    network: false
  lifecycle:
    mode: ephemeral
    max_activations: 1
  steer: "provide architectural guidance on trade-offs"
```

## Minimal Example

Only `name`, `runtime`, `prompt`, `subscribe`, and `publish` are required:

```yaml
agent:
  name: fixer
  runtime: claude-code
  prompt: "Fix the described bug"
  subscribe: [bug_report]
  publish: [fix_ready]
```
