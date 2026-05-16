ALTER TABLE agent_manifest ADD COLUMN allowed_tools_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE agent_manifest ADD COLUMN denied_tools_json TEXT NOT NULL DEFAULT '[]';
