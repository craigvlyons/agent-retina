# U3 Synthesis And Output Plan

> Gathering evidence is not enough. The worker must turn evidence into a produced artifact.

## Purpose

This plan makes the worker able to create useful local outputs from gathered local inputs.

V1 useful worker should be able to:
- combine multiple sources
- synthesize a result
- write a target file
- verify that the requested output now exists

## Current Progress

Implemented:
- explicit output artifact state now exists in task state
- output artifact state now tracks:
  - requested output path
  - intended type
  - output intent (`create`, `modify`, `append`, `overwrite`)
  - whether it exists
  - whether it was written this run
  - whether it is verified
  - last write step
  - last write action
- task progress now distinguishes:
  - output exists
  - output written this run
  - output verified
- dynamic reasoner context now includes:
  - compact source set
  - requested output state
  - output artifact state
- requested outputs now carry explicit output intent, so create-vs-modify tasks are part of harness state instead of only living in free-text phrasing
- task frontier and success markers now phrase output work in intent-aware terms such as:
  - create and verify
  - update and verify
  - append to and verify
  - overwrite and verify
- modify/append-style tasks now explicitly track whether the current content of the target artifact has been ingested
- modify-style frontiers now surface when the existing target content needs to be read before updating the artifact
- file write results now distinguish:
  - created
  - overwritten
  - appended
  so output verification has more exact local evidence than a generic “write succeeded”
- command-assisted output flows can now verify target artifacts when `run_command` names a target path
- command-side file changes now enter task state as generated artifacts instead of disappearing into a generic command result
- operator/task-state rendering now shows output artifact state directly
- regression coverage exists for:
  - output artifact state appearing in assembled task state
  - mixed-source output tasks preserving written/verified artifact state
  - edit-style tasks inferring `modify` output intent
  - modify-style tasks preserving target-content-ingested state after reading and rewriting the same artifact
  - modify tasks preserving overwritten output state
  - structured-input tasks creating CSV output artifacts
  - command-assisted modification tasks verifying changed target artifacts
  - command-assisted structured-output tasks verifying created CSV artifacts

Still left in this plan:
- stronger multi-source synthesis context beyond the first compact source/output block
- clearer command/script selection quality for real edit/update tasks once the harness can verify both direct writes and command-assisted outputs
- clearer synthesis-aware failure surfaces
- broader output-task acceptance coverage such as:
  - choosing when command/script-assisted modification is actually better than direct write
  - choosing when command/script-assisted generation of structured outputs is actually better than direct write

## Research Basis

Use these docs as the governing stack:
1. [docs/plans/v1_useful_worker.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/plans/v1_useful_worker.md)
2. [docs/research_compaction.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_compaction.md)
3. [docs/v1_plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/v1_plan.md)
4. [docs/trait_contracts.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/trait_contracts.md)
5. [docs/research_memory.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_memory.md)

Research rules to preserve:
- compact task state should preserve output goals and evidence references
- exact source evidence should remain retrievable
- the worker should not claim completion before output creation is verified
- the harness should verify output completion without prescribing one synthesis route

## Boundary

What this plan changes:
- synthesis flows
- output artifact tracking
- writing and modification flows
- result verification for created files

What this plan does not change:
- PDF recreation with original layout
- browser or UI form filling
- spreadsheet-native formula editing

## Output Classes In Scope

V1 output types:
- `.txt`
- `.md`
- `.csv`
- structured text
- generated notes, summaries, drafts, extracted outputs

Later:
- recreated PDFs
- styled documents
- visual form outputs

## Implementation Phases

### Phase U3.1: output artifact model

Make output artifacts first-class in task state.

Track:
- requested output path
- intended type
- output intent
- whether it exists
- whether it was written this run
- whether it was verified after write

Status:
- implemented in task state, kernel assembly, and operator rendering

### Phase U3.2: multi-source synthesis context

Improve assembled context so the reasoner sees:
- compact source set
- source roles
- output target
- output intent
- synthesis constraints
- what evidence is already sufficient

This should help the worker move from reading to writing naturally.

Status:
- materially implemented:
  - compact source set and output artifact state are now surfaced to the reasoner
  - requested outputs now expose create-vs-modify intent in task state and dynamic context
Still left:
- better synthesis-specific cues about when gathered evidence is already sufficient to write
- stronger evidence-sufficiency cues for modify-vs-create tasks beyond the current intent model

### Phase U3.3: creation and modification flows

Strengthen the worker’s write path so it can:
- create a new file from gathered content
- overwrite intentionally when the task implies replacement
- append when the task implies incremental output
- perform light modifications to existing local text artifacts
- use commands or helper scripts when they are better than a single direct write

Status:
- started:
  - create-vs-modify intent now exists in the harness and prompt context
  - modify/append-style tasks now carry target-content-ingested state so the worker can tell when the current artifact still needs to be read before updating it
  - write results now preserve create-vs-overwrite-vs-append semantics for verification and memory
  - command-assisted output flows can now preserve changed target paths as verified generated artifacts
Still left:
- better decision quality for when to use direct write vs command/script-assisted output paths
- broader regression coverage for command-assisted append flows

### Phase U3.4: output verification and completion

After writing, the shell and kernel should verify:
- the file exists
- the content changed as expected
- the expected path was targeted
- task state reflects the created artifact

Status:
- partially implemented:
  - output existence and verification now flow into explicit output artifact state
  - write outcomes now preserve created/overwritten/appended status as exact verification evidence
  - command-assisted output changes now preserve observed target paths as exact verification evidence
Still left:
- broader regression coverage for verification on more output classes
- clearer timeline surfacing focused on output verification events

### Phase U3.5: synthesis-aware failure surfaces

If synthesis is blocked, the worker should say why:
- insufficient evidence
- ambiguous source mapping
- unsupported source type
- unsupported output type
- failed write or verification

The failure should still preserve gathered evidence and working state.

Status:
- not started as a distinct pass

## Implementation Tasks

- Add explicit output artifact tracking to task state.
- Improve context assembly for synthesis tasks.
- Strengthen write flows for new and modified text outputs.
- Add verification paths for named output files.
- Improve CLI/chat rendering so created artifacts are obvious to the operator.
- Add regression tests for:
  - create a new output from two local sources
  - update an existing text file from local evidence
  - create a CSV-like output from extracted data
  - distinguish created vs overwritten outputs in task-state evidence
  - preserve command-assisted target paths in task-state evidence

## Acceptance Tests

- The worker can gather from a `.pdf` and a `.txt` file and produce a new `.txt` output.
- The worker can update an existing text file from gathered local evidence and preserve that it was overwritten rather than newly created.
- The worker can ingest structured data and produce a CSV-like output artifact.
- The worker can use `run_command` with a named target path and still verify that the requested artifact changed.
- A task requesting a named output file is only complete when that file exists or a real blocker is surfaced.
- Task state shows the produced artifact clearly.
- Output verification is recorded in the timeline.

## Do Not Drift Into

- fake success without writing the file
- PDF layout reconstruction as a v1 requirement
- hardcoded domain templates for specific user files
- forcing direct write actions when a command or script is the more natural local path

## Done Condition

This plan is done when the worker can reliably produce new local text/markdown/CSV outputs from multiple local inputs and prove that the requested artifact was actually created.

## Resume Point

If we continue `u3`, the next best step is:
- improve decision quality so the worker can choose between direct writes and command/script-based edits for the right reasons once the evidence is ready
- add regression coverage for:
  - command-assisted append flows
  - modify tasks where direct write is better than a command path
- improve synthesis-aware blocker output when evidence is present but not yet mapped cleanly into the requested artifact
