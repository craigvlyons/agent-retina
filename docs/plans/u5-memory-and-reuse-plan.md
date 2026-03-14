# U5 Memory And Reuse Plan

> A useful worker should get better at local work over time, not repeat the same clumsy path forever.

## Purpose

This plan focuses on making repeated useful-worker tasks improve through memory, compaction, and reusable local workflow knowledge.

The worker should remember:
- authoritative sources
- good extraction choices
- good output paths
- effective commands and helper scripts
- failed approaches worth avoiding

## Research Basis

Use these docs as the governing stack:
1. [docs/research_memory.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_memory.md)
2. [docs/research_compaction.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_compaction.md)
3. [docs/memory_layers.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/memory_layers.md)
4. [docs/plans/v1_useful_worker.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/plans/v1_useful_worker.md)
5. [docs/v1_plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/v1_plan.md)

Research rules to preserve:
- compact active task state, full evidence outside prompt
- value-aware retrieval
- memory should help, not bloat prompt context
- repeated success should harden toward knowledge and reflex

## Boundary

What this plan changes:
- local workflow recall
- experience/knowledge quality for file/document tasks
- compaction quality for useful-worker tasks
- operator visibility into learned local workflows

What this plan does not change:
- distributed multi-agent memory
- new storage backend architecture
- browser/hardware memory models

## Desired Memory Behavior

For local useful-worker tasks, memory should help the worker remember:
- “this markdown file was better than the PDF”
- “page 2 was the relevant source”
- “this script cleanly transformed the data”
- “this output naming/path pattern worked”
- “this first-step browse path was low value”

## Implementation Phases

### Phase U5.1: source-choice memory

Make experiences and knowledge better at storing:
- source preference
- source authority
- extraction method that worked
- page-level relevance where applicable

### Phase U5.2: reusable local workflow memory

Record and recall repeated local workflow shapes such as:
- find -> extract page -> read companion text -> write output
- search docs -> gather evidence -> write summary
- inspect CSV -> transform -> write derived output

These should be recalled as compact helpful priors, not giant transcript dumps.

### Phase U5.3: command and script reuse

Preserve successful command/script patterns as reusable knowledge.

This includes:
- useful shell command sequences
- useful helper scripts
- lightweight transformation patterns

### Phase U5.4: compaction support for useful-worker tasks

Ensure compaction preserves:
- authoritative working source set
- output artifact target/status
- successful and failed paths worth keeping
- exact evidence references for reopened sources

### Phase U5.5: inspection and review

Improve operator surfaces so the human can inspect:
- learned source preferences
- promoted transform patterns
- active local-workflow rules
- whether memory is helping or just accumulating noise

## Implementation Tasks

- Extend experience/knowledge metadata for source and output workflow details.
- Improve recall ranking for repeated local transform tasks.
- Promote successful local workflow patterns into knowledge and later reflexive rules.
- Improve compaction ranking to preserve authoritative sources and output goals.
- Improve inspect surfaces for local workflow learning.
- Add tests for:
  - repeated local transform tasks improving over time
  - source preference recall
  - script/command reuse recall
  - compaction preserving working sources and output targets

## Acceptance Tests

- After repeated similar document tasks, the worker chooses better source paths more quickly.
- Repeated useful command/script workflows become easier to reuse.
- Compacted task state still preserves the right working source set and output target.
- Inspect surfaces show why a local workflow was promoted or preferred.

## Do Not Drift Into

- giant transcript replay as “memory”
- similarity-only retrieval with no utility weighting
- hiding bad learning under opaque compaction
- over-promoting brittle task-specific hacks into fake general rules

## Done Condition

This plan is done when the useful worker measurably improves on repeated local file/document/output tasks through compact recall, better source choice, better workflow reuse, and transparent operator inspection.
