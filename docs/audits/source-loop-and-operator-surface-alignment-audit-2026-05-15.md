# Source Loop And Operator Surface Alignment Audit

Date: 2026-05-15

## Scope

This audit focuses on the part of Retina that still feels less like `code_source`:

- the live loop surface
- chat/status rendering
- prompt-fed task continuity
- blocker/frontier interpretation

This is not another file-tool audit.
The file/output layer is already much closer.
This audit is about the harness behavior that still sits on top of the agent and over-explains what is happening.

Reference source files:

- [query.ts](/Users/macc/projects/code_source/src/query.ts)
- [toolExecution.ts](/Users/macc/projects/code_source/src/services/tools/toolExecution.ts)
- [Tool.ts](/Users/macc/projects/code_source/src/Tool.ts)

Current Retina files reviewed:

- [output.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs)
- [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/lib.rs)
- [execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)
- [loop_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs)
- [task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs)
- [task_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- [reasoning.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/reasoning.rs)
- [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)

## Verdict

Yes, Retina still needs another cleanup pass to get closer to the source model.

The remaining drift is no longer mainly in file tools.
It is in the **live interpretation layer**:

- blocker/frontier narration
- prompt-fed loop summaries
- operator-facing status synthesis

`code_source` is lighter here.
It records a lot, but it largely lets:

1. tool results
2. the next model turn
3. the transcript

carry the state forward.

Retina still adds more harness-authored interpretation than the source appears to need.

## What The Source Model Is Actually Doing

From the source files reviewed:

- tool success becomes a `tool_result`
- tool failure becomes a `tool_result` with `is_error: true`
- the next turn reasons from that latest tool result
- the system does not appear to maintain a Retina-style live `frontier.blockers` UI that keeps re-surfacing older misses

Important pattern:

**source records errors strongly, but it does not keep narrating them as the current blocker once newer tool evidence exists.**

That is the core difference.

## Current Retina Drift

### 1. `task_state` still carries too much harness-authored interpretation

Current `TaskState` includes:

- `frontier.blockers`
- `avoid`
- recent action commentary
- working sources
- artifact references

The useful parts are:

- goal
- progress
- working sources
- artifact references

The drift is:

- `frontier.blockers` is mostly harness-authored interpretation layered on top of prior failures
- `avoid` is internal loop control leaking into live model-facing state

This is especially visible in:

- [task_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- [task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs)

### 2. CLI chat rendering still derives too much from interpreted task state instead of direct result flow

Retina currently renders live chat by looking at `TaskStepCompleted` and then deciding whether to print:

- `blocked: ...`
- `observed: ...`

from the assembled `task_state`.

Even after recent fixes, this is still more interpretive than source.

Source is closer to:

- plan
- action
- tool result

Retina is closer to:

- plan
- action
- interpreted synthesized current status

That extra synthesis is the part to remove.

Main file:

- [output.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs)

### 3. Repeat-protection and loop-hardening state still leaks into the user/model layer

Internal loop hardness is fine.

The drift happens when internal safety state becomes part of the live conversational thread:

- `avoid repeating ... because ...`
- blocker-style continuity
- frontier-as-explanation

Source seems to keep this distinction cleaner:

- internal retry/recovery policy exists
- transcript-facing tool results stay closer to the raw action/result stream

Main files:

- [loop_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs)
- [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)

### 4. `task_shape.rs` no longer adds much value

Right now [task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs) is extremely thin:

- `describe_task_phase`
- `build_task_frontier` from `avoid_rules`

That is a sign it should probably not survive as a separate shaping layer.

It is not buying us much.
It still encourages the architecture to think in “frontier/blocker synthesis” terms.

## Clean Target

If Retina is going to be closer to source, the target should be:

### Keep

- timeline recording
- raw action/result events
- shell/tool validation
- compaction
- memory persistence
- internal repeat protection

### Reduce or remove

- live frontier/blocker narration
- prompt-fed avoid rules
- task-state commentary that restates tool results
- any user-facing status derived from old failures when newer result evidence exists

### Make canonical

- latest tool result
- latest authoritative working source
- latest artifact result

## One Clean Implementation Path

This should be implemented as a forward cleanup, not a compatibility layer.

### Step 1. Remove `frontier.blockers` from active prompt-fed continuity

Goal:

Stop telling the model what the harness thinks the blocker is.

Implementation:

- remove `frontier` from [TaskState](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- delete [task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs)
- stop building blocker lists from `avoid_rules`
- stop rendering `Blockers:` in `TaskState::render()`

Replacement:

- rely on:
  - latest recent action status
  - latest working source
  - latest artifact reference
  - last result JSON/summary if still needed internally

No backward compatibility:

- do not keep `frontier` around as a deprecated field
- remove it cleanly from the live state schema and prompt rendering

### Step 2. Remove `avoid` from the model-facing task state

Goal:

Keep repeat protection internal.

Implementation:

- keep `avoid_rules` inside [TaskLoopState](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs) if needed for repetition control
- remove `avoid` from [TaskState](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- stop rendering `Avoid:` in prompt-fed state

Replacement:

- internal loop protection still uses repeat signatures and avoid state
- the model sees only actual recent actions/results, not harness “don’t do this” prose

No backward compatibility:

- do not preserve `avoid` in the prompt contract “just in case”

### Step 3. Rework chat rendering to follow direct event/result flow

Goal:

Make chat look like source:

- plan
- action
- tool result
- done

Implementation:

- stop deriving `TaskStepCompleted` chat output from interpreted `task_state`
- instead, render from direct action/result event payloads
- `ActionResultReceived` should become the main source of live `observed:` lines
- `TaskStepCompleted` should be minimal or omitted in normal chat mode

Main file:

- [output.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/output.rs)

Replacement:

- use raw result rendering:
  - file read preview
  - file write preview
  - document extract preview
  - command stdout/stderr preview
  - explicit tool error when present

No backward compatibility:

- do not keep both “interpreted step summary” and “direct result summary”
- choose one path
- the source-aligned choice is direct result flow

### Step 4. Keep task completion narrow and result-driven

Goal:

Retain the good part we already added.

Implementation:

- keep tool-authored file completion
- keep explicit `respond` for non-file tasks
- do not add a second generalized completion interpreter

Main files:

- [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/lib.rs)
- [support.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/support.rs)

No backward compatibility:

- if any older “respond is always required” assumptions remain in tests or surfaces, update them

### Step 5. Shrink `TaskState` to source-like continuity

Target fields:

- `goal`
- `progress`
- `recent_actions`
- `working_sources`
- `artifact_references`
- `compaction`

Optional:

- `recent_context` in assembled context can remain

Remove:

- `frontier`
- `avoid`

Main files:

- [task_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- [reasoning.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/reasoning.rs)
- [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)

### Step 6. Keep recording richer than prompting

Goal:

Do not lose observability while simplifying the active loop.

Implementation:

- keep timeline events as-is or richer
- keep full debug/introspection surfaces
- keep memory recording
- only shrink the prompt-fed and chat-fed interpretation layer

This is the clean separation that the source seems to follow better:

- record everything
- feed less
- interpret less

## What To Delete Instead Of Preserving

To stay honest about “one clean implementation”:

- delete `TaskFrontier`
- delete `build_task_frontier`
- delete prompt rendering of blockers
- delete prompt rendering of avoid rules
- delete the normal-chat dependency on interpreted `TaskStepCompleted`

Do not:

- keep deprecated frontier fields
- keep compatibility render paths
- keep dual operator surfaces for old vs new status logic

## Acceptance Criteria

Retina is closer to source when:

1. an early failed read does not keep surfacing as the current blocker after later successful steps
2. normal chat output follows action/result flow rather than interpreted frontier flow
3. the model no longer sees harness-authored blocker/avoid prose in its active task state
4. repeat protection still works internally
5. debug/timeline inspection still shows full failure history

## Recommended Order

1. remove `frontier` from prompt-fed task state
2. remove `avoid` from prompt-fed task state
3. delete `task_shape.rs`
4. switch chat rendering to direct result flow
5. update tests to the new one-way behavior

## Bottom Line

Yes, Retina should move closer to the source here.

The next cleanup should not add more nuanced blocker handling.
It should remove the blocker/frontier interpretation layer from the live loop and let:

- tool results
- artifact results
- recent actions

carry the state instead.
