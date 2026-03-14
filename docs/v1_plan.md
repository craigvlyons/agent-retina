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

### Research tie-breaker

- If implementation details or refactors are ambiguous, resolve them from the canonical research stack before changing behavior.
- Use `docs/plans/research-aligned-execution-plan.md` as the top-level execution guardrail for every v1 step.
- For v1 work, use this order:
  1. `docs/v1_plan.md`
  2. `docs/research_overview.md`
  3. `docs/trait_contracts.md`
  4. `docs/research_memory.md`
  5. `docs/architecture.md`
  6. `docs/roadmap.md`
- Do not hide planning or execution failures with broad fallback behavior. If the agent cannot do something, that problem should surface clearly in the timeline and operator surface.
- Do not keep errors, state mismatches, or capability gaps from the agent. Those signals are part of how the harness learns what to fix and what it needs.

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
- the kernel can take bounded follow-up steps for one task instead of stopping after one action
- chat has a real control plane for stop/cancel and one-step operator guidance with `/guide <text>`
- the root worker now persists specialist-ready manifest lifecycle and budget metadata
- the CLI can inspect the current agent registry with `retina inspect agents`

### Phase 3: learning primitives

Done in first research-aligned form:
- utility updates
- memory-owned consolidation
- experience to knowledge promotion
- knowledge to reflex promotion with stronger confidence thresholds
- kernel refresh of promoted rules so learning affects behavior immediately
- positive utility scoring for successful non-mutating exploration actions
- memory recall that can reuse similar prior task phrasing, not only exact matches
- consolidation that can lower confidence and deactivate a promoted rule after later failures

Still to harden:
- richer knowledge deduplication and merging
- stronger repeated-task reuse beyond exact or near-exact task patterns

## What Is Not Done

These are not v1 blockers if the single-agent harness remains strong, but they are not finished:

- deeper autonomous planning for complex repo workflows
- richer repeated-task reuse beyond the first consolidation lifecycle
- full specialist lifecycle orchestration, reactivation, and spawn execution
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

Concrete task execution should not be hidden behind broad natural-language fallback routing.
If the reasoner cannot plan a non-trivial task, that failure should surface clearly so the harness can be improved honestly.
Constraints, errors, and missing capability edges should be recorded and exposed rather than softened away.

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

1. improve autonomous shell planning for real repo work
2. refine learning quality and reuse beyond the first consolidation lifecycle
3. strengthen multi-step task quality for more complex tasks
4. harden specialist-ready seams: lifecycle transitions, registry quality, and routing confidence
5. add richer stop and cancel handling for long-running or background execution later

This keeps the project aligned with the research without jumping early into the colony phase.
