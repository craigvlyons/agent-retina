# U6 Live Context And Loop Hardening Plan

> A lightweight harness only works if the live task thread stays sharp enough for the worker to keep going without re-deriving the task every step.

## Purpose

This plan turns the context and memory audit into a narrow implementation path for hardening Retina's live loop without adding heuristic routing or heavier orchestration.

The goal is not to add a planner layer.

The goal is to make the existing loop stronger by improving:
- L0 working memory quality
- live task continuity
- compaction quality
- provider-aware context handling in real runs

## Audit Basis

This plan is based directly on:
- [docs/audits/context-and-memory-layer-audit-2026-03-17.md](/Users/macc/projects/personal/agent-retina/docs/audits/context-and-memory-layer-audit-2026-03-17.md)

## Research Basis

Use these docs as the governing stack:
1. [docs/research_compaction.md](/Users/macc/projects/personal/agent-retina/docs/research_compaction.md)
2. [docs/research_memory.md](/Users/macc/projects/personal/agent-retina/docs/research_memory.md)
3. [docs/plans/compaction-alignment-plan.md](/Users/macc/projects/personal/agent-retina/docs/plans/compaction-alignment-plan.md)
4. [docs/plans/u1-task-shape-and-completion-plan.md](/Users/macc/projects/personal/agent-retina/docs/plans/u1-task-shape-and-completion-plan.md)
5. [docs/plans/research-aligned-execution-plan.md](/Users/macc/projects/personal/agent-retina/docs/plans/research-aligned-execution-plan.md)
6. [docs/v1_plan.md](/Users/macc/projects/personal/agent-retina/docs/v1_plan.md)

Research rules to preserve:
- cached stable prefix plus compact live task state
- pull-based memory rather than prompt stuffing
- model-led exploration through the shell/body
- exact evidence outside the prompt with references kept in-context
- honest completion and failure surfaces
- no drift into phrase routing or fake workflow scripting

## Boundary

What this plan changes:
- the quality of live task continuity
- the shape and quality of the task-state artifact
- the quality of compaction for long-running tasks
- validation of provider-side context features in real runs

What this plan does not change:
- multi-agent routing
- new memory backend architecture
- browser or device shells
- broad heuristic planners
- domain-specific workflow routing

## Desired End State

The live context passed to the reasoner should behave like:

`cached prefix + current task state + recent verified progress + next frontier + indexed evidence references`

And specifically:
- the worker should always know what it just did
- the worker should know what evidence is authoritative right now
- the worker should know what remains unresolved
- the worker should know what terminal condition remains before completion is honest
- old raw outputs should not dominate the prompt

## Current Gaps To Close

From the audit:
- task-state fields exist, but frontier and success criteria are too generic
- live continuity exists, but unresolved obligation is not strong enough
- compaction is real, but still more "trim and shrink" than "preserve the best continuation state"
- prompt caching and context editing are wired, but server-side compaction is likely inactive under the default model
- the prompt still carries some duplicate continuity channels that should eventually give way to a stronger canonical task-state artifact

## Implementation Phases

### Phase U6.1: strengthen L0 task-state quality

Improve the task-state artifact so it carries better continuation state without adding routing logic.

Focus areas:
- stronger success criteria derived from the task and current state
- stronger frontier fields
- stronger verified-fact summaries
- stronger authoritative-source semantics
- clearer unresolved obligation representation

Implementation rule:
- improve the structured task-state artifact
- do not add phrase-matching completion rules

Done when:
- task state better answers:
  - what is verified
  - what source is authoritative now
  - what remains to be done
  - what would make completion honest

### Phase U6.2: improve compaction quality

Upgrade the live compactor so it preserves better continuation state instead of mostly shrinking context.

Focus areas:
- preserve canonical task frame
- preserve authoritative working sources
- preserve exact artifact and evidence references
- preserve meaningful failed paths
- preserve unresolved frontier
- preserve the last result that still constrains the next action

Implementation rule:
- compaction should improve continuation quality, not just reduce token count

Done when:
- compacted tasks still have enough signal to continue effectively
- long tasks do not depend on growing transcript context

### Phase U6.3: reduce continuity duplication carefully

Retina currently carries:
- structured `task_state`
- `recent_steps`
- `last_result`
- `last_result_summary`

This phase should determine which of those are still necessary once the task-state artifact is stronger.

Implementation rule:
- do not remove redundancy until the structured task-state artifact proves it can carry the loop

Done when:
- the prompt uses the canonical task-state artifact as the primary continuity object
- legacy continuity fields are reduced only where safe

### Phase U6.4: validate provider-side context handling

Confirm that the runtime is actually using the provider-side context features the research recommends.

Focus areas:
- prompt caching behavior
- context editing behavior
- server-side compaction activation
- real token usage visibility

Implementation rule:
- do not assume provider features are active because the code path exists

Done when:
- real runs confirm which Claude context-management features are active
- model configuration matches the intended research path

### Phase U6.5: loop-hardness review

After the above changes, review whether the loop is actually harder in the research sense.

Specifically validate:
- better multi-step continuation
- lower shallow re-orientation cost between steps
- less drift after discovery
- better recovery after compaction
- no growth in heuristic routing

Done when:
- the loop feels more coherent without becoming more scripted

## Implementation Tasks

- strengthen task-state assembly in [crates/retina-kernel/src/execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)
- strengthen task-state representation in [crates/retina-types/src/task_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- improve compaction behavior in [crates/retina-kernel/src/loop_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs)
- improve result compaction quality in [crates/retina-kernel/src/result_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs)
- validate context-management behavior in [crates/retina-llm-claude/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/lib.rs), [crates/retina-llm-claude/src/config.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/config.rs), and [crates/retina-llm-claude/src/payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)
- add regression tests for:
  - continuation after partial discovery
  - continuity preserved after compaction
  - authoritative working-source preservation
  - frontier preservation through multi-step tasks
  - provider-side compaction/config behavior where testable

## Acceptance Tests

- a multi-step local file task can continue coherently after several steps without depending on raw transcript growth
- compacted task state preserves enough signal for the next step to stay on-task
- the worker can still identify authoritative sources and exact artifact references after compaction
- prompt context remains compact while preserving live continuity
- provider-side context features are validated rather than assumed
- no new phrase-routing or domain-specific workflow routing is introduced

## Do Not Drift Into

- patching prompts with ever-growing lists of example phrasings
- adding heuristic task routing as a substitute for better task state
- growing raw transcript context instead of improving compaction
- using giant prose summaries as the primary continuity mechanism
- removing useful continuity fields before the canonical task-state artifact is strong enough
- mistaking code-path existence for real provider-feature activation

## Done Condition

This plan is done when Retina's loop can carry a compact but high-signal live task thread through multi-step work, survive compaction without losing the real obligation, and stay aligned with the research formula:

`cached prefix + current task state + recent verified progress + next action frontier + indexed evidence references`
