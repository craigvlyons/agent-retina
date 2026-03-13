# Retina — Roadmap

> Build the anthill one chamber at a time, but always in the direction of a colony and then a mesh.

## North Star

Retina is meant to become a private agent system with:
- one shared kernel
- many independent or specialized agents
- long-term memory that improves behavior over time
- shells that can operate in CLI, web, server, and hardware environments
- a root coordinator that can route work across the colony
- a mesh of local and external connections when the foundation is ready

The long-term target is not a chatbot.
It is a reliable agent kernel that can inhabit many bodies and coordinate many workers.

## The Ant Analogy

### Phase 0: the seed

One ant hatches.
It must be able to survive on its own.

In Retina terms:
- one kernel
- one shell
- one reasoner
- one memory
- one full observation loop

### Phase 1: the first chamber

The first chamber is the load-bearing room of the anthill.
This is where the colony proves that it can hold weight.

What we build:
- the stable type system
- the 5 traits
- the kernel loop
- the timeline
- the first memory vessel
- the first shell

Outcome:
- one agent can do real work and leave replayable evidence

### Phase 2: the worker becomes reliable

The first worker must become useful before more ants arrive.

What we build:
- stronger action planning
- multi-step execution
- operator stop and cancel controls
- better reflection and retry behavior
- stronger reflex promotion and consolidation

Outcome:
- one agent can handle meaningful repo and system tasks without constant steering

### Phase 3: new chambers for specialists

Now the anthill expands.
The colony gains domain-specific rooms.

What we build:
- local transport
- manifests and lifecycle for specialists
- spawn and reactivate flows
- routing policies
- scoped permissions and shells per specialist

Outcome:
- code, research, browser, ops, and hardware workers can exist as independent agents

### Phase 4: tool-building workers

The colony starts making its own tools.

What we build:
- Wasm fabrication
- tool compilation and testing
- tool registration and promotion
- shared promoted tools between agents

Outcome:
- specialists can build missing capabilities instead of waiting for hand-written adapters

### Phase 5: colony to mesh

The anthill becomes a network.

What we build:
- MCP and A2A style transport surfaces
- external agent and system integrations
- remote or device-local shells
- broader routing, budgeting, and governance

Outcome:
- Retina becomes a mesh of specialized agents that can work alone, together, or on behalf of external systems

## Current Position

Retina is between Phase 1 and Phase 2.

Done:
- workspace and private runtime
- 5 trait boundaries
- shared kernel types
- single-agent execute loop
- CLI shell
- SQLite memory
- Claude-backed reasoner
- timeline persistence
- first learning and reflex promotion path

Still needed before we leave the first chamber:
- better multi-step execution
- better autonomy for real work
- operator stop controls
- stronger consolidation and reuse of learned behavior

## Build Sequence

### Now

Finish the first worker:
- harden v1 loop
- keep prompts small
- make the shell trustworthy
- improve memory-driven behavior

### Next

Add specialist growth paths:
- code specialist
- browser specialist
- research specialist
- hardware specialist

### Later

Introduce the root coordinator:
- registry
- routing
- spawn policy
- authority hierarchy
- cross-agent coordination

### Long term

Extend the colony into a mesh:
- local colony on one machine
- agents on web and devices
- external protocol surfaces
- shared promoted tools and cross-colony interoperability

## Guardrails

Every phase should preserve these rules:
- the kernel remains the intelligence
- the shell remains the body
- memory remains pull-based
- observation remains first-class
- specialist growth is additive, not a rewrite
- long-term direction stays aligned with multi-agent and memory research from 2026

## Success Test

We are still aligned if each phase makes these more true:
- one agent can do real work safely
- learned behavior reduces repeated mistakes
- new specialists can be added without changing the kernel contract
- the colony gains capability without losing reliability
