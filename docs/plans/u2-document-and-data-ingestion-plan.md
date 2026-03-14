# U2 Document And Data Ingestion Plan

> Give the worker a stronger body for local documents and data instead of teaching it narrow tricks.

## Purpose

This plan strengthens the shell/body for real local source ingestion.

The worker should be able to read, extract, and normalize useful content from common local sources so it can act on document-driven tasks without browser or vision support.

## Current Progress

Implemented:
- `extract_document_text` now supports page-aware PDF extraction with `page_start` / `page_end`
- document extraction results now carry richer metadata:
  - extraction method
  - optional page range
  - structured-row detection flag
- the kernel now preserves that metadata in compact result state, artifact references, summaries, and working-source fidelity
- operator output now shows page-scoped extraction and extraction method details instead of flattening all document ingestion into one generic line
- the narrow PDF follow-up helper can carry explicit page hints when the task asks for a specific page
- `ingest_structured_data` now supports CSV/TSV-style local data ingestion
- structured local sources now preserve:
  - headers
  - sample rows
  - total row count
  - extraction method
- mixed-source task state now preserves combined `.txt + .pdf + .csv` evidence
- mixed-source candidate preference now covers:
  - `.md` vs `.csv` when the task asks about rows/data
  - `.txt` vs `.pdf` when the task asks for a specific page
- regression coverage exists for:
  - full PDF extraction
  - page-specific PDF extraction
  - CSV/TSV ingestion
  - task-state / CLI output compatibility with richer source metadata
  - mixed-source working-source fidelity
  - mixed-source candidate preference

Still left in this plan:
- broaden file-type-aware source selection beyond the current CSV/TSV and page-specific PDF cases
- preserve richer structured-source evidence if real tasks require row/column-specific follow-up later
- add more end-to-end mixed-source synthesis tasks, not just ingestion/selection coverage

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
- strengthen ingestion capability without scripting how the worker must use it

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

Status:
- implemented for document extraction results in the CLI shell

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

Status:
- implemented for page-aware local PDF extraction

### Phase U2.3: CSV and simple structured data ingestion

Add explicit CSV ingestion support.

The worker should be able to:
- read rows
- summarize headers
- inspect sample records
- treat CSV as structured local evidence

V1 does not need full spreadsheet intelligence.
It does need useful row/column-aware ingestion.

Status:
- implemented for CSV/TSV-style local files with:
  - headers
  - sample rows
  - row counts
  - extraction metadata

### Phase U2.4: file-type aware source selection

Improve how the worker chooses between candidate sources.

The worker should be able to choose:
- directly readable text when it is sufficient
- extracted document text when raw binary would be unhelpful
- page-level extraction when the task requests a page-specific subsource
- structured data ingestion when the source is CSV-like
- shell commands or small local scripts when they are the best ingestion path

This is selection quality, not hardcoded task routing.

Status:
- partially implemented:
  - matched CSV/TSV candidates now prefer structured ingestion over plain text reads
  - PDFs still prefer page-aware document extraction when requested
  - mixed candidate preference tests now cover:
    - `.md` vs `.csv` when the task asks about rows/data
    - `.txt` vs `.pdf` when the task asks for a specific page
Still left:
- broaden mixed-source selection to more combinations such as `.json` vs `.txt`, `.md` vs `.pdf`, and config/code vs prose when task intent is clearer than the extension alone
- improve selection from the live task state, not only deterministic follow-up from a prior result

### Phase U2.5: working-source fidelity

Working sources should preserve:
- exact path
- kind
- authoritative/supporting role
- page reference when applicable
- extraction status
- why the source matters for the task

This keeps compaction and resumption faithful to the real inputs.

Status:
- partially implemented:
  - structured local sources now preserve:
    - headers
    - sample row count
    - total row count
  - mixed-source task-state tests now verify combined `.txt + .pdf + .csv` working-source fidelity
Still left:
- preserve richer structured evidence references only if row/column-specific follow-up proves necessary in real tasks
- improve operator-facing review of mixed-source provenance when several local inputs are combined

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
- Working sources preserve mixed `.txt + .pdf + .csv` evidence together without losing page or structured-source fidelity.
- The shell surfaces unsupported document cases honestly instead of pretending to succeed.

## Do Not Drift Into

- visual PDF layout recreation
- OCR or screenshot-based extraction
- browser/document viewer logic
- domain-specific hardcoded PDF handling
- forcing one ingestion path when the shell/body already has a better local option

## Done Condition

This plan is done when the worker can treat local documents and simple data files as first-class inputs for real tasks, including page-specific PDF extraction and CSV ingestion, while keeping evidence exact and compact context small.

## Resume Point

If we come back to `u2`, start with:
- end-to-end mixed-source synthesis tasks that ingest `.txt + .pdf + .csv` and then answer or produce an output artifact
- broader file-type-aware source selection beyond the current CSV/TSV and page-specific PDF cases
- richer structured evidence references only if real tasks show row/column follow-up is actually needed
