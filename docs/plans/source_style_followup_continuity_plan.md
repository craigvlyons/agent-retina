# Source-Style Follow-Up Continuity And Artifact Reuse Plan

## Purpose

This plan ports the **specific source-code mechanisms** that make follow-up turns, persisted tool results, and resume/reconstruction behave more reliably in `code_source`.

The trigger for this plan is the current Retina failure mode:

- one chat turn validates a tool or library path
- the very next turn starts from a fresh task with no reusable continuity
- the agent re-explores or swaps stacks instead of reusing the validated path

Example:

- Turn 1: inspect `gabactl`, confirm it is built and can open Chrome
- Turn 2: user says "use the library to open chrome and search pydantic"
- Retina starts from `Task::new(...)`, loses the prior validated path, and falls into Selenium

That is not mainly a reasoning problem.
It is a **session continuity and artifact reuse** problem.

## Goal

Make Retina behave much more like source for follow-up turns:

1. adjacent chat turns continue the same canonical session by default
2. validated tool results and large outputs persist as reusable artifacts
3. replacement decisions survive resume and follow-up turns
4. subagents and specialists share the same continuity model
5. explicit user constraints like "use this library" remain sticky across the next turn

## Source Mechanisms To Port

This plan is grounded in the following source files:

- `/Users/macc/projects/code_source/src/query.ts`
- `/Users/macc/projects/code_source/src/utils/toolResultStorage.ts`
- `/Users/macc/projects/code_source/src/utils/sessionStorage.ts`
- `/Users/macc/projects/code_source/src/utils/sessionRestore.ts`
- `/Users/macc/projects/code_source/src/utils/forkedAgent.ts`
- `/Users/macc/projects/code_source/src/utils/messages.ts`

### What source is doing that Retina still lacks

#### 1. One live message/session chain

Source does not treat every user prompt like a brand new task.
In `/Users/macc/projects/code_source/src/query.ts`, each query continues from the active message chain:

- `messagesForQuery = [...getMessagesAfterCompactBoundary(messages)]`

This means follow-up turns inherit:

- tool discoveries
- persisted result references
- compacted continuity
- prior constraints already present in the conversation

#### 2. Persisted large tool results with stable replacements

In `/Users/macc/projects/code_source/src/utils/toolResultStorage.ts`, source persists large tool results to a session-local `tool-results` directory and replaces them with a stable preview/reference message.

Important functions and structures:

- `persistToolResult(...)`
- `buildLargeToolResultMessage(...)`
- `ContentReplacementState`
- `createContentReplacementState()`
- `cloneContentReplacementState(...)`
- `provisionContentReplacementState(...)`
- `reconstructContentReplacementState(...)`
- `applyToolResultBudget(...)`
- `ContentReplacementRecord`

This matters because it makes the model see the **same replacement text** every time instead of recomputing or losing the result.

#### 3. Session log persistence for replacement decisions

In `/Users/macc/projects/code_source/src/query.ts`, source records replacement decisions:

- `recordContentReplacement(...)`

In `/Users/macc/projects/code_source/src/utils/sessionStorage.ts` and `/Users/macc/projects/code_source/src/utils/sessionRestore.ts`, those decisions are restored on resume.

That is the part Retina is currently missing between adjacent turns.

#### 4. Sidechain transcripts for subagents

Source keeps sidechain transcript paths for agents in `/Users/macc/projects/code_source/src/utils/sessionStorage.ts`, and in `/Users/macc/projects/code_source/src/utils/forkedAgent.ts` it clones content replacement state into subagents.

That gives source a much cleaner shared continuity model:

- main thread has a session chain
- subagents have sidechains
- artifact replacement state is still structurally compatible

## Current Retina Gap

The key current gap is visible in these files:

- [/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/chat.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/chat.rs)
- [/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs)
- [/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs)
- [/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)

### Main issue

Every normal chat turn still becomes:

- `Task::new(root_agent_id(), task_description)`

via:

- `AgentController::spawn_task(...)`

So unless the operator explicitly uses `/resume-last` or `/resume <task_id>`, the next user message does **not** inherit:

- prior continuation window
- prior stored result refs
- prior validated path findings
- prior explicit user constraints

The runtime is transcript-first **within a task**, but not yet transcript-first **across normal follow-up turns in the same chat conversation**.

## Non-Negotiable Rules

1. Do not reintroduce a second continuity model beside the transcript-first runtime.
2. Do not add prompt-only patches where runtime/session ownership should exist.
3. Do not leave normal chat follow-ups as fresh `Task::new(...)` tasks by default.
4. Do not duplicate stored result continuity in multiple formats.
5. Keep Retina's current transcript-first cutover intact and extend it rather than bypassing it.

## Canonical End State

When this plan is complete:

1. normal chat follow-ups continue the active chat session unless the user explicitly starts fresh
2. session-local persisted result replacements survive across turns
3. resume and follow-up use the same reconstruction logic
4. delegated specialists inherit compatible artifact replacement state
5. "use this library" and similar explicit constraints remain active across the next turn

---

## Phase 1: Chat Session Continuity

### Goal

Port the source behavior where the live conversation chain is the default substrate for follow-up turns.

### Main Change

A normal chat message should no longer automatically mean:

- brand new `Task::new(...)`

Instead it should mean:

- continue the current conversation session
- seed the next task from the latest continuation window unless the user explicitly starts over

### Retina Files To Change

- [/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/chat.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/chat.rs)
- [/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs)
- [/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs)

### Implementation

1. Introduce a chat-scoped conversation state object in the CLI.
2. Track:
   - current session id
   - latest continuation window
   - latest artifact replacement state
   - active sticky execution constraints
3. Add a new follow-up task constructor, for example:
   - `Task::follow_up_from_session(...)`
4. Use that instead of `Task::new(...)` for normal adjacent turns.
5. Add an explicit operator way to start fresh, for example:
   - `/new`
   - or `/clear-session`

### Acceptance

- a follow-up prompt in the same chat starts with non-zero continuity by default
- the second turn can reuse the first turn's validated artifacts without explicit `/resume`

---

## Phase 2: Source-Style Content Replacement State

### Goal

Port source's persisted tool-result replacement model into Retina.

### Main Change

Retina already has `StoredResultLedger`, but it does not yet have source's durable **replacement decision state** across adjacent turns.

We need the source-style layer that freezes artifact replacement decisions and reuses the exact same preview/reference text.

### Source Code To Port Or Adapt

From `/Users/macc/projects/code_source/src/utils/toolResultStorage.ts`:

- `ContentReplacementState`
- `createContentReplacementState()`
- `cloneContentReplacementState(...)`
- `provisionContentReplacementState(...)`
- `reconstructContentReplacementState(...)`
- `ContentReplacementRecord`
- `persistToolResult(...)`
- `buildLargeToolResultMessage(...)`
- `applyToolResultBudget(...)`

### Retina Structure

Add a new runtime storage module, likely one of:

- `/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/session_storage.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/content_replacements.rs`

### Key Design Rule

Do **not** replace `StoredResultLedger`.
Instead:

- keep `StoredResultLedger` as the canonical result object model
- add content replacement state as the session-layer representation of what the model actually saw

### Acceptance

- once a stored result is compacted into a preview/reference, follow-up turns see the same replacement deterministically
- artifact reuse does not depend on re-deriving previews every turn

---

## Phase 3: Session Log And Reconstruction

### Goal

Port source's `recordContentReplacement(...)` and restore behavior into Retina so follow-up turns and resume use the same session memory.

### Source Code To Port Or Adapt

From:

- `/Users/macc/projects/code_source/src/query.ts`
- `/Users/macc/projects/code_source/src/utils/sessionStorage.ts`
- `/Users/macc/projects/code_source/src/utils/sessionRestore.ts`

Key behavior to carry over:

- record replacement decisions into a session log
- restore them when rebuilding active session state
- keep subagent sidechain records separate but compatible

### Retina Files To Add Or Change

- new session-log support in `retina-runtime`
- chat/controller restore path
- `/resume-last` and `/resume <task_id>` integration

Potential files:

- `/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/session_storage.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/lib.rs`

### Acceptance

- normal chat follow-up and explicit resume reconstruct from the same replacement/session records
- persisted artifact references are not "resume-only knowledge"

---

## Phase 4: Specialist Sidechains And Shared Artifact State

### Goal

Port the source pattern where subagents/specialists have sidechain transcripts and compatible replacement state.

### Source Code To Port Or Adapt

From:

- `/Users/macc/projects/code_source/src/utils/sessionStorage.ts`
- `/Users/macc/projects/code_source/src/utils/forkedAgent.ts`

Important behaviors:

- sidechain transcript path per agent
- cloned replacement state on subagent spawn
- reconstructed replacement state on resumed sidechain agents

### Retina Files To Change

- [/Users/macc/projects/personal/agent-retina/crates/retina-transport-local/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-transport-local/src/lib.rs)
- [/Users/macc/projects/personal/agent-retina/crates/retina-traits/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-traits/src/lib.rs)
- [/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/model.rs](/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/model.rs)
- session storage module added in earlier phases

### Acceptance

- spawned specialists inherit compatible artifact continuity instead of rediscovering large prior outputs
- parent and child continuity models stay source-like and structurally aligned

---

## Phase 5: Sticky Execution Constraints

### Goal

Close the last gap exposed by the `gabactl` example:

- explicit user constraints like "use this library" must remain active across the next turn

### Important Note

This part is **not** a direct one-to-one source port.
Source gives us the session and artifact substrate.
Retina still needs a small explicit policy layer for validated-path commitments.

### Retina Structure

Add a session-level constraint artifact, for example:

- `SessionConstraintArtifact`

Possible fields:

- `kind`
- `summary`
- `source_turn`
- `evidence_ref`
- `scope`
- `hardness` (`hard`, `preferred`)

Examples:

- `use_library_path`
- `prefer_gabactl`
- `do_not_switch_to_selenium_without_explaining_why`

### Retina Files To Change

- `/Users/macc/projects/personal/agent-retina/crates/retina-types`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs`

### Acceptance

- if the user says "use this library", the next turn treats that as an active constraint
- switching to a new stack requires an explicit justification rather than silent drift

---

## What To Bring Over Directly

These are the source concepts we should bring over as directly as possible:

1. Session-scoped persisted tool result directory model
2. `ContentReplacementState`
3. `ContentReplacementRecord`
4. Replacement-state reconstruction on resume
5. Sidechain transcript path pattern for specialists
6. Cloned artifact replacement state for delegated agents

## What To Adapt, Not Port Literally

Do not port these literally:

- Bun-specific file/session machinery
- GrowthBook feature-flag wiring
- analytics/logging scaffolding
- prompt-cache-specific comments and hacks
- source's exact JSONL transcript implementation

Retina should adapt the behavior into:

- Rust runtime/session storage
- transcript-first continuation window
- existing `StoredResultLedger`
- existing local specialist runtime

## Proposed Retina File Structure

### New Or Expanded Runtime Files

- `/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/session_storage.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/content_replacements.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-runtime/src/agent_sidechains.rs`

### CLI / Controller

- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/chat.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs`

### Types

- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-types/src/reasoning.rs`
- new session constraint / replacement record types as needed

### Kernel

- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs`
- `/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/lib.rs`

## Definition Of Done

This plan is complete when:

1. a follow-up chat turn no longer starts at `transcript 1, refs 0` by default after a just-completed task
2. validated tool/runtime discoveries from one turn are reusable in the next turn
3. persisted replacement decisions survive resume and follow-up equally
4. specialists can inherit compatible artifact continuity
5. user constraints like "use this library" remain active until explicitly cleared or superseded
