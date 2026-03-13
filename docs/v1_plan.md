# Retina — Version One Plan

> Build the first durable chamber of the anthill: one private Rust agent that can observe, act, verify, remember, and reflect while preserving the shape of the future colony.

## Purpose

V1 is not the final multi-agent network.

V1 is the first trustworthy kernel:
- one independent agent
- one CLI shell
- one SQLite memory implementation
- one reasoner implementation
- one complete observation timeline
- one learning path that can harden into reflexes

The point of v1 is to prove the harness loop end-to-end before adding specialists, browser shells, transport, or fabrication.

## Canonical Decisions

### Rust and Wasm

- Kernel and first-party implementations are written in Rust.
- Tool fabrication targets Rust source compiled to Wasm.
- We are not building a Python tool path.

### Operating Mode

- V1 runs in independent mode.
- The first agent executes directly through its `Shell`.
- `Router` and `Transport` stay in the architecture and type system, but they are not the focus of v1 delivery.

### Memory Direction

- V1 ships with a buildable SQLite memory implementation.
- That implementation captures timeline events, experiences, knowledge, rules, and tools.
- Long-term memory should evolve toward richer utility weighting, linked knowledge, and local-model compaction informed by 2026 research.

### Expansion Model

- The first agent must use the same kernel shape future specialists will use.
- Per-agent manifests, IDs, directories, and message types are designed early so the colony grows by addition, not redesign.

## V1 Contract

V1 is successful when the agent can:
- accept a task through CLI surfaces
- choose an action through the kernel
- execute through the shell
- capture pre and post state
- verify what changed
- persist a full observation timeline
- store experience and update utility
- reflect on failure or mismatch
- begin promoting repeated success into reflexive behavior

## What Is Done

The current codebase has completed the first three v1 chambers in rough form:

### Phase 1: contracts and kernel

Done:
- `retina-types`
- `retina-traits`
- `retina-kernel`
- `retina-test-utils`

### Phase 2: first working agent

Done:
- `retina-memory-sqlite`
- `retina-shell-cli`
- `retina-llm-claude`
- `retina-cli`

Current behavior:
- run and chat both go through the same CLI-to-agent path
- the shell can list, find, read, search, write, append, inspect, and run controlled shell commands
- timeline events are persisted
- approvals are enforced for writes and risky actions

### Phase 3: learning primitives

Partially done:
- utility updates
- consolidation hook in memory
- first reflex promotion path

Still to harden:
- stronger rule promotion criteria
- richer consolidation behavior
- better repeated-task reuse and refinement

## What Is Not Done

These are not v1 blockers if the single-agent harness remains strong, but they are not finished:

- iterative multi-step task execution
- stronger stop and cancel controls for long-running task loops
- deeper autonomous planning for complex repo workflows
- browser shell
- hardware and device shells
- Wasm fabrication loop
- transport and specialist spawning
- root-agent routing and worker hierarchy

## Design Rules

### The harness is the intelligence

The model should not carry the system.

The kernel should do as much as possible in compiled behavior:
- reflex checks
- circuit breakers
- state verification
- context assembly
- utility updates
- promotion into rules

### The shell is the body

The shell owns:
- sensing
- action execution
- approvals
- hard constraints
- state capture and comparison

### Memory is pull-based

The context window stays small.
Memory is recalled when needed, not stuffed into prompts by default.

### Observation is a first-class surface

The timeline is not debug noise.
It is the ground truth for reflection, replay, trust, and future multi-agent coordination.

## Near-Term V1 Finish Work

The next work inside v1 should be:

1. strengthen multi-step execution inside the kernel
2. add operator stop and cancel controls
3. improve autonomous shell planning for real repo work
4. harden learning and reflex promotion

This keeps the project aligned with the research without jumping early into the colony phase.
