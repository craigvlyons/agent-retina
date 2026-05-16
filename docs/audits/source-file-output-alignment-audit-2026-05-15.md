# Source File/Output Alignment Audit

Date: 2026-05-15

## Scope

This audit compares Retina's current file/output behavior with the `code_source` model, focusing on:

- write/edit/notebook tool contracts
- mutation validation
- mutation result shape
- operator-facing completion behavior

Reference source files:

- [FileWriteTool.ts](/Users/macc/projects/code_source/src/tools/FileWriteTool/FileWriteTool.ts)
- [FileEditTool.ts](/Users/macc/projects/code_source/src/tools/FileEditTool/FileEditTool.ts)
- [NotebookEditTool.ts](/Users/macc/projects/code_source/src/tools/NotebookEditTool/NotebookEditTool.ts)
- [prompt.ts](/Users/macc/projects/code_source/src/tools/FileWriteTool/prompt.ts)

Current Retina files reviewed:

- [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs)
- [file_edit.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_edit.rs)
- [notebook_edit.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/notebook_edit.rs)
- [text_write.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/text_write.rs)
- [builtins.rs](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/builtins.rs)
- [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)
- [execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)
- [result_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs)
- [output.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs)

## Verdict

Yes, Retina still needs an update if the goal is to behave like `code_source`.

Retina is now much closer on the mutation safety layer:

- read-before-write for existing files
- stale-write rejection
- exact edit vs overwrite separation
- notebook-specific mutation path
- atomic text writes
- better mutation metadata

But the strongest part of the source model is still not fully carried over:

**`code_source` makes the file tool and its result the authoritative truth for output tasks. Retina still lets too much semantic weight fall back onto the freeform model response.**

That is the important gap.

## What `code_source` Is Actually Doing

The source pattern is consistent:

1. validate hard at the tool boundary
2. perform the mutation through a shared write path
3. update read state immediately after mutation
4. return a rich structured result object
5. keep user-facing completion factual and narrow

Examples from source:

- `FileWriteTool` requires prior full read before overwriting an existing file and rejects stale writes
- `FileEditTool` rejects no-op edits, ambiguous replacements, stale reads, missing files, and notebook misuse
- `NotebookEditTool` validates notebook structure and cell targeting before mutation
- file tools return structured output including content, original content, and structured patch data
- prompt guidance explicitly says:
  - prefer edit for existing-file modifications
  - use write mainly for new files or full rewrites

This is a stronger model than “write succeeded, now let the agent explain what happened.”

## Alignment Status

### Aligned or mostly aligned

- Read-before-write safety is present in Retina.
- Stale mutation rejection is present in Retina.
- Notebook mutations are separated from plain text mutations.
- `edit_file` is exact-match based and rejects ambiguous single-match replacement without `replace_all=true`.
- Parent directory creation and atomic writes happen inside the shell mutation layer.
- Retina tool descriptions already push in the same direction as source:
  - `edit_file` for precise existing-file edits
  - `write_file` for new files or full rewrites

### Still drifting from source

- Mutation results are still thinner than source results.
- Final completion still depends too much on model narration after the tool succeeds.
- Validation parity is only partial.
- Notebook result/output parity is only partial.
- Operator-facing success language is still more interpretive than source.

## Findings

### 1. Tool-result authority is still weaker than source

Severity: High

In `code_source`, file-tool output is rich enough to ground the rest of the run:

- create vs update
- exact path
- full written content
- original content when relevant
- structured patch/diff

Retina currently carries useful metadata, but still lighter-weight metadata:

- mutation kind
- hashes
- changed line count
- patch summary
- preview excerpt

That helps, but it is still not the same as preserving the artifact itself as the authoritative result object.

Effect in practice:

- Retina can create the correct file
- but the model can still over-explain, over-summarize, or add lightly grounded interpretation afterward

This is exactly the class of problem shown in the notes-summary runs.

### 2. Retina still allows freeform completion to outrun the artifact

Severity: High

`code_source` keeps file-task completion tight and factual.

Retina still ends successful file tasks with a normal model `respond` step, and even with stronger artifact previews, the model can still:

- claim more than the artifact supports
- phrase inferred themes too confidently
- surface shell-like strings such as literal `$(date)` if they were written into the file

This is not mainly a write-path problem anymore.
It is a completion-authority problem.

### 3. Validation parity is not complete yet

Severity: Medium

Important source validations not fully mirrored yet:

- reject `old_string == new_string`
- explicit creation-vs-existing-file handling using empty-old-string semantics
- similar-file/path suggestions for missing edit targets
- large-file edit refusal
- content-equality fallback when mtime changes but full-read content is unchanged
- quote-normalized exact-match fallback

Retina has the core safety contract, but not the full validation envelope that makes the source feel more reliable and more self-correcting.

### 4. Notebook mutation is functional but not source-parity

Severity: Medium

Retina now has notebook editing and stale-read protection, which is good.

But source still has a richer notebook contract:

- stronger pre-mutation validation
- clearer result semantics
- explicit original/updated notebook surfaces

Retina still treats notebook success more like a generic mutation than a first-class notebook artifact result.

### 5. Tool descriptions are close, but result shaping is still the bigger gap

Severity: Medium

Retina's tool descriptions are no longer the main problem.
They already point in the right direction.

The bigger difference is that `code_source` does not stop at prompt wording.
It backs that wording with:

- strict validation
- rich structured outputs
- narrow factual success rendering

Retina still needs more of that concrete result contract.

## Required Updates

### 1. Promote file mutation results to first-class artifact truth

Retina should preserve source-style artifact payloads, not just summaries of them.

Recommended additions to the result surface:

- `final_content` for create/overwrite/append
- `original_content` when overwriting or editing existing files
- `structured_patch` for exact edits and full rewrites where practical

Primary file:

- [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs)

Flow-through files:

- [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs)
- [result_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs)
- [output.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs)

### 2. Make file-task completion derive from the tool result first

For successful typed file mutations, completion should be grounded primarily in the mutation result object.

The model can still speak, but it should be constrained by:

- saved path
- exact artifact payload
- exact patch/result metadata

Not by post-hoc narrative freedom.

Main files:

- [execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)
- [result_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs)
- [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)

### 3. Port the remaining high-value source validation behaviors

This should happen in the shell tool layer, not the kernel.

Priority ports:

- no-op edit rejection
- missing-target suggestion behavior
- large-file edit guard
- content-equality stale-read fallback
- quote-normalized matching

Main files:

- [file_edit.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_edit.rs)
- [notebook_edit.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/notebook_edit.rs)
- [read_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/read_state.rs)

### 4. Keep operator-facing success factual for file tasks

For file creation or update, success output should look more like source:

- what file changed
- whether it was created or updated
- what exact artifact was saved

Not:

- inflated thematic summaries
- loose interpretation that exceeds the artifact

Main files:

- [output.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs)
- [execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)

## What Should Not Change

Do not solve this by adding more planner logic, more task-shape interpretation, or more harness steering.

The source model is stronger because the tool layer is stronger, not because the harness explains more.

So the right direction is:

- stronger tool contracts
- richer artifact results
- tighter validation
- narrower factual completion

Not:

- more guidance prose
- more inferred task shaping
- more harness-authored explanation

## Recommended Next Pass

If Retina is going to follow the source model more literally, the next pass should be:

1. expand `ActionResult::FileWrite` toward source-style artifact payloads
2. preserve full artifact content in successful mutation results
3. preserve original content where relevant
4. preserve structured patch/diff where relevant
5. make successful file-task completion render from those results first
6. port the remaining source validation behaviors in the shell layer

## Recommended Update Order

To stay close to the source model, the safest implementation order is:

### Pass 1. Result contract parity

Change the result model first so downstream behavior has the right source of truth.

Implement:

- `final_content` for create/overwrite/append
- `original_content` when applicable
- `structured_patch` for edit-style mutations and rewrites where practical
- notebook result parity fields for original vs updated notebook content

Reason:

This is the strongest source-side pattern and it reduces drift without adding more harness logic.

### Pass 2. Completion/output tightening

After result parity exists, tighten file-task completion so it renders from the artifact result rather than from freeform restatement.

Implement:

- operator-facing file success should be factual and short
- final answer should cite created/updated path and artifact-backed summary only
- avoid “theme” expansion unless directly supported by the artifact or exact read evidence

Reason:

This addresses the current trust gap without making the harness more intrusive.

### Pass 3. Validation parity

Once result authority is fixed, port the remaining source-side validations in the shell layer.

Implement:

- reject no-op edits
- suggest similar missing paths for edit targets
- large-file edit refusal
- content-equality stale-read fallback
- quote-normalized matching

Reason:

These improve reliability, but they matter less than fixing result authority first.

## Non-Goals

The following would move Retina away from the source model and should not be used to solve this gap:

- more planner shaping
- more task-shape inference
- more harness-authored explanation of what the agent should do
- file-task-specific behavioral routing in the kernel

If a change does not strengthen tool validation, mutation result authority, or factual completion, it is probably not the right update.

## Success Criteria For Alignment

Retina will be much closer to the source model when these are true:

- a successful file create/update returns enough structured artifact data that the result object can stand on its own
- the final completion for a file task can be generated from the mutation result without adding unsupported interpretation
- edit failures explain exactly why they failed and what must be reread or corrected
- notebook edits return notebook-specific result data rather than generic mutation summaries
- prompt guidance remains light because the tool/result contract is doing the real work

## Practical Recommendation

If we only do one update next, it should be:

**promote file mutation results into source-style artifact results and make file-task completion derive from them first.**

That is the smallest change with the biggest alignment payoff.

## Bottom Line

Yes, the project should be updated.

Retina is now functionally better at file work, but it is still not using the strongest part of the source model yet.

If the goal is to behave like the better source agent, we should stop at “source-style tool/result authority” before adding anything else.
