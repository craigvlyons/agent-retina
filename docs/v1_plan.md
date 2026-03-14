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
- Prefer removing blockers over pre-optimizing behavior. We should give the agent a stronger body, clearer observations, and better review surfaces before we try to predict every action it should take.
- Do not over-guide the agent with brittle hand-written routing or predetermined workflows. The system should learn and adapt to the environment it is deployed into.

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

V1 does not require a worker network.
V1 is allowed to stop at one strong portable worker as long as that worker is genuinely useful, observable, and extensible.

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

### Less guidance is often better

For this project, over-guiding the agent is usually worse than under-guiding it.

Good guidance:
- stronger shell and document tools
- cleaner state capture
- clearer approvals and operator controls
- compact memory recall
- better review surfaces for what happened

Bad guidance:
- brittle phrase routing
- hard-coded task shortcuts
- trying to predict every workflow in advance
- hiding capability gaps behind local fallback logic

The system should improve by:
- observing the environment
- recording what happened
- surfacing real failures
- learning from repeated success
- later building or fabricating what it needs

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

## What Is Left To Finish V1

The remaining v1 work should focus on making the single worker reliable, portable, and honest.

### 1. Worker hardening

Finish the runtime hardening pass:
- clean up remaining fragile paths and test-only `unwrap` use where worthwhile
- improve error messages so planning, shell, memory, and approval failures are easier to understand
- remove rough operator UX edges in chat and CLI output
- keep behavior the same while making failure handling more robust

### 2. Autonomous task quality

Improve the quality of the existing agent loop without scripting it:
- better action selection for real repo and desktop-file tasks
- better multi-step convergence on complex tasks
- lower thrashing and repeated low-value steps
- better use of the existing shell/body and document tools

This should be done by improving the harness and context, not by adding broad heuristic routing.

Current progress:
- compact action results now feed the reasoner with smaller, more useful context slices instead of full noisy raw outputs
- memory recall is formatted more usefully for planning, including prior task, outcome, and utility
- structured follow-up after real discovery results is better at moving from `find` to `read/extract` when the task asks a content question
- candidate selection now prefers more human-readable files when multiple matches exist

### 3. Memory cleanup and retention

Bring the SQLite memory closer to the research lifecycle:
- add retention tiers for episodic timeline data
- summarize or compact old raw observations instead of keeping everything forever
- add confidence decay, deduplication, and cleanup passes where justified
- define practical per-agent storage expectations and cleanup triggers
- keep large future artifacts like screenshots outside SQLite, with references stored in memory

V1 should not pretend memory is solved, but it should stop growing without a policy.

Current progress:
- `retina cleanup` now gives the worker an explicit retention and maintenance path instead of silent hidden pruning
- cleanup can trim older episodic timeline rows, decay stale knowledge confidence, and run SQLite/FTS optimization
- retention is still conservative and operator-invoked, which keeps the timeline honest while avoiding uncontrolled growth

Still to finish:
- better summarization of old episodic history instead of only row trimming
- clearer per-agent storage budgets and review surfaces for DB growth
- a scheduled or policy-driven cleanup trigger once we decide the right cadence

### 4. Review and operator surfaces

Make the single worker easier to inspect and trust:
- improve timeline review for humans, not just debugging
- improve memory inspection so promoted lessons and rules are visible
- improve registry and manifest review
- expose enough health signals to judge whether an agent is effective

Current progress:
- `retina inspect overview` now gives one review surface for worker lifecycle, storage, task outcomes, budgets, authority roots, and active rules
- memory inspection now shows confidence and task context instead of only flat strings
- `retina inspect agents`, `retina inspect memory`, `retina inspect timeline`, and `retina stats` now work together as a clearer operator review stack

Still to finish:
- better grouped timeline replay for one task or session
- clearer long-term learning review once knowledge and rules accumulate in real use
- review surfaces for per-agent storage growth once more workers exist

### 5. Portable deployment readiness

Finish v1 as a reusable worker core:
- keep the kernel reusable across UIs
- keep shell boundaries clean so new shells can be added later
- keep the current CLI worker as the reference implementation
- do not entangle the kernel with a single UI or deployment surface

The goal is a single worker that can later be reused in CLI, browser, desktop, and device deployments with the same structure.

Current progress:
- the kernel remains separate from the CLI surface
- `run` and `chat` are two UI surfaces over the same worker path
- the shell boundary remains explicit, so later browser, desktop, and hardware bodies can reuse the same kernel
- manifest, registry, routing, and memory shapes are no longer CLI-specific

V1 can close without new deployment shells as long as this structure stays clean and the CLI worker remains the reference implementation.

## V1 Production-Ready Checklist

V1 is ready to close when most of these are true:
- the worker can reliably handle terminal and filesystem tasks without brittle fallback logic
- task failures are surfaced honestly and are easy to inspect
- chat and run feel like two surfaces over the same agent, not two different systems
- memory growth has a real cleanup policy
- repeated successful work measurably improves recall or reflex behavior
- the runtime is robust enough that normal operator use does not hit panic paths
- manifest, registry, and routing seams are stable enough that later specialist work is additive

## What Comes After V1

These are important, but they are not required to close v1:
- full specialist lifecycle orchestration, reactivation, and spawn execution
- browser shell
- desktop sight and desktop automation shell
- hardware and device shells
- Wasm fabrication loop
- transport and specialist spawning
- root-agent routing and worker hierarchy

This keeps v1 aligned with the research:
- one strong worker first
- more shells next
- colony behavior after that
