# Code_Source File Edit Adoption Plan

> Purpose: upgrade Retina's file mutation path to match the `code_source` agent style as closely as possible in Rust, while preserving Retina's kernel, shell, and authority model.

## Summary

Retina's current file write path is useful but too weak for high-trust agent editing.

Today Retina mainly has:
- `WriteFile`
- `AppendFile`
- shell-side direct filesystem writes
- post-hoc state verification through `HashScope`

That is enough to create or overwrite files, but it does not yet give the agent the same editing discipline as `code_source`.

The `code_source` style is stronger because it makes file mutation a structured workflow:
- read before write
- reject stale writes
- distinguish in-place edit from full overwrite
- require exact match replacement for edits
- preserve encoding and line-ending details
- keep the critical read-modify-write section tight
- produce structured mutation results
- route notebook edits through a dedicated notebook editor

Retina should copy that behavior exactly in spirit, not just roughly in shape.

## Why Retina Needs This

Current Retina mutation behavior is simple:
- `Action::WriteFile` and `Action::AppendFile` are the only text mutation actions in [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs)
- `CliShell` executes them directly in [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs#L272)
- low-level writes happen with direct filesystem calls in [file_ops.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_ops.rs#L359)

This means Retina currently lacks:
- read-before-write enforcement
- stale-read protection based on file version or content
- exact in-place replacement semantics
- notebook-specific editing
- structured diff results for edits
- edit-vs-overwrite separation in the tool model

That gap is one reason agents feel sloppier around code, text, and Markdown updates.

## What Code_Source Does Better

### Separate tools for different mutation modes

`code_source` splits mutation into:
- in-place edit: [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts#L86)
- full create/overwrite: [FileWriteTool.ts](/Users/macc/projects/code_source/src/tools/FileWriteTool/FileWriteTool.ts#L94)
- notebook cell edit: [NotebookEditTool.ts](/Users/macc/projects/code_source/src/tools/NotebookEditTool/NotebookEditTool.ts#L90)

Retina should adopt the same separation.

### Read-before-write enforcement

`code_source` rejects writes if the file has not been read first:
- [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts#L275)
- [FileWriteTool.ts](/Users/macc/projects/code_source/src/tools/FileWriteTool/FileWriteTool.ts#L153)
- [NotebookEditTool.ts](/Users/macc/projects/code_source/src/tools/NotebookEditTool/NotebookEditTool.ts#L176)

This is the single most important behavior to copy.

### Freshness and stale-write protection

`code_source` rejects edits if the file changed after it was read:
- [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts#L290)
- [FileWriteTool.ts](/Users/macc/projects/code_source/src/tools/FileWriteTool/FileWriteTool.ts#L198)
- [NotebookEditTool.ts](/Users/macc/projects/code_source/src/tools/NotebookEditTool/NotebookEditTool.ts#L234)

It uses mtime first, then compares normalized content in some cases to avoid false positives.

Retina needs the same protection.

### Exact replacement semantics

`code_source` in-place edits do not blindly rewrite whole files.
They:
- locate the exact old string
- reject if not found
- reject if ambiguous unless `replace_all` is explicitly enabled

Reference lines:
- [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts#L321)
- [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts#L332)

This is a much better editing model for code and Markdown than a generic overwrite.

### Dedicated notebook editing

`code_source` explicitly refuses to treat `.ipynb` like a normal text file:
- [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts#L270)
- [NotebookEditTool.ts](/Users/macc/projects/code_source/src/tools/NotebookEditTool/NotebookEditTool.ts#L90)

Retina should do the same.

### Shared write helper for encoding and line endings

`code_source` writes through a shared helper:
- [file.ts](/Users/macc/projects/code_source/src/utils/file.ts#L84)

That helper preserves encoding and handles LF/CRLF normalization intentionally.

Retina should centralize text writes the same way instead of writing raw strings directly through `fs::write`.

## Current Retina State

### Existing strengths

Retina already has:
- typed mutation actions in [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs#L59)
- write authority enforcement in [policy.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/policy.rs#L117)
- state snapshots and content hashing in [state_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/state_helpers.rs#L3)
- post-action verification via `HashScope` in [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs#L116)

These are good foundations.

### Current weaknesses

Retina currently writes like this:
- `write_file` overwrites with `fs::write` in [file_ops.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_ops.rs#L359)
- `append_file` appends directly in [file_ops.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_ops.rs#L375)
- `CliShell` exposes those operations directly in [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs#L272)

The missing pieces are:
- no remembered read state for a path
- no freshness check before mutation
- no in-place targeted edit action
- no notebook mutation action
- no mutation-specific structured result beyond created/overwritten/appended
- no explicit instruction to prefer structured file tools over shell edits

## Target Behavior

Retina should replicate the following `code_source` semantics.

### Rule 1: no mutation without a prior read

If a file already exists, the agent must read it before:
- editing in place
- overwriting it
- modifying a notebook cell

This should apply to:
- code
- text
- Markdown
- config
- notebook files

The only exception should be creating a brand new file.

### Rule 2: reject stale mutations

A mutation must fail if the file has changed since the read that justified the write.

Retina should track for each read:
- absolute path
- normalized content
- file version marker
- whether the read was partial or full
- read timestamp

Then before any mutation:
- compare current file version marker to the saved read
- if changed, reject
- if metadata changed but content is identical, allow only when the prior read was full

### Rule 3: separate edit from overwrite

Retina should split mutation intent into:
- in-place edit
- full overwrite/create
- append
- notebook edit

Do not keep a single generic "write" path as the dominant mutation path.

### Rule 4: in-place edits must be exact

For normal text-like files, an edit must specify:
- file path
- old string
- new string
- optional replace-all

The shell should:
- fail if the old string does not exist
- fail if it appears multiple times and replace-all is false
- only apply the replacement once unless replace-all is explicitly requested

This creates a safer edit contract for agents modifying code and docs.

### Rule 5: notebooks get their own path

Retina should introduce a notebook-specific mutation action/tool for `.ipynb`.

It should support:
- replace cell content
- insert cell
- delete cell
- code or markdown cell types

Normal text edit actions should refuse notebooks.

### Rule 6: centralize text writing

Retina should introduce one text write helper that owns:
- directory creation
- atomic write strategy
- encoding
- line ending preservation or explicit normalization
- final metadata refresh

No mutation path should write raw strings ad hoc once this helper exists.

### Rule 7: structured mutation results

Mutation actions should return structured outputs that let the kernel and CLI explain what happened.

For example:
- create vs update
- bytes written
- original content hash
- updated content hash
- changed line count
- patch summary for in-place edits

This should feed:
- timeline events
- inspect surfaces
- future diff views

## Recommended Retina Design

### New Shell-Level Read Tracking

Retina should add a read-state cache owned by the shell runtime.

Suggested Rust type:

```rust
pub struct FileReadState {
    pub path: PathBuf,
    pub normalized_content: String,
    pub version: FileVersion,
    pub was_partial: bool,
    pub read_at: DateTime<Utc>,
}

pub struct FileVersion {
    pub modified_at: Option<DateTime<Utc>>,
    pub size: Option<u64>,
    pub content_hash: Option<String>,
}
```

The shell should update this cache on:
- `ReadFile`
- `InspectPath` when full content is included
- future notebook/document reads if they expose canonical source content

### New Action Types

Retina should keep `WriteFile` and `AppendFile`, but add:

```rust
EditFile {
    id: ActionId,
    path: PathBuf,
    old_string: String,
    new_string: String,
    replace_all: bool,
}

EditNotebook {
    id: ActionId,
    notebook_path: PathBuf,
    cell_id: Option<String>,
    new_source: String,
    cell_type: Option<NotebookCellType>,
    edit_mode: NotebookEditMode,
}
```

And likely:

```rust
enum NotebookEditMode {
    Replace,
    Insert,
    Delete,
}

enum NotebookCellType {
    Code,
    Markdown,
}
```

### New Result Types

Retina should extend file mutation results to distinguish:
- exact edit
- create
- update
- append
- notebook mutation

Suggested shape:

```rust
pub struct FileMutationResult {
    pub path: PathBuf,
    pub mutation_kind: FileMutationKind,
    pub created: bool,
    pub overwritten: bool,
    pub appended: bool,
    pub bytes_written: usize,
    pub original_hash: Option<String>,
    pub updated_hash: String,
    pub patch_summary: Option<PatchSummary>,
}
```

### New Shell Helper Modules

Retina should add:
- `crates/retina-shell-cli/src/read_state.rs`
- `crates/retina-shell-cli/src/text_write.rs`
- `crates/retina-shell-cli/src/file_edit.rs`
- `crates/retina-shell-cli/src/notebook_edit.rs`

Keep `lib.rs` thin.

## Exact Implementation Changes

### 1. Track full-file reads

Update `CliShell` in [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs) to maintain:
- last read state per path
- whether that read was partial

This should live next to the current `last_command` and `notes` state, but in a dedicated module.

### 2. Introduce `EditFile`

Implement an in-place edit path modeled after:
- [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts#L137)
- [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts#L387)

Required behavior:
- normalize to absolute path
- reject notebooks
- require prior full read
- reject stale edits
- locate old string exactly
- reject ambiguous replacement unless `replace_all`
- produce patch-like summary
- write through shared text helper

### 3. Tighten `WriteFile`

Retina's existing `WriteFile` in [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs#L59) should be reinterpreted as full create/overwrite only.

Required behavior:
- if file exists, require prior full read
- reject stale writes
- preserve or explicitly normalize line endings through shared helper
- distinguish create from update in result data

This should match:
- [FileWriteTool.ts](/Users/macc/projects/code_source/src/tools/FileWriteTool/FileWriteTool.ts#L153)
- [FileWriteTool.ts](/Users/macc/projects/code_source/src/tools/FileWriteTool/FileWriteTool.ts#L223)

### 4. Keep `AppendFile`, but make it stricter

Retina may keep append as a separate mutation path, but it should become safer:
- require prior read for existing files
- reject stale append if file changed after read
- return structured append result

If that slows early progress, phase it in after `EditFile` and hardened `WriteFile`.

### 5. Add notebook editing

Implement `EditNotebook` modeled after:
- [NotebookEditTool.ts](/Users/macc/projects/code_source/src/tools/NotebookEditTool/NotebookEditTool.ts#L176)
- [NotebookEditTool.ts](/Users/macc/projects/code_source/src/tools/NotebookEditTool/NotebookEditTool.ts#L295)

Required behavior:
- require `.ipynb`
- require prior read
- reject stale writes
- replace, insert, or delete a cell
- rewrite JSON through shared text helper

### 6. Add text write helper

Replace direct writes in [file_ops.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_ops.rs#L359) with a dedicated helper.

Required behavior:
- create parent directories
- preserve encoding when reading from an existing file
- preserve or intentionally normalize line endings
- write atomically if practical
- return refreshed file version

### 7. Expose mutation semantics to the reasoner

Retina's tool descriptors currently collapse file mutation into one broad `write_file` description in [support.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/support.rs#L320).

That should become separate descriptors:
- `edit_file`
- `write_file`
- `append_file`
- `edit_notebook`

And the reasoner prompt/tool contract should teach:
- prefer exact edit for existing files
- prefer full write for new files or full rewrites
- prefer notebook edit for `.ipynb`
- avoid shell-based mutation when file tools are available

## Suggested Rust API Shape

### Shell trait

Do not bloat the core `Shell` trait unless necessary.

Keep the trait stable if possible and implement the new behavior inside:
- `Action`
- `ActionResult`
- `CliShell`

If you need extra accessors, add them carefully.

### Types crate

Update:
- [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs)

Add:
- `EditFile`
- `EditNotebook`
- richer `ActionResult::FileWrite` replacement or extension

### Shell crate

Update:
- [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs)
- [file_ops.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_ops.rs)

Add:
- read-state tracking
- exact edit path
- notebook edit path
- shared write helper

### Kernel crate

Update:
- [support.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/support.rs)
- result summarization if needed
- loop-state action labels if needed

The kernel should not own file edit policy logic.
That belongs in shell-level execution rules.

## Migration Phases

### Phase 1: harden existing `WriteFile`

Do first:
- add read-state tracking
- require read-before-overwrite
- add stale-write rejection
- centralize text writing

This gives immediate improvement with minimal prompt churn.

### Phase 2: add `EditFile`

Do second:
- exact string replacement
- ambiguity rejection
- patch summary

This is the biggest quality jump for code and Markdown editing.

### Phase 3: split reasoner tool descriptions

Do third:
- stop presenting all mutation as one generic write
- teach the reasoner when to use edit vs write vs append

### Phase 4: add `EditNotebook`

Do fourth:
- keep `.ipynb` out of plain text edit paths
- support code and markdown cell edits

### Phase 5: polish operator surfaces

Do last:
- show mutation type in timeline
- show patch summaries in inspect output
- show stale-write or unread-file failures clearly

## Acceptance Criteria

Retina should be considered aligned to `code_source` file mutation style when all of these are true:

- an existing file cannot be overwritten unless it was read first
- a stale file edit is rejected clearly
- in-place file edits use exact old-string matching
- ambiguous replacements fail unless replace-all is explicit
- `.ipynb` files no longer go through normal text edit paths
- text writing is centralized through one helper
- mutation results clearly say create, update, append, or exact edit
- the reasoner/tool surface nudges the agent toward structured mutation instead of shell hacks

## Recommendation

If the goal is to make Retina's agents as good as `code_source` at code/text/Markdown updates, the first priority is not "better prompts."

It is this:
- add read-state memory in the shell
- split edit from overwrite
- reject stale writes
- make edits exact and structured

That is the real behavioral upgrade.
