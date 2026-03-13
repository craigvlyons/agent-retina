# Retina — Architecture

> One seed grows into a colony. A colony becomes a mesh.

## The Anthill Model

Retina should be understood as an anthill built in stages.

### Seed

One agent is deployed.
It can observe, act, verify, remember, and reflect.

### Chamber

That agent gains durable internal structure:
- stable traits
- shell control
- observation timeline
- memory
- learning primitives

### Colony

Specialized agents are added as separate chambers:
- email
- code
- research
- ops
- browser or hardware control

Each chamber has:
- its own kernel
- its own memory
- its own tools
- its own scoped authority

### Mesh

The anthill opens outward.
Agents can run across:
- CLI
- browser
- servers
- hardware devices
- local transports
- MCP and A2A style interfaces

The colony becomes a network of cooperating specialists.

## Core Principle

The kernel depends on 5 traits and nothing else:

```text
Shell      — sense, act, verify, ask user
Reasoner   — think, reflect
Memory     — record, recall, learn
Fabricator — build tools
Transport  — talk to other agents
```

Everything else is implementation detail inside those boundaries.

## Authority Model

Retina should grow into a hierarchy without changing the kernel shape.

### Independent worker

The current v1 agent is one worker ant.
It executes tasks directly through its shell.

### Root coordinator

Later, a root agent acts like the queen of the local colony:
- holds the registry of what workers exist
- routes tasks
- decides when to spawn or reactivate specialists
- coordinates multi-agent pipelines

The root is not a magical different runtime.
It is the same kernel with different memory, transport, and policy wiring.

### Specialists

Specialists are workers with scoped bodies and scoped authority.

Examples:
- code specialist with repo and terminal authority
- browser specialist with DOM and form authority
- hardware specialist with device-specific shell authority
- research specialist with web and document authority

Each specialist can:
- work alone
- collaborate with sibling agents
- return structured results to the root

## Why This Shape Matters

This architecture supports the long-term goal:
- one agent kernel that can control software, hardware, and web environments
- long-term memory that improves behavior over time
- Cursor-style orchestrator and worker patterns without turning the system into prompt spaghetti
- additive growth from a single worker into a private multi-agent mesh

## V1 Architectural Stance

V1 deliberately does not start with the queen.

It starts with one strong worker:
- direct execution through shell
- full state verification
- small pull-based memory
- complete observation timeline
- early reflex and utility learning

This is the load-bearing chamber.
If this piece is not strong, the colony will collapse under routing, transport, and specialist complexity later.

## Colony Shape Later

Long term, the shape looks like this:

```text
human / external system
        |
     root agent
        |
  -------------------
  |   |   |   |    |
 code browser ops research hardware
 agent agent   agent  agent    agent
```

And later still:

```text
multiple local colonies <-> shared transports <-> external tool surfaces
```

That is the mesh direction.
But every future chamber should inherit the same kernel, trait boundaries, event schema, and memory rules that v1 establishes now.
