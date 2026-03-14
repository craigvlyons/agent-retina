# Compaction Alignment Plan

> This plan brings Retina's implementation into alignment with the compaction and caching research in [docs/research_compaction.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_compaction.md) without drifting into heuristic shortcuts or transcript-heavy agent design.

## Purpose

Retina already follows part of the research direction:

- small assembled context
- pull-based recall
- compact action-result slices
- durable episodic memory outside the prompt

But it does **not** yet implement the new compaction model:

- cached stable prefix
- canonical task-state artifact
- working source set
- provider-aware Claude compaction path
- context editing / stale tool-result trimming
- compaction metrics and policies

This document defines the phases and tasks needed to close that gap.

## How Far Off We Are

Current alignment is roughly:

- **Strongly aligned**
  - small prompt philosophy
  - pull-based recall
  - durable memory outside prompt
  - compact action result previews
- **Partially aligned**
  - compact recent step history
  - memory cleanup at SQLite level
  - operator visibility into memory and timeline
- **Not yet aligned**
  - prompt caching in Claude requests
  - Claude server-side compaction
  - context editing
  - harness-owned canonical task-state compaction
  - working source-set memory
  - compaction triggers and metrics
  - exact evidence indexing for compacted tasks

Practical assessment:

- **Foundation alignment:** good
- **Research-grade compaction alignment:** early
- **Approximate alignment to the new compaction research:** about 30%

That means the architecture is still compatible, but the actual compaction behavior is not there yet.

## Current Gaps in the Code

Today:

- [crates/retina-kernel/src/lib.rs](/Users/macc/Projects/gabanode_lab/agent-retina/crates/retina-kernel/src/lib.rs) assembles context from:
  - task
  - recalled memory slice
  - a first structured task-state artifact
  - recent steps
  - last result
  - operator guidance
- [crates/retina-types/src/lib.rs](/Users/macc/Projects/gabanode_lab/agent-retina/crates/retina-types/src/lib.rs) now defines structured task-state types, and the kernel populates first-pass working sources and artifact references
- [crates/retina-llm-claude/src/lib.rs](/Users/macc/Projects/gabanode_lab/agent-retina/crates/retina-llm-claude/src/lib.rs) now sends a split stable-prefix/mutable-context request and records cache token metrics, but it still lacks Claude context editing and server-side compaction
- [crates/retina-memory-sqlite/src/lib.rs](/Users/macc/Projects/gabanode_lab/agent-retina/crates/retina-memory-sqlite/src/lib.rs) compacts old SQLite rows, but that is retention cleanup, not live task compaction

The main issue is:

Retina currently compacts **storage** and **single-step outputs**, but not **live task continuity**.

## Non-Negotiable Design Rules

These rules come from the research stack and should govern implementation:

1. Do not replace real compaction with broad fallback logic.
2. Do not keep raw transcripts as the main continuity mechanism.
3. Do not let a rolling prose summary become the only memory of a task.
4. Preserve exact references to files, docs, IDs, URLs, and artifacts.
5. Preserve the working set of sources the agent is actively reasoning from.
6. Keep the harness in charge of compaction state.
7. Use Claude-native caching and compaction features where helpful, but keep the harness provider-portable.

## Phase 1: Canonical Task-State Artifact

### Goal

Replace flat prompt continuity with a structured task-state artifact.

### What must be added

Add new shared types in [crates/retina-types/src/lib.rs](/Users/macc/Projects/gabanode_lab/agent-retina/crates/retina-types/src/lib.rs):

- `TaskState`
- `TaskGoal`
- `TaskProgress`
- `TaskFrontier`
- `RecentActionSummary`
- `WorkingSource`
- `ArtifactReference`
- `AvoidRule` or equivalent failed-path record

### Minimum fields

`TaskState` should hold:

- goal
- success criteria
- hard constraints
- current phase
- completed checkpoints
- verified world-state facts
- next action frontier
- open questions
- blockers
- recent meaningful actions
- working source set
- artifact references
- avoid list

### Tasks

1. Add the new types to `retina-types`.
2. Change `AssembledContext` so it carries structured task state instead of only flat strings.
3. Add a compact renderer for model prompts, but keep the struct canonical in Rust.
4. Keep backward-compatible rendering while the rest of the stack is migrated.

### Done when

- the kernel can assemble a `TaskState`
- the reasoner receives a compact rendering of that state
- the task state can be logged and inspected independently of the raw transcript

### Progress

Status: `in progress`

Completed in this pass:

- added `TaskState`, `TaskGoal`, `TaskProgress`, `TaskFrontier`, `RecentActionSummary`, `WorkingSource`, `ArtifactReference`, and `AvoidRule`
- updated `AssembledContext` to carry structured task state
- updated the kernel to assemble first-pass task state from recent steps, artifact refs, and failed-path records
- updated the prompt renderer so the reasoner now sees the task-state artifact instead of only flat step strings

Still left before Phase 1 is fully closed:

- improve the quality of success criteria and frontier fields
- expose task state in operator inspection surfaces
- reduce duplicate legacy fields once downstream code no longer depends on them

## Phase 2: Working Source Set

### Goal

Make "what docs/files the agent is currently working from" part of durable task continuity.

### What must be added

Track active sources such as:

- files
- extracted PDFs
- URLs
- API references
- generated artifacts

Each source should record:

- locator
- kind
- role
- status
- why it matters
- last used step
- evidence refs

### Tasks

1. Populate and maintain `WorkingSource` records from real shell activity.
2. Update shell action result handling so reads, extracts, and inspections can register or refresh working sources.
3. Update kernel step processing to keep the source set current.
4. Show the working source set in operator inspection output.

### Done when

- the agent can resume a task and know which sources are authoritative
- compaction does not erase what document or file the answer came from

### Progress

Status: `in progress`

Completed in this pass:

- task loop state now tracks working sources from reads, extracts, matches, listings, inspections, searches, writes, and commands
- step-complete timeline events now carry task-state snapshots that include current working sources
- operator review now has a dedicated `retina inspect task-state` view for the latest task-state snapshot

Still left before Phase 2 is fully closed:

- improve source roles and statuses from generic first-pass labels into stronger semantics
- add richer source-set display to other operator surfaces where useful
- use working sources directly in more planning and resumption decisions

## Phase 3: Live Task Compaction Engine

### Goal

Compact long-running work into structured state at step boundaries.

### What must be added

Retina needs a compaction policy inside the kernel, not just SQLite retention cleanup.

### Compaction triggers

Start with simple triggers:

- step count threshold
- large tool result threshold
- token estimate threshold
- phase boundary
- explicit operator handoff / pause boundary

### Tasks

1. Add a `TaskCompactor` module under the kernel.
2. Convert old recent-step lists into task-state updates.
3. Extract high-signal facts from action results into:
   - verified facts
   - recent meaningful actions
   - artifact refs
   - working sources
   - avoid list
4. Replace raw continuation context growth with task-state updates.
5. Add compaction timeline events.

### Done when

- a multi-step task does not depend on growing transcript context
- the agent can keep working from the compact task state

### Progress

Status: `in progress`

Completed in this pass:

- added a first live compaction mechanism at step boundaries inside the kernel
- compaction now triggers on large tool results, step-threshold growth, and working-source growth
- compacted tasks shrink the continuation payload instead of carrying the full previous result forward unchanged
- the kernel now emits explicit `TaskCompacted` events with the compacted task-state snapshot
- exact artifact references continue to survive in the compacted task state

Still left before Phase 3 is fully closed:

- make compaction triggers smarter and more token-aware
- move more continuation logic from legacy `recent_steps` into dedicated task-state fields
- use compacted state more directly during reflection and resume paths

## Phase 4: Exact Evidence and Artifact Indexing

### Goal

Keep compact prompts without losing recoverability.

### What must be added

Compaction must preserve exact evidence by reference, not by embedding everything into the prompt.

### Tasks

1. Extend memory/artifact handling so task state can point to:
   - exact file paths
   - memory IDs
   - timeline event IDs
   - extracted-document records
2. Add a compact evidence reference format to the task state.
3. Ensure recalled evidence can be re-opened on demand.
4. Keep large extracted text outside the prompt unless the current step truly needs it.

### Done when

- the agent can keep exact references without dragging full content forward every step

### Progress

Status: `started as a thin slice`

Completed in this pass:

- compacted task-state snapshots keep exact file/document/path references through `ArtifactReference`
- the continuation path now explicitly relies on those refs when the raw last result is compacted

Still left before Phase 4 is fully closed:

- add stronger evidence IDs beyond path-based refs
- connect refs to memory IDs, timeline IDs, and extracted-document records
- add explicit re-open / dereference flows from compacted state

## Phase 5: Claude Prompt Caching

### Goal

Use Claude caching correctly to reduce token spend and latency.

### What must be added

Prompt caching should be added to [crates/retina-llm-claude/src/lib.rs](/Users/macc/Projects/gabanode_lab/agent-retina/crates/retina-llm-claude/src/lib.rs).

### Tasks

1. Add reasoner config for:
   - cache enabled
   - cache TTL strategy
   - cache breakpoint policy
2. Split request construction into:
   - stable prefix
   - mutable task-state block
   - current step / recent result block
3. Add Anthropic `cache_control` where appropriate.
4. Record token metrics:
   - `input_tokens`
   - `cache_creation_input_tokens`
   - `cache_read_input_tokens`
5. Surface cache metrics in timeline and inspect views.

### Done when

- stable system/tool/agent context is cached
- mutable task continuity does not invalidate the whole prefix

### Progress

Status: `mostly complete for v1`

Completed in this pass:

- Claude request building is now split into a stable instruction prefix and a mutable task-state block
- stable instructions are sent with Anthropic prompt caching enabled by default
- reasoner capabilities now report caching support when enabled
- Anthropic cache token metrics now flow into `TokenUsage`
- Phase 6 request-shape work now builds on top of this instead of replacing it

Still left before Phase 5 is fully closed:

- add explicit cache breakpoint policy beyond the first stable system block
- surface cache token metrics in more operator inspection output
- add a clearer TTL strategy once the preferred Anthropic request pattern is locked in for Retina

## Phase 6: Claude Context Editing and Server-Side Compaction

### Goal

Use Claude-native long-context features without making Retina dependent on them.

### Tasks

1. Add a capability-aware Claude request builder:
   - prompt caching
   - context editing
   - compaction on supported models
2. Add support for clearing stale tool results and old thinking blocks.
3. Add custom compaction instructions tuned to Retina task-state preservation:
   - preserve task goal
   - preserve progress
   - preserve next frontier
   - preserve exact artifact references where possible
   - preserve working source set
4. Persist returned compaction artifacts into task state / memory for continuity.

### Done when

- Claude-native compaction helps long-running tasks
- Retina still works correctly when those features are unavailable

### Progress

Status: `started`

Completed in this pass:

- Claude request building now supports provider-native `context_management` edits
- tool-result clearing can now be requested through Anthropic context-management beta support
- server-side compaction instructions can now be requested for supported Claude 4.6 models
- Anthropic beta headers are now added only when the relevant provider-native features are enabled
- the harness-owned task-state artifact remains the primary continuity layer even when provider-native features are present

Still left before Phase 6 is fully closed:

- wire provider-native usage/compaction signals into operator inspection surfaces
- decide whether Retina should change request shape by model more aggressively once 4.6 is the default
- add real evaluation of long-running behavior on supported Claude compaction models
- add finer-grained context-editing policy beyond the first default tool-result clearing rule

## Phase 7: Memory-Aware Compaction Ranking

### Goal

Decide what survives compaction based on value, not only recency.

### Tasks

1. Add compaction ranking based on:
   - goal criticality
   - forward utility
   - state dependency
   - recovery value
   - irreversibility
   - exactness requirement
2. Reuse utility signals from episodic memory where useful.
3. Distinguish:
   - keep in prompt
   - compact into task state
   - archive outside prompt
4. Add operator inspection to show why an item survived compaction.

### Done when

- compaction is explainable
- repeated noise is dropped
- important constraints and evidence survive

### Progress

Status: `started`

Completed in this pass:

- compaction decisions now record explicit ranking explanations
- task-state snapshots now preserve why items were kept, compacted, or kept as exact refs
- artifact refs and failed-path records are now part of the explainable compaction trail

Still left before Phase 7 is fully closed:

- improve ranking quality beyond the current first-pass heuristics
- incorporate more direct utility-weighted scoring into compaction decisions
- distinguish keep/compact/archive more richly for larger task histories

## Phase 8: Inspection and Evaluation

### Goal

Make compaction visible and testable.

### Tasks

1. Add `retina inspect task-state` or equivalent.
2. Add a grouped timeline view per task/session that shows:
   - compaction points
   - source-set updates
   - frontier changes
3. Add test scenarios for:
   - long-running read/search tasks
   - document-heavy tasks
   - repo tasks with many tool outputs
   - pause/resume continuity
4. Measure:
   - token reduction
   - cache hit rates
   - task success after compaction
   - recovery quality after pause/resume

### Done when

- we can tell whether compaction helped or hurt
- compaction is debuggable, not hidden

### Progress

Status: `started`

Completed in this pass:

- `retina inspect task-state` now shows compaction reasons and ranking explanations
- worker overview now includes first compaction and cache-usage counters
- chat/timeline output now surfaces compaction events in a more readable way

Still left before Phase 8 is fully closed:

- add grouped replay and per-task/session review views
- add stronger long-run evaluation workflows for pause/resume recovery
- add more explicit metrics for token reduction and post-compaction task success

## Suggested Order

Build in this order:

1. Phase 1: canonical task-state artifact
2. Phase 2: working source set
3. Phase 3: live task compaction engine
4. Phase 4: exact evidence and artifact indexing
5. Phase 5: Claude prompt caching
6. Phase 6: Claude context editing and server-side compaction
7. Phase 7: memory-aware compaction ranking
8. Phase 8: inspection and evaluation

This order keeps the harness in charge first, then adds Claude-native optimization second.

## What This Means for V1

Retina can still close v1 as a strong single worker without finishing all of this.

But if the goal is to align the implementation with the new compaction research, the next most important work is:

1. canonical task-state artifact
2. working source set
3. live task compaction engine

Those three are the core.

Prompt caching and Claude-native compaction are important, but they should sit on top of the harness model rather than replacing it.
