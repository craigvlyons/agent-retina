# Code Source Core Refactor Master Plan

## Purpose

This plan replaces Retina's current mixed architecture with a source-aligned core in bounded phases. The target is not "improved compatibility." The target is to make Retina feel and behave much more like `code_source` in the areas that are still structurally weaker:

- reasoner / tool contract
- MCP and search tool surface
- loop behavior after tool results
- result shaping and evidence semantics
- search semantics
- compaction

This is a development refactor plan. We are allowed to break old paths between phases. When a phase lands, the old path for that area should be deleted rather than kept as a fallback.

## Source Anchors

These source files are the main references for the plan:

- `/Users/macc/projects/code_source/src/Tool.ts`
- `/Users/macc/projects/code_source/src/query.ts`
- `/Users/macc/projects/code_source/src/services/tools/toolExecution.ts`
- `/Users/macc/projects/code_source/src/tools/AgentTool/prompt.ts`
- `/Users/macc/projects/code_source/src/tools/AgentTool/built-in/generalPurposeAgent.ts`
- `/Users/macc/projects/code_source/src/utils/messages.ts`
- `/Users/macc/projects/code_source/src/utils/toolResultStorage.ts`
- `/Users/macc/projects/code_source/src/utils/attachments.ts`

## Current Gap Summary

Retina is now functionally better than it was, especially around Brave MCP, but the remaining misses are mostly structural:

- the model still reasons through a translated action layer more than a tool-native surface
- search results are not yet first-class evidence in the loop the way they are in source
- post-search behavior still relies too much on prompt nudges instead of stronger tool/result semantics
- compaction is still task-state-centric instead of transcript/message-centric
- old wrapper-era concepts still exist in code even when they are no longer the desired path

## Refactor Rules

These rules apply to every phase:

1. No backward-compatible parallel paths once the new path is proven.
2. Delete obsolete wrappers, prompts, labels, and policy branches in the same phase that replaces them.
3. Prefer stronger semantics over more prompt patching.
4. Prefer first-class tool/result objects over special-case string parsing.
5. Keep changes bounded by phase, but do not leave "temporary" compatibility shims behind.

## End State

At the end of this program, Retina should have:

- a tool-native reasoner contract
- first-class concrete MCP tools
- search and MCP results that behave like normal evidence objects in the loop
- a cleaner post-tool turn model: answer, reformulate, or state limitation
- fewer shell-first habits for search and web tasks
- transcript-aware compaction with explicit boundaries and durable tool-result handling

## Phase Order

The recommended order is:

1. Reasoner / Tool Contract
2. MCP and Search Tool Surface
3. Loop and Turn Semantics
4. Result Shaping and Evidence Model
5. Search Semantics and Query Reformulation
6. Compaction and Transcript Architecture

The order matters. Later phases should build on the earlier cutovers instead of compensating for them.

---

## Phase 1: Reasoner / Tool Contract Cutover

### Goal

Replace Retina's action-first reasoning contract with a more source-like tool-first contract.

In `code_source`, the model sees a real tool world. `Tool.ts`, `toolExecution.ts`, and `query.ts` are built around concrete tools, tool uses, and tool results, not around an application-specific action schema that then needs heavy interpretation later.

### Problems To Remove

- generic action-schema thinking as the primary contract
- prompt dependence to explain basic tool calling shape
- dual concepts where the model "thinks action" but the runtime "executes tool"
- parser fallback behavior that guesses missing fields or silently translates malformed output

### Main Changes

1. Re-center the provider contract around concrete tools and tool inputs.
2. Reduce the Claude-facing action schema to the minimal translation layer needed by the provider.
3. Make tool names, schemas, and result pairing the primary mental model in prompts and parser code.
4. Remove old wrapper language from normal prompts and CLI rendering once concrete tools are the default.

### Retina Areas To Touch

- `/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/response.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/builtins.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/policy.rs`

### Deletions After Cutover

- stale wrapper-oriented prompt guidance
- old "generic MCP wrapper is the default" assumptions
- any parser defaults that still invent missing required fields

### Acceptance Criteria

- the model sees concrete tools as the normal path
- malformed tool actions fail honestly
- prompts no longer need repeated reminders about which abstraction layer to use

---

## Phase 2: MCP and Search Tool Surface Cutover

### Goal

Finish the move from wrapper-era MCP behavior to a source-like first-class tool surface.

In `code_source`, MCP tools fit into the normal tool system. The goal is for Retina to behave the same way consistently, not just in the happy path.

### Problems To Remove

- lingering `mcp_call` era assumptions
- resource/file/tool confusion around MCP outputs
- provider or policy branches that still privilege wrapper naming over concrete MCP tools
- search behavior that still feels like "special MCP handling" instead of normal tool usage

### Main Changes

1. Make concrete MCP tools the only normal MCP execution path when the server exposes tools.
2. Keep resource handling explicit and separate, not mixed into file-style flows.
3. Standardize MCP tool identity, schema rendering, labels, and result metadata across the kernel and CLI.
4. Ensure new MCP servers can promote dynamically without server-specific code.

### Retina Areas To Touch

- `/Users/macc/projects/personal/agent-retina/crates/retina-mcp-client/src/lib.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/builtins.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/policy.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs`

### Deletions After Cutover

- generic `mcp_call` as a normal surfaced path for tool-capable MCP servers
- any shell or file tool route that can still masquerade as valid MCP access
- MCP-specific rendering branches that reflect the old wrapper identity

### Acceptance Criteria

- MCP tool servers surface as concrete tools automatically
- MCP resource servers remain explicit resource flows
- the agent no longer confuses MCP tool outputs with file paths

---

## Phase 3: Loop and Turn Semantics Refactor

### Goal

Make the turn loop behave more like `code_source` after a tool result arrives: the next move should be grounded in evidence, not drift.

`code_source/src/query.ts` and `toolExecution.ts` are better because tool results are first-class turn artifacts, not just loosely summarized text that the next prompt has to reinterpret.

### Problems To Remove

- repeated identical search steps without reformulation
- over-reliance on prompt admonitions to stop repetition
- weak transition rules after search or MCP results
- inconsistent "answer vs reformulate vs limitation" behavior

### Main Changes

1. Treat tool-result continuation as a stricter loop transition problem.
2. Introduce stronger internal categories for next-step decisions:
   - answer from grounded results
   - reformulate search
   - gather one more missing fact
   - state limitation honestly
3. Make repeated-result detection compare evidence identity, not just action text.
4. Keep failure and blocked steps visible to the next turn as structured state, not only display text.

### Retina Areas To Touch

- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/support.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs`

### Deletions After Cutover

- ad hoc repetition rules that exist only in prompt text
- stale step-label heuristics used to compensate for weak loop memory
- generic "just try again" continuation behavior for search-heavy tasks

### Acceptance Criteria

- repeated portal hits lead to reformulation or limitation, not the same search again
- post-tool behavior is visibly more stable without relying on more prompt text
- blocked steps are structurally meaningful in the loop

---

## Phase 4: Result Shaping and Evidence Model Refactor

### Goal

Promote results into a stronger evidence model that looks more like source message and tool-result handling.

`code_source/src/utils/toolResultStorage.ts` and `src/utils/messages.ts` show a more mature model: results are durable, referenceable, and shaped for the transcript rather than treated as raw strings everywhere.

### Problems To Remove

- over-stringified tool results
- search results that lose structure too early
- inconsistent distinctions between evidence, summaries, previews, and UI render text
- prompt dependence to infer whether a result is a portal, a listing, a detail page, a failure, or a limitation

### Main Changes

1. Introduce clearer internal result classes for:
   - search results
   - local listings
   - extracted document results
   - MCP tool outputs
   - persisted large outputs
2. Preserve structured metadata longer in the loop.
3. Separate:
   - raw tool output
   - evidence summary
   - user-facing rendered summary
4. Build search-aware result summaries that expose signal like:
   - result type
   - likely portal vs specific event/article/page
   - snippet confidence
   - distinct result IDs or URLs already seen

### Retina Areas To Touch

- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/support.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs`

### Deletions After Cutover

- result shaping that collapses structured search outputs into thin strings too early
- duplicated result-summary logic split across CLI, kernel, and provider prompt text

### Acceptance Criteria

- evidence identity survives across turns
- repeated-result detection can key on URLs or result IDs
- the model can distinguish "I found a portal" from "I found concrete event listings"

---

## Phase 5: Search Semantics and Query Reformulation Refactor

### Goal

Make search behave like a first-class semantic tool family instead of a raw tool call that prompt rules must babysit.

This is where we get closer to source behavior without inventing city- or task-specific hacks.

### Problems To Remove

- generic answers from generic portal hits
- weak query reformulation after low-signal search results
- wrong tool choice between web, local, and news search
- too much free-form reasoning about search quality in prompt text alone

### Main Changes

1. Define explicit search semantics in the tool/result layer:
   - broad discovery search
   - local search
   - news/current-events search
   - follow-up narrowing search
2. Teach the loop to recognize when the first result is insufficiently specific.
3. Add structured search outcome tags such as:
   - `generic_portal`
   - `specific_listing`
   - `news_roundup`
   - `single_event`
   - `no_local_signal`
4. Make reformulation a structural next step when the result class is low-signal.

### Retina Areas To Touch

- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/builtins.rs`

### Deletions After Cutover

- query-quality logic expressed only as natural-language warnings in prompts
- shell-first web habits as a backup mental model for current-information tasks

### Acceptance Criteria

- current-events queries either return concrete observed events or explicitly state the evidence limitation
- low-signal search results trigger reformulation instead of filler
- venue, neighborhood, or attraction filler is not introduced unless present in observed results

---

## Phase 6: Compaction and Transcript Architecture Refactor

### Goal

Move Retina compaction closer to source by making it transcript-aware and message-aware instead of mainly task-state-aware.

This is the largest and most architectural phase. `code_source/src/query.ts`, `src/utils/messages.ts`, `src/utils/toolResultStorage.ts`, and `src/utils/attachments.ts` together show a fuller compaction system:

- compact boundaries
- microcompaction
- persisted tool results
- attachment re-announcement
- transcript-aware restoration after compaction

### Problems To Remove

- compaction centered mostly on shrinking task state
- weak compact boundaries
- evidence loss after long tool-heavy turns
- attachment, MCP, and tool-result continuity that depends too much on rebuilt prompts

### Main Changes

1. Introduce explicit compact-boundary message/state concepts.
2. Separate full transcript, compacted transcript, and active continuation window.
3. Add microcompaction for oversized tool results instead of only coarse task-state trimming.
4. Persist and re-reference large results in a source-like way.
5. Re-announce only the minimum needed post-compact context:
   - active tools
   - relevant attachments
   - surviving evidence references
   - current task objective
6. Make compaction decisions operate on message/evidence units, not just ranked state items.

### Retina Areas To Touch

- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/support.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs`
- any new transcript/message storage modules needed to break compaction out of the current loop-state model

### Deletions After Cutover

- old task-state-only compaction ranking as the primary mechanism
- one-off compact prompt band-aids that exist only to restore lost context
- result loss that forces the model to re-search or re-read after compaction

### Acceptance Criteria

- long search-heavy and document-heavy runs retain evidence continuity after compaction
- compact boundaries are explicit in internal state
- large tool outputs survive through stable references instead of repeated re-ingestion

---

## Cross-Phase Cleanup List

These removals should happen progressively as their replacements land:

- obsolete wrapper vocabulary around MCP
- shell-first search assumptions
- parser guesses for missing required fields
- prompt-only repetition controls that compensate for weak loop state
- duplicated result-shaping logic across provider, kernel, and CLI
- compatibility branches that exist only to support old tool naming or old evidence forms

## Suggested Execution Style

Because we are in development and do not need backward compatibility, each phase should be executed as a real cutover:

1. implement the new path
2. migrate tests
3. rerun real task probes
4. delete the old path in the same PR or immediately after validation

Do not leave the repo in a state where both the new and old architectures are first-class for long.

## Validation Matrix

Each phase should be validated with live tasks, not just unit tests.

### Search / MCP probes

- "search the web for what is going on in denver this weekend"
- "search the web for what is happening in colorado springs this weekend"
- "get me a date idea for colorado springs"
- "search web for the latest local event roundup"

### Local evidence probes

- "research all txt files on desktop and summarize what is in them"
- "there is a folder on desktop called resume check for a pdf and tell me what is in the pdf"

### Long-turn / compaction probes

- repeated multi-search tasks with compaction forced
- mixed local + web research tasks
- document extraction followed by synthesis after compaction

## Recommendation

This should be treated as one coordinated refactor program with six phases, not as another sequence of isolated prompt patches.

The highest-value first move is Phase 1 through Phase 3 as one push:

- reasoner / tool contract
- MCP and search tool surface
- loop semantics

That cluster should remove most of the "we keep fighting the structure" feeling. After that, result shaping and compaction can be rebuilt on top of a cleaner core instead of compensating for it.
