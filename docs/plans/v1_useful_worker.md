# Retina V1 Useful Worker Plan

> V1 is not finished when the architecture is clean. V1 is finished when one worker can do real work through its shell, learn from that work, and stay honest about what it still cannot do.

## Purpose

This document defines the corrected boundary for **functional v1**.

Retina v1 is still:
- one private worker
- one kernel
- one memory spine
- one primary shell/body
- one operator surface or many interchangeable operator surfaces

But v1 must also be **useful**.

That means the worker should be able to:
- navigate files and directories
- read and extract useful document content
- combine information from multiple local sources
- create new files from gathered information
- modify existing files when appropriate
- use the terminal broadly enough to explore, transform, and produce outputs
- remember what it has learned from repeated work

This is still v1, not v2.

V2 begins when the single worker is strong enough that adding specialists, browser shells, desktop sight, or transport is more valuable than continuing to strengthen the base worker.

## Research Guardrails

If implementation details are ambiguous, resolve them in this order:

1. [docs/v1_plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/v1_plan.md)
2. [docs/plans/research-aligned-execution-plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/plans/research-aligned-execution-plan.md)
3. [docs/research_overview.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_overview.md)
4. [docs/trait_contracts.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/trait_contracts.md)
5. [docs/research_memory.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_memory.md)
6. [docs/research_compaction.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_compaction.md)
7. This document

Rules that must remain true:
- Do not hide agent weakness behind broad fallback routing.
- Do not over-predict what the agent should do with brittle heuristics.
- Prefer giving the worker a stronger body over narrowing its choices.
- Surface capability gaps honestly in the timeline and operator surfaces.
- Let the harness observe, verify, remember, and compact what happened.

## Corrected V1 Boundary

### V1 architecture

Already largely in place:
- Rust kernel
- CLI shell/body
- SQLite memory
- full timeline
- experience to knowledge to reflex pipeline
- control plane
- specialist-ready seams

### V1 useful worker

Still needs to be completed.

The useful worker boundary is:
- one worker
- local filesystem and terminal body
- no browser shell yet
- no desktop vision yet
- no hardware shell yet
- no worker spawning required

But it must be able to complete real local information-to-output tasks.

Examples that should be in-bounds for v1:
- find a PDF and a `.txt` file, extract what matters, and create a new `.txt` or `.md` output
- search docs, gather evidence from several files, and write a summary file
- inspect a data file, transform it, and write a derived file
- read a markdown or text source and fill a template-like output
- use commands or scripts when that is the most effective path

Examples that are out-of-bounds for v1:
- visual webpage navigation
- browser automation
- OCR-driven desktop UI control
- distributed root/worker routing
- Wasm fabrication loops
- multi-agent task sharing

## Design Position

The worker is weak right now because its body is too narrow for real transformation tasks, not because the idea is wrong.

The fix is not:
- more phrase routing
- more hardcoded follow-up actions
- more fake task shortcuts

The fix is:
- broader terminal freedom
- stronger document tools
- better transformation-oriented task understanding
- better output creation flows
- better judgment around when a task is truly complete

This keeps Retina aligned with the research:
- model-led exploration
- rich shell/body
- small context
- compact task state
- exact evidence outside the prompt
- visible failures instead of hidden ones

## V1 Useful Worker Capabilities

### 1. Filesystem and document ingestion

The worker should be able to:
- list directories
- inspect paths
- find files and folders by pattern
- search text across files
- read text files
- read code/config/docs as text
- extract text from PDFs
- extract specific pages from PDFs when page-level work is requested
- ingest multiple local sources into one working task state

Supported v1 source types:
- `.txt`
- `.md`
- `.json`
- `.toml`
- `.yaml`
- `.yml`
- code/config files
- `.csv`
- `.pdf` via document extraction tools

Advanced-later source types:
- `.docx`
- `.pages`
- scanned PDFs needing OCR
- images/tables/forms requiring vision

### 2. Transformation and synthesis

The worker should be able to:
- combine information from multiple sources
- answer questions from gathered sources
- rewrite source content into a new target format
- create new `.txt` or `.md` outputs
- create derived `.csv` or structured text outputs when the task is local-data oriented
- use extracted evidence to fill in a new text representation of a form or document

V1 does not need to recreate the original PDF layout.
V1 does need to create a correct text output from the available source data.

### 3. Local writing and modification

The worker should be able to:
- create new files
- overwrite existing files
- append to existing files
- create notes, drafts, reports, and extracted outputs
- write temporary scripts to complete a task
- write query files, transforms, or small helpers when useful

Examples:
- create `Emily_wittenberge.txt`
- generate a markdown summary from several documents
- create a small shell or Python script for parsing a local file
- generate a CSV from local extracted data

### 4. Terminal freedom

The worker should not be overly constrained to a tiny list of hand-curated actions.

V1 should allow the worker to:
- run shell commands broadly
- use common local utilities
- inspect command output
- write and run small task-local scripts
- chain local exploration and transformation through the shell

The shell must still:
- record what happened
- verify effects
- capture outputs
- preserve task state
- enforce hard safety boundaries

But the shell should not kneecap the worker by forcing every real task into:
- `list`
- `find`
- `read`

when a command or script is the better choice.

### 5. Memory and working recall

If the worker is given files, docs, or facts repeatedly, it should be able to:
- remember them as working sources in the task state
- recall prior relevant experiences
- reuse prior successful paths
- create compact knowledge and reflexes from repeated successful workflows

This includes remembering:
- which docs were authoritative for a task
- which file was the better source than another
- which extraction path worked
- which script or command solved a repeated pattern

## Approval Policy for Useful V1

The approval policy should be narrowed so the worker can act without constant friction.

Default v1 policy:
- allow read/navigation/extraction without approval
- allow create/modify/write/overwrite without approval
- allow normal local commands without approval
- require approval for delete/remove/destructive cleanup
- require approval for kill/termination of external processes not clearly owned by the task

This is the correct balance for the useful worker phase:
- enough freedom to actually complete tasks
- enough hard boundaries to avoid obvious destructive behavior

The harness must still log:
- writes
- modifications
- command execution
- state changes
- approvals when required

## What The Worker Should Do On Real Tasks

For tasks like:

`take page 2 of a PDF, use info from a text file, and save a filled text output`

the worker should naturally attempt a flow like:

1. locate the source files
2. extract or isolate the requested PDF page content
3. read the companion text source
4. determine the target output artifact
5. synthesize the result into a new text file
6. verify the file was written
7. report what was created

If a capability is missing, the worker should say exactly what is missing:
- page-level extraction
- OCR
- form layout understanding
- unsupported file type

It should not confuse first-step discovery with task completion.

## Capability Gaps That Must Be Closed In V1

### 1. Transformation task understanding

The reasoner and kernel need to distinguish:
- discovery tasks
- answer tasks
- transformation tasks
- output-producing tasks

Transformation tasks should not be treated like simple browse-and-read tasks.

### 2. Better document tools

The shell should gain:
- page-level PDF extraction
- stronger text extraction interfaces
- clearer extracted document result metadata
- file-type-aware read behavior

### 3. Better output workflows

The shell and kernel should better support:
- write target planning
- output artifact tracking
- multi-source synthesis
- verifying that named output files were created

### 4. Better completion judgment

The worker should not mark tasks complete when:
- it only found files
- it only listed a directory
- it only read one of several required sources
- it has not yet created the requested output

### 5. Better command freedom

The worker should be able to choose:
- structured shell actions
- raw commands
- task-local helper scripts

based on usefulness, not because the harness over-prefers narrow structured calls.

## Implementation Plan

### Phase U1: task-shape correction

Goal:
- make the worker recognize transformation and output-producing tasks correctly

Tasks:
- add task-shape hints to assembled context
- separate discovery, answer, transform, and output task intents
- penalize completion on pure discovery when output is still missing
- improve step-quality checks in the kernel

Acceptance:
- listing a directory is no longer treated as a good stopping point for transformation tasks
- tasks requesting a named output file continue until the file exists or a real blocker is surfaced

### Phase U2: document and data ingestion

Goal:
- strengthen the body for real document-driven work

Tasks:
- add page-level PDF text extraction
- add `.csv` ingestion and structured text output support
- improve extraction metadata so the task state knows what was read and how
- make working-source tracking reflect page-level sources when relevant

Acceptance:
- the worker can read the relevant page of a PDF and combine it with another local text source

### Phase U3: synthesis and output creation

Goal:
- let the worker produce useful outputs from gathered evidence

Tasks:
- strengthen multi-source synthesis prompts
- add explicit output artifact tracking in task state
- improve write flows for new `.txt` and `.md` files
- verify output creation in the shell and task state

Acceptance:
- the worker can create a requested text output from multiple local inputs
- the timeline clearly shows the produced artifact

### Phase U4: broader terminal freedom

Goal:
- remove unnecessary constraints that make the worker timid or ineffective

Tasks:
- let the worker choose raw shell commands more naturally
- let it write and run small task-local scripts when useful
- preserve output capture, verification, and cancellation
- keep delete/kill approval boundaries intact

Acceptance:
- the worker can choose commands or helper scripts when they are better than the narrow structured path

### Phase U5: memory and reusable local workflows

Goal:
- make repeated document and file workflows improve over time

Tasks:
- promote repeated successful local transforms into stronger knowledge/rules
- remember good source selection decisions
- remember effective extraction and synthesis paths
- surface these patterns in operator inspection

Acceptance:
- repeated local document workflows become more efficient and more reliable

## Operator Rules For Useful V1

The operator should mostly give intent, not implementation.

Good:
- “find the second page of this PDF, use this text file, and create a new text output”
- “read these local docs and make a summary”
- “use this CSV and create a cleaned markdown report”

Not required:
- spelling out every command
- telling the worker exactly when to `cat`, `grep`, or `awk`

Control-plane rules remain:
- `/s` and `/stop` are harness controls
- `/guide <text>` is advisory guidance for the next step
- guidance is not a substitute for hardcoded workflows

## Definition of Done For Useful V1

V1 useful worker is complete when one worker can:
- navigate the local environment
- read and extract useful information from supported local file types
- combine multiple sources into one task
- create and modify local text outputs without approval friction
- use terminal commands or helper scripts when useful
- keep exact evidence outside the prompt while preserving compact task continuity
- fail honestly when a missing capability blocks completion

At that point:
- the worker is strong enough to justify specialist expansion
- browser, desktop sight, hardware, and multi-agent routing can become the next phase

## Explicit Non-Goals For This Document

This document does not require, for v1:
- PDF layout recreation
- perfect form-filling fidelity
- browser navigation
- desktop OCR and mouse control
- multi-agent execution
- distributed transport
- Wasm fabrication

Those are important, but they come after the useful single worker.
