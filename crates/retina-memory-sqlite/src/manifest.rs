use crate::{
    MemoryStats, SqliteMemory,
    registry::registry_snapshot,
    storage::{count_table, parse_datetime, to_storage},
};
use chrono::Utc;
use retina_types::*;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
struct ManifestFile {
    agent_id: String,
    domain: String,
    status: String,
    description: String,
    #[serde(default)]
    role_prompt: Option<String>,
    #[serde(default)]
    initial_prompt: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    created_at: String,
    updated_at: String,
    parent_agent_id: Option<String>,
    capabilities: Vec<String>,
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default)]
    denied_tools: Vec<String>,
    #[serde(default)]
    required_mcp_servers: Vec<String>,
    authority: AgentAuthority,
    lifecycle: AgentLifecycle,
    budget: AgentBudget,
}

impl SqliteMemory {
    pub fn save_manifest(&self, manifest: &AgentManifest) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO agent_manifest
                 (agent_id, domain, status, description, role_prompt, initial_prompt, model_id, created_at, updated_at, parent_agent_id, capabilities_json, allowed_tools_json, denied_tools_json, required_mcp_servers_json, authority_json, lifecycle_json, budget_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    manifest.agent_id.0,
                    manifest.domain,
                    serde_json::to_string(&manifest.status).map_err(to_storage)?,
                    manifest.description,
                    manifest.role_prompt.clone(),
                    manifest.initial_prompt.clone(),
                    manifest.model_id.clone(),
                    manifest.created_at.to_rfc3339(),
                    manifest.updated_at.to_rfc3339(),
                    manifest.parent_agent_id.as_ref().map(|value| value.0.clone()),
                    serde_json::to_string(&manifest.capabilities).map_err(to_storage)?,
                    serde_json::to_string(&manifest.allowed_tools).map_err(to_storage)?,
                    serde_json::to_string(&manifest.denied_tools).map_err(to_storage)?,
                    serde_json::to_string(&manifest.required_mcp_servers).map_err(to_storage)?,
                    serde_json::to_string(&manifest.authority).map_err(to_storage)?,
                    serde_json::to_string(&manifest.lifecycle).map_err(to_storage)?,
                    serde_json::to_string(&manifest.budget).map_err(to_storage)?,
                ],
            )
            .map_err(to_storage)?;
            Ok(())
        })
    }

    pub fn load_manifest(&self, agent_id: &AgentId) -> Result<Option<AgentManifest>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT domain, status, description, role_prompt, initial_prompt, model_id, created_at, updated_at, parent_agent_id, capabilities_json, allowed_tools_json, denied_tools_json, required_mcp_servers_json, authority_json, lifecycle_json, budget_json
                 FROM agent_manifest WHERE agent_id = ?1",
                params![agent_id.0],
                |row| {
                    let status_json: String = row.get(1)?;
                    let created_at: String = row.get(6)?;
                    let updated_at: String = row.get(7)?;
                    let capabilities_json: String = row.get(9)?;
                    let allowed_tools_json: String = row.get(10)?;
                    let denied_tools_json: String = row.get(11)?;
                    let required_mcp_servers_json: String = row.get(12)?;
                    let authority_json: String = row.get(13)?;
                    let lifecycle_json: String = row.get(14)?;
                    let budget_json: String = row.get(15)?;
                    Ok(AgentManifest {
                        agent_id: agent_id.clone(),
                        domain: row.get(0)?,
                        status: serde_json::from_str(&status_json).unwrap_or(AgentStatus::Spawned),
                        description: row.get(2)?,
                        role_prompt: row.get(3)?,
                        initial_prompt: row.get(4)?,
                        model_id: row.get(5)?,
                        created_at: parse_datetime(&created_at),
                        updated_at: parse_datetime(&updated_at),
                        parent_agent_id: row.get::<_, Option<String>>(8)?.map(AgentId),
                        capabilities: serde_json::from_str(&capabilities_json)
                            .unwrap_or_else(|_| Vec::new()),
                        allowed_tools: serde_json::from_str(&allowed_tools_json)
                            .unwrap_or_else(|_| Vec::new()),
                        denied_tools: serde_json::from_str(&denied_tools_json)
                            .unwrap_or_else(|_| Vec::new()),
                        required_mcp_servers: serde_json::from_str(&required_mcp_servers_json)
                            .unwrap_or_else(|_| Vec::new()),
                        authority: serde_json::from_str(&authority_json)
                            .unwrap_or_else(|_| AgentAuthority::default()),
                        lifecycle: serde_json::from_str(&lifecycle_json)
                            .unwrap_or_else(|_| AgentLifecycle::ready()),
                        budget: serde_json::from_str(&budget_json)
                            .unwrap_or_else(|_| AgentBudget::default()),
                    })
                },
            )
            .optional()
            .map_err(to_storage)
        })
    }

    pub fn list_manifests(&self) -> Result<Vec<AgentManifest>> {
        self.with_conn(|conn| {
            let mut statement = conn
                .prepare(
                    "SELECT agent_id, domain, status, description, role_prompt, initial_prompt, model_id, created_at, updated_at, parent_agent_id, capabilities_json, allowed_tools_json, denied_tools_json, required_mcp_servers_json, authority_json, lifecycle_json, budget_json
                     FROM agent_manifest
                     ORDER BY domain, agent_id",
                )
                .map_err(to_storage)?;
            let rows = statement
                .query_map([], |row| {
                    let status_json: String = row.get(2)?;
                    let created_at: String = row.get(7)?;
                    let updated_at: String = row.get(8)?;
                    let capabilities_json: String = row.get(10)?;
                    let allowed_tools_json: String = row.get(11)?;
                    let denied_tools_json: String = row.get(12)?;
                    let required_mcp_servers_json: String = row.get(13)?;
                    let authority_json: String = row.get(14)?;
                    let lifecycle_json: String = row.get(15)?;
                    let budget_json: String = row.get(16)?;
                    Ok(AgentManifest {
                        agent_id: AgentId(row.get(0)?),
                        domain: row.get(1)?,
                        status: serde_json::from_str(&status_json).unwrap_or(AgentStatus::Spawned),
                        description: row.get(3)?,
                        role_prompt: row.get(4)?,
                        initial_prompt: row.get(5)?,
                        model_id: row.get(6)?,
                        created_at: parse_datetime(&created_at),
                        updated_at: parse_datetime(&updated_at),
                        parent_agent_id: row.get::<_, Option<String>>(9)?.map(AgentId),
                        capabilities: serde_json::from_str(&capabilities_json)
                            .unwrap_or_else(|_| Vec::new()),
                        allowed_tools: serde_json::from_str(&allowed_tools_json)
                            .unwrap_or_else(|_| Vec::new()),
                        denied_tools: serde_json::from_str(&denied_tools_json)
                            .unwrap_or_else(|_| Vec::new()),
                        required_mcp_servers: serde_json::from_str(&required_mcp_servers_json)
                            .unwrap_or_else(|_| Vec::new()),
                        authority: serde_json::from_str(&authority_json)
                            .unwrap_or_else(|_| AgentAuthority::default()),
                        lifecycle: serde_json::from_str(&lifecycle_json)
                            .unwrap_or_else(|_| AgentLifecycle::ready()),
                        budget: serde_json::from_str(&budget_json)
                            .unwrap_or_else(|_| AgentBudget::default()),
                    })
                })
                .map_err(to_storage)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(to_storage)
        })
    }

    pub fn agent_registry(&self) -> Result<AgentRegistrySnapshot> {
        Ok(registry_snapshot(self.list_manifests()?))
    }

    pub fn update_manifest_lifecycle(
        &self,
        agent_id: &AgentId,
        status: AgentStatus,
        phase: AgentLifecyclePhase,
        reason: Option<&str>,
    ) -> Result<Option<AgentManifest>> {
        let Some(mut manifest) = self.load_manifest(agent_id)? else {
            return Ok(None);
        };
        let now = Utc::now();
        manifest.status = status;
        manifest.updated_at = now;
        manifest
            .lifecycle
            .transition(phase, now, reason.map(str::to_string));
        self.save_manifest(&manifest)?;
        Ok(Some(manifest))
    }

    pub fn stats(&self) -> Result<MemoryStats> {
        self.with_conn(|conn| {
            Ok(MemoryStats {
                timeline_events: count_table(conn, "timeline_events")?,
                experiences: count_table(conn, "experiences")?,
                knowledge: count_table(conn, "knowledge")?,
                rules: count_table(conn, "reflexive_rules")?,
                tools: count_table(conn, "tool_registry")?,
            })
        })
    }
}

pub fn write_manifest(path: PathBuf, manifest: &AgentManifest) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| KernelError::Configuration("manifest path missing parent".to_string()))?;
    std::fs::create_dir_all(parent)?;
    let file = ManifestFile {
        agent_id: manifest.agent_id.0.clone(),
        domain: manifest.domain.clone(),
        status: format!("{:?}", manifest.status),
        description: manifest.description.clone(),
        role_prompt: manifest.role_prompt.clone(),
        initial_prompt: manifest.initial_prompt.clone(),
        model_id: manifest.model_id.clone(),
        created_at: manifest.created_at.to_rfc3339(),
        updated_at: manifest.updated_at.to_rfc3339(),
        parent_agent_id: manifest
            .parent_agent_id
            .as_ref()
            .map(|value| value.0.clone()),
        capabilities: manifest.capabilities.clone(),
        allowed_tools: manifest.allowed_tools.clone(),
        denied_tools: manifest.denied_tools.clone(),
        required_mcp_servers: manifest.required_mcp_servers.clone(),
        authority: manifest.authority.clone(),
        lifecycle: manifest.lifecycle.clone(),
        budget: manifest.budget.clone(),
    };
    std::fs::write(path, toml::to_string_pretty(&file).map_err(to_storage)?)
        .map_err(|error| KernelError::Storage(error.to_string()))
}
