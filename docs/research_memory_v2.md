# Research: The Memory Vessel — Beyond RAG

> RAG is a search engine pretending to be memory. Real memory doesn't wait to be queried — it shapes every thought, every decision, every reflex. This document designs a memory system that **changes how the agent behaves**, not just what it retrieves.

**Note:** The retrieval concepts here (experiential store, knowledge graph, reflexive rules, utility scoring) inform the `Memory` trait's internal implementation. The kernel calls `memory.recall_experiences(query)` and `memory.recall_knowledge(query)` — how the Memory impl searches (vectors, graph traversal, hybrid) is its business. See [trait_contracts.md](trait_contracts.md) for the 5-trait architecture.

---

## The Problem with Vector/RAG Memory

The standard agent memory playbook is:
1. Something happens → embed it → store the vector
2. Something new happens → embed the query → cosine similarity → retrieve top-K
3. Stuff retrieved text into the context window → hope the LLM uses it

This is **search**, not **memory**. It fails in critical ways:

- **It's passive.** The agent must explicitly query. If it doesn't ask the right question, the memory doesn't surface.
- **It's lossy.** Embedding a paragraph into 384 floats destroys most of the information. You can't reconstruct the original from the vector.
- **It's disconnected.** Retrieved memories are injected as text blobs — they don't change the agent's behavior, preferences, or reflexes.
- **It's flat.** Every memory is a point in vector space. There's no structure, no causality, no temporal ordering, no "this caused that."
- **It forgets by accident.** If the retrieval score is below threshold, the memory effectively doesn't exist — even if it's critical.

**The question isn't "how do we store and retrieve memories." It's: "if this agent had to always remember everything that mattered, what would it need?"**

---

## Five Types of Memory That Actually Matter

Forget the L0/L1/L2/L3 tier model from the previous doc. That's an implementation detail. Let's think about what the agent actually needs to remember and HOW that memory should manifest:

### 1. Observational Memory — "What I See"

**What it is:** The raw sensory stream. AX tree snapshots, screenshots, terminal output, file states. This is the agent's perception of the world at each moment.

**How it should work:** Not stored forever — **compacted**. The MIT "Fast KV Compaction via Attention Matching" paper (Feb 2026) shows that you can compress KV cache entries by up to 50x while preserving the two mathematical properties that matter: the **attention output** (what information the model extracts) and the **attention mass** (the relative weight of each token). This means instead of deleting old observations, you **condense** them — the agent still "remembers" them, but they take up 50x less space in the context.

**Key insight from the paper:** Previous approaches like SnapKV and H2O worked by **evicting** tokens (keeping "heavy hitter" tokens and dropping the rest). Compaction is fundamentally different — it **merges** tokens, preserving information that eviction destroys. It's the difference between throwing away old photos and compressing them.

**For Retina:** Every kernel_execute cycle produces observational data. Instead of logging it to SQLite and hoping to retrieve it later, we compact it into the KV cache in real-time. The agent carries a compressed representation of everything it's seen — always present, never retrieved.

**Research to build on:**
- [Fast KV Compaction via Attention Matching](https://arxiv.org/abs/2602.16284) (MIT, Feb 2026) — 50x compression, 100x faster than gradient-based methods
- [PyramidKV](https://openreview.net/forum?id=ayi7qezU87) — Dynamic cache allocation: more cache in lower layers, less in higher (mirrors how LLMs funnel attention)
- [ChunkKV](https://arxiv.org/html/2502.00299v5) — Preserves semantic continuity by compressing contiguous chunks rather than sparse tokens
- [StreamingLLM](https://github.com/mit-han-lab/streaming-llm) — Keeps "attention sinks" (initial tokens) that anchor the attention pattern
- [NVIDIA kvpress](https://github.com/NVIDIA/kvpress) — Production framework for KV cache compression

### 2. Experiential Memory — "What I Did and What Happened"

**What it is:** Action-outcome pairs with causal links. Not just "I clicked the Save button" but "I clicked the Save button **because** the AX tree said it was at (450, 220) **and** the screen didn't change **because** the AX tree coordinates were stale **so** I learned to use Vision Mode for Chrome."

**How it should work:** This is where [MemRL](https://arxiv.org/abs/2601.03192) (Jan 2026) is critical. MemRL treats episodic memories not as text to retrieve but as **experiences with learned utility values**. Each experience gets a Q-value through reinforcement learning:

- Experience helped the agent succeed → Q-value increases
- Experience led to failure → Q-value decreases
- Retrieval uses **two-phase filtering**: first semantic relevance, then Q-value ranking

This means the agent doesn't just remember what happened — it remembers **what was useful**. The memory system itself learns which experiences are worth surfacing.

**The Voyager pattern:** [Voyager](https://voyager.minedojo.org/) (the Minecraft agent) stores successful action sequences as **executable code** — not descriptions, not embeddings, but actual runnable skills. When it encounters a similar situation, it doesn't retrieve a text memory and hope the LLM interprets it correctly — it retrieves and executes the proven solution. This is procedural memory that directly becomes behavior.

**For Retina:** Every action-outcome pair is stored with a utility score. When the agent encounters a similar situation, it doesn't just retrieve "similar text" — it retrieves the experience with the highest proven utility. Failed experiences aren't deleted — they're kept with negative utility so the agent actively avoids repeating them.

**Research to build on:**
- [MemRL](https://arxiv.org/abs/2601.03192) — Q-value-weighted episodic retrieval, separates frozen LLM reasoning from evolving memory
- [AgentRR: Record & Replay](https://arxiv.org/abs/2505.17716) — Multi-level experience design: low-level for precise replay, high-level for adaptation
- [ECHO: Hindsight Experience Replay](https://arxiv.org/abs/2506.06698) — Generates counterfactual trajectories ("what if I had done X instead?")
- [Voyager](https://arxiv.org/abs/2305.16291) — Skill library as executable code, compositional skill building

### 3. Encompassing Memory — "What I Know About the World"

**What it is:** The agent's world model. Not individual facts but an interconnected understanding: how apps behave, what the user's workflow looks like, which tools work for which situations, how this operating system responds to different inputs.

**How it should work:** This is where the [A-MEM (Agentic Memory)](https://arxiv.org/abs/2502.12110) approach matters. Accepted at NeurIPS 2025, A-MEM uses Zettelkasten principles — every memory is a **note** with structured attributes (context, keywords, tags), and notes are dynamically **linked** to each other. When a new memory is added, it doesn't just get embedded — the system finds related existing memories and **updates their context**. The memory network continuously refines its own understanding.

**The ACT-R insight:** [Research on ACT-R + LLMs](https://dl.acm.org/doi/10.1145/3765766.3765803) shows that memory retrieval should use **activation spreading** — when you think about "Excel," it automatically activates related concepts ("Save button," "AX tree blind," "use Cmd+S"). This isn't cosine similarity — it's a network of associations where activating one node partially activates connected nodes. Memories prime other memories.

**For Retina:** The world model isn't a flat table of facts — it's a **graph** where nodes are knowledge and edges are relationships (causal, temporal, similarity, dependency). When the agent encounters Chrome, it doesn't just search for "Chrome memories" — the activation of "Chrome" spreads to "Electron wrapper" → "AX tree blind" → "use Vision Mode" → "OmniParser" automatically, without explicit retrieval.

**Research to build on:**
- [A-MEM](https://arxiv.org/abs/2502.12110) — Zettelkasten-style self-organizing memory with dynamic linking (NeurIPS 2025)
- [ACT-R + LLM Memory](https://dl.acm.org/doi/10.1145/3765766.3765803) — Activation-based retrieval with temporal decay, semantic similarity, and probabilistic noise
- [From Experience to Strategy: Graph Memory](https://arxiv.org/html/2511.07800v1) — Trainable graph-based memory that learns retrieval weights

### 4. Compacted Memory — "What I've Condensed"

**What it is:** The mathematical compression of long-term context so the agent can carry MORE experience in LESS space — without the lossy destruction of summarization.

**How it should work:** There are two layers:

**Layer A: KV Compaction (attention-level)**
The MIT paper shows that for any block of context, you can construct a much smaller set of "synthetic" keys and values that reproduce the same attention outputs. The compacted KV pairs aren't summaries — they're **mathematical reconstructions** that preserve how the model would have attended to the original tokens.

The process:
1. Process a block of context normally → get KV cache entries
2. Use Attention Matching to compress N key-value pairs into M pairs (where M << N)
3. The compressed pairs preserve: attention output (what the model extracts) + attention mass (relative importance)
4. Replace the original KV entries with the compact ones
5. New queries against the compact cache produce nearly identical results

**Layer B: Compressive Memory (architecture-level)**
[InfiniAttention](https://arxiv.org/abs/2404.07143) (Google, 2024) builds a **compressive memory** directly into the attention mechanism. As the model processes long sequences, older information is compressed into a fixed-size memory bank using linear attention. The model can attend to both local (recent) context via standard attention AND compressed long-term memory via linear attention — in the same forward pass.

Result: 114x less memory than standard attention, with a 1B model solving tasks on 1M token contexts.

**For Retina:** Instead of choosing what to remember and what to forget, the agent compresses everything. Old observations, past experiences, world knowledge — all compacted into dense representations that stay in the active context. The agent doesn't retrieve memories; it **already has them**, just compressed.

**Research to build on:**
- [Fast KV Compaction](https://arxiv.org/abs/2602.16284) — Per-head attention matching, 50x compression
- [InfiniAttention](https://arxiv.org/abs/2404.07143) — Compressive memory in attention, 114x memory reduction, infinite context
- [MemOS](https://arxiv.org/abs/2507.03724) — Treats memory as a first-class OS resource with three types: parametric, activation, and plaintext

### 5. Reflexive Memory — "What Changes How I Act"

**What it is:** Memory that doesn't get retrieved — it **modifies the agent's behavior directly**. When the agent learns "never click coordinates from AX tree in Electron apps," that shouldn't be a fact it looks up. It should be a **reflex** — a rule that fires before the agent even considers the action.

**How it should work:** This is the least explored but most important type. Current approaches:

**Prompt mutation:** Successful strategies get encoded as system prompt rules. The agent's prompt literally evolves based on experience. Not "retrieve this memory" but "this memory IS now part of the prompt."

**Tool preference weighting:** Each tool gets a context-dependent success weight. In Chrome → Vision Mode weight = 0.9, AX Tree weight = 0.3. These weights are updated after every action based on outcome. The agent doesn't "decide" to use Vision Mode — its weights already favor it.

**Experience-following:** [Research from 2025](https://arxiv.org/abs/2505.16067) shows LLM agents exhibit strong "experience-following" — when a retrieved memory closely matches the current input, the agent almost exactly reproduces the past action. This means **what you put in memory directly controls what the agent does**. This is both powerful (reliable behavior) and dangerous (error propagation).

**For Retina:** The most important lessons don't live in a database — they live in the agent's active prompt and tool weights. The memory consolidation process periodically "promotes" high-confidence, frequently-validated knowledge from the experience store into the system prompt. The agent's personality, preferences, and reflexes literally evolve.

**Research to build on:**
- [How Memory Management Impacts LLM Agents](https://arxiv.org/abs/2505.16067) — Experience-following behavior, error propagation risks
- [MemOS](https://arxiv.org/abs/2505.22101) — Memory as schedulable OS resource (parametric + activation + plaintext)
- [ICLR 2026 MemAgents Workshop](https://openreview.net/pdf?id=U51WxL382H) — Emerging research on memory for agentic systems

---

## The Integrated Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                    THE MEMORY VESSEL                              │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  REFLEXIVE LAYER (Always Active)                           │  │
│  │  System prompt mutations, tool weights, behavioral rules   │  │
│  │  Updated by: consolidation from experiential layer         │  │
│  │  Storage: In-memory config, loaded at boot                 │  │
│  └──────────────────────┬─────────────────────────────────────┘  │
│                         │ promotes validated patterns             │
│  ┌──────────────────────▼─────────────────────────────────────┐  │
│  │  COMPACTED CONTEXT (KV Cache)                              │  │
│  │  Compressed observations, condensed conversation history   │  │
│  │  Method: Attention Matching (50x compression)              │  │
│  │  Storage: KV cache in model runtime                        │  │
│  │  Always present — never retrieved, never lost              │  │
│  └──────────────────────┬─────────────────────────────────────┘  │
│                         │ feeds into                              │
│  ┌──────────────────────▼─────────────────────────────────────┐  │
│  │  EXPERIENTIAL STORE (Q-Value Weighted)                     │  │
│  │  Action-outcome pairs with learned utility scores          │  │
│  │  Retrieval: semantic relevance × utility (MemRL pattern)   │  │
│  │  Storage: SQLite + utility weights                         │  │
│  │  Updated: after every action (utility reinforcement)       │  │
│  └──────────────────────┬─────────────────────────────────────┘  │
│                         │ linked to                               │
│  ┌──────────────────────▼─────────────────────────────────────┐  │
│  │  WORLD MODEL (Knowledge Graph)                             │  │
│  │  Interconnected knowledge with activation spreading        │  │
│  │  Pattern: A-MEM Zettelkasten — notes link to notes         │  │
│  │  Storage: SQLite (nodes + edges + activation scores)       │  │
│  │  Retrieval: spreading activation, not cosine similarity    │  │
│  └──────────────────────┬─────────────────────────────────────┘  │
│                         │ raw data from                           │
│  ┌──────────────────────▼─────────────────────────────────────┐  │
│  │  OBSERVATION LOG (Append-Only)                             │  │
│  │  Raw sensory stream: AX trees, screenshots, state hashes  │  │
│  │  Compacted into KV cache via Attention Matching            │  │
│  │  Consolidated into experiences after each task             │  │
│  │  Storage: SQLite ring buffer (last N days raw, rest gone)  │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

---

## How Memory Flows Through the System

### On Every Action (milliseconds):

```
1. Agent decides on action
2. Reflexive layer checks: any behavioral rules that apply?
   → "Electron apps: skip AX tree, use Vision Mode" fires automatically
3. Action executes
4. Observation logged (raw state hashes, screen data)
5. Observation compacted into KV cache (Attention Matching)
6. Experience recorded: {action, context, outcome, pre/post state}
```

### After Each Task (seconds):

```
1. Experience store updated: utility scores adjusted based on task success/failure
2. World model updated: new nodes/edges if new knowledge emerged
3. A-MEM linking: new knowledge triggers re-contextualization of related nodes
4. Observation log: raw entries older than threshold compacted or pruned
```

### Periodic Consolidation (minutes/hours):

```
1. High-utility experiences with high confidence → promoted to reflexive rules
2. Contradictory knowledge nodes resolved (keep validated, archive contradicted)
3. Activation decay: unused world model nodes lose activation over time
4. Experience utility recalibration: old experiences re-scored against recent outcomes
5. KV cache recompaction: merge older compacted blocks into even denser representations
```

---

## What This Means for Implementation

### What stays from the v1 tech spec:

- **SQLite + rusqlite** — Still the right storage engine. Single file, WAL mode, FTS5.
- **Schema tables** — `state_log`, `knowledge`, `tool_registry` all still needed.
- **fastembed / BGE-small** — Still needed for the world model embedding layer.

### What changes:

| v1 (RAG-centric) | v2 (Memory Vessel) |
|---|---|
| Vector similarity search as primary retrieval | Spreading activation + Q-value utility as primary retrieval |
| Flat embedding store | Zettelkasten-style linked graph (nodes + edges table) |
| Memories injected as text into context | Memories compacted into KV cache (always present) |
| Static confidence scores | Dynamic utility scores updated by RL (experience success/failure) |
| Retrieval is explicit (agent asks) | Reflexive rules fire automatically (no retrieval needed) |
| Summarization to compress old memories | Attention Matching to condense without information loss |
| All memories weighted the same way | Experiences weighted by proven utility |

### New components needed:

1. **Activation Graph Engine** — SQLite-backed graph with nodes (knowledge), edges (relationships), and activation scores. When a node activates, connected nodes get partial activation (spreading).

2. **Utility Scorer** — Tracks action-outcome pairs and maintains Q-values. After each action, updates utility based on whether the state changed as intended. Two-phase retrieval: semantic filter → utility ranking.

3. **Reflexive Rule Engine** — A set of `if-then` rules derived from high-confidence, high-utility experiences. Loaded into memory at boot. Checked before every action. Mutated during consolidation.

4. **KV Compaction Module** — Interface to the LLM runtime to compress KV cache entries via Attention Matching. This is the hardest part — requires integration with the model's inference engine (likely via a custom Wasmtime host function or llama.cpp integration).

5. **Consolidation Daemon** — Background process that runs memory lifecycle: utility recalibration, activation decay, experience-to-reflex promotion, world model linking.

### New schema additions:

```sql
-- Knowledge graph edges (world model linking)
CREATE TABLE knowledge_edges (
    source_id   INTEGER NOT NULL REFERENCES knowledge(id),
    target_id   INTEGER NOT NULL REFERENCES knowledge(id),
    relation    TEXT NOT NULL,       -- 'causes', 'resolves', 'depends_on', 'similar_to', 'contradicts'
    weight      REAL DEFAULT 1.0,    -- Strength of connection
    created_at  TEXT DEFAULT (datetime('now')),
    PRIMARY KEY (source_id, target_id, relation)
);

-- Activation scores for spreading activation
CREATE TABLE knowledge_activation (
    knowledge_id INTEGER PRIMARY KEY REFERENCES knowledge(id),
    activation   REAL DEFAULT 0.0,   -- Current activation level (decays over time)
    base_level   REAL DEFAULT 0.0,   -- Base-level activation (ACT-R: log of access frequency)
    last_activated TEXT DEFAULT (datetime('now'))
);

-- Experience store with utility values (MemRL-style)
CREATE TABLE experiences (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,
    situation   TEXT NOT NULL,        -- What the agent was trying to do
    action      TEXT NOT NULL,        -- What the agent did
    outcome     TEXT NOT NULL,        -- What happened ('success', 'failure', 'partial')
    context     JSON NOT NULL,        -- Full context: app, state, tools used
    utility     REAL DEFAULT 0.0,    -- Q-value: learned utility from RL
    access_count INTEGER DEFAULT 0,
    created_at  TEXT DEFAULT (datetime('now')),
    metadata    JSON DEFAULT '{}'
);

CREATE INDEX idx_exp_utility ON experiences(utility DESC);
CREATE INDEX idx_exp_outcome ON experiences(outcome);

-- Reflexive rules (promoted from high-utility experiences)
CREATE TABLE reflexive_rules (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    condition   TEXT NOT NULL,        -- When this rule fires: "app_name = 'Chrome' AND element_type = 'button'"
    action      TEXT NOT NULL,        -- What to do: "prefer_vision_mode"
    confidence  REAL DEFAULT 1.0,
    source_experience_id INTEGER REFERENCES experiences(id),
    active      BOOLEAN DEFAULT TRUE,
    created_at  TEXT DEFAULT (datetime('now')),
    last_fired  TEXT
);

-- KV compaction metadata
CREATE TABLE compaction_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT NOT NULL,
    original_tokens INTEGER NOT NULL,  -- How many tokens before compaction
    compacted_tokens INTEGER NOT NULL, -- How many after
    compression_ratio REAL NOT NULL,
    quality_score   REAL,              -- Attention output preservation score
    created_at      TEXT DEFAULT (datetime('now'))
);
```

---

## Sub-Agent Memory: Independent Brains, Shared Lessons

Retina spawns sub-agents — full kernel copies — scoped to specific domains (email, browser, data pipelines, etc.). Each sub-agent has its own memory. This is critical.

### Why Not Shared Memory?

Shared memory between agents creates the same problem as bloated context — everyone's experiences pollute everyone else's retrieval. The email agent's IMAP lessons are noise to the browser agent.

### The Model: Isolated Memory, Promoted Tools

```
~/.retina/
├── main/agent.db           ← Main agent's memory
├── agents/
│   ├── email/agent.db      ← Email agent's memory (IMAP quirks, sender patterns)
│   ├── browser/agent.db    ← Browser agent's memory (DOM quirks, scroll tricks)
│   └── pipeline/agent.db   ← Pipeline agent's memory (SQL patterns, ETL lessons)
└── shared/promoted_tools/  ← Tools promoted from sub-agents to shared registry
```

Each sub-agent:
- **Owns its experiences** — its action-outcome pairs with utility scores
- **Owns its reflexes** — domain-specific behavioral rules
- **Owns its knowledge graph** — domain-specific linked knowledge
- **Fabricates its own tools** — builds what it needs for its domain
- **Can promote tools upward** — proven tools get offered to the parent registry (with human approval)

### Sub-Agent Memory Lifecycle

```
SPAWN:
  Sub-agent boots with empty memory + task description.
  Gets read-only access to parent's tool registry (can copy, not modify).

LEARN:
  Sub-agent records experiences in its own SQLite.
  Fabricates domain-specific tools as needed.
  Builds its own reflexive rules from its own experiences.

PERSIST:
  Long-running sub-agents keep their memory across sessions.
  Archived sub-agents preserve memory for reactivation.
  Reactivated sub-agent boots with all its reflexes and tools intact.
  It doesn't re-learn anything.

PROMOTE:
  High-utility tools offered to parent registry.
  High-confidence knowledge offered to parent knowledge graph.
  Parent decides what to absorb (Human Pulse gate).
```

### What Flows Between Agents

| What | Direction | Mechanism |
|---|---|---|
| Task assignments | Root → Specialist | Message with task description (~200 tokens) |
| Results | Specialist → Root | Structured outcome (data, status, errors) |
| Data handoffs | Specialist → Specialist (via root) | Structured data between pipeline steps |
| Tool promotions | Specialist → Shared | Tool source + test results + usage stats (human approved) |
| Knowledge promotions | Specialist → Root | Knowledge nodes that passed confidence threshold |
| Tool copies | Shared → Specialist (read-only) | Specialist can copy proven shared tools |
| Raw experience | **Never shared** | Each agent's experiences are private to its domain |
| Reflexive rules | **Never shared** | Each agent's reflexes are domain-specific |

This means:
- Specialists start lean and grow only the knowledge they need
- No context bloat from cross-domain contamination
- Tools bubble up through proven utility, not pre-loading
- If a specialist is archived and reactivated months later, it picks up exactly where it left off
- The network grows smarter without any single agent getting heavier

### Network-Level Memory (Root Agent Only)

The root agent has a unique memory role — it doesn't do domain work, it knows **who can do what**:

```sql
-- Root agent's agent.db has these additional tables:

-- Which agent handles which domain
CREATE TABLE agent_registry (
    agent_id    TEXT PRIMARY KEY,         -- "email-a1b2"
    domain      TEXT NOT NULL,            -- "email"
    capabilities TEXT NOT NULL,           -- "imap, send, invoice parsing"
    status      TEXT DEFAULT 'idle',      -- spawned/tooling/running/idle/persistent/archived
    tool_count  INTEGER DEFAULT 0,
    experience_count INTEGER DEFAULT 0,
    reflex_count INTEGER DEFAULT 0,
    tokens_spent INTEGER DEFAULT 0,
    cost_total  REAL DEFAULT 0.0,
    spawned_at  TEXT DEFAULT (datetime('now')),
    last_active TEXT DEFAULT (datetime('now')),
    manifest_path TEXT NOT NULL           -- path to agent's manifest.toml
);

-- Routing history: which agent handled which kind of task
CREATE TABLE routing_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_hash   TEXT NOT NULL,            -- semantic hash of the task
    routed_to   TEXT NOT NULL,            -- agent_id
    outcome     TEXT NOT NULL,            -- 'success', 'failure', 'rerouted'
    created_at  TEXT DEFAULT (datetime('now'))
);

-- The root agent learns routing patterns as reflexes:
-- "email tasks" → email agent (confidence 0.95)
-- "spreadsheet tasks" → data agent (confidence 0.88)
-- "deploy monitoring" → ops agent (confidence 0.92)
```

The root agent's reflexes are routing decisions, not domain knowledge. Over time, it routes instantly without LLM calls.

---

## The "Always Remember" Answer

If the agent had to always remember everything that mattered:

1. **Observations** don't get stored and retrieved — they get **compacted into the active context** via Attention Matching (for local models) or stay in the Rust harness as state hashes (for API models). Nothing is deleted — it's condensed. (KV Compaction / State Hashing)

2. **Experiences** don't get embedded into vectors — they get **scored by proven utility** via reinforcement learning. The agent retrieves what WORKED, not what's similar. (MemRL)

3. **Knowledge** doesn't sit in a flat table — it lives in a **self-organizing graph** where activating one concept activates related concepts automatically. No explicit query needed. (A-MEM + ACT-R Activation)

4. **Lessons** don't get looked up — they become **reflexive rules** that fire before the agent even considers the wrong action. Memory becomes behavior. Zero tokens in context. (Reflexive Layer)

5. **Nothing gets deleted** — it gets **condensed**. Compaction preserves information at lower fidelity. Summarization destroys it. (Compaction > Eviction)

6. **Sub-agents own their domain** — each spawned agent builds its own memory, reflexes, and tools. No shared context bloat. Proven tools and knowledge get promoted upward. The system grows smarter without any single agent getting heavier. (Isolated Memory + Tool Promotion)

7. **Context stays tiny** — the LLM sees ~500 tokens. The harness handles the rest. Memory is pull-only — the agent reaches for specific memories when it needs them, not because they were pre-loaded. (Pull Architecture)

---

## API vs Local Model Strategy

| Capability | Claude API | Local Model (llama.cpp etc.) |
|---|---|---|
| KV compaction | Not possible (no KV access) | Full Attention Matching via runtime |
| Context management | Structured state + prompt caching + server-side compaction fallback | Direct KV manipulation |
| Memory injection | Pull-only tools + minimal context assembler | Pull-only tools + KV compaction of past context |
| Cost optimization | Prompt caching (90% savings on cached prefix) | Free (local compute) |
| Reflection calls | Use cheaper model (Haiku) for reflection | Use same local model |

The harness architecture (reflexes, pull memory, sub-agents) works identically regardless of which LLM backend is used. The only difference is how context compression happens at the model level.

---

## Sources

- [Fast KV Compaction via Attention Matching](https://arxiv.org/abs/2602.16284) — MIT, Feb 2026
- [VentureBeat: KV compaction cuts memory 50x](https://venturebeat.com/orchestration/new-kv-cache-compaction-technique-cuts-llm-memory-50x-without-accuracy-loss)
- [PyramidKV: Dynamic KV Cache Compression](https://openreview.net/forum?id=ayi7qezU87)
- [ChunkKV: Semantic-Preserving KV Cache Compression](https://arxiv.org/html/2502.00299v5)
- [NVIDIA kvpress: KV cache compression framework](https://github.com/NVIDIA/kvpress)
- [InfiniAttention: Infinite Context Transformers](https://arxiv.org/abs/2404.07143) — Google, 2024
- [MemRL: Self-Evolving Agents via Episodic Memory](https://arxiv.org/abs/2601.03192) — Jan 2026
- [Voyager: Open-Ended Embodied Agent](https://arxiv.org/abs/2305.16291)
- [AgentRR: Record & Replay](https://arxiv.org/abs/2505.17716) — May 2025
- [ECHO: Hindsight Experience Replay for LLM Agents](https://arxiv.org/abs/2506.06698) — Jun 2025
- [A-MEM: Agentic Memory for LLM Agents](https://arxiv.org/abs/2502.12110) — NeurIPS 2025
- [ACT-R + LLM Memory Architecture](https://dl.acm.org/doi/10.1145/3765766.3765803)
- [How Memory Management Impacts LLM Agents](https://arxiv.org/abs/2505.16067)
- [From Experience to Strategy: Trainable Graph Memory](https://arxiv.org/html/2511.07800v1)
- [MemOS: Memory Operating System for AI](https://arxiv.org/abs/2507.03724)
- [ICLR 2026 MemAgents Workshop](https://openreview.net/pdf?id=U51WxL382H)
- [HackerNoon: Fast KV Compaction](https://hackernoon.com/fast-kv-compaction-makes-long-context-llms-practical)
