# Model-First Kernel Refactor Plan

> Make the model own interpretation. Make the harness own reality.

## Purpose

This plan refactors Retina's kernel away from early heuristic intent-routing and toward a model-first loop backed by strong verification.

The immediate trigger for this plan is the gap we just hit in the Windows CLI worker:
- the harness misread a file question as an output-writing task
- the harness treated folder aliases as stable truth instead of machine-specific facts
- the worker spent steps obeying a wrong harness frame instead of solving the user's request

The deeper issue is architectural:
- the kernel currently interprets too much
- it interprets too early
- its first-pass classification has too much control over planning and completion

The goal is not to remove structure.
The goal is to move semantic interpretation upward into the reasoner and keep correctness, safety, and verification in the harness.

## Research Basis

Use these docs as the governing stack:
1. [docs/v1_plan.md](/C:/Projects/agent-retina/docs/v1_plan.md)
2. [docs/plans/research-aligned-execution-plan.md](/C:/Projects/agent-retina/docs/plans/research-aligned-execution-plan.md)
3. [docs/plans/v1_useful_worker.md](/C:/Projects/agent-retina/docs/plans/v1_useful_worker.md)
4. [docs/research_overview.md](/C:/Projects/agent-retina/docs/research_overview.md)
5. [docs/trait_contracts.md](/C:/Projects/agent-retina/docs/trait_contracts.md)
6. [docs/research_memory.md](/C:/Projects/agent-retina/docs/research_memory.md)

Rules that must remain true:
- the harness is an execution and verification spine, not a phrase router
- the model should lead normal task interpretation
- the shell/body should stay rich and reality-facing
- failures must remain visible
- completion must be grounded in evidence, not only model assertion

External alignment we are explicitly targeting:
- Cursor publicly describes an agent harness as instructions, tools, and user messages, with the harness orchestrating tools for each model instead of replacing the model's interpretation with local routing rules.
- Cursor's public research on Composer emphasizes enabling the model to call the full tool harness effectively and minimizing unnecessary responses or claims without evidence.
- Retina should mirror that shape: rich tools, open shell exploration, strong verification, minimal semantic interference.

## Problem Statement

Retina currently has a mixed control model:
- the kernel infers task kind from heuristic phrase cues
- the inferred kind changes frontier logic, completion rules, and step budget
- the reasoner receives a partially interpreted task rather than owning the interpretation

This creates fragile behavior:
- mentioning `file` can accidentally imply output production
- vague questions can get bent into the wrong task class
- the worker can keep satisfying harness-generated constraints that do not reflect the user's true goal
- fixing one heuristic often adds another heuristic

The result is drift away from model-led behavior and toward a brittle local router.

## Cleanup Mandate

This plan is not only additive.
It is also subtractive.

If a harness behavior is more restrictive than the research direction and does not clearly improve:
- safety
- reality verification
- state tracking
- memory
- operator control

then it should be removed or demoted to fallback-only behavior.

This includes restrictions that:
- over-parse filenames or paths from free text
- hard-route user intent before the reasoner sees the request
- force frontier behavior from heuristic task classes
- block natural shell exploration without a concrete safety reason
- require artifact verification for requests that only ask for an answer

The burden of proof is now reversed:
- interpretive harness logic must justify itself
- otherwise it goes

Live-loop audit conclusion:
- per-step completion checking is harness interference and should not drive the loop
- the agent should keep working until it explicitly responds, hard-fails, is cancelled, or burns the hard budget
- low-value exploration policing should be removed from the normal path until there is clear evidence it helps more than it hurts
- validation belongs at explicit loop completion, not after every step
- the worker needs enough room to roam before the harness concludes anything

Immediate operating rules:
- set the normal task budget to `50`
- set the interactive chat task budget to `50`
- do not ask "are we done?" after each step
- do not stop just because the reasoner marked `task_complete=true` on a non-terminal action
- treat explicit `respond` as the normal terminal path
- keep observation, logging, approvals, cancellation, and hard-failure handling
- postpone final validation if it is still steering behavior instead of merely checking it

## Cleanup Inventory

This section is the working removal list.
It should be updated as cleanup progresses.

Legend:
- `remove`: delete from the normal path
- `fallback`: keep only for bootstrap or no-reasoner mode
- `keep`: retain because it protects safety, verification, or operator control

### Kernel interpretation inventory

1. `crates/retina-kernel/src/task_shape.rs`
   Current role:
   - carries compact frontier and completion helpers only
   Planned disposition:
   - `keep` for evidence-backed completion and frontier helpers
   Notes:
   - task-shape inference and bootstrap framing are removed
   - remaining logic is observation and verification oriented

2. `crates/retina-kernel/src/execution.rs`
   Current role:
   - builds task state
   - records observed state and reasoner framing
   Planned disposition:
   - `keep` bounded loop, verification, event logging
   Notes:
   - completion now pivots on observed evidence and model completion claims

3. `crates/retina-kernel/src/result_helpers.rs`
   Current role:
   - summarizes verified facts and output state
   Planned disposition:
   - `keep`
   Notes:
   - this is close to the right layer because it is evidence-oriented

4. `crates/retina-kernel/src/support.rs`
   Current role:
   - action utility, approval rules, context helpers
   Planned disposition:
   - `keep` for approval and utility logic
   - audit for any hidden “preferred action path” bias

5. `crates/retina-kernel/src/router.rs`
   Current role:
   - routing decisions for direct handling vs future specialist shapes
   Planned disposition:
   - `keep`
   Notes:
   - not the main source of current over-restriction for v1 local tasks

### Shell/body inventory

1. `crates/retina-shell-cli/src/policy.rs`
   Current role:
   - scoped authority over reads/writes/commands
   Planned disposition:
   - `keep`
   Notes:
   - restrictions here must remain explicit safety/authority boundaries, not semantic routing

2. `crates/retina-shell-cli/src/file_ops.rs`
   Current role:
   - list/find/read/extract/write helpers
   Planned disposition:
   - `keep`
   Notes:
   - prefer adding capability here over adding kernel-side interpretation

3. `crates/retina-shell-cli/src/process_control.rs`
   Current role:
   - open shell command execution and cancellation
   Planned disposition:
   - `keep`
   Notes:
   - this is part of the natural body we want more of, not less

4. `crates/retina-shell-cli/src/state_helpers.rs`
   Current role:
   - path resolution and known-folder alias handling
   Planned disposition:
   - `keep`
   Notes:
   - this should remain discovery/OS-truth logic, not user-intent logic

### Prompt/harness contract inventory

1. `crates/retina-llm-claude/src/payload.rs`
   Current role:
   - still presents task-shape wording and some harness-preferred workflow hints
   Planned disposition:
   - `keep`, but keep only observation, verification, and safety-oriented prompt structure
   Notes:
   - prompt should favor evidence, tool results, and verifiable next steps over harness-authored interpretation

2. `crates/retina-llm-claude/src/response.rs`
   Current role:
   - reasoner response schema
   Planned disposition:
   - `keep`, expand for model-authored framing and completion basis

## Cleanup Workstreams

### Workstream C1: remove filename and path over-parsing

Objective:
- stop the kernel from pretending it understands artifact mentions better than the model

Tasks:
- remove token-by-token filename extraction as a control primitive
- stop splitting multi-word filenames from free text
- if path-like hints are surfaced, mark them as raw mentions or shell-discovered facts
- prefer shell verification of actual paths over kernel inference of likely paths

Primary files:
- [task_shape.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/task_shape.rs)
- [state_helpers.rs](/C:/Projects/agent-retina/crates/retina-shell-cli/src/state_helpers.rs)

Acceptance:
- `Patent Center.pdf` remains intact through planning
- the kernel does not force exploration around partial filenames like `Center.pdf`

### Workstream C2: remove category-driven frontier control

Objective:
- stop heuristic task category from over-directing the next step

Tasks:
- reduce frontier blockers that exist only because of inferred task category
- prefer observed evidence gaps and failed tool results
- make “required input” hints advisory unless confirmed by shell observations or reasoner framing

Primary files:
- [task_shape.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/task_shape.rs)
- [execution.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/execution.rs)

Acceptance:
- a named file-reading question does not get stuck on inferred missing-input loops when the model can directly inspect or read the file

### Workstream C3: remove output verification from answer-only requests

Objective:
- ensure answer tasks are not trapped by artifact logic they did not ask for

Tasks:
- require verified output only when the deliverable is genuinely an artifact
- separate answer completion checks from output completion checks
- verify grounded answers through evidence ingestion rather than artifact state

Primary files:
- [task_shape.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/task_shape.rs)
- [execution.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/execution.rs)
- [result_helpers.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/result_helpers.rs)

Acceptance:
- “can you read this file?” ends after a grounded answer
- “create/update this file” still requires artifact verification

### Workstream C4: preserve only truth-enforcing restrictions

Objective:
- keep the harness disciplined without being overbearing

Retain:
- delete/kill approval boundaries
- path/authority scope enforcement
- anti-thrash after repeated no-progress loops
- grounded-answer requirements
- artifact verification for real output tasks

Remove or demote:
- weak phrase routing
- narrow structured-action bias
- filename tokenization from whitespace
- frontier blockers based only on guessed task kind

Acceptance:
- every remaining restriction can be justified as safety, verification, or operator control

## Cleanup Execution Order

Use this order for the aggressive cleanup pass:
1. C1 filename/path over-parsing removal
2. C3 answer-vs-output verification split
3. C2 category-driven frontier cleanup
4. C4 retained-restriction audit and documentation

## One-Shot Cleanup Applied

The current cleanup pass intentionally removed compatibility behavior rather than preserving it behind softer wording.

Applied changes:
- default loop step budget now follows `ExecutionConfig` directly instead of heuristic task-shape inflation
- unresolved inferred inputs no longer block completion
- answer-task frontier no longer treats file mentions as mandatory checklist items
- output and transform frontier no longer manufactures blocker chains from inferred missing inputs alone
- answer-task success markers no longer pretend named inputs must all be ingested before progress can count
- transform classification is no longer part of the kernel's normal control path
- requested output is no longer inferred from user wording
- output artifact state is no longer invented from prompt parsing; the harness now relies on observed writes and artifact references
- required input parsing is removed from the normal path
- prompt payload no longer serializes required-input scaffolding from free text
- heuristic lexical answer/discovery inference is removed from bootstrap task-shape inference
- default bootstrap framing now falls back to `Unknown` unless a concrete verified-output path is already present
- generic success markers now talk about observed evidence instead of harness-authored task meaning
- operator-facing task-state rendering now downplays empty heuristic framing instead of presenting it as semantic truth
- required-input progress accounting is removed from live task-state construction
- `required_inputs` is removed from the task-state data model and operator-facing render path
- low-value exploration brakes now key off observed evidence plus unresolved completion instead of task categories or inferred input readiness
- bootstrap task-shape inference now defaults straight to `Unknown`; the old lexical kind helper is deleted
- prompt instructions now treat `intent_kind`, `deliverable`, and `completion_basis` as optional continuity metadata instead of workflow control
- `TaskShape`, `TaskShapeSource`, requested output scaffolding, output artifact scaffolding, and success markers are removed from task state
- task state now carries only an optional `intent_hint` plus observed progress, sources, artifacts, blockers, and reasoner framing
- completion no longer depends on harness-owned task categories; it checks observed evidence and the actual terminal move

Implication:
- any tests or downstream assumptions that depended on heuristic step-budget boosts or required-input gatekeeping should be updated to the thinner verifier-backed model instead of restoring the removed behavior

Remaining high-value removals:
- only further prompt trimming or debug-surface simplification if live behavior still feels over-mediated
- otherwise keep the current floor: observation, verification, anti-thrash, safety, and operator control

Current state now in place:
- the reasoner response contract now supports an explicit compact framing block:
  - `intent_kind`
  - `deliverable`
  - `completion_basis`
- the kernel stores that framing as advisory task-state interpretation
- completion now begins to verify model-authored `completion_basis` claims against observed evidence instead of trusting the claim by itself
- the kernel no longer carries a separate task-shape subsystem

Reason:
- C1 fixes the most visible “harness in the way” behavior
- C3 removes answer-task traps
- C2 frees the loop more broadly once the evidence plumbing is safer
- C4 locks the philosophy in after the main simplifications land

## Remaining Gut List

Anything below should be removed unless it clearly supports observation, validation, debugging, safety, or operator control.

1. Remaining lexical task-kind inference as a mainline control path
   Status:
   - removed
   Next action:
   - none unless fallback mode later needs a tiny bootstrap hint

2. Prompt wording that treats task shape as a meaningful planning artifact
   Status:
   - removed
   Next action:
   - keep trimming only if live runs still show prompt-induced bias

3. Frontier generation that prefers harness-authored synthesis prompts over pure evidence gaps
   Status:
   - mostly removed
   Next action:
   - tune only from real run evidence

4. Task-state fields that imply semantic certainty from heuristics
   Status:
   - removed
   Next action:
   - none

5. Completion checks that depend on shape more than observed evidence
   Status:
   - removed
   Next action:
   - none

## Design Target

Retina should use this division of labor:

### Reasoner-owned

- infer the user's likely intent from the full request
- propose the current goal in plain language
- choose the next best verifiable step
- revise its interpretation when evidence changes
- decide whether the task is probably answer-oriented, discovery-oriented, transform-oriented, or artifact-oriented

### Kernel-owned

- bounded execution loop
- context assembly
- tool dispatch
- state capture and comparison
- cancellation and stop control
- reflection hooks
- memory updates
- verification of claimed completion

### Shell-owned

- access to filesystem, documents, and commands
- exact path resolution and OS-specific discovery
- effect capture
- output capture
- safety boundaries and approvals

This shell should feel natural and open:
- broad read/navigation access by default
- broad local command execution within v1 safety policy
- direct exploration before extra harness mediation
- verification after action instead of pre-emptive narrowing

### Memory-owned

- recall of similar past tasks
- useful prior action paths
- learned source-selection patterns
- repeated-success promotion into stronger guidance

## Non-Goals

This refactor does not require:
- removing all task-shape data
- removing all kernel-side completion checks
- replacing verification with model judgment
- adding browser support, vision, or OCR in this plan

This plan also does not require a full new trait boundary. It should work within the existing kernel, reasoner, shell, and memory seams.

## Core Shift

The current model is:
- harness classifies
- model plans inside that classification
- harness enforces completion against that classification

The target model is:
- model proposes interpretation
- harness records that interpretation as provisional task state
- model acts and can revise interpretation
- harness verifies whether the claimed outcome satisfies reality

In short:
- task framing becomes provisional
- verification stays strict

## Current Kernel Responsibilities To Reduce

The kernel should do less of the following:
- infer hard task kind from shallow lexical cues
- infer requested outputs from weak phrase matches
- lock frontier behavior to a single early classification
- drive completion entirely from harness-side task class

These behaviors can remain temporarily as fallback hints, but they should stop acting as the primary source of truth.

## Responsibilities That Must Stay Strong

The harness must remain strict about:
- whether a file exists
- whether an output changed
- whether required evidence was actually read or extracted
- whether a response is grounded in gathered evidence
- whether the agent is repeating low-value steps
- whether a claimed output artifact is verified

This refactor is not a move toward looser execution.
It is a move toward looser interpretation and stronger verification.

## Refactor Phases

### Phase M1: make task shape provisional instead of authoritative

Goal:
- keep task-shape data, but demote it from hard controller to soft hint

Implementation:
- change `TaskShape` semantics from "kernel-decided truth" to "current framing hint"
- rename or document `TaskShape` accordingly if needed
- mark shape fields as inferred, revisable, and confidence-limited
- stop treating `requested_output.is_some()` alone as sufficient to force an output task path

Kernel changes:
- [crates/retina-kernel/src/task_shape.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/task_shape.rs)
- [crates/retina-kernel/src/execution.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/execution.rs)
- [crates/retina-types/src/task_state.rs](/C:/Projects/agent-retina/crates/retina-types/src/task_state.rs)

Acceptance:
- a vague file question is not forced into output verification logic
- early task framing can be revised without the loop fighting the revision
- heuristic framing is visibly labeled as non-authoritative in task state and prompts

### Phase M2: add reasoner-authored task framing

Goal:
- let the model propose intent and completion shape directly

Implementation:
- extend the reasoner response schema with a compact framing block such as:
  - `goal`
  - `intent_kind`
  - `deliverable`
  - `completion_claim`
  - `confidence`
- assemble current harness observations and current provisional shape into the prompt
- prefer the reasoner's framing over heuristic task shape when present
- preserve heuristic inference only as bootstrap fallback when the reasoner does not provide framing

Reasoner changes:
- [crates/retina-llm-claude/src/payload.rs](/C:/Projects/agent-retina/crates/retina-llm-claude/src/payload.rs)
- [crates/retina-llm-claude/src/response.rs](/C:/Projects/agent-retina/crates/retina-llm-claude/src/response.rs)
- [crates/retina-types/src/reasoning.rs](/C:/Projects/agent-retina/crates/retina-types/src/reasoning.rs)

Acceptance:
- the model can say "this is a question about an existing file" without the harness overriding it
- the kernel can continue executing even if heuristic shape and model framing disagree

### Phase M3: split interpretation from verification

Goal:
- completion should be checked against reality, not just against task class

Implementation:
- replace one broad `completion_guard` path with composable verification checks:
  - grounded-answer verification
  - output-artifact verification
  - required-evidence verification
  - no-progress / anti-thrash verification
- require the reasoner to explicitly claim why the task is complete
- have the kernel verify that claim against observed state

Examples:
- if the model says "I answered the question about the file", verify that the file was actually read/extracted
- if the model says "I created the output", verify the artifact exists and changed
- if the model says "blocked", verify the blocker is grounded in observed constraints or tool failures

Kernel changes:
- [crates/retina-kernel/src/task_shape.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/task_shape.rs)
- [crates/retina-kernel/src/result_helpers.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/result_helpers.rs)
- [crates/retina-kernel/src/execution.rs](/C:/Projects/agent-retina/crates/retina-kernel/src/execution.rs)

Acceptance:
- answer tasks complete when a grounded answer is returned, even if no named output exists
- file-question tasks do not get stuck on artifact verification loops
- output tasks still cannot complete without verified artifacts

### Phase M4: move frontier generation from category rules toward evidence gaps

Goal:
- make next-step guidance come from missing evidence and unverified claims, not from static task buckets

Implementation:
- derive frontier from:
  - missing sources
  - unresolved user asks
  - unverified outputs
  - unsupported source types
  - recent failed attempts
- keep `answer`, `transform`, and `output` distinctions only as soft weighting, not absolute routing
- add a notion of "current unsatisfied claim"

Examples:
- if the worker says "I can answer after reading this PDF", frontier should ask for the PDF text
- if the worker says "I wrote the report", frontier should ask for artifact verification
- if the worker has enough evidence but no answer text yet, frontier should ask for final synthesis

Acceptance:
- frontier quality improves without adding more phrase heuristics
- repeated loops become rarer because the gap is based on actual missing evidence

### Phase M4.2: strip prompt-side semantic steering

Goal:
- stop telling the model what kind of task it is beyond observed state and hard verification rules

Implementation:
- remove prompt blocks that serialize task-shape interpretation as if it were operational truth
- keep prompt context focused on:
  - observed sources
  - observed artifacts
  - blockers grounded in tool results
  - available tools
  - hard verification expectations
- remove workflow preference text that exists only because the harness wants a certain route

Acceptance:
- the model sees the body, evidence, and constraints, not a pseudo-plan authored by the harness
- prompt updates do not reintroduce filename/output inference through wording

### Phase M4.5: remove restrictive harness mediation that duplicates model reasoning

Goal:
- actively strip away kernel-side interpretation layers that get between the user and the model

Implementation:
- audit all heuristic language-parsing paths in the kernel and classify each as:
  - keep as verification support
  - demote to fallback/bootstrap only
  - remove entirely
- remove filename tokenization logic that splits multi-word artifact names from natural language
- stop using harness-parsed "required inputs" as authoritative control state when they were inferred only from free-text hints
- prefer model-selected exploration over kernel-generated pseudo-workflows
- preserve only the minimum task-state hints needed for verification and continuity

Targets for audit/removal:
- filename extraction from `split_whitespace()` style parsing
- requested-output inference from weak lexical cues
- frontier blockers that arise only from heuristic interpretation rather than observed evidence
- any completion rule that depends on inferred task category more than verified state

Acceptance:
- multi-word filenames in natural language are no longer broken apart by kernel parsing
- natural file-reading requests flow through direct exploration and reading rather than heuristic control loops
- the shell remains open enough that the agent can choose a natural path without fighting the harness

### Phase M5: add interpretation-revision events to task state

Goal:
- let the system visibly change its mind

Implementation:
- record when the model reframes a task mid-run
- track:
  - initial framing
  - revised framing
  - reason for revision
  - evidence that triggered revision
- surface these in the timeline and debug views

Why this matters:
- a real agent should be able to say "I first thought this was an output task, but evidence shows it is a question about an existing file"
- this prevents hidden drift and makes debugging easier

Acceptance:
- timeline shows reframing events clearly
- debug output explains why the current framing changed

### Phase M6: reduce heuristic inference to bootstrap and fallback only

Goal:
- keep local inference only where it is truly useful

Implementation:
- retain heuristic task-shape inference for:
  - first-turn bootstrap before the first model plan
  - no-network or no-model fallback mode
  - sanity hints for obvious named artifacts
- document that heuristics are advisory
- forbid new phrase heuristics from becoming hard completion logic unless there is a strong justification

Acceptance:
- most normal runs are model-framed, verifier-backed
- fallback mode still works when the reasoner is unavailable
- the default path for normal requests no longer depends on kernel-side phrase routing

### Phase M7: explicit restriction audit against research

Goal:
- remove any remaining harness behaviors that are more restrictive than the intended research direction

Implementation:
- create an audit checklist for every interpretive restriction in the kernel, shell policy, and reasoner prompt
- for each restriction, record:
  - what it blocks
  - why it exists
  - whether it protects safety, verification, or operator control
  - whether the same outcome could be achieved with a thinner harness
- remove or demote restrictions that fail the audit
- document retained restrictions as intentional research-aligned guardrails, not accidents

Examples of acceptable retained restrictions:
- delete/kill approval boundaries
- output verification for real artifact-producing tasks
- anti-thrash limits after repeated no-progress loops
- grounded-answer requirements before claiming success

Examples of restrictions that should likely be removed:
- weak lexical task routing that overrides model interpretation
- brittle filename extraction from whitespace tokens
- artifact verification gates for answer-only requests
- narrow structured-action bias when shell exploration would be more natural

Acceptance:
- every remaining restrictive behavior has a documented justification
- the harness is thinner in semantics and stronger in verification
- Retina feels closer to Codex/Claude/Cursor-style agent behavior: open tools, flexible reasoning, post-hoc reality checks

## Data Model Changes

Add or revise task-state concepts to support the new flow:
- `current_goal`
- `intent_kind_hint`
- `intent_kind_source`
  - `heuristic`
  - `reasoner`
  - `revised_reasoner`
- `intent_confidence`
- `deliverable_claim`
- `completion_basis`
- `reframe_events`

These should be compact and operator-readable.

## Prompt Contract Changes

The reasoner prompt should explicitly ask for:
- what the user is actually asking for
- what evidence is still needed
- whether an output artifact is requested, or whether the user only wants an answer
- what would make the task complete
- whether the current framing should be revised

The prompt should explicitly forbid:
- pretending a named file mention is automatically an output target
- marking completion without tying it to observed evidence
- letting heuristic harness hints override direct evidence from tool results

## Test Plan

### Regression cases from current failures

- `read the Patent Center.pdf on Desktop and tell me what it's about`
  - should resolve as an answer task
  - should not become an output task
  - should surface OCR/text-layer absence as the real blocker if extraction fails

- `what about this file can you read it "C:\...\Dominican_MOH_Approval_filled.pdf"`
  - should be interpreted as a question about an existing artifact
  - should not enter output-verification loops
  - should terminate after a grounded answer

### General behavior tests

- vague requests about existing files should default toward answer/discovery, not output production
- named output requests should still require artifact verification
- interpretation can change after new evidence arrives
- heuristic and reasoner disagreement should not crash or deadlock the loop
- fallback mode should still function when the reasoner is unavailable
- multi-word filenames in natural language remain intact through planning and execution
- shell-oriented exploration is allowed when it is the clearest next step
- the agent is not forced into narrow structured file actions when command-based exploration is more natural

### Timeline/debug tests

- reframing events are visible
- completion claims have a clear verification basis
- blockers are tied to actual tool results or missing capabilities

## Migration Order

Recommended implementation order:
1. M1 provisional task shape
2. M3 verification split
3. M2 reasoner-authored framing
4. M4 evidence-gap frontier
5. M4.2 prompt-side semantic steering removal
6. M4.5 restrictive mediation cleanup
7. M5 reframing events
8. M6 fallback cleanup
9. M7 restriction audit

Reason:
- first reduce damage from wrong classification
- then make completion checks more evidence-based
- then let the reasoner take over more of the interpretation

## Risks

### Risk 1: too little structure

If the kernel gives up too much structure too quickly, the worker may wander.

Mitigation:
- keep strict verification
- keep anti-thrash logic
- keep bounded step budgets

### Risk 2: too much schema pressure on the reasoner

If the framing schema is too large, prompt quality and response reliability may get worse.

Mitigation:
- keep framing compact
- prefer a few high-value fields over a taxonomy

### Risk 3: heuristic fallback keeps leaking into primary behavior

If old classification code remains wired into completion or frontier logic, the refactor will only be partial.

Mitigation:
- explicitly document which paths are fallback-only
- add tests for disagreement between heuristic and reasoner framing

## Done Condition

This plan is complete when Retina behaves like a model-first worker with harness-backed verification:
- vague user requests are primarily interpreted by the reasoner
- task framing can be revised mid-run
- the kernel verifies completion against evidence instead of over-trusting task class
- file questions do not get trapped in output verification loops
- named-output tasks still require real artifact verification
- restrictive harness logic has been audited and removed unless it protects safety, verification, or operator control
- the shell feels open and natural enough for real exploration rather than pre-scripted routes
- the system feels less like a local phrase router and more like a real agent with a strong body and honest guardrails

## Save Path

- `docs/plans/model-first-kernel-refactor-plan.md`
