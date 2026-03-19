# Mixed Overlap Audit — 2026-03-19

## Summary

Retina is in a transitional but much healthier state.

The latest live run shows the worker behaving well at the loop level:
- read the existing target file
- overwrite it
- verify by reading it back
- respond with a grounded completion message

That is the right overall shape for the current single worker.

At the same time, the implementation is still mixed across layers:
- some strategy lives in the prompt
- some strategy lives in kernel task-shaping
- some strategy lives in the deterministic planner
- some continuity fields are pure state, while others are “state plus advice”

So yes: the system is functioning better, but the responsibilities are still somewhat blended.

## Evidence From The Latest Run

Latest successful task:
- task id: `6e966ae3-a9ad-42c5-a9cc-a81e574bfc87`

Observed sequence from the timeline:
1. `read_file:/Users/macc/Desktop/emily_wittenberge_do.txt`
2. `write_file:/Users/macc/Desktop/emily_wittenberge_do.txt`
3. `read_file:/Users/macc/Desktop/emily_wittenberge_do.txt`
4. `respond`

Important observations:
- the reasoner marked `task_complete=true` even on the first read and the write step
- the kernel did not let those intermediate steps end the task
- the file write verified correctly
- the agent stayed on task and completed successfully

This is a strong sign that:
- loop hardness is improved
- file verification is improved
- continuation is improved

## What Is Clean And Research-Aligned

These parts are in the right general place:

### 1. Hard loop behavior in the kernel

Files:
- [execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)
- [task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs)

Good:
- bounded step loop
- state capture before and after actions
- verification and state-delta handling
- reflection path
- non-terminal actions no longer end tasks too early

This is the kind of “hard loop, light harness” behavior the research wants.

### 2. Pull-based memory and layered memory shape

Files:
- [memory_layers.md](/Users/macc/projects/personal/agent-retina/docs/memory_layers.md)
- [execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)

Good:
- live task state is compact
- experiences and knowledge are recalled in small slices
- the timeline remains the source of truth

This is aligned with the memory research and should remain shared across future specialists.

### 3. Output verification via the shell/body

Files:
- [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs)
- [execution.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/execution.rs)

Good:
- write success is not trusted blindly
- path tracking and alias resolution are now much better
- post-action verification is concrete and observable

This belongs in the shell/body plus loop, not in agent-specific prompting.

## Where The Layers Are Still Mixed

### 1. Output-task strategy is split between prompt and kernel

Files:
- [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)
- [task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs)

Current state:
- prompt says to prefer `write_file` / `append_file`, prefer overwrite on updates, prefer completion moves when authoritative evidence exists
- kernel task-shape also computes `remaining_obligation`, `pending_deliverable`, `target_output_path`, and `should_reconsider_low_value_action`

Why this is mixed:
- both layers are steering the same behavior
- the prompt is strategy
- the kernel fields are partly continuity and partly strategy

Assessment:
- acceptable for the current worker
- not ideal as a long-term shared base for specialists

### 2. Task state is partly state and partly inferred intent

Files:
- [task_state.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/task_state.rs)
- [task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs)

Current state:
- `remaining_obligation`
- `pending_deliverable`
- `target_output_path`

These are useful, but they are not all the same kind of thing:
- `target_output_path` is an inferred artifact target
- `pending_deliverable` is a compact planning hint
- `remaining_obligation` is essentially a synthesized next-goal statement

Why this is mixed:
- it makes task state more helpful
- but it also means task state is not only “what is true now”
- it is also “what the kernel thinks should happen next”

Assessment:
- still lightweight enough to keep
- but this is one of the main blended areas

### 3. The deterministic planner still overlaps with model-led follow-through

Files:
- [planner.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/planner.rs)

Current state:
- greetings and capability replies are short-circuited
- some follow-ups from prior `find_files` / `text_search` results are deterministic
- those follow-up responses are returned with `task_complete=true`

Why this is mixed:
- the planner is doing some “next-step intelligence” before the model reasons
- the kernel then has to compensate when `task_complete=true` arrives on a non-terminal step

Assessment:
- this is lightweight, not a disaster
- but it is the clearest remaining place where “planner behavior” and “loop behavior” overlap awkwardly

### 4. File/output heuristics are living in the shared kernel

Files:
- [task_shape.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/task_shape.rs)

Current state:
- `task_requests_output(...)`
- `parse_output_target_from_task(...)`
- output-path alias resolution
- low-value discovery reconsideration for output tasks

Why this is mixed:
- some of this is generic continuity support
- some of it is file-task strategy specific to the current CLI worker

Assessment:
- fine for the current worker
- likely too file-centered to become a universal base for future specialists unchanged

### 5. The prompt is carrying more operational policy than a pure base agent prompt should

File:
- [payload.rs](/Users/macc/projects/personal/agent-retina/crates/retina-llm-claude/src/payload.rs)

Current state:
- a lot of detailed rules about:
  - file targeting
  - output path behavior
  - overwrite behavior
  - when to respond
  - how reflection should recover

Why this is mixed:
- this works
- but some of these are current-worker tactics, not universal agent laws

Assessment:
- currently effective
- should eventually be split into:
  - shared base prompt rules
  - worker/body-specific prompt rules

## What The Latest Run Says About The Current Balance

The latest run suggests Retina is now in a workable middle state:

- the loop is finally strong enough to keep going
- the write/verify cycle is functioning
- the model is still doing most of the task interpretation
- but the kernel and prompt are still carrying worker-specific file/output opinions

So the current truth is:
- not overbuilt
- not fully cleanly separated
- working better than the architecture is “organized”

That is a normal transition point.

## Current Separation Score

### Shared-base generic pieces: solid

- bounded loop
- verification
- observation timeline
- memory recall
- reflection hooks
- completion enforcement

### Shared-base but a little opinionated: acceptable for now

- `remaining_obligation`
- `pending_deliverable`
- `target_output_path`
- low-value discovery reconsideration

### Likely future refactor targets

- deterministic planner follow-up logic
- file-centric prompt steering in the shared base prompt
- file/output-specific task heuristics in kernel task-shape

## Recommended Direction

Do not rip this apart now.

The current system is finally behaving well enough that the right move is not another large cleanup. The right move is to preserve the gains and separate concerns gradually.

Recommended principle:

### Keep in the shared kernel

- loop hardness
- verification
- terminal-result enforcement
- compact live task continuity
- reflection hooks
- memory recall plumbing

### Keep in the shared prompt only if truly universal

- use evidence before answering
- do not mark intermediate discovery as done
- keep working toward the original objective
- prefer the next verifiable step

### Move later into worker/body-specific prompt layers

- Desktop/Documents/Downloads path habits
- overwrite defaults for file updates
- file-output completion tactics
- document/file-specific recovery preferences

### Shrink later or isolate behind interfaces

- deterministic planner follow-up shortcuts
- natural-language output-path inference
- file-centered low-value action rules if specialists diverge

## Bottom Line

Yes, Retina is in a mixed-overlap phase.

But it is mixed in a productive way right now:
- the overlap is helping the worker function
- the ambiguity-detector detour was the wrong kind of overlap and has been removed
- the remaining overlap is mostly “shared worker tactics living in shared layers”

That means the system is not in trouble.

It just means the next cleanup, when it comes, should be a **separation pass**, not another behavior experiment:
- keep the hard loop
- keep the good context
- keep the gains
- gradually move current-worker tactics out of the universal base
