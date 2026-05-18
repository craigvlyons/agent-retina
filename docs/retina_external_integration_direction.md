# Retina External Integration Direction

## Purpose

This document captures how Retina could absorb the strongest parts of:

- `/Users/macc/projects/personal/voice-agent`
- `/Users/macc/projects/personal/gabanode-desktop`

without drifting back into a second competing agent architecture.

The goal is not to merge whole apps.

The goal is:

- keep Retina as the canonical brain, runtime, and orchestrator
- pull in the best body/interface layers from the other projects
- move toward a stronger desktop, web, voice, and eventually multi-device system

## Core Position

Retina should remain:

- the main agent
- the transcript-first runtime
- the parent/child task supervisor
- the routing and specialist control plane
- the long-term multi-device swarm foundation

The other projects should contribute:

- stronger body layers
- stronger user interface surfaces
- stronger interaction channels

They should not replace Retina's runtime ownership model.

## Project 1: Voice Agent

### What It Looks Like Today

`/Users/macc/projects/personal/voice-agent` is a Rust-first voice system with:

- wake-word handling
- STT/TTS provider plumbing
- session and turn handling
- local persistence
- daemon and CLI surfaces

The strongest likely reuse areas are:

- wake and conversation lifecycle
- voice turn capture
- STT/TTS adapters
- daemon model for local always-on listening

### Best Fit For Retina

This project fits Retina as:

- a `voice` specialist
- or a voice front door into the main Retina agent

Recommended role:

- Retina stays the decision-maker
- voice-agent contributes the audio capture and speech interaction substrate
- spoken requests become tasks for Retina
- spoken follow-up and spoken status become one more operator surface

### What Not To Do

- do not create a second independent voice-first orchestrator beside Retina
- do not let voice state become a separate truth model for tasks
- do not fork command routing between "voice world" and "Retina world"

### Integration Shape

The likely long-term shape is:

```text
microphone / wake word / speech session
        ->
voice runtime
        ->
Retina task creation / task resume / delegated specialist work
        ->
spoken response / spoken progress / spoken clarification
```

## Project 2: Gabanode Desktop

### What It Looks Like Today

`/Users/macc/projects/personal/gabanode-desktop` is much more than a UI shell.
It already has a serious desktop-control substrate.

Important parts:

- `/Users/macc/projects/personal/gabanode-desktop/gabactl`
  Swift CLI for native macOS control
- `/Users/macc/projects/personal/gabanode-desktop/backend/app/platform/gabactl.py`
  structured driver wrapper around the Swift CLI
- `/Users/macc/projects/personal/gabanode-desktop/docs/architecture/architecture-deep-dive.md`
  system architecture description

The Swift command surface already covers major body capabilities:

- app discovery and activation
- accessibility tree inspection
- AX query/click/set value
- mouse move/click/drag/scroll
- key press/combo/type/paste
- screen capture
- OCR
- permissions checks
- event streaming

### Why It Matters

This is the strongest reusable body layer we have for Retina right now.

It could move Retina much closer to:

- serious desktop control
- web control through accessibility and page extraction
- screenshot + OCR grounded execution
- richer operator-grade automation on macOS

### Core Limitation

Gabanode Desktop is still fundamentally a one-loop app-centered agent system.

That is good for proving a desktop executor.
It is not the right main architecture for where Retina is going.

Retina is now in a better position to be:

- the planner
- the supervisor
- the transcript owner
- the parent of specialists
- the eventual multi-device controller

So the right move is not:

- "make Retina become Gabanode Desktop"

The right move is:

- "make Retina control a stronger desktop body and optionally reuse the UI shell"

## Recommended Integration Principle

Retina should absorb **body and interface layers**, not **agent ownership**.

That means:

- keep Retina runtime ownership
- keep Retina task continuity
- keep Retina delegation model
- keep Retina resume/recovery model
- keep Retina multi-device direction

Then reuse from Gabanode Desktop:

- native macOS control substrate
- overlay or desktop UI concepts where useful
- screenshot and OCR pipeline pieces
- browser/page extraction patterns

## Best Reuse Order

### Phase 1: `gabactl` As A Retina Shell Backend

This is the highest-value and cleanest seam.

Target:

- add a Retina shell/runtime adapter that talks to `gabactl`
- expose desktop/body capabilities to Retina as part of the shared worker tool pool

Likely first capabilities:

- permissions status
- frontmost app
- app list / activate
- AX tree query
- AX click / set value
- mouse move/click/drag/scroll
- key type / combo / paste
- capture
- OCR

This would give Retina a premium macOS body without inheriting Gabanode's whole control loop.

### Phase 2: Desktop Operator Surface

After the body exists, evaluate whether parts of the Gabanode desktop UI should become a Retina operator shell.

Good candidates:

- overlay surface
- activity stream views
- screen annotation display
- desktop task controls

Bad candidates:

- carrying over the one-loop agent runtime
- keeping parallel planning/execution ownership in the Electron/FastAPI app

### Phase 3: Voice Entry And Voice Specialist

Once Retina has a stronger desktop body, voice becomes much more valuable.

At that point:

- spoken commands can create real Retina tasks
- spoken follow-ups can steer current work
- spoken updates can read out task progress or blocked states

This turns voice-agent into a natural interaction layer instead of a separate assistant.

## Recommended End State

The strongest combined architecture likely looks like:

```text
                    Retina
     transcript-first runtime / planner / router
            /               |                \
           /                |                 \
    local specialists   remote specialists   operator surfaces
           |                |                 |
           |                |                 |
      desktop body      future devices      UI / voice
           |                                  |
      gabactl + capture + AX            Gabanode UI ideas
                                        + voice-agent runtime
```

## Concrete Recommendation

If we are choosing where to pull first:

1. pull `gabactl` concepts and command surface first
2. keep Retina as the controlling agent
3. treat Gabanode's UI as optional later
4. treat voice-agent as the next interaction layer after desktop control

## What We Should Not Do

Do not:

- merge the whole Gabanode backend into Retina as the main runtime
- run two competing orchestrators
- split task truth between Retina and a desktop-side loop
- let the UI shell become the task owner
- let voice-agent become a second assistant product next to Retina

## Immediate Next Docs Or Plans

When we are ready to move past this direction note, the next documents should be:

1. `gabactl_retina_integration_audit.md`
   Exact command-by-command fit map into Retina shell/runtime seams

2. `retina_desktop_body_adoption_plan.md`
   Stepwise implementation plan for using `gabactl` inside Retina

3. `retina_voice_specialist_adoption_plan.md`
   Stepwise plan for bringing voice-agent in as a specialist or front door

## Bottom Line

The right architecture is not:

- Gabanode as the brain
- Retina as one more worker

The right architecture is:

- Retina as the brain
- Gabanode's native control layer as the body
- Gabanode's desktop UI as an optional future operator shell
- voice-agent as an interaction layer and eventual specialist
