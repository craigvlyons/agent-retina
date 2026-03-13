CREATE TABLE IF NOT EXISTS timeline_events (
    event_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    event_type TEXT NOT NULL,
    intent_id TEXT,
    action_id TEXT,
    pre_state_hash TEXT,
    post_state_hash TEXT,
    delta_summary TEXT,
    duration_ms INTEGER,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS state_log (
    event_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS experiences (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    intent_id TEXT NOT NULL,
    action_summary TEXT NOT NULL,
    outcome TEXT NOT NULL,
    utility REAL NOT NULL,
    created_at TEXT NOT NULL,
    metadata TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS knowledge (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    content TEXT NOT NULL,
    confidence REAL NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    metadata TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_fts USING fts5(
    content,
    content=knowledge,
    content_rowid='rowid'
);

CREATE TABLE IF NOT EXISTS knowledge_id_map (
    rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_id TEXT NOT NULL UNIQUE
);

CREATE VIRTUAL TABLE IF NOT EXISTS knowledge_vec USING vec0(
    embedding float[384]
);

CREATE TABLE IF NOT EXISTS knowledge_edges (
    source_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    relation TEXT NOT NULL,
    PRIMARY KEY (source_id, target_id, relation)
);

CREATE TABLE IF NOT EXISTS reflexive_rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    condition_json TEXT NOT NULL,
    action_json TEXT NOT NULL,
    confidence REAL NOT NULL,
    active INTEGER NOT NULL,
    last_fired TEXT
);

CREATE TABLE IF NOT EXISTS tool_registry (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    source_lang TEXT NOT NULL,
    test_status TEXT NOT NULL,
    metadata TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_manifest (
    agent_id TEXT PRIMARY KEY,
    domain TEXT NOT NULL,
    status TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS knowledge_ai AFTER INSERT ON knowledge BEGIN
    INSERT INTO knowledge_fts(rowid, content)
    VALUES ((SELECT rowid FROM knowledge_id_map WHERE knowledge_id = new.id), new.content);
END;

CREATE TRIGGER IF NOT EXISTS knowledge_ad AFTER DELETE ON knowledge BEGIN
    INSERT INTO knowledge_fts(knowledge_fts, rowid, content)
    VALUES('delete', (SELECT rowid FROM knowledge_id_map WHERE knowledge_id = old.id), old.content);
END;

CREATE TRIGGER IF NOT EXISTS knowledge_au AFTER UPDATE OF content ON knowledge BEGIN
    INSERT INTO knowledge_fts(knowledge_fts, rowid, content)
    VALUES('delete', (SELECT rowid FROM knowledge_id_map WHERE knowledge_id = old.id), old.content);
    INSERT INTO knowledge_fts(rowid, content)
    VALUES ((SELECT rowid FROM knowledge_id_map WHERE knowledge_id = new.id), new.content);
END;
