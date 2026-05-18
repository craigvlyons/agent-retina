# Source File and Project Inspection Alignment Plan

## Purpose

This plan is the next implementation program for Retina's local file, folder, project, and code inspection surface.

The goal is to align Retina with the stronger inspection behavior already present in source code, while keeping Retina useful for non-code tasks too.

This is not a "make Retina a coding-only agent" plan.

It is a "give Retina the same strong local inspection body that source already has" plan, so the agent can:

- inspect folders and files more reliably
- understand larger projects with less guesswork
- search code and text with better tooling
- preserve what it already learned about files across follow-up turns and resume
- manipulate files and code from a stronger grounded inspection base

## Cutover Rule

For the capabilities in this plan:

1. source behavior is the design truth
2. Retina should adopt the source behavior or the closest Rust-native equivalent
3. the new path should become the only authoritative path
4. superseded Retina inspection behavior should be retired in the same patch when practical

We are not keeping old and new inspection semantics around indefinitely.

## Why This Matters

Right now Retina can:

- inspect a path
- list directories
- find files
- search text
- read files

That is enough for simple work, but it is still weaker than source for project inspection.

Source already has a stronger stack:

- dedicated `Read` semantics with targeted ranges, limits, and format-aware handling
- dedicated `Glob` and `Grep` tools with better search ergonomics
- workspace-scale search and preview workflows
- read-file state restoration from transcript history
- persistent continuity for what files were already read and how

If Retina is going to control more of the desktop, manipulate files, and sometimes understand code or project structure, this inspection substrate should be strong before we build too much more on top of it.

## Success Criteria

Retina should end this program with:

- source-style file reading behavior for normal text, large files, and targeted range reads
- source-style project discovery via filename globbing and ripgrep-backed content search
- source-style continuity for already-read files across follow-up turns, resume, and delegated work
- source-style project inspection prompts and heuristics so the model chooses the right inspection tool more consistently
- fewer shell fallbacks for jobs that should be solved by native inspection tools
- a stronger local inspection substrate that future specialists can reuse

## Current Status

Progress snapshot as of 2026-05-17:

- `done`: Phase 1 first slice landed
- `done`: `read_file` now supports `start_line` and `limit_lines` in addition to `max_bytes`
- `done`: Retina now preserves read metadata in the result path, including start line, line count, total lines, total bytes, and read bytes
- `done`: partial reads now remain canonical read-state truth instead of being dropped, which means later edits correctly require a full read
- `done`: trailing newline preservation now matches file truth closely enough for exact-edit flows to keep working
- `done`: Phase 2 first slice landed
- `done`: `find_files` now supports source-style paging semantics via `offset`, plus explicit truncation signaling in results
- `done`: `search_text` now supports `offset`, `glob`, and `case_insensitive` parameters
- `done`: `search_text` now prefers a ripgrep-backed execution path when `rg` is available, with a Rust fallback when it is not
- `done`: file-match and text-search results now carry explicit paging/truncation truth instead of only raw arrays
- `done`: Phase 2 second slice landed
- `done`: `search_text` now supports source-style `output_mode` values for `content`, `files_with_matches`, and `count`
- `done`: ripgrep-backed search now returns richer result shapes for line matches, matching filenames, and per-file counts
- `done`: operator output, compaction summaries, and mock shell/test surfaces now understand the richer `TextSearch` result contract
- `done`: prompt/tool guidance now teaches when to use `search_text` in `content`, `files_with_matches`, and `count` modes, and when to continue paged searches with `offset`
- `done`: tool catalog rendering now exposes enum choices directly, which makes mode selection more visible in the model-facing prompt
- `done`: Phase 3 first slice landed
- `done`: canonical file-read state now rides inside the continuation window instead of living only in in-process shell memory
- `done`: resumed and follow-up tasks now restore cached full vs partial read state into the shell before execution
- `done`: restored full reads can unlock exact edits, while restored partial reads still correctly block mutations
- `done`: controller-side continuation reconstruction now rebuilds read-state cache from canonical `ActionResultReceived` history when explicit continuation blobs do not carry it
- `done`: Phase 3 second slice landed
- `done`: canonical search-state cache for `find_files` and `search_text` now rides inside the continuation window alongside read state
- `done`: kernel loop state now records paged search frontier metadata from native action results instead of leaving it implicit in transcript text alone
- `done`: controller-side continuation reconstruction now rebuilds search-state cache from canonical `ActionResultReceived` history when explicit continuation blobs do not carry it
- `done`: inspect surfaces now show cached search frontier state so paged exploration can be verified after follow-up and resume
- `next`: keep broadening Phase 3 and Phase 4 toward richer search-state heuristics and stronger follow-up inspection decisions across turns

## Source Truth

Primary source files to follow:

- `/Users/macc/projects/code_source/src/tools/FileReadTool/FileReadTool.ts`
- `/Users/macc/projects/code_source/src/tools/GlobTool/GlobTool.ts`
- `/Users/macc/projects/code_source/src/tools/GrepTool/GrepTool.ts`
- `/Users/macc/projects/code_source/src/utils/readFileInRange.ts`
- `/Users/macc/projects/code_source/src/utils/queryHelpers.ts`
- `/Users/macc/projects/code_source/src/utils/fileStateCache.ts`
- `/Users/macc/projects/code_source/src/screens/REPL.tsx`
- `/Users/macc/projects/code_source/src/components/GlobalSearchDialog.tsx`

Current Retina targets:

- `/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_ops.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/read_state.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/builtins.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/executor.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs`

## Phase 1: Source-Style Read Semantics

### Goal

Replace Retina's simpler read behavior with a stronger file-reading surface modeled on source's `Read` tool.

### Source Behaviors to Port

- targeted line-range reading
- explicit offset and limit semantics
- read-size limits and token-aware limits
- better "file too large" behavior
- directory read rejection with clear errors
- better binary/device/path validation
- format-aware handling for PDFs, images, notebooks, and other structured files where appropriate
- stable path normalization before caching or continuity decisions

### Retina Work

- extend `Action::ReadFile` and its result shape in [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs)
- upgrade read execution in [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs) and [file_ops.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_ops.rs)
- move read-range logic into a dedicated reusable helper, similar in role to source's `readFileInRange`
- replace older "small fixed read blob" semantics as truth

### Outcome

Retina gets one canonical read tool path that is good enough for both ordinary file inspection and deeper project/code inspection.

## Phase 2: Source-Style Glob and Grep Surface

### Goal

Bring Retina's file discovery and content search closer to source's `Glob` and `Grep` tools.

### Source Behaviors to Port

- filename globbing with clear path scope
- ripgrep-backed content search
- truncation and pagination semantics
- explicit `head_limit` / offset style output control
- better search result shape for files vs matching lines vs counts
- path relativization where it reduces noise
- permission-aware search behavior

### Retina Work

- expand `Action::FindFiles` and `Action::SearchText` in [actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs)
- replace simple recursive scans in [file_ops.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/file_ops.rs) with a stronger search substrate
- prefer `rg` when available, with a clean fallback only when necessary
- update tool descriptors in [builtins.rs](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/builtins.rs)
- update prompt guidance in [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)

### Outcome

Retina becomes much better at answering:

- what files exist here
- what files match this pattern
- where in this project does this text appear
- what parts of this codebase seem relevant

without overusing shell commands.

## Phase 3: Read-State and Inspection Continuity

### Goal

Port source's "the agent remembers what it has already read" behavior into Retina's canonical continuity path.

### Source Behaviors to Port

- extract already-read files from transcript history
- restore read state on resume
- merge file-read state from prior turns
- keep stable normalized paths in the cache
- preserve enough metadata to know whether the model has seen the full file or only a partial/ranged view

### Retina Work

- add a canonical read-state cache substrate inspired by [fileStateCache.ts](/Users/macc/projects/code_source/src/utils/fileStateCache.ts)
- reconstruct read state from canonical transcript/tool results in the same way source uses [queryHelpers.ts](/Users/macc/projects/code_source/src/utils/queryHelpers.ts)
- thread that state through continuation assembly, follow-up turns, and delegated local agents
- retire any weaker "just reread or rediscover" behavior where read-state continuity should answer the question

### Outcome

Follow-up turns should be much better at:

- editing a file that was previously read
- knowing whether a file was fully or partially inspected
- avoiding redundant rereads when continuity already has the answer

## Phase 4: Project and Folder Inspection Heuristics

### Goal

Make Retina choose the right inspection action more like source does.

### Source Behaviors to Port

- use `Read` when content is needed
- use `Glob` when file names/patterns are needed
- use `Grep` when content search is needed
- avoid shell fallbacks when native tools already fit
- avoid replaying the same listing when the listing already answers the question

### Retina Work

- tighten prompt instructions in [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)
- tighten result shaping and anti-repetition guidance in [result_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs)
- improve observed result summaries so the model sees stronger grounded inspection output
- remove stale guidance that assumes simpler file tools

### Outcome

Retina will waste fewer turns on:

- repeated folder listings
- weak shell detours
- opening too many files too early
- guessing the wrong inspection tool

## Phase 5: Operator and Inspection UX

### Goal

Give operators a better view of inspection work and make project exploration easier to verify.

### Source Behaviors to Borrow

- clearer inspection output
- stronger search result summaries
- preview-oriented workflows for search and read results

### Retina Work

- improve render paths in [output.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs)
- improve inspect surfaces in [controller.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs)
- add better summaries for read, grep, glob, and listing results
- where practical, make project/code inspection traces easier to audit in timeline and continuation views

### Outcome

When Retina explores a project, we should be able to tell quickly:

- what it searched
- what it read
- what evidence it grounded on
- what file-state continuity it is carrying forward

## Phase 6: Specialist Reuse

### Goal

Make this inspection substrate reusable by future specialists, not only the general agent.

### Why

This matters for:

- desktop specialists that need strong local file/project understanding
- browser or automation specialists that will sometimes inspect local configs, logs, or code
- future deployed subagents on other devices

### Retina Work

- ensure the stronger read/search/cache substrate is carried through continuation and specialist inheritance
- avoid baking this only into main-thread CLI behavior
- keep the inspection substrate transportable into the multi-device specialist runtime direction

### Outcome

Retina gets one strong local inspection body that future specialists can share.

## Implementation Notes

- prefer a Rust-native equivalent when a source implementation detail is TypeScript-specific
- keep one truth path per capability
- if a stronger read/search path lands, the weaker one should stop being authoritative
- do not preserve old prompt wording that describes a weaker tool surface after the stronger one lands

## Validation Plan

Every phase should include:

1. targeted unit tests
2. integration tests where appropriate
3. at least one live validation task

Live validation tasks should include:

- top-level folder inventory
- recursive project discovery
- filename pattern matching
- content search across a project
- partial file reads followed by follow-up edits or summaries
- resume and follow-up continuity after file reads
- delegated specialist inspection tasks once the substrate is threaded through sidechains

## Definition of Done

This plan is done when:

- Retina's file/project/code inspection behavior is source-aligned enough that it is no longer meaningfully weaker for ordinary local exploration
- read/search continuity survives follow-up turns and resume
- inspection-native tools are preferred over shell fallbacks in the common cases source already handles well
- the stronger inspection substrate is available to future specialist work, not just the main thread
