# Multi-Device Specialist Runtime Plan

## Purpose

This is the active build plan for getting Retina from a strong local agent runtime to a dependable multi-device specialist system.

The transcript-first refactor is treated as baseline.
This plan does not reopen the old architecture work.

The target state is:

- one main agent can spawn, supervise, and resume child workers
- specialists can run on other devices, not just locally
- delegated work streams status and results back to the parent
- interrupted or disconnected workers can recover without losing the task story

## What This Plan Assumes Is Already True

- transcript and stored result refs are the runtime truth
- continuation is the live continuity object
- blocked and resumed tasks preserve continuation
- local delegation and specialist routing already exist

## Non-Goals

- no new parallel runtime model
- no separate control loop for remote workers
- no product-cloud rewrite
- no old plan resurrection

## Phase 1: Remote Transport Contract

Define the transport contract for off-device workers using the existing runtime seams.

Deliverables:

- extend the runtime around [lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-traits/src/lib.rs)
- formalize remote-capable agent lifecycle messages
- define task dispatch, heartbeat, cancellation, resume, and completion envelopes
- define artifact and transcript excerpt return messages

Acceptance:

- remote execution can be modeled through the same parent/child task types as local agents
- transport messages are explicit enough to support retries and reconnects

## Phase 2: Remote Agent Registry And Discovery

Add a real registry for reachable remote workers.

Deliverables:

- remote agent cards with device identity, capability summary, status, and trust state
- discovery and registration flow
- explicit online/offline and stale-worker handling
- registry integration with routing

Acceptance:

- the router can choose among local agents, reusable specialists, and remote workers from one registry view

## Phase 3: Parent/Child Remote Task Supervision

Make remote delegated work a first-class supervised task.

Deliverables:

- remote task kind execution path
- progress streaming back into the main timeline
- transcript excerpt and output attachment handling for child tasks
- bounded retry and reconnect behavior

Acceptance:

- a parent can see remote child progress, blocked state, completion, and outputs through the same runtime task model

## Phase 4: Trust, Tool Scope, And Device Authority

Make remote workers safe enough to deploy intentionally.

Deliverables:

- trust model for approved devices
- per-agent tool policy and working-root scope on remote workers
- explicit allowed and denied tool surfaces for remote manifests
- approval and policy failure reporting back to the parent

Acceptance:

- remote specialists do not inherit the whole local authority surface by accident

## Phase 5: Resume And Recovery Across Devices

Use the transcript-first runtime to recover remote work cleanly.

Deliverables:

- reconnect flow for interrupted remote workers
- resume from recovery snapshot on the same or replacement device
- parent-visible blocked state with actionable recovery reason
- transcript/result continuity preserved across resume

Acceptance:

- a dropped worker does not force the parent to restart the task from scratch

## Phase 6: Specialist Deployment Model

Turn reusable specialists into deployable workers.

Deliverables:

- specialist packaging or manifest distribution format
- remote specialist bootstrap process
- device-local runtime startup for approved specialists
- versioned specialist definitions

Acceptance:

- the same specialist concept works locally or remotely with the same parent-facing lifecycle

## Phase 7: Operator Surfaces

Make the system usable when more than one device is involved.

Deliverables:

- runtime inspect views for remote workers
- device and worker status summaries
- child-task transcript and output inspection
- clear blocked, disconnected, and resumed states in the CLI

Acceptance:

- the main operator can understand what each worker is doing and why

## Definition Of Done

This plan is complete when:

1. the main agent can delegate bounded work to remote specialists
2. remote workers stream progress and results back into the same task supervision model
3. remote workers can disconnect and resume without destroying task continuity
4. routing can intentionally choose local or remote workers from one registry
5. specialists remain scoped variants of the same worker architecture rather than a separate subsystem
