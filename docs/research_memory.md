# Research: Long-Term Memory Vessel for Agent Retina

> The memory system is the foundation of self-awareness. Without durable, queryable memory, the agent cannot learn from failures, recall past strategies, or evolve its own tooling.

**Note:** This document describes the **default SQLite implementation** of the `Memory` trait defined in [trait_contracts.md](trait_contracts.md). The kernel calls `memory.record_experience()`, `memory.recall_knowledge()`, etc. — it never knows about SQLite, vectors, or FTS5. All of that is internal to this implementation.

**V1 alignment:** This is the buildable memory plan for version one. It assumes a small kernel-assembled prompt, pull-based recall, Rust + Wasm tooling, and a full observation timeline. Longer-term memory evolution is tracked in `research_memory_v2.md`.

---

## 1. Design Philosophy

The memory vessel follows four principles:

1. **Trait-First** — The kernel defines a single `Memory` trait (record, recall, learn). This doc specifies the SQLite implementation. Vectors, FTS5, embeddings — all internal. Other backends (in-memory, DuckDB) implement the same trait differently.
2. **Local-First** — The default implementation uses a single SQLite file. No cloud, no network dependency. Portable and private.
3. **Tiered** — Not all memories are equal. Working memory is fast and ephemeral; lessons learned are slow and permanent.
4. **Self-Managed** — The agent writes to and queries its own memory via tool calls (MemGPT pattern). Memory management is an agent skill, not hidden infrastructure.

---

## 2. Memory Architecture: The Four Tiers

Inspired by cognitive science and validated by production systems (MemGPT/Letta, Generative Agents, Mem0):

```
┌─────────────────────────────────────────────────────────────┐
│  L0: WORKING MEMORY (Assembled Context)                     │
│  Current task state, active plan, last result, tool index   │
│  Storage: kernel-built prompt (~500-1,000 tokens in v1)     │
│  Lifetime: Current session only                             │
│  Access: Always present, intentionally minimal              │
├─────────────────────────────────────────────────────────────┤
│  L1: EPISODIC MEMORY (What Happened)                        │
│  Every action, state-hash, tool call, CLI output            │
│  Storage: SQLite `state_log` table + FTS5 index             │
│  Lifetime: Permanent, decays in retrieval priority          │
│  Access: By recency, time-range filter, keyword search      │
├─────────────────────────────────────────────────────────────┤
│  L2: SEMANTIC MEMORY (What We Learned)                      │
│  Extracted facts, "Lessons Learned", UI quirk maps          │
│  Storage: SQLite `vec_knowledge` (sqlite-vec) + FTS5        │
│  Lifetime: Permanent, updated on contradiction              │
│  Access: By vector similarity + keyword hybrid retrieval    │
├─────────────────────────────────────────────────────────────┤
│  L3: PROCEDURAL MEMORY (How To Do Things)                   │
│  Tool registry, fabricated .wasm tools, learned strategies  │
│  Storage: SQLite `tool_registry` + filesystem (.wasm files) │
│  Lifetime: Permanent until explicitly deprecated            │
│  Access: By tool name, capability description match         │
└─────────────────────────────────────────────────────────────┘
```

### Why This Tiering Matters

- **L0** is intentionally scarce. The kernel manages it aggressively so prompts stay small and stable.
- **L1** is the raw event log. It answers: "What did I do 5 minutes ago?" and "Did the screen change after my last click?" This is the backbone of the state-hash verification loop.
- **L2** is distilled intelligence. It answers: "Have I seen this failure before?" and "What works for Excel 16.8?" This is what makes the agent smarter over time.
- **L3** is the agent's skill library. It answers: "Do I have a tool for this?" and "Can I build one?"

---

## 3. Technology Stack

### 3.1 Database: SQLite via `rusqlite`

**Why SQLite:**
- Single file = portable, backupable, zero-config
- WAL mode = concurrent readers + single writer, no blocking
- Built-in JSON support (since 3.38.0) for flexible metadata
- Built-in FTS5 for keyword/lexical search
- Handles millions of rows comfortably in a single file
- Battle-tested, zero-dependency

**Rust crate:** `rusqlite` with `bundled` feature (compiles SQLite from source, guarantees FTS5 + JSON)

```toml
[dependencies]
rusqlite = { version = "0.32", features = [
    "bundled",        # Compile SQLite from source (includes FTS5, JSON)
    "backup",         # Online backup API
    "blob",           # Incremental blob I/O for embeddings
    "functions",      # Custom SQL functions in Rust
    "load_extension", # Load sqlite-vec
    "serde_json",     # JSON column support
    "modern_sqlite",  # SQLite 3.38.0+ features
] }
```

**Connection architecture:**
```
┌──────────────────┐     ┌──────────────────────┐
│  Write Connection │     │  Read Pool (4 conns)  │
│  (Mutex-guarded)  │     │  (r2d2 connection     │
│  All INSERTs,     │     │   pool, read-only)    │
│  UPDATEs, DELETEs │     │  All SELECTs, search  │
└──────────────────┘     └──────────────────────┘
         │                          │
         └──────────┬───────────────┘
                    │
            ┌───────▼───────┐
            │  agent.db     │
            │  (WAL mode)   │
            │  single file  │
            └───────────────┘
```

**Critical pragmas:**
```sql
PRAGMA journal_mode = WAL;          -- Concurrent read/write
PRAGMA synchronous = NORMAL;        -- Fast, safe enough for local agent
PRAGMA busy_timeout = 5000;         -- Retry on lock contention
PRAGMA cache_size = -64000;         -- 64MB page cache
PRAGMA mmap_size = 268435456;       -- 256MB memory-mapped I/O
PRAGMA temp_store = MEMORY;         -- In-memory temp tables
PRAGMA foreign_keys = ON;
```

### 3.2 Vector Search: `sqlite-vec`

**What:** Pure-C SQLite extension by Alex Garcia. Successor to `sqlite-vss`. Adds vector similarity search directly into SQL.

**How it works:**
- Registers a `vec0` virtual table module
- Stores vectors as compact binary blobs (float32, int8, or bit)
- Supports L2 distance, cosine distance, and inner product
- Uses brute-force (exact) scan — no ANN index
- Zero dependencies, single C file, statically linkable

**Rust integration via `sqlite-vec` crate:**
```rust
use rusqlite::ffi::sqlite3_auto_extension;
use sqlite_vec::sqlite3_vec_init;

// Register before opening any connection
unsafe {
    sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
}
let conn = rusqlite::Connection::open("agent.db")?;

// Verify
let version: String = conn.query_row("SELECT vec_version()", [], |r| r.get(0))?;
```

**SQL usage:**
```sql
-- Create vector table (384 dimensions for BGE-small)
CREATE VIRTUAL TABLE vec_knowledge USING vec0(
    embedding float[384]
);

-- Insert (vector as JSON array)
INSERT INTO vec_knowledge(rowid, embedding)
VALUES (1, vec_f32('[0.1, 0.2, ...]'));

-- KNN search (top 10 nearest)
SELECT rowid, distance
FROM vec_knowledge
WHERE embedding MATCH vec_f32('[0.05, 0.15, ...]')
  AND k = 10
ORDER BY distance;
```

**Performance sweet spot:**
| Vector Count | Query Latency (384-dim, float32) |
|---|---|
| 10,000 | ~1-5 ms |
| 100,000 | ~10-50 ms |
| 1,000,000 | ~100-500 ms |

For an agent's "lessons learned" database, we'll likely stay well under 100K entries — plenty fast.

**Quantization option:** `int8` vectors give ~4x speedup + 4x storage reduction if needed later.

### 3.3 Full-Text Search: FTS5

FTS5 is built into SQLite when using `bundled`. It provides BM25-ranked keyword search.

**Why both FTS5 and vector search:**
- FTS5 catches exact terms, code identifiers, app names that embeddings may miss
- Vector search catches semantic similarity even with different wording
- Hybrid retrieval (combine both scores) outperforms either alone

```sql
-- External content FTS5 index on the knowledge table
CREATE VIRTUAL TABLE knowledge_fts USING fts5(
    content,
    content=knowledge,
    content_rowid=id,
    tokenize='porter unicode61'
);

-- Hybrid search pattern:
-- 1. Get FTS5 candidates
-- 2. Get vector candidates
-- 3. Reciprocal rank fusion or weighted combination
```

### 3.4 Embedding Model: BGE-small-en-v1.5 via `fastembed`

**Why this model:**
- 33M params, 384 dimensions — small enough for instant CPU inference
- 62.17 MTEB score — best quality among "small" models
- MIT license
- ONNX export available, works with `fastembed` crate out of the box

**Rust integration — easiest path:**
```toml
[dependencies]
fastembed = "4"  # Built on ort (ONNX Runtime), handles download + tokenization + pooling
```

```rust
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};

let model = TextEmbedding::try_new(InitOptions {
    model_name: EmbeddingModel::BGESmallENV15,
    show_download_progress: true,
    ..Default::default()
})?;

let embeddings = model.embed(vec!["Clicking Save in Excel 16.8 fails via AX Tree"], None)?;
// embeddings[0] = Vec<f32> of length 384, L2-normalized
```

**Latency:** ~5-10ms per embedding on CPU (Apple Silicon or modern x86). Batches of 100 in ~200-500ms.

**Alternative models if needs change:**

| Need | Model | Dims | Notes |
|---|---|---|---|
| Fastest possible | all-MiniLM-L6-v2 | 384 | 22M params, ~3ms/embed |
| Long context (>512 tokens) | nomic-embed-text-v1.5 | 768 (or 256 Matryoshka) | 8192 token context |
| Best quality | gte-modernbert-base | 768 | 149M params, 64.38 MTEB |

**Alternative runtimes if `fastembed` is too heavy:**

| Runtime | Crate | Tradeoff |
|---|---|---|
| ONNX Runtime (direct) | `ort` + `tokenizers` | More control, same perf, more code |
| Candle (pure Rust) | `candle-core` + `candle-transformers` | No C++ dep, Metal accel on macOS, less optimized CPU |
| Tract (pure Rust) | `tract-onnx` | Smallest binary, slowest inference |

---

## 4. Schema Design

```sql
-- =====================================================
-- L1: EPISODIC — Raw event log
-- =====================================================
CREATE TABLE state_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL,           -- Groups actions within a task
    timestamp   TEXT NOT NULL DEFAULT (datetime('now')),
    action_type TEXT NOT NULL,           -- 'click', 'type', 'tool_call', 'screenshot', etc.
    action_data JSON NOT NULL,           -- Full action payload
    pre_hash    TEXT,                    -- State hash before action
    post_hash   TEXT,                    -- State hash after action
    state_delta TEXT,                    -- 'changed', 'unchanged', 'error'
    app_context TEXT,                    -- Which app/window was targeted
    duration_ms INTEGER                  -- How long the action took
);

CREATE INDEX idx_state_log_session ON state_log(session_id, timestamp);
CREATE INDEX idx_state_log_delta ON state_log(state_delta);
CREATE INDEX idx_state_log_app ON state_log(app_context);

-- FTS5 index for searching action history
CREATE VIRTUAL TABLE state_log_fts USING fts5(
    action_data,
    content=state_log,
    content_rowid=id,
    tokenize='porter unicode61'
);

-- =====================================================
-- L2: SEMANTIC — Lessons learned, facts, UI quirks
-- =====================================================
CREATE TABLE knowledge (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    content     TEXT NOT NULL,           -- The lesson/fact in natural language
    category    TEXT NOT NULL,           -- 'lesson', 'ui_quirk', 'user_pref', 'fact'
    source      TEXT,                    -- Where this knowledge came from
    confidence  REAL DEFAULT 1.0,       -- 0.0-1.0, decays or increases over time
    access_count INTEGER DEFAULT 0,     -- How often retrieved
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    metadata    JSON DEFAULT '{}'       -- Flexible: app name, OS version, tags, etc.
);

CREATE INDEX idx_knowledge_category ON knowledge(category);
CREATE INDEX idx_knowledge_confidence ON knowledge(confidence);

-- FTS5 for keyword search on knowledge
CREATE VIRTUAL TABLE knowledge_fts USING fts5(
    content,
    content=knowledge,
    content_rowid=id,
    tokenize='porter unicode61'
);

-- Vector embeddings for semantic search
CREATE VIRTUAL TABLE knowledge_vec USING vec0(
    embedding float[384]
);
-- rowid in knowledge_vec maps 1:1 to knowledge.id

-- =====================================================
-- L2: UI EXPERIENCE MAP
-- =====================================================
CREATE TABLE ui_experience (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    app_name    TEXT NOT NULL,           -- 'Chrome', 'Excel', 'Terminal'
    app_version TEXT,
    element_id  TEXT,                    -- AX tree element identifier
    quirk_type  TEXT NOT NULL,           -- 'blind_coords', 'missing_element', 'stale_tree'
    resolution  TEXT NOT NULL,           -- 'use_vision', 'use_keyboard', 'use_tab_nav'
    success_rate REAL DEFAULT 1.0,      -- How often this resolution works
    last_used   TEXT DEFAULT (datetime('now')),
    metadata    JSON DEFAULT '{}'
);

CREATE UNIQUE INDEX idx_ui_exp_app_element ON ui_experience(app_name, element_id, quirk_type);

-- =====================================================
-- L3: PROCEDURAL — Tool registry
-- =====================================================
CREATE TABLE tool_registry (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL UNIQUE,    -- 'keyboard_shortcut_save', 'ocr_coord_finder'
    description TEXT NOT NULL,           -- What this tool does (for LLM selection)
    version     INTEGER DEFAULT 1,
    source_lang TEXT DEFAULT 'rust',     -- v1 ships with Rust-to-Wasm tools
    source_code TEXT,                    -- Original source (for re-compilation)
    wasm_path   TEXT,                    -- Path to compiled .wasm file
    interface   JSON NOT NULL,           -- Input/output schema: { "input": {...}, "output": {...} }
    builtin     BOOLEAN DEFAULT FALSE,   -- TRUE for core tools, FALSE for fabricated
    test_status TEXT DEFAULT 'untested', -- 'untested', 'passed', 'failed'
    use_count   INTEGER DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    metadata    JSON DEFAULT '{}'
);

-- Vector embeddings for tool description search
CREATE VIRTUAL TABLE tool_vec USING vec0(
    embedding float[384]
);
-- rowid maps to tool_registry.id

-- =====================================================
-- L3: BLUEPRINT — Agent self-spec (read-only at runtime)
-- =====================================================
CREATE TABLE blueprint (
    key         TEXT PRIMARY KEY,        -- 'max_ram_mb', 'api_quota_rpm', 'os', 'arch'
    value       TEXT NOT NULL,
    description TEXT
);

-- =====================================================
-- SYNC: Triggers to keep FTS5 indexes in sync
-- =====================================================
CREATE TRIGGER knowledge_ai AFTER INSERT ON knowledge BEGIN
    INSERT INTO knowledge_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER knowledge_ad AFTER DELETE ON knowledge BEGIN
    INSERT INTO knowledge_fts(knowledge_fts, rowid, content) VALUES('delete', old.id, old.content);
END;
CREATE TRIGGER knowledge_au AFTER UPDATE OF content ON knowledge BEGIN
    INSERT INTO knowledge_fts(knowledge_fts, rowid, content) VALUES('delete', old.id, old.content);
    INSERT INTO knowledge_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER state_log_ai AFTER INSERT ON state_log BEGIN
    INSERT INTO state_log_fts(rowid, action_data) VALUES (new.id, new.action_data);
END;
```

---

## 5. Retrieval Strategy

### The Scoring Formula

Adapted from the Generative Agents paper (Park et al., Stanford 2023):

```
score = α × recency + β × relevance + γ × importance
```

**Starting weights:** `α=0.3, β=0.5, γ=0.2`

Where:
- **Recency** = `decay_rate ^ hours_since_last_access` (decay_rate ≈ 0.995)
- **Relevance** = cosine similarity from sqlite-vec + BM25 score from FTS5 (reciprocal rank fusion)
- **Importance** = `knowledge.confidence` field (set at creation, boosted on successful use, decayed on contradiction)

### Hybrid Retrieval Pipeline

```
Query: "How do I click the Save button in Excel?"
                    │
        ┌───────────┴───────────┐
        │                       │
   ┌────▼─────┐          ┌─────▼──────┐
   │  FTS5    │          │ sqlite-vec │
   │  MATCH   │          │  KNN k=20  │
   │  top 20  │          │  cosine    │
   └────┬─────┘          └─────┬──────┘
        │                       │
        └───────────┬───────────┘
                    │
           ┌────────▼────────┐
           │ Reciprocal Rank │
           │ Fusion (RRF)    │
           │ + recency boost │
           │ + importance    │
           └────────┬────────┘
                    │
           ┌────────▼────────┐
           │  Top ranked     │
           │  memories made  │
           │  available to   │
           │  recall/context │
           └─────────────────┘
```

### Context Window Budget

V1 does not dump raw memory into the prompt. The context assembler keeps the prompt small and only includes compact, task-relevant memory slices when needed:

| Slot | Token Budget | Contents |
|---|---|---|
| Identity + rules | ~100 | Agent identity, guardrails, domain |
| Current task | ~100-150 | The active request or step |
| Available tools | ~150-250 | Filtered tool descriptions only |
| Last result / current step | ~100-150 | Immediate execution context |
| Optional memory slice | ~100-250 | 1-3 compact recalled items, not dumps |
| **Total target** | **~500-1,000** | Small assembled context, stable across runs |

---

## 6. Memory Lifecycle

### Writing Memories

**Episodic (L1):** Automatic. Every `kernel_execute()` call writes a `state_log` row. No LLM involvement.

**Semantic (L2):** Two paths:
1. **Agent self-managed** — The agent has `memory_save(content, category)` and `memory_search(query)` tools (MemGPT pattern). It decides when a lesson is worth saving.
2. **Automatic extraction** — After each task completion, a background pass extracts facts and UI quirks from the episodic log. This catches things the agent forgot to save.

**Procedural (L3):** Written by the Tool Fabricator after a new .wasm tool passes tests.

### Consolidation (Garbage Collection for Memory)

Run periodically (e.g., on `retina cleanup` or after every 100 tasks):

1. **Decay confidence** — Memories not accessed in 30+ days get `confidence *= 0.9`
2. **Deduplicate** — Find near-duplicate knowledge entries (cosine similarity > 0.95), merge into one
3. **Contradiction resolution** — If two memories contradict (detected by LLM), keep the newer one, archive the old
4. **Summarize episodic** — Compress old `state_log` entries: keep last 7 days verbatim, summarize older sessions into single knowledge entries
5. **FTS5 optimize** — `INSERT INTO knowledge_fts(knowledge_fts) VALUES('optimize')`
6. **SQLite optimize** — `PRAGMA optimize`

### The "Human Pulse" Gate

Any modification to L3 (tool_registry, blueprint) requires explicit user approval:

```
[!] Agent wants to register new tool: "ocr_coord_finder"
    Description: Finds UI element coordinates via screenshot OCR
    Source: Rust → .wasm (142 lines)
    Test status: PASSED (3/3 assertions)

    Approve? [y/n]: _
```

This prevents recursive drift — the agent cannot endlessly modify its own capabilities without human oversight.

---

## 7. Migration Strategy

Using `refinery` crate for versioned SQL migrations:

```toml
[dependencies]
refinery = { version = "0.8", features = ["rusqlite"] }
```

```
migrations/
├── V1__initial_schema.sql        # Core tables
├── V2__add_fts5_indexes.sql      # FTS5 virtual tables + triggers
├── V3__add_vector_tables.sql     # sqlite-vec virtual tables
└── V4__add_ui_experience.sql     # UI quirk tracking
```

```rust
use refinery::embed_migrations;
embed_migrations!("./migrations");

fn init_db(conn: &mut rusqlite::Connection) -> Result<()> {
    migrations::runner().run(conn)?;
    Ok(())
}
```

---

## 8. Full Dependency Map

```toml
# Cargo.toml for retina-memory crate

[dependencies]
# Database
rusqlite = { version = "0.32", features = ["bundled", "backup", "blob", "functions", "load_extension", "serde_json", "modern_sqlite"] }
sqlite-vec = "0.1"                      # Vector search extension
r2d2 = "0.8"                            # Connection pooling
r2d2_sqlite = "0.25"                    # SQLite adapter for pool
refinery = { version = "0.8", features = ["rusqlite"] }  # Migrations

# Embeddings
fastembed = "4"                          # ONNX-based embedding (BGE-small)

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Async (optional, if kernel is async)
tokio-rusqlite = "0.6"

# Time
chrono = { version = "0.4", features = ["serde"] }

# Hashing (for state hashes)
blake3 = "1"
```

---

## 9. Open Questions for Further Research

1. **Graph layer** — Should we add a lightweight knowledge graph on top of SQLite for entity-relation queries? (e.g., "Which tools depend on the AX tree module?") Could use a simple `edges` table rather than Neo4j.

2. **Embedding model updates** — If we upgrade the embedding model later, all existing vectors become incompatible. Strategy: store model version in metadata, re-embed on migration.

3. **Multi-agent memory sharing** — If Retina participates in A2A collaboration, should other agents be able to query its memory? Read-only SQLite access over MCP?

4. **Memory size budgets** — How aggressively should we prune? What's the max acceptable DB size for a "portable" agent? (100MB? 1GB? 10GB?)

5. **Encryption at rest** — Should the memory file be encrypted? SQLCipher exists but adds complexity. Relevant if agent memory contains sensitive user data.

---

## 10. Summary: The Stack at a Glance

```
┌─────────────────────────────────────────────┐
│              retina-memory crate             │
├──────────┬──────────┬───────────┬───────────┤
│ rusqlite │sqlite-vec│   FTS5    │ fastembed │
│ (SQLite) │(vectors) │(keywords) │(embeddings│
│ WAL mode │ float[384]│ BM25     │ BGE-small)│
├──────────┴──────────┴───────────┴───────────┤
│              Single File: agent.db           │
│         + agent.db-wal + agent.db-shm        │
└─────────────────────────────────────────────┘
```

**One file. Zero cloud. Full memory.**
