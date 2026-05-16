# Source Brave MCP Alignment Audit

> Date: 2026-05-15
>
> Scope: compare Retina's new Brave/MCP web-research path with the relevant `code_source` architecture and behavior, then identify the remaining gaps in source alignment.

## Summary

Retina's Brave path is now operational:
- Brave MCP server connects successfully
- MCP tools are visible in the runtime
- the root worker can use Brave web search and answer from the results
- the Tokio runtime crash is gone
- resource-less MCP servers no longer get discarded as failures

That is a real step forward.

The main remaining difference from `code_source` is not "can Retina search the web?" It can.

The main remaining difference is:

`code_source` exposes external search as first-class tools in the normal agent tool world, while Retina still exposes MCP mostly through a generic wrapper surface.`

That wrapper surface is good enough to work, but it still causes avoidable friction:
- repeated search retries
- occasional argument-shape mistakes
- the model having to reason about `server` / `tool` / `input_json` instead of just using the concrete tool

So the biggest next source-aligned improvement is:

`move from generic MCP wrapper actions toward concrete MCP tool exposure in the reasoner-facing tool surface.`

## What Matches Source Well Now

### 1. MCP is the extension surface

This is aligned.

Retina is not adding a custom Brave HTTP client inside the kernel. Brave comes in through MCP and is consumed through the normal runtime/tool path.

That matches the source direction:
- one shared worker shape
- one shared tool world
- MCP as pluggable external capability

Relevant Retina files:
- [retina-mcp-client/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-mcp-client/src/lib.rs)
- [retina-cli/src/controller.rs](/Users/macc/projects/personal/agent-retina/crates/retina-cli/src/controller.rs)
- [retina-tools/src/builtins.rs](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/builtins.rs)

Relevant source files:
- [query.ts](/Users/macc/projects/code_source/src/query.ts)
- [agentToolUtils.ts](/Users/macc/projects/code_source/src/tools/AgentTool/agentToolUtils.ts)

### 2. Subagents still use the same worker shape

This is aligned.

Retina did not create a special Brave agent runtime. Root and specialists both consume the same shared worker architecture with scoped tools.

That is consistent with source's general-purpose subagent direction:
- [generalPurposeAgent.ts](/Users/macc/projects/code_source/src/tools/AgentTool/built-in/generalPurposeAgent.ts)

### 3. Tool scope is now much better than before

This is materially closer to source than where Retina started.

Retina now:
- respects scoped tool surfaces
- carries MCP through authority/tool policy
- can expose Brave to the research path without a separate harness branch

Relevant Retina files:
- [retina-tools/src/policy.rs](/Users/macc/projects/personal/agent-retina/crates/retina-tools/src/policy.rs)
- [retina-transport-local/src/lib.rs](/Users/macc/projects/personal/agent-retina/crates/retina-transport-local/src/lib.rs)

## Main Gaps Vs Source

## 1. Retina still exposes MCP as generic wrapper actions

This is the biggest remaining gap.

Today the reasoner mainly sees:
- `list_mcp_resources`
- `read_mcp_resource`
- `mcp_call`

That means the model must decide:
- which server
- which tool
- what input JSON shape

By contrast, `code_source` passes actual resolved tools into the normal tool set. The agent reasons over the concrete tools it has, not over a generic MCP RPC wrapper.

Source evidence:
- [query.ts](/Users/macc/projects/code_source/src/query.ts) passes `mcpTools: appState.mcp.tools`
- [agentToolUtils.ts](/Users/macc/projects/code_source/src/tools/AgentTool/agentToolUtils.ts) resolves the actual allowed tools for the child

Why this matters:
- generic wrapper surfaces increase planning burden
- they make argument-shape mistakes more likely
- they create weaker source-style ergonomics than a first-class tool list

This is likely why Retina still needed:
- prompt nudges
- retry tolerance
- extra evidence shaping

Recommended next move:
- expose MCP tools to the reasoner as concrete tool descriptors, not only via the generic `mcp_call` abstraction
- keep the generic runtime backend if helpful, but stop making the model think in raw `server/tool/input_json` terms by default

## 2. MCP tool argument ergonomics are still weaker than source

Retina currently leaves too much schema assembly burden on the model.

In practice this showed up as:
- Brave search input validation errors
- repeated calls before successful argument shape

Source handles this better because the agent interacts with named tools in the normal tool world, not through a low-level generic call envelope.

Recommended next move:
- add richer argument-shape descriptions into MCP-backed tool descriptors
- ideally expose each MCP tool as its own callable action surface
- keep `mcp_call` as an escape hatch, not the normal happy path

## 3. Retina still has a thinner MCP result model than source-style tool use

Retina has improved this already:
- connected MCP server discovery
- better `inspect mcp`
- better carried-forward result highlights

But it still mostly stores:
- `content_preview`
- `structured_content`

That is enough to work, but it is still more generic than ideal.

Source-style behavior would feel stronger if the agent saw a more natural result shape for web search:
- titles
- URLs
- snippets
- maybe category-specific top hits

Recommended next move:
- keep the runtime generic
- but enrich the MCP result shaping layer for search-like tool outputs when structured fields are obvious
- do this in a generic "structured search result summarizer" way, not as a Brave-only special case

## 4. Retina is still somewhat more prompt-dependent than source on web search

Retina now needs prompt guidance like:
- prefer MCP over shell web scraping
- answer from successful search results instead of repeating search

These nudges are fine and helpful, but they are compensating for the weaker tool surface described above.

In `code_source`, better behavior comes more naturally from:
- concrete tool availability
- better tool semantics
- better child prompt construction

So the prompt work is acceptable, but it should not be the long-term main solution.

Recommended next move:
- keep the prompt nudges
- but reduce reliance on them by improving concrete MCP tool exposure

## 5. Retina still has more shell fallback gravity than source for web tasks

This is improved, not solved.

Before the Brave fixes, Retina would slide into `curl`-based scraping for web/recommendation tasks.
Now it can use Brave correctly.

But the architecture still makes shell fallback more available than source-style first-class search tools.

Recommended next move:
- once concrete MCP tool exposure exists, prefer those tools in the normal tool ordering/description
- keep shell available, but stop making it feel like the peer/default path for ordinary web search tasks

## What We Learned From The Live Runs

The runs were useful because they separated different classes of failure:

### Solved
- Tokio runtime panic on MCP process launch
- MCP server rejection caused by unsupported resource listing
- `respond` field mismatch (`content` vs `message`)
- `read_mcp_resource` dead-end for a tool-only server

### Improved
- repeated Brave calls before answer
- answer-only tasks creating files they did not need

### Still not fully source-tight
- the model still has to think too hard about generic MCP invocation
- the tool/result surface is still not as natural as source's concrete tool world

## Recommended Next Order

### 1. Promote MCP tools into first-class reasoner-facing tools

This is the highest-value source-alignment move.

Target direction:
- instead of only `mcp_call`
- expose concrete tool descriptors like the MCP tool world the source agent sees

This can still be implemented on top of the same MCP runtime backend.

### 2. Keep `inspect mcp`

This is not in conflict with source alignment.

It is a practical operator/debug surface and helped identify the real Brave issue immediately:
- server connected
- 8 tools
- 0 resources

That should stay.

### 3. Enrich generic search-result shaping

Do this in the result/evidence layer, not by hardcoding specific tasks.

Target:
- turn structured search results into better recent-action summaries
- help the next step feel obviously answer-ready

### 4. Only after that, decide whether to narrow shell fallback further

Do not ban shell.

Just make the real MCP search surface strong enough that the model chooses it naturally.

## What Not To Do

- Do not add a Brave-specific kernel subsystem
- Do not add hardcoded "date ideas" or "recommendation" routes
- Do not special-case Colorado Springs-style tasks
- Do not remove local-first reasoning
- Do not replace the shared worker model with a bespoke web-search agent

## Bottom Line

Retina is now functionally using Brave through MCP, which is the correct source-aligned capability direction.

The biggest remaining source gap is not runtime stability anymore.

It is this:

`Retina still presents MCP too much like a generic transport wrapper, while source presents external capabilities more like normal concrete tools.`

That is the next real alignment move.
