# U2 Document And Data Ingestion Plan

> Give the worker a stronger body for local documents and data instead of teaching it narrow tricks.

## Purpose

This plan strengthens the shell/body for real local source ingestion.

The worker should be able to read, extract, and normalize useful content from common local sources so it can act on document-driven tasks without browser or vision support.

## Research Basis

Use these docs as the governing stack:
1. [docs/plans/v1_useful_worker.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/plans/v1_useful_worker.md)
2. [docs/v1_plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/v1_plan.md)
3. [docs/trait_contracts.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/trait_contracts.md)
4. [docs/research_compaction.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_compaction.md)
5. [docs/research_memory.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_memory.md)

Research rules to preserve:
- shell is the body
- exact evidence stays outside the prompt
- compact task state tracks authoritative working sources
- missing capabilities should surface honestly

## Boundary

What this plan changes:
- shell document/data capabilities
- extracted result metadata
- working-source fidelity for documents/data

What this plan does not change:
- browser automation
- OCR desktop sight
- PDF layout recreation
- UI form filling

## V1 Source Support Target

Primary v1 sources:
- `.txt`
- `.md`
- `.json`
- `.toml`
- `.yaml`
- `.yml`
- code/config files
- `.csv`
- `.pdf`

Deferred-later sources:
- `.docx`
- `.pages`
- scanned image PDFs requiring OCR
- spreadsheets with layout-sensitive formulas

## Implementation Phases

### Phase U2.1: normalized source result model

Unify extracted source results around a more explicit model.

The shell should return enough metadata to know:
- source path
- source kind
- extraction method
- page or range if partial
- whether content was truncated
- whether structured rows/records were detected

This should improve working-source tracking and downstream synthesis.

### Phase U2.2: page-level PDF extraction

Add page-aware PDF extraction.

Required worker capability:
- extract full text from a PDF
- extract only selected pages when requested
- preserve page references in result metadata and task state

This is enough for v1 document tasks that say:
- “use page 2”
- “read the first page”
- “compare page 1 and page 3”

### Phase U2.3: CSV and simple structured data ingestion

Add explicit CSV ingestion support.

The worker should be able to:
- read rows
- summarize headers
- inspect sample records
- treat CSV as structured local evidence

V1 does not need full spreadsheet intelligence.
It does need useful row/column-aware ingestion.

### Phase U2.4: file-type aware source selection

Improve how the worker chooses between candidate sources.

The system should prefer:
- directly readable text when it is sufficient
- extracted document text when raw binary would be unhelpful
- page-level extraction when the task requests a page-specific subsource
- structured data ingestion when the source is CSV-like

This is selection quality, not hardcoded task routing.

### Phase U2.5: working-source fidelity

Working sources should preserve:
- exact path
- kind
- authoritative/supporting role
- page reference when applicable
- extraction status
- why the source matters for the task

This keeps compaction and resumption faithful to the real inputs.

## Implementation Tasks

- Extend action/result types for page-aware PDF extraction and structured CSV ingestion.
- Update shell implementations to support those actions.
- Update working-source generation to preserve page and extraction metadata.
- Improve output rendering so operators can see what was actually ingested.
- Add tests for:
  - full PDF text extraction
  - page-specific PDF extraction
  - CSV ingestion
  - mixed-source tasks with `.txt` + `.pdf` + `.csv`

## Acceptance Tests

- The worker can extract only page 2 from a PDF when asked.
- A task asking for text from a PDF page does not read raw PDF bytes.
- The worker can inspect a CSV and use it as evidence for a synthesis task.
- Working sources clearly show page-level and extraction-method details.
- The shell surfaces unsupported document cases honestly instead of pretending to succeed.

## Do Not Drift Into

- visual PDF layout recreation
- OCR or screenshot-based extraction
- browser/document viewer logic
- domain-specific hardcoded PDF handling

## Done Condition

This plan is done when the worker can treat local documents and simple data files as first-class inputs for real tasks, including page-specific PDF extraction and CSV ingestion, while keeping evidence exact and compact context small.
