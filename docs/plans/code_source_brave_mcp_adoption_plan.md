# Code_Source Brave MCP Adoption Plan

> Purpose: add Brave web search to Retina through the existing MCP extension path, following the `code_source` direction of using MCP as a first-class tool surface rather than building a custom one-off search client inside the kernel.

## Summary

Retina's local worker path is now in a much healthier place:
- local file/document tasks work
- delegated local specialists work
- MCP client/runtime support already exists

That makes Brave web search the right next capability layer.

The source-aligned approach is:
- do **not** add a custom Brave HTTP client directly to Retina's kernel
- do **not** special-case web search as a separate runtime path
- do use MCP as the extension surface
- let the base agent and research specialist consume Brave through the same tool registry/policy path as other MCP-backed tools

This keeps Retina aligned with the `code_source` architecture:
- one shared worker shape
- one shared kernel loop
- one shared tool registry
- MCP as the pluggable external capability layer

## Why This Is The Right Next Step

The current gap is not local work anymore.
The next big missing capability is external research.

Brave search adds:
- current web information
- broader research beyond local files
- a strong use case for the research specialist

And it does so without requiring a new kernel subsystem.

## Source-Aligned Direction

Retina already has the right base for this:
- MCP client runtime in [crates/retina-mcp-client/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-mcp-client/src/lib.rs)
- MCP-aware tool exposure in [crates/retina-tools/src/builtins.rs](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/builtins.rs)
- MCP-aware policy in [crates/retina-tools/src/policy.rs](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/policy.rs)
- MCP runtime wiring in [crates/retina-cli/src/controller.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs)
- local transport/runtime support for MCP-gated specialists in [crates/retina-transport-local/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-transport-local/src/lib.rs)

That matches the source direction well enough:
- external capability comes in through MCP
- agents consume it through scoped tools
- specialists can require it through manifest metadata

So Brave should be added through that path, not beside it.

## Non-Goals

This plan is **not**:
- a custom Brave REST client inside Retina
- a kernel-level `web_search` special case
- Brave-specific routing logic in the kernel
- a rewrite of the current MCP client path

It is also not yet:
- browser automation
- page interaction
- form filling
- remote/distributed worker work

## Target Behavior

After this plan:
- the root worker can use Brave-backed MCP tools for web research when needed
- the research specialist can use Brave-backed MCP tools through the same scoped tool path
- MCP availability is visible in inspect/debug surfaces
- local-first behavior remains intact
- web search becomes an additive capability, not a replacement for local evidence gathering

## Implementation Phases

### Phase 1: Brave MCP server configuration

Set up a local Brave MCP server on macOS and wire it into Retina's MCP config.

Retina already expects MCP server config at the default path used by:
- [crates/retina-mcp-client/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-mcp-client/src/lib.rs)

Tasks:
- choose the Brave MCP server package/runtime to use
- make sure `BRAVE_API_KEY` is available to that server process
- add an entry to `~/.retina/mcp/servers.toml`
- verify the configured server launches successfully through `ConfiguredMcpRuntime`

Success criteria:
- the server appears in the MCP snapshot
- tools/resources are discoverable

### Phase 2: MCP visibility and tool exposure validation

Retina already exposes generic MCP actions:
- `list_mcp_resources`
- `read_mcp_resource`
- `mcp_call`

The next step is to confirm the Brave server appears through the normal tool path.

Tasks:
- validate the Brave server is present in runtime snapshots
- validate tool descriptors are present through the tool registry
- verify root-agent tool scope includes MCP when authority allows it
- verify the research specialist inherits or requires MCP correctly

Success criteria:
- Brave-backed MCP tools are visible to the agent through the normal prompt/tool surface
- no extra Brave-specific tool plumbing is required in the kernel

### Phase 3: Specialist alignment

The research specialist should be able to use Brave search through the same general-purpose worker shape.

Tasks:
- ensure the research specialist keeps MCP authority available
- optionally add `required_mcp_servers = ["brave"]` later if we want strict specialist gating
- keep wildcard or source-aligned tool inheritance unless there is a strong reason to narrow further

Important constraint:
- do not make Brave a special hardcoded requirement for the whole system
- only use manifest-level MCP requirements where they materially help

### Phase 4: Prompt/usage guidance

Only add light generic guidance after the MCP path is working.

Desired behavior:
- prefer local evidence first when the task is clearly local
- use Brave when the task needs current or external information
- for research specialists, use web search as one tool among others, not as the default first move for every task

This should stay prompt-level and generic:
- no Brave-specific kernel routing
- no hardcoded search workflow

### Phase 5: End-to-end validation

Test real search tasks through the live agent.

Core tests:
- direct web research question
- mixed local + web task
- research specialist delegation using Brave
- no-regression test for local-only file/document tasks

Success criteria:
- Brave-backed searches complete through MCP
- the agent can cite/use the results in grounded answers
- local behavior remains stable

## Configuration Notes

The main local seam to use is:
- MCP server config in `~/.retina/mcp/servers.toml`

The main runtime assumptions:
- the Brave MCP server runs as a stdio server
- `BRAVE_API_KEY` is passed through the MCP server environment
- Retina consumes it through the generic MCP runtime

This keeps Brave fully outside the kernel's core logic.

## Recommended Tool/Authority Position

### Base agent

The base/root agent should continue to have MCP available when authority allows it.

Reason:
- source-style agents use one shared tool world, then scope/narrow when needed
- Brave search is a capability, not a separate base-agent subsystem

### Research specialist

The research specialist should definitely be able to use MCP/Brave.

Reason:
- that is the most natural place to benefit from external search
- it keeps the delegated research path valuable

### Other specialists

Do not force Brave into every specialist definition up front.
Let shared MCP authority and tool inheritance do the work unless a specific specialist needs stricter gating.

## Test Plan

### 1. MCP config sanity

Verify:
- Brave server launches from Retina config
- runtime snapshot shows the server

### 2. Root-agent web research

Example shape:
- "search the web for current Brave Search API details and summarize them"

Expected:
- agent uses MCP path
- grounded answer based on returned MCP content

### 3. Research specialist delegation

Example shape:
- "research current information about X"

Expected:
- research specialist can use Brave-backed MCP tools
- delegated result returns cleanly to the parent

### 4. Mixed local + web task

Example shape:
- compare a local document claim with current public web information

Expected:
- local evidence and web evidence can coexist in one run

### 5. No local regression

Repeat:
- local file summary task
- local PDF summary task

Expected:
- no unnecessary web search when local evidence is sufficient

## Success Criteria

This plan is complete when:
- Brave is available to Retina through the MCP client path
- the root worker can use it
- the research specialist can use it
- the kernel remains generic and unchanged in strategy
- local-first behavior is preserved
- no custom Brave-specific execution path was added

## Current Recommendation

Do this before the bigger form/document filling project.

Reason:
- Brave/MCP is a clean capability addition
- form/template filling is a larger new feature area
- finishing Brave first keeps the next step small, source-aligned, and high value
