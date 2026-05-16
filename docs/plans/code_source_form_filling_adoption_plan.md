# Code_Source Form Filling Adoption Plan

> Purpose: add source-aligned form and template filling to Retina as a separate capability area, instead of trying to stretch generic PDF extraction plus file writing into a full form system.

## Summary

Retina's current document flow is now good enough for:
- finding documents
- extracting document text
- summarizing local PDFs
- combining local evidence into new files

It is **not** yet good enough for real form or template filling.

The recent Dominican-template run showed the exact gap:
- Retina extracted the template PDF
- read the supporting note file
- then wrote a plausible prose document
- but it did **not** preserve the template structure
- did **not** resolve role assignment correctly
- did **not** behave like a true form-filling worker

That is expected, because Retina does not yet have the source-style form system.

The `code_source` direction for forms is not:
- generic PDF-to-text
- generic summarize-and-write
- freeform prose synthesis

It is:
- a specialist form worker
- structured document extraction
- observed field mapping
- cautious fill behavior
- user-visible grounding
- hard boundaries around what can and cannot be submitted or changed

Retina should follow that direction.

## Why This Needs Its Own Plan

Retina's current stack can already do:
- [extract_document_text](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/executor.rs)
- [read_file](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/executor.rs)
- [write_file](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/executor.rs)
- [edit_file](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/executor.rs)

That is enough for:
- summaries
- basic transformations
- derived local text artifacts

It is not enough for:
- template-preserving output
- field-aware filling
- signer/subject or applicant/provider role mapping
- document-to-form mapping
- browser or DOM-backed form completion later

So this should not be treated like "one more prompt tweak."

## Source-Aligned Direction

The project research already points toward the right shape:
- browser specialist with DOM and form authority in [architecture.md](/Users/macc/projects/personal/agent-retina/docs/architecture.md)
- Chrome-extension form worker model in [research_overview.md](/Users/macc/projects/personal/agent-retina/docs/research_overview.md)

Important source-aligned ideas:
- the form worker is a **specialist**
- it reads supporting documents
- it observes the form or template structure
- it maps extracted facts into target fields
- it highlights or reports what it filled and why
- it does not rely on generic prose rewrite as the main fill path

This means Retina should move toward:
- a distinct form/document specialist
- richer document structure handling
- field-oriented outputs
- eventually a browser shell for DOM-backed forms

Not:
- more ad hoc prompt steering in the general worker
- hardcoded task-specific template hacks
- generic PDF text pasted into a prose file

## Current Gap

Retina currently treats many template tasks like:
1. extract document text
2. read local notes
3. synthesize a new text artifact

That is why the Dominican result came out as a plausible informational document instead of a properly filled medical/licensing form.

Specific weaknesses shown by the run:
- template layout was not preserved
- field boundaries were not preserved
- multiple roles in the note were not mapped correctly
- output semantics drifted into freeform prose

## Target Behavior

Retina should support a stronger form/document flow with these rules.

### Rule 1: distinguish form filling from generic synthesis

When the task is clearly:
- use this template
- fill this form
- populate this document
- map this note into this form

the specialist should treat the work as a **field/template task**, not a generic summary/output task.

### Rule 2: preserve the target artifact structure

If the target is a template:
- preserve section structure
- preserve visible field order where possible
- prefer field/value output over prose rewrite

Retina should not collapse a form into a generic narrative unless the user explicitly asked for a prose summary instead of a filled artifact.

### Rule 3: resolve roles before filling

For form tasks with multiple named people, the worker must determine role mapping before writing:
- applicant vs certifier
- patient vs doctor
- provider vs participant
- signer vs subject

The source direction here is:
- resolve from local evidence first
- if still unresolved in live chat, ask narrowly
- if unattended later, record the grounded assumption

This should stay model-led and evidence-led, not hardcoded to specific names or domains.

### Rule 4: keep filled output grounded

Each filled value should come from:
- source document text
- local note text
- explicit user instruction
- or a surfaced grounded assumption

The worker should prefer:
- literal extracted values
- template labels
- structured field/value output

Over:
- broad explanatory paraphrase
- invented section prose
- inferred policy/process text not present in the source

### Rule 5: separate extraction from mapping

Retina should eventually represent these as distinct conceptual steps:
- document/template extraction
- evidence extraction
- role mapping
- field mapping
- output rendering

The current generic document flow compresses these together too much.

## Recommended Architecture

### Phase 1: local text-template form support

Start without browser automation.

Build a specialist path that can:
- read local text/PDF templates
- extract target sections or labels
- map local evidence into structured field/value output
- render a grounded filled text artifact

This is the first useful step and should stay compatible with the current CLI/body.

### Phase 2: richer template representation

Add a better internal representation for template-like artifacts:
- section headings
- labeled fields
- ordered blocks
- maybe line-oriented placeholders

The goal is to make the output renderer preserve form shape better than today's generic prose synthesis.

### Phase 3: browser/form specialist

Follow the research direction:
- separate browser shell
- DOM and form authority
- read labels and validation state
- fill fields
- never submit automatically

This should be a specialist/body addition, not a kernel rewrite.

## Implementation Areas

### 1. Specialist definition

Add a dedicated form/document specialist definition aligned with the source direction:
- scoped toward template/document tasks
- grounded fill behavior
- strong role-mapping caution

Likely home:
- [specialists.rs](/Users/macc/projects/personal/agent-retina/crates/retina-transport-local/src/specialists.rs)

### 2. Tool/result semantics

Retina likely needs richer document/template semantics than plain extracted text.

Potential additions later:
- labeled section extraction
- field-oriented document parse results
- template block or placeholder extraction

These should be generic document tools/results, not Dominican-specific code.

### 3. Prompt contract

Update the specialist and/or reasoner contract so form/template tasks:
- preserve structure
- resolve roles before writing
- prefer field/value output
- avoid prose drift

This should remain generic:
- no template-name routing
- no hardcoded document phrases

### 4. Browser shell later

When Retina grows into real browser form filling, align with:
- [architecture.md](/Users/macc/projects/personal/agent-retina/docs/architecture.md)
- [research_overview.md](/Users/macc/projects/personal/agent-retina/docs/research_overview.md)

Likely future crate:
- `crates/retina-shell-browser`

## Non-Goals

This plan is **not**:
- a Dominican Republic template hack
- a one-off medical form handler
- more generic write-flow steering
- kernel-level task-specific routing

It is also not yet:
- OCR-heavy vision form filling
- browser submission automation
- fully layout-faithful PDF editing

## Test Plan

### 1. Template-preserving local form fill

Input:
- local template PDF or text form
- local notes with named people and facts

Expected:
- output preserves template/field structure better than prose rewrite
- role assignment is explicit and grounded

### 2. Two-person role mapping

Input:
- source notes with two named people
- one is signer/certifier
- one is subject/applicant

Expected:
- worker either resolves correctly from evidence
- or asks one narrow question in live chat if still unresolved

### 3. No prose drift

Input:
- form/template fill task

Expected:
- output is field/template-oriented
- not a generic explanatory summary

### 4. Regression on normal document tasks

Normal PDF summary tasks should still work:
- extract one PDF
- summarize what it is about

This plan should improve form/document filling without regressing simple document summarization.

## Success Criteria

This feature area is in a good first source-aligned state when Retina can:
- distinguish form/template fill from generic document summary
- preserve template structure in the output better than freeform prose
- map multi-person source notes into the correct roles before writing
- produce grounded filled artifacts from local evidence
- do all of that without hardcoded domain-specific template logic

## Current Recommendation

Do **not** fold this into the current generic file-output refinements.

Treat it as the next major document capability after:
- local worker stabilization
- external web research via Brave/MCP

That sequencing keeps the project clean:
- Brave/MCP next as a new capability layer
- source-aligned form/document filling after that as a dedicated feature area
