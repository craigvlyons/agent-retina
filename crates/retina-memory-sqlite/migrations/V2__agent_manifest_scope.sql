ALTER TABLE agent_manifest ADD COLUMN parent_agent_id TEXT;
ALTER TABLE agent_manifest ADD COLUMN capabilities_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE agent_manifest ADD COLUMN authority_json TEXT NOT NULL DEFAULT '{}';
