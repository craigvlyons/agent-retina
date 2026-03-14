# U1 Task Shape And Completion Plan

> Make the worker understand what kind of task it is in before it chooses step one.

## Purpose

This plan closes the gap where the worker treats transformation tasks like simple browse tasks.

The goal is not to hardcode workflows. The goal is to give the harness and reasoner enough structure that the worker can distinguish:
- discovery tasks
- answer tasks
- transformation tasks
- output-producing tasks

This is the first useful-worker plan because weak task-shape understanding poisons every later step.

## Research Basis

Use these docs as the governing stack:
1. [docs/v1_plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/v1_plan.md)
2. [docs/plans/research-aligned-execution-plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/plans/research-aligned-execution-plan.md)
3. [docs/plans/v1_useful_worker.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/plans/v1_useful_worker.md)
4. [docs/research_overview.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_overview.md)
5. [docs/trait_contracts.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/trait_contracts.md)
6. [docs/research_compaction.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_compaction.md)

Research rules to preserve:
- model-led exploration
- shell/body freedom
- compact task-state continuity
- no broad heuristic fallback routing
- honest completion and failure surfaces

## Boundary

What this plan changes:
- task interpretation
- completion judgment
- frontier quality
- output-aware progress tracking

What this plan does not change:
- adding new document parsers
- adding OCR or browser support
- changing memory storage backends
- adding specialist spawning

## Desired Worker Behavior

The worker should understand that:
- “find the file” is a discovery task
- “what does this file say” is an answer task
- “use these sources to create another file” is a transformation/output task

For transformation or output tasks, the worker should not consider the task complete until:
- required inputs were gathered or a real blocker was surfaced
- the requested output was created, or the inability to create it was stated honestly

## Implementation Phases

### Phase U1.1: task-shape model

Add a compact task-shape artifact to the assembled context.

Required fields:
- `task_kind`
  - `discovery`
  - `answer`
  - `transform`
  - `output`
- `requested_output`
  - path if named
  - type if implied
- `required_inputs`
  - source-like inputs mentioned in the task
- `success_markers`
  - conditions that must be true before `task_complete=true` is reasonable

Implementation rule:
- this is a harness-owned framing artifact
- it is not a hardcoded action router

### Phase U1.2: better frontier and progress state

Improve task state so it reflects:
- what has been located
- what has been read/extracted
- what remains unresolved
- whether the output artifact exists
- whether the worker is still missing required sources

Progress should track:
- `inputs_located`
- `inputs_read`
- `output_planned`
- `output_written`
- `output_verified`

### Phase U1.3: completion guardrails

Add kernel-side checks so these steps cannot be treated as enough for transform/output tasks by themselves:
- listing a directory
- finding a file
- inspecting a path
- reading only one source when multiple required sources remain

The kernel should be able to reject weak `task_complete=true` cases when:
- named output is still missing
- required source set is incomplete
- the current result is discovery-only

This is not hardcoded domain logic.
It is a generic completion-quality check.

### Phase U1.4: reasoner prompt correction

Strengthen the prompt so the reasoner explicitly sees:
- task shape
- required inputs
- output target
- output-not-yet-created status

Prompt rules should include:
- do not mark transform/output tasks complete before output creation or a surfaced blocker
- discovery-only results are intermediate progress, not final completion
- if multiple sources are required, gather them before final synthesis

### Phase U1.5: step quality and anti-thrash checks

Improve the kernel’s step quality logic so it can detect:
- repeated shallow discovery
- repeated non-progressful browsing
- failure to move from found source to read/extract
- failure to move from gathered evidence to output creation

The worker should still be free to explore, but the harness should detect low-value repetition.

## Implementation Tasks

- Add `task_kind`, `required_inputs`, `requested_output`, and `success_markers` to task-state-oriented types.
- Extend context assembly to compute those fields from the original task and current state.
- Track output artifact existence in task progress.
- Add a generic `completion_guard` in the kernel for transform/output tasks.
- Improve the reasoner prompt to reflect task shape and completion expectations.
- Add regression coverage for tasks that require:
  - gather -> read -> synthesize -> write
  - multiple source inputs
  - named output files

## Acceptance Tests

- “find a PDF and a txt file, use both, and create a new txt file” is not considered complete after listing a directory.
- A task with a named output file cannot finish successfully unless that file exists or the worker surfaces a real blocker.
- Discovery-only first steps remain allowed, but they are tracked as intermediate progress.
- The worker can distinguish between an answer task and a transformation task over the same sources.
- Repeated shallow discovery on an output task produces a clear failure or reflection path, not silent fake success.

## Do Not Drift Into

- hardcoding domain-specific workflows for PDFs or forms
- phrase-routing based on specific user wording
- faking completion because the model said so
- giant prompt templates that try to script the full workflow

## Done Condition

This plan is done when the worker can correctly understand that a multi-source output task is still in progress after discovery, and the kernel no longer lets shallow exploration masquerade as completion.
