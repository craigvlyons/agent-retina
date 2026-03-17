# Context And Memory Layer Audit

Date: 2026-03-17

Scope:
- compare the current Retina implementation against the repo's context, compaction, and memory research
- focus on the live task loop, short-horizon continuity, prompt tiers, and pull-based memory
- avoid adding new design direction beyond what the docs already justify

Sources reviewed:
- [docs/research_compaction.md](/Users/macc/projects/personal/agent-retina/docs/research_compaction.md)
- [docs/research_memory.md](/Users/macc/projects/personal/agent-retina/docs/research_memory.md)
- [docs/plans/compaction-alignment-plan.md](/Users/macc/projects/personal/agent-retina/docs/plans/compaction-alignment-plan.md)
- [crates/retina-kernel/src/loop_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs)
- [crates/retina-kernel/src/result_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs)
- [crates/retina-kernel/src/execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)
- [crates/retina-types/src/task_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- [crates/retina-types/src/reasoning.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/reasoning.rs)
- [crates/retina-llm-claude/src/payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)
- [crates/retina-llm-claude/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/lib.rs)
- [crates/retina-llm-claude/src/config.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/config.rs)

## Executive Read

Retina is structurally aligned with the research direction, but only partially implemented at the live-task continuity layer.

The current stack already has:
- a cached stable prefix path
- a compact live task thread
- pull-based episodic and semantic memory
- first-pass working-source and artifact tracking
- first-pass live compaction of task continuity

The current stack does not yet fully deliver the research target:
- the task-state artifact is present, but still underpowered
- the loop preserves continuity, but not enough high-signal unresolved state
- compaction is present, but still coarse and mostly size-triggered
- Claude context editing is wired
- Claude server-side compaction is wired but likely inactive under the default model

Short verdict:

`Foundation: good`

`Tiered context shape: present`

`Research-grade live continuity and compaction: incomplete`

## Research Target

The strongest concise formulation in the docs is in [docs/research_compaction.md](/Users/macc/projects/personal/agent-retina/docs/research_compaction.md):

`cached prefix + current task state + recent verified progress + next action frontier + indexed evidence references`

The memory document in [docs/research_memory.md](/Users/macc/projects/personal/agent-retina/docs/research_memory.md) frames this as four tiers:
- L0 working memory
- L1 episodic memory
- L2 semantic memory
- L3 procedural memory

For the current audit, the important requirement is:

L0 should be intentionally small but should still carry the current task thread well enough for the agent to continue coherently without re-deriving where it is every step.

## What Retina Actually Has

### 1. Stable prefix layer

Implemented:
- [crates/retina-llm-claude/src/payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs) builds a stable system block
- [crates/retina-llm-claude/src/config.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/config.rs) enables prompt caching by default
- the payload marks the stable block with cache control

Assessment:
- aligned with research
- good lightweight choice

### 2. Live task continuity layer

Implemented in [crates/retina-kernel/src/loop_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs):
- `last_result_json`
- `last_result_summary`
- `recent_steps`
- `recent_action_summaries`
- `working_sources`
- `artifact_references`
- `avoid_rules`
- `last_reasoner_framing`

Rendered into the prompt through:
- [crates/retina-types/src/task_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- [crates/retina-types/src/reasoning.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/reasoning.rs)
- [crates/retina-kernel/src/execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)

Assessment:
- correct tier exists
- this is the right place for short-horizon continuity
- the shape is good, but the contents are still fairly weak for complex continuation

Main gaps:
- success criteria are generic and not task-specific
- frontier is generic rather than obligation-specific
- verified facts are present, but still fairly shallow summaries
- unresolved state is not strong enough to anchor next-step continuation
- the loop carries recent steps and last result, but not yet a very strong "what remains unsatisfied" artifact

### 3. Pull-based memory layer

Implemented in [crates/retina-kernel/src/execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs):
- recalls a small memory slice per task
- pulls `experiences` and `knowledge`
- formats them into compact strings

Assessment:
- aligned with research
- appropriately lightweight
- avoids transcript stuffing

Main gap:
- memory is being pulled, but the active task layer still does more work than it should because live continuity is not yet strong enough

### 4. Compaction layer

Implemented in [crates/retina-kernel/src/loop_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs):
- compacts on step threshold
- compacts on large tool result
- compacts on working source growth
- shrinks `last_result_json`
- trims recent steps, recent action summaries, working sources, and artifact references

Complemented by:
- [crates/retina-kernel/src/result_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs), which turns raw tool results into compact context slices

Assessment:
- this is real live compaction, not just storage cleanup
- that is a strong positive
- but it is still an early compactor, not yet a research-grade task-state compactor

Main gaps:
- triggers are mostly coarse heuristics
- compaction is still "trim and shrink" more than "preserve the best continuation state"
- exact unresolved obligations are not strengthened during compaction
- compaction quality is not token-aware enough yet

### 5. Provider-aware context management

Implemented:
- prompt caching is wired and enabled by default
- context editing is wired and enabled by default
- server-side compaction is wired

Important implementation detail from [crates/retina-llm-claude/src/config.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/config.rs):
- server-side compaction only activates for `claude-sonnet-4-6` and `claude-opus-4-6`

Important implementation detail from [crates/retina-llm-claude/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/lib.rs):
- default model is `claude-sonnet-4-20250514`

Assessment:
- prompt caching: likely active
- context editing: likely active
- server-side compaction: likely not active by default

This means Retina is not currently getting the full provider-side compaction stack described in the research unless the model is changed explicitly.

## Alignment Scorecard

### Strongly aligned

- small prompt philosophy
- pull-based memory
- stable-prefix caching
- live task continuity exists as a dedicated layer
- working sources and artifact references are present
- raw tool outputs are compacted before reuse
- durable evidence stays outside the prompt

### Partially aligned

- task-state artifact quality
- frontier quality
- verified progress quality
- compaction policy
- exact preservation of unresolved obligations
- provider-aware compaction activation

### Not yet fully aligned

- strong task-state-driven continuation after partial progress
- compaction that actively improves next-step continuation rather than mostly trimming
- clear tier discipline between "active task state" and "legacy recent step strings"
- full research-grade use of server-side compaction on supported models

## The Most Important Findings

### Finding 1

Retina already has a tiered context architecture.

This is not the core problem. The architecture is not "missing tiers." The problem is that the live task tier is still underpowered.

### Finding 2

The short-horizon continuity layer is currently the most important gap.

The docs want L0 to preserve:
- current task state
- recent verified progress
- next action frontier
- indexed evidence references

Retina does preserve these categories, but not yet strongly enough to anchor reliable continuation on harder tasks.

### Finding 3

Compaction exists, but it is still early-stage compaction.

Current compaction mostly:
- reduces payload size
- trims history
- shrinks large result blobs

Research wants compaction to do more:
- preserve the canonical task frame
- preserve the authoritative working set
- preserve exact evidence references
- preserve what remains unresolved
- preserve the next best move

Retina is not fully there yet.

### Finding 4

The repo is provider-ready for better Claude context handling, but default runtime settings likely leave some of that value unused.

If the project wants the research-recommended Claude stack in practice, the runtime should actually use a supported compaction model.

### Finding 5

The current system is closer to "light harness, medium loop hardness" than "light harness, hard loop."

It already avoids transcript bloat well.
It does not yet preserve and enforce enough high-signal continuation state to feel consistently strong on multi-step tasks.

## What The Research Most Strongly Supports Next

Based on the repo docs, the next improvements should not be:
- more phrase routing
- more hardcoded workflow logic
- more heuristic task completion patches

The strongest research-aligned next moves are:

### 1. Harden L0 around unresolved obligation

Make the task-state artifact better at carrying:
- what has been verified
- what exact source is authoritative now
- what is still missing
- what terminal condition remains before the task can honestly finish

This should be done in the live task state, not by adding phrase rules.

### 2. Improve compaction quality, not just compaction quantity

The compactor should preserve:
- authoritative sources
- the best exact evidence references
- unresolved frontier
- recent failed paths
- last meaningful result that still constrains the next step

This is more important than adding more triggers.

### 3. Reduce duplicate continuity channels

Right now the active prompt still carries both:
- structured `task_state`
- flat `recent_steps`
- `last_result`
- `last_result_summary`

Some duplication is useful, but the research direction suggests the canonical task-state artifact should become the primary continuity object over time.

### 4. Use provider-side compaction where the docs recommend it

If Claude 4.6 is the intended research path, the runtime should eventually validate that:
- model choice matches the context-management path
- server-side compaction is actually in use when expected

### 5. Keep memory pull-based

This part should not change.
The docs are clear that bigger prompts are not the answer.
Retina should keep:
- small active context
- durable full evidence outside prompt
- exact retrieval when needed

## Recommended Priority Order

### Priority 1

Audit and strengthen what the model sees in `TaskState`, especially:
- frontier quality
- verified facts quality
- authoritative-source quality
- unresolved-obligation quality

### Priority 2

Upgrade live compaction so it preserves better continuation state instead of mostly trimming by size and count.

### Priority 3

Validate whether the intended Claude context-management features are active in real runs, especially server-side compaction.

### Priority 4

Only after the above, consider removing redundant continuity fields if the structured task-state artifact proves strong enough.

## Bottom Line

Retina does not need a totally different context architecture.

It already has the correct research-shaped layers:
- cached stable prefix
- active task state
- pull-based memory
- compacted tool-result continuity
- durable evidence outside the prompt

What it still needs is a stronger live task-state layer and a smarter compaction policy so the loop can carry forward exactly the right things without drifting, forgetting, or overloading the prompt.

That is the clearest research-aligned path to:
- a light harness
- a hard loop
- a tiered context system that actually performs well
