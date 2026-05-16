CREATE TABLE IF NOT EXISTS runtime_tasks (
    task_id TEXT PRIMARY KEY,
    task_kind TEXT NOT NULL,
    owner_agent_id TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    description TEXT NOT NULL,
    prompt_or_objective TEXT NOT NULL,
    output_path TEXT,
    output_offset INTEGER NOT NULL,
    progress_summary TEXT,
    last_activity TEXT NOT NULL,
    notified INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS runtime_tasks_last_activity_idx
ON runtime_tasks(last_activity DESC);
