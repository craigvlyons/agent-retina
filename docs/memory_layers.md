# Retina Memory Layers

> Build the memory spine correctly now so the anthill can grow without redesign later.

This document explains the memory and learning layers of Retina in project language.

It is intended to keep implementation aligned with the research and the long-term architecture.

## Canonical Role

Retina does not learn from one giant chat transcript.

Retina learns through a layered memory system:

```text
observation timeline -> experiences -> knowledge -> reflexive rules
```

This is the memory spine of the agent.

It is part of the architecture from the start because every future worker and specialist should inherit the same learning lifecycle.

## Research Basis

This direction comes directly from the canonical research stack:

1. [docs/v1_plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/v1_plan.md)
2. [docs/research_overview.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_overview.md)
3. [docs/trait_contracts.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/trait_contracts.md)
4. [docs/research_memory.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_memory.md)
5. [docs/architecture.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/architecture.md)
6. [docs/roadmap.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/roadmap.md)

If a future refactor or feature idea conflicts with this memory model, use those docs to resolve it before changing behavior.

## Layer 1: Observation Timeline

### What It Is

The observation timeline is the raw event trail for what actually happened during execution.

Examples:

- task received
- intent created
- reasoner called
- action dispatched
- action result received
- state captured
- delta computed
- experience persisted
- consolidation completed

### What It Will Do

The timeline is the ground truth for:

- replay
- debugging
- trust
- reflection
- later multi-agent coordination

The timeline is not just debug noise.

### Design Rule

Do not hide or soften important failures in the timeline.

If the agent cannot plan, cannot act, loops, or hits a constraint, that signal must remain visible.

## Layer 2: Experiences

### What It Is

An experience is an action-outcome record with utility.

An experience should capture:

- the task
- the chosen action
- the result
- the observed delta
- the utility of the action

### What It Will Do

Experiences are the first reusable memory layer.

They let the agent remember:

- what worked
- what failed
- what produced useful information
- what action patterns are worth reusing

This is the main short-to-mid-term learning layer in v1.

### Design Rule

Do not score usefulness only by state mutation.

Read, search, inspect, and document extraction actions can be highly useful even when state does not change.

## Layer 3: Knowledge

### What It Is

Knowledge is distilled memory promoted from repeated experience.

It is not the full timeline and not the raw episode.

It is the compact lesson or pattern that emerges from repeated successful work.

Examples:

- for tasks like this, `read_file` is a good next step
- when the target is a PDF, `extract_document_text` is better than `read_file`
- this kind of repo task usually needs `find_files` before `read_file`

### What It Will Do

Knowledge should become:

- easier to retrieve than raw episodes
- easier to include in a small prompt
- more stable than single observations

This is how Retina moves from “I saw this once” to “I have a lesson here.”

### Design Rule

Knowledge should be pull-based and compact.

Do not dump raw memory into prompts.

The kernel should assemble a small memory slice only when it is relevant.

## Layer 4: Reflexive Rules

### What It Is

Reflexive rules are high-confidence patterns promoted into fast harness behavior.

They are the beginning of agent skill.

At this layer, the system is no longer only remembering.
It is starting to internalize.

### What It Will Do

Reflexive rules should let mature agents:

- skip unnecessary reasoning
- act quickly on known patterns
- reduce cost and latency
- become more reliable in familiar domains

This is how a worker becomes skilled over time.

### Design Rule

Reflexes must be earned.

They should require:

- repeated success
- meaningful confidence
- the ability to decay or deactivate after later failures

Do not promote weak patterns into reflexes just because they happened once or twice.

## Utility Scoring

### What It Is

Utility is the value signal attached to experience.

It is not identical to state change.

### What It Will Do

Utility affects:

- experience ranking
- consolidation quality
- knowledge confidence
- reflex promotion

It is one of the main signals that tells the system whether a pattern is genuinely useful.

### Design Rule

Utility must reflect actual usefulness.

Examples:

- a successful file read can have positive utility even with no state change
- a failed shell command should have strongly negative utility
- a successful write with verified change should have high positive utility

## Consolidation

### What It Is

Consolidation is the memory-owned process that transforms lower layers into higher ones.

In project terms:

- experiences accumulate
- repeated successful patterns become knowledge
- high-confidence knowledge becomes reflexive rules
- later contradictions can reduce confidence or deactivate rules

### What It Will Do

Consolidation is how learning becomes durable.

Without it, the agent only logs and never matures.

### Design Rule

Consolidation belongs to memory, not to ad hoc kernel shortcuts.

The kernel should call it.
The memory implementation should own how it works.

## Architectural Role

This layered model is important because Retina is not only building one CLI worker.

The long-term architecture is:

- one kernel shape
- many shells/bodies
- many workers
- later a queen/root coordinator
- later a colony and mesh

Every future agent should inherit the same memory lifecycle:

```text
observe -> record -> score -> consolidate -> reuse -> promote
```

That means:

- the browser shell should use the same memory layers
- the hardware shell should use the same memory layers
- future specialists should keep private memory with the same structure
- promoted tools and reflexes should still fit the same learning path

## V1 Scope

In v1, this memory stack should be:

- real
- observable
- small-context
- pull-based
- honest about failure

It does not need to be fully mature yet.

But it must already have the right shape.

Current v1 target:

- full observation timeline
- experience recording with utility
- memory recall for relevant prior work
- experience to knowledge promotion
- knowledge to reflex promotion
- decay or deactivation after contradiction or failure

## What Not To Do

Do not drift into these patterns:

- giant transcript memory dumps
- broad fallback logic that hides failure instead of recording it
- model-only memory with no durable store
- reflex promotion without strong confidence
- knowledge that cannot be traced back to experience
- architecture that makes memory different for every shell in incompatible ways

## Long-Term Direction

V1 uses a buildable SQLite memory implementation.

Longer term, the research points toward richer memory behavior:

- stronger utility weighting
- better deduplication and contradiction handling
- linked knowledge edges
- activation-style retrieval
- local-model compaction and summarization
- private per-agent memory plus selective promotion upward

Those are later improvements, not a reason to break the v1 shape now.

## Bottom Line

The memory layers are not optional scaffolding.

They are the durable spine of the agent:

```text
timeline -> experience -> knowledge -> reflex
```

If we build this spine cleanly now, the anthill can expand chamber by chamber without a major rewrite.
