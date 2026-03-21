# Harness Interpretation Gut Plan

> Record reality. Enforce a few hard loop invariants. Stop steering the model.

## Summary

This plan removes harness-authored interpretations that guide or interfere with the agent's next move.

The goal is not to gut:
- logging
- timeline/memory persistence
- concrete artifact verification
- approval/cancellation/operator controls
- bounded loop safety

The goal is to gut:
- harness-side task classification that changes the next step
- strategy hints derived from local heuristics instead of raw evidence
- success criteria or frontier wording that smuggles in a preferred route
- semantic steering added in the name of “helping” the model recover

The target end state is:
- the agent sees compact live state and raw evidence
- the harness records and verifies
- the harness does not decide what kind of task this is in order to guide execution
- the loop remains hard only on safety, approval, cancellation, anti-thrash, and final honesty

## Hard Boundary

### Keep in the harness

- Exact event logging and timeline capture
- Memory persistence, recall, and compaction artifacts
- Concrete artifact verification for writes/appends/deletes and tracked file changes
- Approval gating for delete-like and kill-like actions
- Cancellation handling
- Bounded step loop
- Minimal anti-thrash on repeated no-progress families
- Final completion honesty: do not accept fake completion without a real terminal move or grounded blocker

### Remove from the harness

- Task-type interpretation used to steer behavior
- Branching on “this is an operational task”, “this is a file task”, or similar categories for next-step guidance
- Success criteria that imply a preferred workflow rather than a truth condition
- Frontier/open-question text that tells the model how to solve the task instead of what remains unresolved
- Reflection prompts that inject strategy beyond:
  - choose a materially different action
  - or report grounded blocker/current status

## Implementation Changes

### 1. Remove steering interpretations from task shaping

- Audit [crates/retina-kernel/src/task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs) and remove any helper whose primary purpose is to classify a task in order to influence the next action.
- Keep only evidence-derived state that answers:
  - what was observed
  - what artifacts/sources exist
  - what blockers/avoid rules exist
  - whether there is a concrete output artifact that still requires verification
- Replace interpreted frontier language with minimal neutral language:
  - `latest evidence available`
  - `current blocker recorded`
  - `artifact present and unverified`
  - `authoritative source available`
- Do not generate “preferred next step” wording tied to a task class.

### 2. Strip workflow guidance out of success criteria

- Audit [crates/retina-kernel/src/execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs) success-criteria derivation.
- Keep only truth-oriented criteria:
  - completion grounded in observed evidence
  - explicit verification for produced artifacts/state changes when verification is concrete
  - verified outputs remain consistent with task goal
- Remove criteria like:
  - discovered paths should lead to inspection/read/answer
  - next step should reduce unresolved obligation in a particular way
  - system-state tasks should follow a harness-authored control shape
- Success criteria should stop being strategy hints.

### 3. Reduce reflection to loop hygiene only

- Audit reflection and reconsideration paths in:
  - [crates/retina-kernel/src/execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)
  - [crates/retina-kernel/src/task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs)
- Keep reflection reasons short and non-strategic:
  - repeated same-family action without new evidence
  - command failed
  - explicit output artifact still unverified
- Remove reflection reasons that encode preferred recovery strategies beyond:
  - do something materially different
  - or report grounded blocker/current status
- Reflection should not classify tasks or recommend domain-style actions.

### 4. Keep repeat detection, but make it evidence-only

- Keep command-family normalization and repeated-family detection in [crates/retina-kernel/src/result_helpers.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/result_helpers.rs) and [crates/retina-kernel/src/loop_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/loop_state.rs).
- Treat this as loop hygiene only.
- Repeat detection may say:
  - this is materially the same verification/control family
- It may not say:
  - therefore this is an operational task and should be solved in a harness-selected way

### 5. Keep approval-denied closure, but as operator truth only

- Preserve grounded blocker closure when approval for a stronger step is denied.
- The closure should report:
  - what was attempted
  - what remains unresolved according to latest evidence
  - which stronger step was declined
- Do not treat approval denial as a basis for further harness steering.
- This is operator control and truthfulness, not strategy.

### 6. Slim the prompt contract further if needed

- Audit [crates/retina-llm-claude/src/payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs).
- Remove any remaining workflow-preference lines that exist only because the harness wants a certain route.
- Keep prompt rules focused on:
  - use evidence
  - take one concrete next step
  - do not claim completion early
  - prefer grounded final responses
  - use tools/body rather than guessing
- The prompt may encourage good behavior, but it should not mirror harness-side task classification.

### 7. Planner audit for overlap only

- Audit [crates/retina-llm-claude/src/planner.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/planner.rs) after the above cleanup.
- Keep it tiny and limited to:
  - greetings/help/capability replies
  - obvious follow-up from already-known structured results
- Remove any planner shortcut that depends on the harness owning task interpretation.
- Do not grow planner logic to compensate for removed harness steering.

## Test Plan

### Core regression tests

- File question still works:
  - inspect/read local file
  - answer from evidence
- Output task still works:
  - read source
  - write artifact
  - verify artifact
  - respond
- Terminal control task still works:
  - check state
  - act
  - re-check
  - either respond or block

### Harness-removal regression tests

- `task_state` and frontier do not include task-class steering language.
- Success criteria contain truth conditions, not workflow preferences.
- Reflection reasons stay short and non-strategic.
- Planner follow-ups remain non-terminal unless they are actual direct replies.

### Anti-thrash regression tests

- Repeated same-family verification commands still trigger anti-thrash.
- Anti-thrash does not inject task-class-specific recovery advice.
- Denied approval still closes cleanly with a grounded blocker.

## Assumptions

- The project standard is:
  - harness interpretation for recording is acceptable
  - harness interpretation for steering is not
- The desired architecture is:
  - model-first worker
  - thin harness
  - hard loop only where necessary
- If there is a tradeoff, choose:
  - less harness semantic guidance
  - more raw evidence plus compact state
  - even if that means the model must work harder
- This plan intentionally prefers removing helpful-looking control if that control changes the agent's reasoning path.

## Done Condition

This plan is complete when Retina can still:
- record thoroughly
- verify concrete changes
- enforce approval/cancellation/completion honesty

while no longer:
- classifying tasks to guide behavior
- authoring workflow hints into task state
- nudging the agent toward harness-preferred strategies through frontier/success/reflection text

At that point, the harness is an execution-and-truth spine, not a shadow planner.
