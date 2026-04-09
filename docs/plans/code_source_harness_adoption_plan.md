# Vital-Swarm Harness Adoption Plan

> Purpose: extract the highest-value harness patterns from the code_source TypeScript agent runtime and adapt them into Retina without importing the product-specific shell wholesale.

## Summary

Retina should stay CLI-first and kernel-centered.

The code_source harness is valuable as a reference implementation for:
- task supervision
- background execution
- subagent spawning
- tool orchestration
- MCP integration
- agent definitions and scoped tool pools

Retina should not try to port that codebase directly.
It is too coupled to:
- Bun
- React/Ink UI
- product bootstrap
- analytics and feature flags
- remote/cloud session infrastructure
- policy and settings layers that Retina does not need

The right move is:
- keep Retina's Rust kernel, trait boundaries, and manifest/memory model
- borrow the code_source harness's runtime patterns
- re-express those patterns as small Rust crates and kernel-adjacent services

This gives `agent-retina` a path from one useful worker to a real `vital-swarm` without breaking the current architecture.

## Design Position

Retina already has the right skeleton:
- kernel loop and routing seam in [crates/retina-kernel/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/lib.rs)
- routing and future specialist decisions in [crates/retina-kernel/src/router.rs](/Users/macc/projects/personal/agent-retina/crates/retina-kernel/src/router.rs)
- shared task and agent types in [crates/retina-types/src/tasking.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs) and [crates/retina-types/src/agents.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/agents.rs)
- strict trait seams in [crates/retina-traits/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-traits/src/lib.rs)
- a CLI runtime/controller in [crates/retina-cli/src/controller.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs)
- a real shell body in [crates/retina-shell-cli/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-shell-cli/src/lib.rs)

That means Retina does not need a new foundation.
It needs a stronger runtime layer around the existing foundation.

## What To Keep From The Code_Source Harness

### 1. Task model

Keep the idea that long-running work is normalized as tracked tasks with:
- task ids
- type
- lifecycle status
- output stream or transcript
- notifications
- resumability

Reference files:
- [code_source/src/Task.ts](/Users/macc/projects/code_source/src/Task.ts)
- [code_source/src/tasks.ts](/Users/macc/projects/code_source/src/tasks.ts)
- [code_source/src/utils/task/framework.ts](/Users/macc/projects/code_source/src/utils/task/framework.ts)

Why it matters for Retina:
- today Retina can execute and spawn a thread per task, but it does not yet have a full task supervisor
- `RunningTask` in [crates/retina-cli/src/controller.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs) is a useful start, but it is still a thin handle rather than a real task registry

Recommendation:
- add a dedicated task runtime crate, likely `crates/retina-runtime`
- model task types explicitly:
  - `main_session`
  - `local_command`
  - `local_agent`
  - `specialist_agent`
  - `remote_agent` later

### 2. Agent tool as delegation primitive

Keep the idea that "spawn an agent" is a first-class operation, not an ad hoc shell trick.

Reference files:
- [code_source/src/tools/AgentTool/AgentTool.tsx](/Users/macc/projects/code_source/src/tools/AgentTool/AgentTool.tsx)
- [code_source/src/tools/AgentTool/runAgent.ts](/Users/macc/projects/code_source/src/tools/AgentTool/runAgent.ts)
- [code_source/src/tools/AgentTool/loadAgentsDir.ts](/Users/macc/projects/code_source/src/tools/AgentTool/loadAgentsDir.ts)

Why it matters for Retina:
- Retina already has routing decisions for `RouteToExisting`, `Reactivate`, and `SpawnSpecialist`
- what is missing is the execution path after the router decides

Recommendation:
- do not make "spawn specialist" a kernel shortcut
- represent delegation through a runtime service that the kernel can ask for
- keep the kernel's job limited to deciding:
  - handle directly
  - route
  - reactivate
  - spawn

### 3. Scoped tool pools

Keep the pattern where each agent gets a filtered tool pool rather than inheriting the whole world.

Reference files:
- [code_source/src/tools.ts](/Users/macc/projects/code_source/src/tools.ts)
- [code_source/src/tools/AgentTool/AgentTool.tsx](/Users/macc/projects/code_source/src/tools/AgentTool/AgentTool.tsx)
- [code_source/src/tools/AgentTool/builtInAgents.ts](/Users/macc/projects/code_source/src/tools/AgentTool/builtInAgents.ts)

Why it matters for Retina:
- this matches Retina's authority model
- it fits naturally with manifest-scoped permissions already present in the CLI/controller path

Recommendation:
- add a `ToolRegistry` and `ToolPolicy` layer outside the kernel
- let each agent manifest define:
  - allowed tools
  - denied tools
  - memory scope
  - budget
  - working root
  - MCP requirements

### 4. Tool orchestration and concurrency

Keep the pattern that some tools are concurrency-safe and some are exclusive.

Reference files:
- [code_source/src/services/tools/toolOrchestration.ts](/Users/macc/projects/code_source/src/services/tools/toolOrchestration.ts)
- [code_source/src/services/tools/StreamingToolExecutor.ts](/Users/macc/projects/code_source/src/services/tools/StreamingToolExecutor.ts)

Why it matters for Retina:
- this is one of the highest-value runtime ideas in the leak
- it will matter immediately once Retina grows beyond the current `Action` enum into richer delegated tool calls

Recommendation:
- add a Rust `ToolExecutor` that marks tools as:
  - read_only
  - mutation
  - long_running
  - streaming
- allow read-only tools to batch later
- keep mutation tools serial for now

### 5. MCP as a first-class extension surface

Keep the architecture, not the exact implementation.

Reference file:
- [code_source/src/services/mcp/client.ts](/Users/macc/projects/code_source/src/services/mcp/client.ts)

Why it matters for Retina:
- Retina's docs already point toward MCP transport and MCP-facing surfaces
- MCP is the cleanest way to expose Retina tools outward and consume external tools inward

Recommendation:
- add MCP in two layers:
  - `retina-mcp-client` for using external MCP tools/resources
  - `retina-mcp-server` later for exposing Retina as an MCP agent/tool host

Start with client-side only.

## What Not To Port

Do not port these directly:
- `main.tsx` bootstrap shell
- React/Ink UI tree
- Bun-specific feature-gating
- remote session cloud orchestration
- analytics and telemetry wiring
- giant command surface copied one-for-one
- product policy/settings sync layers

Reference files to avoid direct translation:
- [code_source/src/main.tsx](/Users/macc/projects/code_source/src/main.tsx)
- [code_source/src/commands.ts](/Users/macc/projects/code_source/src/commands.ts)

These files are useful for understanding system shape, but they are not good adoption targets.

## Files To Study Closely

Best keep/adapt candidates from the code_source tree:

- [code_source/src/Task.ts](/Users/macc/projects/code_source/src/Task.ts)
  - task identity, status model, base state
- [code_source/src/utils/task/framework.ts](/Users/macc/projects/code_source/src/utils/task/framework.ts)
  - registry, polling, eviction, notifications
- [code_source/src/tasks/LocalAgentTask/LocalAgentTask.tsx](/Users/macc/projects/code_source/src/tasks/LocalAgentTask/LocalAgentTask.tsx)
  - background agent lifecycle and progress tracking
- [code_source/src/tasks/RemoteAgentTask/RemoteAgentTask.tsx](/Users/macc/projects/code_source/src/tasks/RemoteAgentTask/RemoteAgentTask.tsx)
  - useful later for remote workers, not an MVP dependency
- [code_source/src/tools/AgentTool/AgentTool.tsx](/Users/macc/projects/code_source/src/tools/AgentTool/AgentTool.tsx)
  - delegation contract, isolation modes, backgrounding semantics
- [code_source/src/tools/AgentTool/runAgent.ts](/Users/macc/projects/code_source/src/tools/AgentTool/runAgent.ts)
  - subagent query loop wiring
- [code_source/src/tools/AgentTool/loadAgentsDir.ts](/Users/macc/projects/code_source/src/tools/AgentTool/loadAgentsDir.ts)
  - agent definitions, source layering, MCP requirements
- [code_source/src/services/tools/toolOrchestration.ts](/Users/macc/projects/code_source/src/services/tools/toolOrchestration.ts)
  - concurrency partitioning
- [code_source/src/services/tools/StreamingToolExecutor.ts](/Users/macc/projects/code_source/src/services/tools/StreamingToolExecutor.ts)
  - streaming and cancellation ideas
- [code_source/src/services/mcp/client.ts](/Users/macc/projects/code_source/src/services/mcp/client.ts)
  - MCP client/runtime patterns
- [code_source/src/utils/agentContext.ts](/Users/macc/projects/code_source/src/utils/agentContext.ts)
  - agent context propagation idea
- [code_source/src/utils/forkedAgent.ts](/Users/macc/projects/code_source/src/utils/forkedAgent.ts)
  - child-context isolation idea

## Target Retina Architecture

Retina should stay mostly CLI-based.

Not because CLI is the final form, but because it is the best root body for:
- code work
- local shell access
- file manipulation
- scripting
- MCP interop
- background task supervision

The right target shape is:

```text
operator / external caller
        |
    retina-cli
        |
   session controller
        |
   task supervisor
        |
      kernel
   /    |    \
shell reasoner memory
        |
   tool executor
   /   |   |   \
fs  shell mcp  agent-delegation
              |
         local specialists
              |
         transport later
```

## Recommended Crate Layout

### Keep as-is or evolve in place

- `retina-kernel`
  - keep the bounded execute loop
  - keep routing decisions
  - keep outcome and timeline semantics
- `retina-types`
  - extend existing task and agent types instead of replacing them
- `retina-traits`
  - keep trait boundaries stable
- `retina-cli`
  - remain the primary operator surface
- `retina-shell-cli`
  - remain the first body

### Add next

- `retina-runtime`
  - session runtime
  - task registry
  - background task handles
  - output buffering
  - task progress events
  - specialist lifecycle manager

- `retina-tools`
  - tool registry
  - tool metadata
  - tool policy filtering
  - tool executor
  - concurrency safety flags

- `retina-mcp-client`
  - external MCP connections
  - resource listing/reading
  - MCP-backed tools

- `retina-transport-local`
  - local specialist message bus
  - reactivation and spawn channel
  - this should be the first transport implementation

## Agent Tasks Model

Retina should formalize task types now even if only two are fully active at first.

Recommended task types:
- `session`
  - the foreground interactive worker task
- `command`
  - long-running shell commands
- `local_agent`
  - delegated worker in the same process or a sibling process
- `specialist`
  - manifest-scoped code/research/browser/ops worker
- `remote_agent`
  - deferred until after local transport works

Recommended task fields:
- `task_id`
- `task_kind`
- `owner_agent_id`
- `status`
- `started_at`
- `ended_at`
- `description`
- `prompt_or_objective`
- `output_path` or output buffer handle
- `progress_summary`
- `last_activity`
- `notified`

This should extend Retina's current `Task` type rather than replacing the user-task concept in [crates/retina-types/src/tasking.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/tasking.rs).

Recommendation:
- keep `Task` as the semantic user objective
- add `RuntimeTask` as the supervision record

That distinction is important.

## Subagents and Specialists

Retina should support two related but distinct concepts.

### Subagent

A short-lived delegated worker used to:
- search broadly
- investigate a bounded question
- write a draft or patch
- run verification in parallel

Properties:
- child of a parent task
- scoped tool set
- usually local
- may share memory or use a temporary child memory view

### Specialist

A named worker chamber with its own manifest and lifecycle.

Examples:
- `code`
- `research`
- `browser`
- `ops`

Properties:
- persistent identity
- lifecycle state
- budget
- authority scope
- memory scope
- reusable over many tasks

This aligns directly with Retina's existing agent lifecycle types in [crates/retina-types/src/agents.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/agents.rs).

Recommendation:
- implement subagents first as runtime children
- implement specialists second as manifest-backed reusable workers

Do not jump straight to a full queen/colony network.

## Tools

Retina should move toward a tool registry model instead of encoding all capability only as direct kernel actions.

That does not mean deleting `Action`.
It means layering tools above it.

Recommended split:

### Kernel action layer

Keep these as low-level body actions:
- read
- search
- inspect
- list
- write
- append
- run command
- respond

These already exist in [crates/retina-types/src/actions.rs](/Users/macc/projects/personal/agent-retina/crates/retina-types/src/actions.rs).

### Tool layer

Add higher-level tools that compile down into one or more actions:
- `file_read`
- `file_write`
- `glob`
- `grep`
- `bash`
- `structured_ingest`
- `agent_spawn`
- `task_get`
- `task_stop`
- `mcp_call`
- `mcp_read_resource`

Each tool should declare:
- name
- description
- input schema
- concurrency class
- approval policy
- authority requirements
- whether it is streaming

This is the single most useful structural adoption from the code_source harness.

## MCP

MCP should be treated as both:
- a source of external tools and resources
- a future interface surface for Retina itself

### Phase 1

Consume MCP only.

Use cases:
- connect to external tool servers
- expose MCP tools to the reasoner as available tools
- allow resource read/list operations

### Phase 2

Expose Retina as MCP.

Use cases:
- other agents can delegate into Retina
- Cursor/Claude/Desktop can use Retina's worker runtime
- Retina specialists can become interoperable chambers

### MCP architecture recommendation

Keep MCP outside the kernel trait surface at first.

Reason:
- MCP is best modeled as a tool/runtime concern first
- once stable, you can expose transport or tool-host traits around it

So the order should be:
1. `retina-mcp-client` feeds `retina-tools`
2. `retina-tools` feeds the reasoner context
3. only later add MCP server or MCP transport crates

## Mapping From Source Harness To Retina

### Direct conceptual mappings

- code_source `Task` -> Retina `RuntimeTask`
- code_source `AgentTool` -> Retina `agent_spawn` tool plus runtime supervisor
- code_source `LocalAgentTask` -> Retina local delegated worker runtime
- code_source tool pool assembly -> Retina manifest-scoped `ToolRegistry`
- code_source task framework -> Retina `retina-runtime`
- code_source MCP client -> Retina `retina-mcp-client`

### Do not map directly

- code_source `main.tsx` -> no Rust equivalent needed
- code_source React UI task panels -> replace with CLI inspect/status commands
- code_source Bun feature flags -> replace with manifest/config booleans
- code_source analytics context -> replace with timeline events already in memory

## Phased Adoption Plan

### Phase A: strengthen the runtime shell around the current worker

Build:
- `RuntimeTask` model
- task registry
- background command supervision
- `retina inspect tasks`
- output/progress capture

This is the minimum load-bearing step.

### Phase B: introduce a real tool registry

Build:
- `retina-tools`
- metadata for current shell capabilities
- concurrency classes
- policy filtering

Goal:
- keep the current `Action` layer
- let the reasoner reason over tools instead of only raw actions later

### Phase C: local subagents

Build:
- `agent_spawn` tool
- local child worker runtime
- task parent/child linkage
- specialist-independent child prompts

Goal:
- let the current worker delegate bounded work without full transport

### Phase D: specialist manifests and local transport

Build:
- specialist definitions
- manifest-scoped tool pools
- local transport crate
- reactivation/spawn execution path for router outputs

Goal:
- make `RoutingDecision::RouteToExisting`, `Reactivate`, and `SpawnSpecialist` real

### Phase E: MCP client integration

Build:
- external MCP server connections
- tool/resource adapters
- manifest requirements for specialists that depend on MCP

### Phase F: remote workers and MCP server surface

Only after the local swarm is strong.

## Effectiveness Assessment

This adoption path should be very effective for Retina if kept disciplined.

Why:
- Retina already has better architectural boundaries than the code_source app
- the code_source app has stronger runtime patterns than Retina currently has
- the two are complementary

What will make it ineffective:
- trying to port the code_source product layer wholesale
- copying UI-driven concepts into the kernel
- adding specialists before task supervision exists
- adding MCP before tool registry and policy layers exist

## Recommendation

For `vital-swarm`, the best path is:
- stay mostly CLI based
- make CLI the reference body
- add a real runtime supervisor
- add local subagents first
- add manifest-backed specialists second
- add MCP client third
- keep the kernel small and stable the entire time

That path preserves Retina's architecture and absorbs the strongest parts of the code_source harness without inheriting its coupling.
