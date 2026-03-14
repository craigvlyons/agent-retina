# U4 Terminal Freedom And Scripts Plan

> The worker should have a strong CLI body, not a tiny fenced yard of pre-approved actions.

## Purpose

This plan broadens the CLI shell so the worker can use the terminal more naturally and effectively for local tasks.

The goal is not unrestricted chaos. The goal is:
- broader command freedom
- temporary helper-script freedom
- preserved observability
- preserved safety boundaries

## Research Basis

Use these docs as the governing stack:
1. [docs/plans/v1_useful_worker.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/plans/v1_useful_worker.md)
2. [docs/plans/research-aligned-execution-plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/plans/research-aligned-execution-plan.md)
3. [docs/trait_contracts.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/trait_contracts.md)
4. [docs/v1_plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/v1_plan.md)

Research rules to preserve:
- shell is the body
- the harness should remove blockers rather than over-guide behavior
- control, verification, and safety stay in the harness
- the model should choose how to explore rather than being boxed into phrase-routed steps

## Boundary

What this plan changes:
- command selection freedom
- command execution policy
- task-local script creation and execution
- approval policy for local write/create behavior

What this plan does not change:
- network-heavy shell freedom by default
- browser or desktop UI control
- destructive unrestricted commands

## Desired Policy

Default worker policy for useful v1:
- read, inspect, search: allowed
- create, write, overwrite, append: allowed
- normal local commands: allowed
- helper scripts for local work: allowed
- delete/remove/destructive cleanup: approval required
- kill/terminate processes not clearly owned by the task: approval required

## Implementation Phases

### Phase U4.1: command selection freedom

Reduce over-bias toward narrow structured shell actions when commands are the better path.

The worker should be able to choose:
- structured action
- shell command
- helper script

based on usefulness and verifiability.

### Phase U4.2: helper-script workflow

Allow the worker to:
- create small task-local scripts
- run them
- capture their output
- preserve them as artifacts or temporary outputs as appropriate

This is important for transforms that are awkward to express as one command.

### Phase U4.3: local command/output verification

Keep broad command freedom observable by ensuring:
- stdout/stderr are captured
- exit status is captured
- duration is captured
- produced/modified files can be tracked in task state

### Phase U4.4: safer policy boundaries

Refine what requires approval.

V1 useful worker should not be constantly blocked from doing normal local work.
But it should still require approval for:
- destructive delete/remove operations
- dangerous cleanup
- uncertain process termination outside task-owned subprocesses

### Phase U4.5: command/script memory hooks

Record successful local command/script patterns so repeated tasks can improve later.

The worker should remember:
- useful command patterns
- useful script patterns
- when a script solved a repeated transform cleanly

## Implementation Tasks

- Adjust reasoner prompt guidance so shell commands are viable when better than narrow structure.
- Extend shell/action model to represent helper-script creation/execution cleanly.
- Narrow approval policy to destructive delete/kill boundaries.
- Improve task state and artifacts for command/script outputs.
- Add tests for:
  - command-chosen task solving
  - helper-script-created output
  - denied delete/kill requiring approval

## Acceptance Tests

- The worker can choose a command or small script for a local transform task.
- It can create a new file through that route and verify the result.
- It does not require approval for normal local writes and modifications.
- Delete/remove actions still require approval.
- Kill-like actions still require approval unless clearly task-owned and safe.

## Do Not Drift Into

- unbounded destructive shell freedom
- opaque command execution without result capture
- special UI escape hatches instead of agent-owned action choice
- replacing reasoning with handwritten command templates

## Done Condition

This plan is done when the CLI worker can use the terminal naturally enough to solve real local tasks, including temporary helper scripts, without constant approval friction and without losing observability or safety boundaries.
