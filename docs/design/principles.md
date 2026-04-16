# Design Principles

> **This is a living document.** The core vision (loosely coupled swarms, Zenoh communication, declarative definitions, eventual convergence, composable building blocks, no topological constraints, full observability) is fixed. Everything else — including how these principles are expressed below — can evolve.

These principles guide every decision in yrr's design and implementation.

## 1. Think in Building Blocks

A developer reasons about one agent at a time. What does it do? What can it see? What tools does it need? What should it never touch? Each agent is a complete, self-contained unit with its own security boundary, prompt, and signal interface.

Then you compose these blocks into swarms. The agent doesn't know it's part of a swarm. It just reacts to signals and emits signals.

This means:
- Agent definitions are standalone YAML files, reusable across swarms
- Permissions and security live on the agent (where you reason about them)
- Orchestration concerns (replicas, collect, lifecycle) live on the swarm
- Any agent field can be overridden at the swarm level for context-specific tuning

## 2. Swarms Over Monoliths

Instead of one monolithic agent trying to do everything, run many small, focused agents doing narrow tasks collaboratively. A few bigger agents handle synthesis and judgment.

Intelligence emerges from the interactions, not from a central brain. Take advantage of large numbers to achieve eventual convergence to good solutions.

This means:
- The system supports replicas (N copies of the same agent)
- Fan-out is natural (multiple agents listen to the same signal)
- There is no central coordinator — just signals flowing between agents
- The system is designed for many concurrent agents, not a few sequential ones

## 3. Codebase is Shared Memory

Agents read and write files and git. Signal payloads are lightweight pointers and summaries — "Plan written to PLAN.md", "See commit a1b2c3, files: src/auth.rs" — not bulk data.

Agents are expected to explore the codebase themselves. They don't receive the full context via signals; they receive a nudge about where to look.

This means:
- Signal payloads are short strings (a sentence or two)
- Agents need appropriate file/git permissions to do their work
- The codebase is the single source of truth, not the signal history

## 4. Implicit Wiring

Agents declare what they listen to (`listens`) and what they can emit (`emit`). If signal names match within a swarm, agents are connected. No from→to mappings. No connections section. No explicit wiring.

This is real pub/sub decoupling. An agent that emits `plan_ready` has no idea who (if anyone) listens. An agent that listens for `plan_ready` has no idea who emits it.

This means:
- Feedback loops emerge naturally (planner listens on `review_failed`)
- Fan-out is free (multiple agents listen to `code_ready`)
- Adding a new agent to a swarm is just adding it to the agents list — if its signals match, it's wired in
- The only explicit wiring is signal remapping at swarm composition boundaries

## 5. No Vendor Lock-In

yrr is designed to work with any AI agent backend. The `runtime` field on an agent is a string: `claude-code`, `ollama`, `openai`, `script`, or anything else. A runtime trait abstracts the backend.

This means:
- The core (schema, bus, sidecar) has no dependency on any specific AI provider
- New runtimes are added by implementing a trait
- Agents can mix runtimes within the same swarm
- Non-AI agents (`runtime: script`) are first-class citizens

## 6. Composable Primitives

A small set of orthogonal primitives that combine to create any architecture. No special-purpose features — everything is built from the same building blocks.

The primitives:
- `listens` / `emit` — basic pub/sub
- `replicas` — spawn N copies
- `collect` — buffer N signals
- `lifecycle` — when agents live and die (including `die_on` for signal-driven death)
- `permissions` — ACLs at the agent level
- `override` — override any agent field from the swarm
- `include` + `signals` — nest swarms with signal remapping
- `cron` — scheduled execution

These compose into pipelines, debates, best-of-N, MapReduce, tournaments, self-healing loops, and anything else.

## 7. Full Observability

Every agent has a unique ID. Every signal published, every query/reply, every agent activation is logged and traceable. You can reconstruct the full history of a swarm run: which agent emitted what, when, in response to what, and what happened next.

This is a first-class requirement, not an afterthought. Observability is baked into the communication layer — the bus itself records interactions. This means:
- Every `SignalMessage` carries the source agent's ID, a correlation ID for chain tracing, and a trace of the full path
- Agent activations, completions, failures, and lifecycle events are all observable
- The system can be monitored in real-time or audited after the fact
- Observer agents (`emit: []`) can tap into any signal for dashboards, metrics, or debugging
- The observability data itself flows through Zenoh, so it's subject to the same decoupling — consumers of observability data don't know about producers
