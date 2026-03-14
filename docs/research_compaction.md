# Research: Prompt Caching and Context Compaction for Long-Running Agents

> As of March 14, 2026, the best Claude-backed strategy is not raw transcript growth and not generic rolling summaries. It is a layered system: cache the stable prefix, compact working context around task state and progress, keep full evidence outside the prompt, and retrieve exact details only when needed.

## Purpose

This document defines the recommended direction for Retina's token-saving and long-horizon context strategy.

It answers two related questions:

1. What is the most effective way to save tokens with Claude?
2. What is the most effective way to compact context for long-running agents when direct KV compaction is not available?

If implementation details are ambiguous, resolve them against:

1. [docs/v1_plan.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/v1_plan.md)
2. [docs/research_overview.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_overview.md)
3. [docs/trait_contracts.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/trait_contracts.md)
4. [docs/research_memory.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_memory.md)
5. [docs/research_memory_v2.md](/Users/macc/Projects/gabanode_lab/agent-retina/docs/research_memory_v2.md)
6. This document

## Short Answer

For Claude-backed Retina agents, the strongest current strategy is:

1. Use **prompt caching** for the stable prefix only.
2. Keep the active prompt small and **task-centered**, not transcript-centered.
3. Compact old working context into a **canonical task-state artifact**, not a loose prose summary.
4. Keep full evidence outside the prompt as indexed artifacts and episodic memory.
5. Retrieve exact evidence only when the current subgoal needs it.
6. Use Claude **server-side compaction** and **context editing** when the chosen model supports them, but do not make them the only memory mechanism.

The practical formula is:

`cached prefix + current task state + recent verified progress + next action frontier + indexed evidence references`

That is the next-best thing to KV compaction for API models, and for Claude it is better than naive summaries alone.

## The Key Distinction

These are related, but they are not the same thing:

- **Prompt caching**
  - Saves cost and latency on repeated prefixes.
  - Does not solve long-horizon reasoning by itself.
- **Context compaction**
  - Reduces what stays in the active prompt.
  - Decides how earlier work is represented going forward.
- **Durable memory**
  - Stores the full history, evidence, and lessons outside the prompt.
  - Allows recovery, replay, retrieval, and learning.

Retina should treat them as separate layers that work together.

## What Claude Supports Today

### 1. Prompt caching

Claude supports prompt caching with automatic caching or explicit `cache_control` breakpoints. Anthropic recommends putting static content first and notes that cache prefixes are hierarchical in this order: `tools -> system -> messages`. Changes at a higher level invalidate lower levels. Prompt caching can reduce costs by up to 90% and latency by up to 80%, with 5-minute and 1-hour TTL options. [Prompt caching docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching), [API release notes](https://platform.claude.com/docs/en/release-notes/overview)

Important implementation implications:

- Cache tools and system separately when possible.
- Do not mix highly mutable transcript content into the same cache segment as your long-lived system prompt.
- Use 5-minute TTL for hot loops.
- Use 1-hour TTL for side agents, longer pauses, or workflows where follow-ups often land after 5 minutes.

### 2. Server-side compaction

Claude now has server-side compaction for long-running conversations. Anthropic describes it as the recommended primary strategy for long-running conversations and agentic workflows on supported models. It summarizes older conversation context into a `compaction` block and continues from there. It also allows custom summarization instructions and can be combined with prompt caching. [Compaction docs](https://platform.claude.com/docs/en/build-with-claude/compaction), [Context windows docs](https://platform.claude.com/docs/en/build-with-claude/context-windows)

As of March 14, 2026, Anthropic documents compaction support for:

- `claude-opus-4-6`
- `claude-sonnet-4-6`

This matters for Retina because server-side compaction is the best built-in Claude mechanism for long-running API sessions. But it is still summary-based, so it should not be the only long-term continuity mechanism.

### 3. Context editing

Anthropic's context editing can clear stale tool results and thinking blocks. Anthropic explicitly positions this as the more granular option, while server-side compaction is the primary strategy. Their context-management writeup reports that combining memory plus context editing improved performance by 39% over baseline, while context editing alone improved it by 29%, and in a 100-turn web-search evaluation context editing reduced token consumption by 84%. [Context editing docs](https://platform.claude.com/docs/en/build-with-claude/context-editing), [Managing context on the Claude Developer Platform](https://claude.com/blog/context-management)

This is especially relevant for Retina because tool-heavy agents generate lots of stale output that should not stay in the prompt.

## What the Research Says

### 1. Context is a finite attention budget

Anthropic's context-engineering guidance is clear: more context is not automatically better, and long contexts degrade focus. The goal is the smallest set of high-signal tokens that maximizes success. [Effective context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)

This strongly supports Retina's core design:

- small assembled context
- pull, don't push
- just-in-time discovery
- durable memory outside the prompt

### 2. Compaction is necessary, but summary-only compaction is not enough

Anthropic's long-running harness research is equally clear: compaction helps, but by itself it is not sufficient. Their effective harness for long-running agents adds explicit state artifacts like:

- an initializer artifact
- a progress log
- a feature list
- git history / recovery anchors

The goal is not just to summarize a transcript. It is to make the next session immediately understand:

- what the real task is
- what has already been verified
- what remains
- what the current clean starting point is

[Effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)

That is a direct match for Retina.

### 3. Indexed memory beats summary-only memory

Recent research strengthens the same conclusion:

- **Memex(RL)** argues that truncation and running summaries are fundamentally lossy, and proposes a compact working context with concise structured summaries plus stable indices to recover exact underlying evidence when needed. [Memex(RL)](https://arxiv.org/abs/2603.04257)
- **MemRL** shows that episodic retrieval should not be ranked by similarity alone; utility matters. High-value experiences should outrank merely similar noise. [MemRL](https://arxiv.org/abs/2601.03192)
- **A-MEM** shows that memory should be structured, linked, and continuously reorganized, not treated as flat chunks. [A-MEM](https://arxiv.org/abs/2502.12110)
- **E-mem** shows that reconstructing episodic context from compact higher-level structure can outperform passive preprocessing and lower token cost substantially. [E-mem](https://arxiv.org/abs/2601.21714)

Taken together, the research does **not** point toward "one rolling summary." It points toward:

- compact active state
- full evidence stored elsewhere
- explicit indices / references
- value-aware retrieval
- structured task continuity

### 4. KV compaction is the ideal for local runtimes, not for Claude API

The MIT attention-matching paper makes the case that direct KV compaction is superior to summary-based token-space compaction when you control the runtime. [Fast KV Compaction via Attention Matching](https://arxiv.org/abs/2602.16284)

That is important for Retina's long-term local-model direction.

But with Claude API, we do not control the runtime KV cache directly. So Retina should treat KV compaction as:

- **long-term local-model target**
- **not the current API-model answer**

For Claude, the next-best approach is a provider-aware harness strategy, not a fake KV layer.

## Recommendation for Retina

### The recommended stack

For Claude-backed long-running agents, Retina should use this stack in order:

1. **Stable prefix prompt caching**
2. **Context editing for stale tool outputs**
3. **Server-side compaction on supported Claude 4.6 models**
4. **Harness-owned canonical task-state compaction**
5. **Indexed episodic memory outside the prompt**
6. **Utility-ranked recall and promotion into rules/knowledge**

This keeps the model free to reason while the harness prevents context bloat.

### The canonical task-state artifact

Retina should compact long-running work into a structured task-state artifact, not a generic prose summary.

The compacted state should preserve:

- **Goal**
  - task identity
  - success criteria
  - hard constraints
- **Working set**
  - which docs, files, URLs, or artifacts are currently authoritative for this task
  - why each source matters
  - whether it has been read, partially read, or superseded
- **Progress**
  - completed checkpoints
  - verified outcomes
  - current phase
- **Actions**
  - last meaningful actions
  - last result
  - failed paths worth avoiding
- **World state**
  - key observed facts that still constrain the next step
  - artifact references and exact paths/IDs
- **Open frontier**
  - unresolved questions
  - blockers
  - next best actions

This is the core of the compaction policy.

### Recommended compact state shape

Retina should eventually compact long-running tasks into a structure close to:

```json
{
  "task": {
    "goal": "Find Craig Lyons resume on Desktop/resume and answer the last-job question",
    "success_criteria": [
      "Locate the correct resume artifact",
      "Extract readable text",
      "Answer with evidence from the artifact"
    ],
    "constraints": [
      "Prefer readable or extracted text over binary reads",
      "Do not mutate files"
    ]
  },
  "progress": {
    "phase": "answering",
    "completed": [
      "Searched Desktop/resume",
      "Located candidate files",
      "Read markdown resume"
    ],
    "verified_state": [
      "resume/Craig Lyons resume.md exists",
      "resume/Craig Lyons.pdf exists"
    ]
  },
  "frontier": {
    "next_action": "Answer from markdown evidence",
    "open_questions": [],
    "blockers": []
  },
  "recent_actions": [
    {
      "action": "find_files",
      "result": "matched 2 candidate resume artifacts"
    },
    {
      "action": "read_file",
      "result": "extracted current role from markdown resume"
    }
  ],
  "artifacts": [
    {
      "kind": "file",
      "path": "/Users/macc/Desktop/resume/Craig Lyons resume.md",
      "status": "read"
    }
  ],
  "avoid": [
    "do not read raw PDF bytes directly"
  ]
}
```

The exact schema can change, but the idea should stay stable:

- immutable task frame
- authoritative working set
- verified progress
- next-step frontier
- references to exact evidence

## What to Keep, Compact, and Archive

### Keep in active prompt

Keep only the smallest high-signal set:

- current task / subgoal
- success criteria
- active constraints
- current authoritative docs or files being worked from
- current phase
- next action frontier
- last 1-3 meaningful step results
- one small recalled memory slice if directly relevant
- artifact references needed immediately

### Compact into canonical task state

Compact material that still matters but does not need raw form:

- explored branches and why they mattered
- key decisions
- completed checkpoints
- which documents have already been inspected and what role they play
- unresolved blockers
- verified paths / identifiers / URLs / handles
- tool preferences discovered during the task
- summary of important extracted evidence

### Archive outside the prompt

Keep full fidelity outside the prompt:

- raw tool outputs
- full transcripts
- full file contents after extraction unless currently needed
- screenshots, PDFs, and other large artifacts
- old command outputs and logs
- detailed exploratory traces

These should stay addressable by path, row ID, hash, or memory record ID.

## Ranking Rules for Compaction

When Retina decides what survives compaction, rank by this order:

1. **Goal criticality**
   - Does this directly define what success means?
2. **Forward utility**
   - Is this likely to change the next action?
3. **State dependency**
   - Would losing this make the agent misread the current world state?
4. **Recovery value**
   - Would a fresh session need this to resume quickly?
5. **Irreversibility**
   - Does this capture a mutation, decision, or failure that must not be repeated?
6. **Exactness requirement**
   - Does this need an exact reference instead of a summary?

This implies some clear rules:

- preserve exact file paths, IDs, URLs, selectors, hashes, and commit refs
- preserve the active source set the agent is reasoning from
- preserve exact blockers and failed assumptions
- preserve exact user constraints
- compact repeated confirmations
- compact verbose tool output after extracting the usable state
- compact exploration traces once they are no longer decision-critical

## Source-set memory matters

For Retina, compaction must preserve not just "what happened" but also "what I am currently working from."

If the agent is using:

- a specific README
- one architecture doc
- a PDF it extracted
- a source file it is modifying
- a spec or API page that defines the task boundary

then that source set becomes part of the task state.

This should be represented explicitly, not left implicit in transcript history.

Recommended fields per active source:

- `source_id`
- `kind` (`file`, `pdf`, `doc`, `url`, `api_reference`, `artifact`)
- `locator` (absolute path or canonical URL)
- `role` (`authoritative`, `supporting`, `candidate`, `superseded`)
- `status` (`queued`, `read`, `partially_read`, `excerpted`, `invalidated`)
- `why_it_matters`
- `last_used_step`
- `evidence_refs`

Why this matters:

- it prevents compaction from severing the link between conclusions and the document they came from
- it makes resuming work much easier after long pauses or handoffs
- it lets the agent keep a compact "reading desk" instead of replaying whole transcripts
- it matches the research direction of just-in-time context, externalized memory, and explicit task artifacts rather than giant conversation history

For implementation, Retina should maintain a compact **working source set** alongside the canonical task-state artifact. The agent should be able to answer:

- what sources am I currently depending on?
- which of them are authoritative?
- which ones have already been read or extracted?
- which exact source should I revisit if I need more detail?

## Prompt Caching Guidance for Retina

### What to cache

Cache the prefix that changes least often:

- tool definitions
- system prompt
- agent identity
- authority / safety / policy instructions
- stable project guidance
- stable environment overview

For long-running sessions, also cache:

- the latest compaction block when using Claude compaction
- the canonical task-state artifact when it becomes large enough to cross caching thresholds

### What not to cache aggressively

Avoid tying frequently changing material to the same cache segment as the stable prefix:

- fresh user requests
- volatile tool outputs
- rapidly changing message history
- mutable state summaries that are rewritten every turn

### Practical Claude caching recommendations

- Cache the system prompt separately from the evolving conversation.
- Use explicit breakpoints when you need stable cache hits across mutable messages.
- Add additional breakpoints if content exceeds Anthropic's ~20-block lookback window.
- Use 1-hour TTL for side-agents or workflows likely to idle longer than 5 minutes.
- Track:
  - `cache_read_input_tokens`
  - `cache_creation_input_tokens`
  - `input_tokens`

## The Best Claude-Side Compaction Strategy

### If using Claude 4.6 models

Use all three:

1. **Prompt caching**
2. **Server-side compaction**
3. **Harness-owned canonical task state**

Recommended pattern:

- cache tools and system prompt separately
- let Claude compact the conversation when near threshold
- customize compaction instructions so the generated summary preserves:
  - task state
  - progress
  - next steps
  - key learnings
  - exact artifact references when possible
- persist the compacted state into Retina memory as well
- do not trust compaction summary alone as the only recovery source

### If using older Claude models or provider-portable mode

Use:

1. **Prompt caching**
2. **Context editing / tool-result trimming where available**
3. **Retina-managed canonical task-state compaction**
4. **Indexed evidence retrieval**

This should be the default provider-independent Retina design anyway.

## What Retina Should Not Do

Do not:

- keep growing a single transcript indefinitely
- rely on one rolling prose summary as the main continuity artifact
- dump raw memory or large episodic chunks into prompts
- summarize away exact identifiers, paths, or failure constraints
- use cache as if it were durable memory
- hide compaction errors behind heuristic fallback logic
- over-script what the agent must remember in advance

The system should learn from the environment and its own experience, not from brittle hardcoded routing.

## Recommended Implementation Order

### Phase 1: make working context task-centric

Implement a canonical task-state artifact in the harness with fields for:

- goal
- constraints
- completed checkpoints
- current frontier
- recent actions
- artifact references
- avoid list / failed paths

This is the most important step.

### Phase 2: split cached prefix from mutable state

Update the Claude reasoner integration so:

- tools and system prompt are cached independently
- mutable task-state blocks are separated from the stable prefix
- cache metrics are recorded in the timeline

### Phase 3: add compaction policies

Add compaction triggers based on:

- token count
- number of tool interactions
- number of step transitions
- explicit phase boundary
- inactivity / handoff boundary

### Phase 4: add state-aware compaction writers

When compaction is triggered, write:

- updated canonical task state
- exact artifact references
- extracted evidence records
- continuation frontier

Do not write only a prose summary.

### Phase 5: add Claude-native compaction path

When running on Claude 4.6:

- enable server-side compaction
- provide custom compaction instructions tuned for Retina's task-state format
- cache compaction blocks when beneficial

### Phase 6: evolve toward indexed memory compaction

Build toward a Memex-style pattern:

- compact active state in prompt
- full evidence in SQLite / filesystem artifacts
- exact dereferencing on demand
- utility-aware ranking on retrieval

### Phase 7: local-model KV compaction later

When Retina runs on local models with runtime control, revisit:

- attention-level KV compaction
- latent compaction
- local model memory compression

That belongs to the local-model path, not the Claude API path.

## Final Recommendation

The best compaction strategy for Retina on Claude is:

**cache the stable prefix, compact the live task into structured state, keep raw evidence outside the prompt, and retrieve exact details by reference.**

If Claude server-side compaction is available, use it.
If it is not, the harness should still work well because the real continuity layer is not "conversation summary." It is:

- task
- progress
- actions
- verified state
- indexed evidence
- next frontier

That is the compaction model Retina should implement.

## Sources

- Anthropic, [Prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- Anthropic, [Compaction](https://platform.claude.com/docs/en/build-with-claude/compaction)
- Anthropic, [Context windows](https://platform.claude.com/docs/en/build-with-claude/context-windows)
- Anthropic, [Context editing](https://platform.claude.com/docs/en/build-with-claude/context-editing)
- Anthropic, [Memory tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
- Anthropic, [Effective context engineering for AI agents](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- Anthropic, [Managing context on the Claude Developer Platform](https://claude.com/blog/context-management)
- Anthropic, [Effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)
- Anthropic, [New capabilities for building agents on the Anthropic API](https://www.claude.com/blog/agent-capabilities-api)
- Wang et al., [Memex(RL): Scaling Long-Horizon LLM Agents via Indexed Experience Memory](https://arxiv.org/abs/2603.04257)
- Zhang et al., [MemRL: Self-Evolving Agents via Runtime Reinforcement Learning on Episodic Memory](https://arxiv.org/abs/2601.03192)
- Xu et al., [A-MEM: Agentic Memory for LLM Agents](https://arxiv.org/abs/2502.12110)
- Wang et al., [E-mem: Multi-agent based Episodic Context Reconstruction for LLM Agent Memory](https://arxiv.org/abs/2601.21714)
- Zweiger et al., [Fast KV Compaction via Attention Matching](https://arxiv.org/abs/2602.16284)
