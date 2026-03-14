# Retina — Agent Kernel

Retina is a private Rust agent kernel built to start as one strong independent worker and grow into a network of specialized agents.

The long-term goal is an anthill that becomes a mesh:
- one seed agent proves the harness
- specialized agents grow into their own chambers
- a root coordinator eventually behaves like the queen, routing work and managing the colony
- workers can act alone, collaborate with siblings, or be exposed through external interfaces

The model is not the product. The harness is the product.

## Canonical Docs

Build from these docs in this order:

1. `README.md`
2. `docs/v1_plan.md`
3. `docs/plans/research-aligned-execution-plan.md`
4. `docs/roadmap.md`
5. `docs/architecture.md`
6. `docs/trait_contracts.md`
7. `docs/research_overview.md`
8. `docs/research_memory.md`
9. `docs/research_memory_v2.md`

If an older doc disagrees with these, the list above wins.

## Current Status

Retina has completed the first solid chamber of v1:
- Rust workspace and private local runtime
- stable shared types and 5 trait boundaries
- kernel execute loop with timeline logging
- bounded multi-step execution in the kernel
- CLI shell with read, search, write, approval, and command execution
- SQLite memory with experiences, knowledge, rules, and tool registry
- Claude-backed reasoner with local planner fallback
- CLI surfaces for `run`, `chat`, `inspect`, `stats`, and `init`

Retina is not yet the full colony:
- no specialist spawning
- no browser or device shells yet
- no transport layer yet
- no Wasm fabrication loop yet
- no root-agent routing network yet

## Project Rules

- Rust is the implementation language for the kernel and first-party crates.
- Fabricated tools are Rust compiled to Wasm.
- The kernel depends on 5 traits only: `Shell`, `Reasoner`, `Memory`, `Fabricator`, `Transport`.
- The shell owns action constraints, approvals, and state verification.
- Memory is pull-based and should keep prompts small.
- Every meaningful action must be captured in the observation timeline.
- Expansion should be additive. We do not want future specialist support to require major refactors of the kernel.
- `lib.rs` files should stay focused on exports and top-level wiring. If a change adds a new responsibility or a `lib.rs` starts accumulating multiple concerns, split it into modules before adding more feature code.
- If implementation intent is unclear, consult the canonical research stack before adding or refactoring behavior.
- Do not hide real agent failures behind broad fallback behavior. If the agent cannot plan, act, or verify correctly, that gap should be visible in the timeline and operator surface so the system can improve honestly.
- Do not cover up or withhold signals the agent needs in order to learn. The harness should expose constraints, errors, state mismatches, and missing capability edges clearly so the agent or operator can respond.

## Anthill Direction

Think of the system in stages:
- `seed`: one independent agent proves observe → act → verify → remember → reflect
- `chamber`: the first reliable shell, memory, and learning loop
- `colony`: multiple specialists with their own memory and tools
- `mesh`: agents across CLI, web, servers, hardware, and external protocols

Each stage should strengthen the tunnels and load-bearing walls before we add more workers.
