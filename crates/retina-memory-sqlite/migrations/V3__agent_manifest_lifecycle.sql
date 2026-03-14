ALTER TABLE agent_manifest ADD COLUMN updated_at TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_manifest ADD COLUMN lifecycle_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE agent_manifest ADD COLUMN budget_json TEXT NOT NULL DEFAULT '{}';
