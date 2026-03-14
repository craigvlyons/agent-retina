# U3 Synthesis And Output Plan

> Gathering evidence is not enough. The worker must turn evidence into a produced artifact.

## Purpose

This plan makes the worker able to create useful local outputs from gathered local inputs.

V1 useful worker should be able to:
- combine multiple sources
- synthesize a result
- write a target file
- verify that the requested output now exists

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
- whether it exists
- whether it was written this run
- whether it was verified after write

### Phase U3.2: multi-source synthesis context

Improve assembled context so the reasoner sees:
- compact source set
- source roles
- output target
- synthesis constraints
- what evidence is already sufficient

This should help the worker move from reading to writing naturally.

### Phase U3.3: creation and modification flows

Strengthen the worker’s write path so it can:
- create a new file from gathered content
- overwrite intentionally when the task implies replacement
- append when the task implies incremental output
- perform light modifications to existing local text artifacts
- use commands or helper scripts when they are better than a single direct write

### Phase U3.4: output verification and completion

After writing, the shell and kernel should verify:
- the file exists
- the content changed as expected
- the expected path was targeted
- task state reflects the created artifact

### Phase U3.5: synthesis-aware failure surfaces

If synthesis is blocked, the worker should say why:
- insufficient evidence
- ambiguous source mapping
- unsupported source type
- unsupported output type
- failed write or verification

The failure should still preserve gathered evidence and working state.

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

## Acceptance Tests

- The worker can gather from a `.pdf` and a `.txt` file and produce a new `.txt` output.
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
