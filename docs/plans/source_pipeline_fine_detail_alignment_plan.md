# Source Pipeline Fine-Detail Alignment Plan

## Purpose

This plan is the next source-alignment program after the transcript-first refactor and the follow-up continuity work.

The goal is to go pipeline by pipeline through the source codebase, identify the smaller implementation details that make source dependable in practice, and port those details into Retina in a clean cutover style.

This is not a compatibility plan.

Every imported behavior should:

1. become the canonical implementation in Retina
2. replace the older Retina path for that responsibility
3. remove or retire the superseded logic in the same change set when practical

We are not preserving parallel truth paths just because they already exist.

## Cutover Rules

For every pipeline below:

1. inspect the exact source behavior first
2. identify the current Retina seam that differs
3. port the source behavior or the closest Rust-native equivalent
4. cut over the runtime to the new path as truth
5. delete or collapse the superseded path
6. pin the behavior with targeted tests and a full workspace run

Implementation rule:

- no backward-compatible duplicate runtime path unless a temporary bridge is unavoidable inside one working patch
- no “old path for some events, new path for others” as a lasting design
- no plan item is complete until the new path is the default and the old path is no longer authoritative

## Success Criteria

Retina should end this program with:

- source-style continuity behavior across live turns, resume, compaction, and sidechains
- source-style replacement and artifact reuse decisions that stay stable across code changes
- source-style subagent inheritance for the continuity pieces that affect cache and replay correctness
- fewer inferred or regenerated continuity artifacts at runtime
- fewer places where the model has to guess what prior context “really meant”

## Current Status

Progress snapshot as of 2026-05-17:

- `done`: core replacement-state carry-forward is now canonical live state, not a render-only summary
- `done`: continuity-critical replacement decisions are persisted as explicit timeline records and reused by follow-up, inspect, and recoverable resume reconstruction
- `done`: follow-up turns can rebuild from persisted session/timeline history instead of only live chat-process memory
- `done`: delegated local agents and specialists inherit parent continuation state through the same continuity substrate
- `done`: active-window assembly is now boundary-driven and keeps the full pre-compaction thread until a real compact boundary exists
- `done`: model-facing continuation render is now thread-first, with replacement text inlined into transcript entries and duplicate ledger sections removed
- `done`: compaction carry-forward summary is now a real transcript unit (`CompactSummary`), not a render-only synthetic block
- `done`: remaining carryover reminders are now materialized during active-window assembly as transcript units (`CarryoverMessage`) instead of being invented inside the renderer
- `done`: Claude structured-output truncation now gets one clean larger-budget retry before final failure, which brings the provider recovery path closer to source’s max-output escalation behavior
- `done`: bounded same-turn structured-output recovery now continues through transcript-native recovery messages instead of immediately surfacing a recoverable block on the first truncation failure
- `done`: prompt-too-long/context-too-large reasoner failures now get a one-shot reactive compaction recovery path instead of immediately blocking when compaction can still shrink the live thread
- `done`: recovery transitions are now first-class timeline events, which makes source-style continue reasons inspectable without scraping transcript text
- `done`: overview recovery stats now read explicit transition events instead of inferring state from free-form reason strings
- `done`: recovery transition payloads now carry structured metadata like `attempt`, and operator/runtime summaries read that metadata instead of flattening everything to raw reason text
- `done`: provider-side `max_output_tokens_escalate` is now exposed as an explicit reasoner transition and emitted into the same canonical recovery event stream as kernel-side continuations
- `done`: ordinary loop continuations like `next_turn` and `completion_blocker` are now explicit timeline truth instead of being silently implied by later state
- `done`: specialist reasoner budgets now flow into task metadata, and the kernel enforces `max_reasoner_calls_per_task` while honoring declared `max_tokens_per_task` for reasoner requests
- `done`: reasoner and budget terminal events now carry structured live budget state (`used`, `remaining`, `budget`, `pct`) so budget truth is inspectable throughout the loop instead of appearing only as a plain-English hard-stop reason
- `done`: cumulative reasoner token budget now survives resume and delegated continuation because the live spent-token state is carried inside the canonical continuation window instead of resetting on restore
- `done`: continuation and recovery transition events now carry the same structured budget state as `ReasonerCalled`, so loop-pressure context stays attached to the transition that happened under it
- `done`: the canonical continuation window now carries the last loop transition reason and metadata, so resumed/delegated work inherits the same explicit continuation-state thread source keeps in its query loop
- `done`: operator continuation inspection now shows last transition state and cumulative reasoner token usage, so the query-budget pipeline is visible through the same canonical continuity object rather than a separate debug guess
- `done`: recovery ordering guards are now explicit loop state (`max_output_tokens_recovery_count`, prompt-too-long compaction-attempt flag) carried through the continuation substrate instead of being re-derived from transcript markers
- `done`: later reasoner calls now use the remaining declared per-task token budget instead of reusing the original total on every iteration, which brings request sizing closer to source’s remaining-budget model
- `done`: operator continuation inspection now exposes the explicit recovery-guard counters alongside tokens and last transition, so the query-budget state is fully inspectable from the canonical continuation object
- `in progress`: operator/timeline surfaces are mostly aligned, but still need a final sweep against the newest carryover-as-transcript model
- `next`: query budget and recovery fine details still need the same source-by-source tightening pass, but the remaining work is now mostly narrower ordering and transition parity

## Pipeline Inventory

### 1. Message Window Pipeline

Status:

- mostly complete
- active window is now boundary-driven
- full pre-compaction thread is preserved until a real boundary exists
- model-facing transcript excludes control-only entries
- reannounced carryover now becomes transcript material during window assembly instead of a render-only side block
- latest remaining work is checking for any last message-order or hidden-history details source handles that Retina still approximates

Source files:

- `/Users/macc/projects/code_source/src/query.ts`
- `/Users/macc/projects/code_source/src/utils/messages.ts`

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/reasoning.rs`

What to port:

- the exact “messages after last compact boundary” style windowing behavior
- the distinction between full transcript retention and model-facing active slice
- any source behavior that excludes dead or hidden history from the model-facing request while still retaining it for operator surfaces
- the small boundary semantics that keep compaction and follow-up continuity from overlapping incorrectly

Implementation outcome:

- one canonical active-window assembly path in Retina
- no older side heuristics for what should be re-shown if the transcript window already answers that

### 2. Tool Result Replacement Pipeline

Status:

- largely complete
- frozen replacement fate is now carried as canonical state
- exact replacement text is preserved and reused across resume and follow-up reconstruction
- model-facing render now inlines replacement text through transcript entries instead of exposing a raw replacement ledger
- latest remaining work is continuing to verify whether any source-side replacement ordering details still differ under stress

Source files:

- `/Users/macc/projects/code_source/src/utils/toolResultStorage.ts`
- `/Users/macc/projects/code_source/src/query.ts`

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/reasoning.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`

What to port:

- frozen replacement fate across turns
- exact replacement text persistence rather than regeneration
- per-thread replacement state as live mutable runtime truth
- gap-fill behavior when inherited parent replacements are needed in children
- the “same input yields same replacement” rule that protects replay and cache stability

Implementation outcome:

- replacement records become canonical carried state, not just a rendered summary
- newly created replacement records append to the live state immediately
- old regenerated-only behavior disappears

### 3. Session Log and Reconstruction Pipeline

Status:

- largely complete
- explicit replacement records are persisted as their own timeline events
- follow-up, inspect, and recoverable resume now reconstruct from shared event-driven continuity logic
- redundant serialized recovery blobs and duplicate recent-context copies were removed from the authoritative path
- latest remaining work is mostly cleanup of any small reconstruction path that still leans on broad payload inference instead of dedicated persisted records

Source files:

- `/Users/macc/projects/code_source/src/utils/sessionStorage.ts`
- `/Users/macc/projects/code_source/src/utils/sessionRestore.ts`

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-memory-sqlite/src/lib.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/timeline.rs`

What to port:

- explicit persistence for continuity decisions that must survive process death
- reconstruction logic that does not depend on live RAM-only chat state
- separation between main-thread and sidechain persisted continuity records when needed
- restoration ordering details so resume rebuilds the same working state source had

Implementation outcome:

- continuity-critical records are persisted as their own truth-bearing entries
- follow-up and resume reconstruction no longer rely mainly on inference from generic timeline payloads when a dedicated persisted record should exist

### 4. Compaction Boundary Pipeline

Status:

- mostly complete
- post-boundary active thread now starts after the latest compact boundary for model-facing assembly
- only the latest compaction boundary remains in the live continuation window
- compaction carry-forward now lands as a real `CompactSummary` transcript unit, which is much closer to source’s boundary-plus-summary-message model
- latest remaining work is verifying whether any preserved-segment or attachment-style source details still need a closer Rust-native equivalent

Source files:

- `/Users/macc/projects/code_source/src/query.ts`
- `/Users/macc/projects/code_source/src/utils/messages.ts`
- related source compact services as needed

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs`

What to port:

- exact compact-boundary semantics for what survives into the next active query window
- the distinction between transcript history, preserved segment, and model-facing live thread
- any source detail that prevents stale pre-boundary continuity records from leaking back in

Implementation outcome:

- compaction boundaries become the only truth for post-compact carryover rules
- any remaining fallback rules based on broader live state get removed

### 5. Follow-Up Turn Pipeline

Status:

- largely complete
- adjacent chat turns now reuse a follow-up continuity seed by default
- `/new` exists to explicitly break the thread
- persisted timeline/session history can now reconstruct the latest usable follow-up seed on startup
- sticky constraints are rendered into the reasoner-visible context and carry validated-path commitments forward
- latest remaining work is continuing to test complex real-world follow-up chains against the source behavior

Source files:

- `/Users/macc/projects/code_source/src/query.ts`
- `/Users/macc/projects/code_source/src/utils/sessionStorage.ts`
- `/Users/macc/projects/code_source/src/utils/sessionRestore.ts`

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/chat.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs`

What to port:

- session-scoped follow-up continuity as a default behavior
- persisted reconstruction of the next usable follow-up seed
- exact carryover of validated-path constraints and prior replacement state

Implementation outcome:

- adjacent chat turns behave like one evolving session thread by default
- follow-up seeds stop depending on ephemeral process-local memory

### 6. Sidechain and Subagent Continuity Pipeline

Status:

- largely complete for local delegation
- spawned local agents and routed specialists now inherit parent continuation state through the same substrate
- replacement-state continuity and follow-up continuity now survive delegation much more like source sidechains
- latest remaining work is future remote/runtime integration, not the local continuity shape itself

Source files:

- `/Users/macc/projects/code_source/src/utils/forkedAgent.ts`
- `/Users/macc/projects/code_source/src/utils/toolResultStorage.ts`

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-transport-local/src/lib.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/lib.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs`

What to port:

- clone-vs-override rules for mutable continuity state
- parent replacement-state inheritance for cache-stable forks
- sidechain reconstruction rules for resumed child tasks
- explicit decision about what mutable state is isolated and what is inherited

Implementation outcome:

- specialist and delegated tasks inherit the continuity state they need
- child tasks do not silently drift into different replacement or replay behavior

### 7. Operator and Timeline Projection Pipeline

Status:

- in progress
- operator surfaces already prefer continuation-derived truth over parallel task-state blobs
- recovery and inspect views now rebuild from the same reconstructed continuity path
- latest remaining work is a final sweep to ensure the newest transcript-materialized carryover units are reflected consistently everywhere without stale fallback assumptions

Source files:

- source session loading and registry projection behavior as relevant

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/timeline.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs`

What to port:

- any source detail where operator projection is derived from the same canonical thread objects instead of alternate summaries
- ordering and reconstruction details that keep inspect/debug aligned with runtime truth

Implementation outcome:

- operator surfaces read from the same persisted continuity model the runtime uses
- fewer “close enough” derived summaries

### 8. Query Budget and Recovery Pipeline

Status:

- mostly complete
- recovery-side truth has improved because resume now depends on reconstructed continuation rather than serialized recovery snapshots
- Claude provider retries now include a one-shot larger `max_tokens` retry for parse-truncated structured output before final failure
- structured-output truncation can now keep the same turn alive through a bounded transcript-native recovery continuation instead of immediately blocking the task
- prompt-too-long failures can now do a one-shot reactive compaction retry using the canonical compaction path before surfacing a block
- recovery continuations are now explicit timeline events and operator overview stats count them by reason
- recovery continuation payloads now include structured metadata such as `attempt`, and runtime/chat summaries render from that metadata
- provider-side larger-budget retries now surface as explicit `max_output_tokens_escalate` transitions instead of remaining hidden inside the Claude client
- ordinary loop continuations now surface as explicit `TaskContinued` events for `next_turn` and `completion_blocker`, which brings the loop state closer to source’s explicit transition model
- declared reasoner budgets are now partially real: `max_reasoner_calls_per_task` is enforced, `max_tokens_per_task` now drives reasoner request sizing when present, live budget usage is emitted as structured event payload state instead of only a terminal reason string, cumulative token usage now survives resume through the continuation substrate, continuation/recovery transitions now carry that same budget snapshot instead of severing the state from the reason, and the continuation substrate now preserves the last transition reason instead of forcing later code to infer it from side effects
- resumed tasks now respect the carried recovery guards immediately: a resumed task at the structured-output recovery limit will not emit another truncation recovery continuation, and a resumed task that already attempted prompt-too-long compaction will not compact again
- ordinary forward progress now has explicit regression coverage showing recovery state resets the source-like way: after a real `next_turn`, both structured-output recovery and prompt-too-long recovery can fire again from a clean attempt count instead of staying permanently exhausted
- the other ordinary continuation branch is now pinned too: `completion_blocker` resets both recovery families and becomes the new `last_transition`, which keeps its semantics aligned with source’s normal “continue the loop from fresh recovery posture” behavior
- `next_turn` now has the same carried-state regression coverage: even when a resumed task starts with old recovery state, ordinary forward progress clears both recovery families and replaces `last_transition` with `next_turn`, matching source’s normal continue-path semantics
- provider-side `max_output_tokens_escalate` transitions now survive a later same-turn failure instead of getting dropped on the error path, and kernel-side recovery events preserve that source-style ordering so escalation remains visible even when the larger-budget retry still falls through into bounded truncation recovery
- provider-side escalation events now snapshot budget state before the successful retried response is counted, which keeps their ordering and token accounting closer to the source loop’s “retry happened before the next response existed” semantics
- continuation inspection and blocked-task continuity now have explicit regression coverage showing `last_transition` behaves as the latest continuation reason, not a frozen historical provider transition, which matches the source loop’s “current loop state” semantics under later budget blocks
- resumed tasks that hard-block on token or reasoner-call budgets before another model call now keep the carried recovery/compaction transition in their blocked continuation state, which gives resume/inspect the same recovery posture source would still consider current at that point
- provider-side escalation transitions now also stay current when a resumed task cannot take the follow-up truncation recovery branch because the recovery limit is already exhausted; the blocked continuation keeps that escalation as the last transition instead of regressing to the older carried recovery note
- provider-side `max_output_tokens_escalate` is now explicitly pinned as distinct from bounded truncation recovery: escalation leaves `max_output_tokens_recovery_count` untouched, while the later `max_output_tokens_recovery` branch increments it, which keeps provider retry and loop recovery semantics separated the same way source does
- the two recovery families now have explicit interaction coverage: a truncation recovery preserves any prior prompt-too-long compaction guard while incrementing only the truncation recovery count, matching source’s “carry both pieces of state forward without collapsing them into one flag” behavior
- the mirror interaction is now pinned too: a prompt-too-long compaction recovery preserves any prior truncation recovery count while flipping only the prompt-too-long guard, so the two recovery families remain independently stateful in both directions
- provider-side escalation now has the same carryover guarantee: it preserves both the truncation recovery count and the prompt-too-long guard instead of clobbering either recovery family’s state, which keeps all three continuation-state strands independent the way source’s loop does
- the deeper source ordering details around budget, retry, replacement, and compaction are still the clearest remaining gap in this plan
- this is the strongest remaining `next` pipeline after the operator/timeline cleanup sweep

Source files:

- `/Users/macc/projects/code_source/src/query.ts`

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/lib.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/lib.rs`

What to port:

- source’s smaller recovery loop details around retry, continuation, and state reuse where they materially improve correctness
- prompt-budget interactions that should happen before or after replacement/compaction

Implementation outcome:

- fewer subtle differences between “what source would have kept visible” and “what Retina keeps visible” under stress

## Execution Order

### Phase 1. Inventory and Source Notes

For each pipeline:

- extract the exact source behavior
- record the Retina mismatch
- decide whether to port directly, adapt to Rust, or intentionally skip

Deliverable:

- short implementation notes appended to this plan or to linked issue notes

### Phase 2. Replacement-State and Reconstruction Cutovers

Prioritize the pipelines that most affect continuity correctness:

1. tool result replacement
2. session log and reconstruction
3. follow-up turns
4. sidechains/subagents

Deliverable:

- explicit cutover patches with old path removal

Status:

- effectively complete
- the replacement-state, reconstruction, follow-up, and sidechain core cutovers listed here have landed

### Phase 3. Active Window and Compaction Precision

Then tighten:

1. message window assembly
2. compaction boundary semantics
3. operator/timeline projection alignment

Deliverable:

- single canonical active-window model

Status:

- well underway and mostly landed
- the major remaining work here is small source-detail cleanup, not another broad architecture shift

### Phase 4. Recovery and Query Loop Hardening

Finally:

1. budget/recovery ordering
2. residual query-loop fine details
3. removal of any leftover inferred continuity shortcuts

Deliverable:

- cleaner parity under long-running and recovery-heavy tasks

Status:

- not finished
- recovery truth paths are much cleaner now, but query-loop ordering and budget/retry parity still need direct source comparison work

## Definition of Done

This plan is done when:

- each listed pipeline has been inspected against source
- each kept behavior is either ported or explicitly rejected with reason
- each ported behavior is the default truth path in Retina
- the older competing Retina path has been removed or collapsed
- focused tests and `cargo test --workspace -q` pass after each major cutover

## Non-Goals

This plan does not cover:

- desktop/browser specialist capability adapters
- multi-device transport and swarm deployment
- UI migration from Gabanode

Those remain active in separate plans.
