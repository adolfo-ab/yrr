# Agent Collaboration Guide

This file is the entry point for any AI agent working on the yrr codebase. Read this first, then follow the pointers.

## The One Rule: Philosophy is Fixed, Everything Else is Yours to Change

**The following core vision is the only thing set in stone:**

> yrr is a swarm of loosely coupled, lego-like agents working autonomously via Zenoh (pub/sub, query/reply, and whatever else Zenoh offers), defined in a declarative language, to achieve eventual convergence into good solutions. Agents and swarms are composable building blocks that can be reused and nested into complex architectures. Developers focus on one piece at a time while having maximum flexibility and complete architectural control. The system imposes zero topological constraints — any agent interaction pattern is possible. Every agent has a unique identity and every interaction is observable — full traceability of the swarm's behavior is a first-class requirement, not an afterthought.

**Everything else is mutable.** Docs, README, technical solutions, YAML field names, crate structure, formatting, error handling approaches, CLI design — all of it. If you're working on this codebase and you find a better way to do something, change it. Update the docs to reflect what you changed and why. Don't ask permission, don't self-impose constraints beyond the vision above.

The docs in this repo describe the *current* state of thinking, not commandments. They exist to help you understand context, not to limit you. If a design doc says "we do X" and you discover Y is better — do Y, update the doc, log your reasoning in `docs/development/`.

## What is yrr?

yrr is an AI coding agent orchestrator built in Rust. It lets developers declaratively define agents and swarms, then runs them as decoupled swarms communicating via [Zenoh](https://zenoh.io/) under the hood. See [README.md](README.md) for the full project philosophy.

## Documentation Flow

When working on any feature, bug, or design decision, follow this flow:

```
AGENTS.md (you are here)    →  How to work on this codebase
README.md                   →  Project overview, philosophy, quick start
docs/README.md              →  Index of all documentation
docs/design/                →  Design decisions, format specs, rationale
docs/architecture/          →  System architecture, crate structure, data flow
docs/development/           →  Development log: problems, solutions, thought processes
```

Always check `docs/README.md` for the latest index before creating new docs.

## How to Work on This Codebase

### Before starting any task

1. Read `README.md` for project context and philosophy
2. Read `docs/README.md` to find relevant existing documentation
3. Read the specific design/architecture docs related to your task
4. Check `docs/development/` for prior work, known issues, and decisions already made

### While working

1. **Change anything that needs changing** — if a doc, a field name, a technical approach, or a README section doesn't serve the project well, change it. You have full autonomy over everything except the core vision above.
2. **Document what you change and why** — when you make a significant decision, change direction, or encounter a problem, log it in `docs/development/`. Use filenames that describe the topic, e.g., `docs/development/signal-parsing-approach.md`.
3. **Update the index** — when you create a new doc, add it to `docs/README.md`.
4. **Codebase is source of truth** — don't duplicate code in docs. Reference file paths and line numbers. Docs explain *why*, code shows *what*.

### Documentation conventions

- **Design docs** (`docs/design/`): Describe *what* we're building and *why*. These are living documents — update them freely when the design evolves.
- **Architecture docs** (`docs/architecture/`): Describe *how* the system is structured. Update when the architecture evolves.
- **Development docs** (`docs/development/`): The working log. Problems encountered, debugging sessions, trade-offs explored, decisions made during implementation. These capture the development journey so other agents (and humans) can understand the reasoning behind the current state.

### Development doc format

When creating a development doc, use this structure (or change it if you find a better one):

```markdown
# Title

**Date**: YYYY-MM-DD
**Status**: in-progress | resolved | abandoned
**Related**: links to design docs, issues, or other dev docs

## Context
What were you trying to do?

## Problem
What went wrong or what decision needed to be made?

## Approach
What did you try? What worked? What didn't?

## Decision
What was decided and why?

## Consequences
What does this decision affect going forward?
```

## Project Structure

```
yrr/
├── AGENTS.md                   # You are here
├── README.md                   # Project overview and philosophy
├── Cargo.toml                  # Workspace root (once implementation starts)
├── crates/
│   ├── yrr-core/            # Schema types, traits, message types, errors
│   ├── yrr-bus/             # Signal bus abstraction (Zenoh implementation)
│   ├── yrr-runtime/         # Agent lifecycle, sidecar, runtime backends
│   └── yrr-cli/             # CLI binary
├── docs/
│   ├── README.md               # Documentation index
│   ├── design/                 # Design specs and decisions
│   ├── architecture/           # System architecture
│   └── development/            # Development log
├── examples/
│   ├── agents/                 # Example agent definitions
│   └── swarms/              # Example swarm definitions
└── tests/
    └── integration/            # Integration tests
```

## Core Vision (for reference)

These are the non-negotiable principles — the spirit of the project:

1. **Loosely coupled, lego-like agents** — self-contained building blocks with clear interfaces. Reason about one at a time.
2. **Swarms and eventual convergence** — many small agents doing narrow tasks. Intelligence emerges from interactions. Large numbers drive convergence.
3. **Zenoh-powered communication** — pub/sub, query/reply, and any other Zenoh pattern. Transparent to users — they think in signals and triggers, not topics.
4. **Declarative definitions** — agents and swarms defined in YAML (or whatever format proves best). No imperative orchestration code.
5. **Maximum flexibility, zero topological constraints** — any agent architecture is possible. Pipelines, debates, best-of-N, MapReduce, tournaments, feedback loops, whatever else.
6. **Composable at every level** — agents compose into swarms, swarms compose into bigger swarms.
7. **Full observability** — every agent has a unique ID. Every signal, every interaction, every query/reply is logged and observable. You can trace the full history of how agents collaborated, what signals flowed, and what each agent did. The system is transparent by default.
8. **No vendor lock-in** — any AI backend, not just Claude Code.

Everything else — field names, crate boundaries, CLI design, message formats, doc structure — is an implementation detail subject to change.

## Tech Stack

- **Language**: Rust
- **Async runtime**: Tokio
- **Communication**: Zenoh (pub/sub + query/reply, transparent to users)
- **Serialization**: serde + serde_yaml_ng (YAML), serde_json (messages)
- **CLI**: clap
- **Logging**: tracing + tracing-subscriber
