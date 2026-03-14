# Research-Aligned Execution Plan

## Summary

This plan resets Retina to the research-defined direction and prevents further drift while we build the base.

The project now follows a two-layer planning model:
- a **master roadmap** that follows the research from seed to colony to mesh
- an **immediate base recovery path** that finishes the first worker correctly before adding more architecture

The core correction is this:
- Retina should be a **harness that enables model-led exploration through a rich shell/body**
- not a phrase router that substitutes local heuristics for agent behavior

When adding or refactoring any feature, implementation must follow the canonical research stack:
1. `docs/v1_plan.md`
2. `docs/research_overview.md`
3. `docs/trait_contracts.md`
4. `docs/research_memory.md`
5. `docs/architecture.md`
6. `docs/roadmap.md`

If a feature or refactor is ambiguous, stop and resolve it from that stack before continuing.

## Key Changes

### 1. Research-governed delivery rules

- This file is the master execution plan in `docs/plans/`.
- Every future phase or major refactor should include:
  - the research docs it depends on
  - the intended harness behavior
  - what must stay in the kernel vs shell vs reasoner vs memory
  - explicit “do not drift into” warnings
- If a change cannot be justified from the canonical research stack, do not implement it yet.
- Do not hide core planning, execution, or verification failures behind broad fallback behavior. Failures should surface so the harness and agent can improve from real signals.
- Do not cover up or withhold constraints, errors, state mismatches, or missing-capability signals that the agent needs in order to learn what to fix or request.

### 2. Phase 0: recover the base to the research direction

This is the immediate reset phase before more v1 feature work.

- Re-center the harness around **model-led exploration** with bounded autonomy.
- Reduce deterministic task routing to:
  - trivial operator replies
  - offline fallback behavior
  - structured follow-up from already-known results
- Make normal task handling go through the reasoner first.
- Treat the shell as the body:
  - rich exploration tools
  - action execution
  - approvals
  - hard constraints
  - state verification
- Keep the kernel responsible for:
  - bounded step loop
  - context assembly
  - reflection
  - utility updates
  - reflex promotion
  - cancellation and stop controls
- Do not add more natural-language phrase matching unless it is explicitly documented as fallback-only behavior.
- When the reasoner cannot solve a concrete task, surface that failure directly instead of substituting broad local task routing that masks the problem.

### 3. Phase 1: finish the first worker correctly

This phase completes the base harness in the way the research describes.

- Keep one independent agent as the only shipped runtime.
- Make the worker feel agentic through:
  - natural-language task interpretation by the reasoner
  - exploratory use of shell actions
  - bounded multi-step progression
  - reflection that improves the next action
  - memory recall that informs behavior without bloating prompt context
- Expand shell capability only where it improves exploration and action freedom, not by adding restrictive routing layers.
- Keep prompts small and pull-based.
- Keep the observation timeline first-class and replayable.
- Strengthen the acceptance criteria for v1 around:
  - agent-led exploration
  - useful follow-up actions
  - safe stop/cancel
  - memory-informed retries
  - reflex promotion from repeated success

### 4. Phase 2: prepare the first worker to become a colony worker

Do this only after the first worker is strong.

- Preserve the same kernel shape future specialists will use.
- Formalize scoped authority so later workers can have different levels of shell power.
- Keep per-agent IDs, manifests, memory layout, and message types stable.
- Introduce specialist-ready seams without activating the network yet:
  - scoped shell policies
  - agent manifests and lifecycle metadata
  - transport-ready message contracts
  - explicit promoted-tool boundaries

This phase is still about strengthening the worker shape, not spawning agents yet.

### 5. Phase 3: specialist chambers

Add new workers only after the base harness is research-aligned and reliable.

- Add local transport and routing policies.
- Add specialist spawn/reactivate flows.
- Introduce scoped workers such as:
  - code
  - browser
  - research
  - hardware
- Keep specialists as full kernel copies with:
  - their own memory
  - their own tools
  - their own scoped authority
- Root/queen behavior remains orchestration, not execution-heavy history accumulation.

### 6. Phase 4: fabrication and tool growth

Add fabrication only after worker execution and specialist boundaries are stable.

- Build Rust-to-Wasm fabrication.
- Keep fabrication behind the `Fabricator` boundary.
- Register, test, and promote tools through memory and policy, not ad hoc shell logic.
- Promote tools upward only through explicit approval paths.

### 7. Phase 5: colony to mesh

Only after the colony works locally.

- Add MCP and A2A style transport surfaces.
- Add broader root/queen policies for routing, authority, and coordination.
- Extend shells to browser, server, and hardware/device contexts.
- Keep the same kernel, event model, and memory rules across all bodies.

## Test Plan

- Research alignment checks:
  - each new phase or refactor cites its governing research docs
  - features can be mapped back to the canonical research stack
  - no new behavior is implemented from intuition alone when the docs already define direction

- Base harness acceptance:
  - natural-language tasks trigger agent-led exploration instead of immediate fallback replies
  - shell tools are used as an exploration body, not as a rigid phrase router
  - bounded multi-step execution remains observable and cancellable
  - reflection changes the next move when possible
  - memory remains pull-based and small-context
  - repeated successful behavior can promote toward reflexes

- Architecture safety:
  - the single-agent base still preserves specialist-ready boundaries
  - root/queen and worker hierarchy work remains deferred until the worker is strong
  - fabrication and transport stay behind trait boundaries and do not leak into kernel shortcuts

## Assumptions

- Save path: `docs/plans/research-aligned-execution-plan.md`
- This plan replaces “implement what seems useful” with “implement what the research and canonical docs justify.”
- The next implementation work should start at Phase 0 of this plan, not by adding more surface features.
- If a future feature or refactor is unclear, the implementer must stop and consult the canonical research stack before proceeding.
