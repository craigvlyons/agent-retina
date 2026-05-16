use retina_types::{AgentAuthority, AgentBudget, AgentManifest, AgentStatus, KernelError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecialistDefinition {
    pub description: String,
    pub role_prompt: String,
    pub initial_prompt: String,
    #[serde(default)]
    pub model_id: Option<String>,
    pub capabilities: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    #[serde(default)]
    pub required_mcp_servers: Vec<String>,
    pub max_steps: usize,
}

pub fn resolve_definition(agents_dir: &Path, domain: &str) -> Result<SpecialistDefinition> {
    let path = definition_path_for(agents_dir, domain);
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .map_err(|error| KernelError::Configuration(error.to_string()))?;
        let definition: SpecialistDefinition = toml::from_str(&content).map_err(|error| {
            KernelError::Configuration(format!(
                "invalid specialist definition at {}: {error}",
                path.display()
            ))
        })?;
        validate_definition(&definition, &path)?;
        return Ok(definition);
    }
    Ok(builtin_definition_for(domain))
}

pub fn definition_path_for(agents_dir: &Path, domain: &str) -> std::path::PathBuf {
    agents_dir
        .join(format!("specialist-{}", slug(domain)))
        .join("definition.toml")
}

pub fn apply_definition(
    mut manifest: AgentManifest,
    definition: &SpecialistDefinition,
    parent_authority: &AgentAuthority,
) -> AgentManifest {
    manifest.description = definition.description.clone();
    manifest.role_prompt = Some(definition.role_prompt.clone());
    manifest.initial_prompt = Some(definition.initial_prompt.clone());
    manifest.model_id = definition.model_id.clone();
    manifest.capabilities = definition.capabilities.clone();
    manifest.allowed_tools = definition.allowed_tools.clone();
    manifest.denied_tools = definition.denied_tools.clone();
    manifest.required_mcp_servers = definition.required_mcp_servers.clone();
    manifest.authority = scoped_authority(
        parent_authority,
        &manifest.allowed_tools,
        &manifest.denied_tools,
        true,
    );
    manifest.budget = AgentBudget {
        max_steps_per_task: definition.max_steps,
        ..AgentBudget::default()
    };
    manifest.status = AgentStatus::Idle;
    manifest
}

pub fn scoped_authority(
    parent: &AgentAuthority,
    allowed_tools: &[String],
    denied_tools: &[String],
    allow_agent_delegation_default: bool,
) -> AgentAuthority {
    let mut scoped = parent.clone();
    scoped.allow_agent_delegation = allow_agent_delegation_default;
    if !allowed_tools.is_empty() {
        scoped.allow_command_execution = allowed_tools.iter().any(|tool| tool == "run_command");
        scoped.allow_file_reads = allowed_tools.iter().any(|tool| {
            matches!(
                tool.as_str(),
                "inspect_path" | "read_file" | "ingest_structured_data" | "extract_document_text"
            )
        });
        scoped.allow_mcp = allowed_tools.iter().any(|tool| {
            matches!(
                tool.as_str(),
                "list_mcp_resources" | "read_mcp_resource" | "mcp_call"
            )
        });
        scoped.allow_file_search = allowed_tools.iter().any(|tool| {
            matches!(
                tool.as_str(),
                "list_directory" | "find_files" | "search_text"
            )
        });
        scoped.allow_file_writes = allowed_tools.iter().any(|tool| {
            matches!(
                tool.as_str(),
                "edit_file" | "write_file" | "append_file" | "edit_notebook"
            )
        });
        scoped.allow_notes = allowed_tools.iter().any(|tool| tool == "record_note");
        scoped.allow_text_responses = allowed_tools.iter().any(|tool| tool == "respond");
        scoped.allow_agent_delegation = allow_agent_delegation_default
            && allowed_tools.iter().any(|tool| tool == "agent_spawn");
    }
    for tool in denied_tools {
        match tool.as_str() {
            "run_command" => scoped.allow_command_execution = false,
            "inspect_path" | "read_file" | "ingest_structured_data" | "extract_document_text" => {
                scoped.allow_file_reads = false
            }
            "list_mcp_resources" | "read_mcp_resource" | "mcp_call" => scoped.allow_mcp = false,
            "list_directory" | "find_files" | "search_text" => scoped.allow_file_search = false,
            "edit_file" | "write_file" | "append_file" | "edit_notebook" => {
                scoped.allow_file_writes = false
            }
            "record_note" => scoped.allow_notes = false,
            "respond" => scoped.allow_text_responses = false,
            "agent_spawn" => scoped.allow_agent_delegation = false,
            _ => {}
        }
    }
    scoped
}

fn validate_definition(definition: &SpecialistDefinition, path: &Path) -> Result<()> {
    if definition.description.trim().is_empty() {
        return Err(KernelError::Configuration(format!(
            "specialist definition at {} is missing description",
            path.display()
        )));
    }
    if definition.role_prompt.trim().is_empty() {
        return Err(KernelError::Configuration(format!(
            "specialist definition at {} is missing role_prompt",
            path.display()
        )));
    }
    if definition.initial_prompt.trim().is_empty() {
        return Err(KernelError::Configuration(format!(
            "specialist definition at {} is missing initial_prompt",
            path.display()
        )));
    }
    if matches!(definition.model_id.as_deref(), Some(value) if value.trim().is_empty()) {
        return Err(KernelError::Configuration(format!(
            "specialist definition at {} has empty model_id",
            path.display()
        )));
    }
    if definition.max_steps == 0 {
        return Err(KernelError::Configuration(format!(
            "specialist definition at {} must set max_steps > 0",
            path.display()
        )));
    }
    Ok(())
}

fn builtin_definition_for(domain: &str) -> SpecialistDefinition {
    match domain {
        "code" => code_specialist(),
        "research" => research_specialist(),
        "browser" => browser_specialist(),
        "ops" => ops_specialist(),
        _ => generalist_specialist(),
    }
}

fn code_specialist() -> SpecialistDefinition {
    SpecialistDefinition {
        description: "Local code specialist for implementation, patches, and command-backed verification.".to_string(),
        role_prompt: "You are a local code specialist. Focus on implementation, precise file changes, command-backed verification, and grounded completion for the parent worker.".to_string(),
        initial_prompt: "Work only on the delegated implementation scope. Prefer grounded file edits and command-backed verification, then return the essential result for the parent worker.".to_string(),
        model_id: None,
        capabilities: vec![
            "code".to_string(),
            "filesystem".to_string(),
            "search".to_string(),
            "command".to_string(),
            "patching".to_string(),
        ],
        allowed_tools: vec![
            "inspect_path".to_string(),
            "list_directory".to_string(),
            "find_files".to_string(),
            "search_text".to_string(),
            "read_file".to_string(),
            "ingest_structured_data".to_string(),
            "extract_document_text".to_string(),
            "edit_file".to_string(),
            "write_file".to_string(),
            "append_file".to_string(),
            "edit_notebook".to_string(),
            "run_command".to_string(),
            "record_note".to_string(),
            "respond".to_string(),
            "agent_spawn".to_string(),
        ],
        denied_tools: Vec::new(),
        required_mcp_servers: Vec::new(),
        max_steps: 14,
    }
}

fn research_specialist() -> SpecialistDefinition {
    SpecialistDefinition {
        description: "Local research specialist for reading, extracting, and synthesizing local sources.".to_string(),
        role_prompt: "You are a local research specialist. Focus on reading available sources, extracting grounded facts, and returning a concise synthesis for the parent worker.".to_string(),
        initial_prompt: "Read the available sources, extract the strongest grounded facts, and return a concise synthesis for the parent worker without unnecessary interpretation.".to_string(),
        model_id: None,
        capabilities: vec![
            "research".to_string(),
            "documents".to_string(),
            "synthesis".to_string(),
            "search".to_string(),
        ],
        allowed_tools: vec![
            "inspect_path".to_string(),
            "list_directory".to_string(),
            "find_files".to_string(),
            "search_text".to_string(),
            "read_file".to_string(),
            "ingest_structured_data".to_string(),
            "extract_document_text".to_string(),
            "edit_file".to_string(),
            "write_file".to_string(),
            "append_file".to_string(),
            "edit_notebook".to_string(),
            "record_note".to_string(),
            "respond".to_string(),
            "agent_spawn".to_string(),
        ],
        denied_tools: vec!["run_command".to_string()],
        required_mcp_servers: Vec::new(),
        max_steps: 12,
    }
}

fn browser_specialist() -> SpecialistDefinition {
    SpecialistDefinition {
        description: "Local browser-style specialist for forms, documents, and interactive artifacts.".to_string(),
        role_prompt: "You are a local browser-style specialist. Focus on document and artifact handling, careful field interpretation, and grounded output for the parent worker.".to_string(),
        initial_prompt: "Handle the delegated document or artifact task carefully, stay grounded in the available evidence, and return only the result the parent worker needs.".to_string(),
        model_id: None,
        capabilities: vec![
            "browser".to_string(),
            "forms".to_string(),
            "documents".to_string(),
            "interaction".to_string(),
        ],
        allowed_tools: vec![
            "inspect_path".to_string(),
            "list_directory".to_string(),
            "find_files".to_string(),
            "search_text".to_string(),
            "read_file".to_string(),
            "extract_document_text".to_string(),
            "edit_file".to_string(),
            "write_file".to_string(),
            "append_file".to_string(),
            "edit_notebook".to_string(),
            "record_note".to_string(),
            "respond".to_string(),
        ],
        denied_tools: vec!["run_command".to_string(), "agent_spawn".to_string()],
        required_mcp_servers: Vec::new(),
        max_steps: 12,
    }
}

fn ops_specialist() -> SpecialistDefinition {
    SpecialistDefinition {
        description: "Local ops specialist for command-heavy system, service, and deploy work.".to_string(),
        role_prompt: "You are a local ops specialist. Focus on command-heavy system work, status verification from observed evidence, and concise grounded reporting for the parent worker.".to_string(),
        initial_prompt: "Take the next operational steps that materially advance the task, verify status from observed evidence, and return a concise grounded result for the parent worker.".to_string(),
        model_id: None,
        capabilities: vec![
            "ops".to_string(),
            "services".to_string(),
            "deploy".to_string(),
            "command".to_string(),
        ],
        allowed_tools: vec![
            "inspect_path".to_string(),
            "list_directory".to_string(),
            "find_files".to_string(),
            "search_text".to_string(),
            "read_file".to_string(),
            "edit_file".to_string(),
            "write_file".to_string(),
            "append_file".to_string(),
            "edit_notebook".to_string(),
            "run_command".to_string(),
            "record_note".to_string(),
            "respond".to_string(),
            "agent_spawn".to_string(),
        ],
        denied_tools: Vec::new(),
        required_mcp_servers: Vec::new(),
        max_steps: 14,
    }
}

fn generalist_specialist() -> SpecialistDefinition {
    SpecialistDefinition {
        description: "Local generalist specialist for reusable multi-step work.".to_string(),
        role_prompt: "You are a local generalist specialist. Focus on bounded multi-step execution, grounded use of available tools, and a concise result for the parent worker.".to_string(),
        initial_prompt: "Stay within the delegated scope, use the available tools in grounded steps, and return a concise result that the parent worker can use directly.".to_string(),
        model_id: None,
        capabilities: vec![
            "generalist".to_string(),
            "filesystem".to_string(),
            "search".to_string(),
            "notes".to_string(),
        ],
        allowed_tools: vec![
            "inspect_path".to_string(),
            "list_directory".to_string(),
            "find_files".to_string(),
            "search_text".to_string(),
            "read_file".to_string(),
            "ingest_structured_data".to_string(),
            "extract_document_text".to_string(),
            "edit_file".to_string(),
            "write_file".to_string(),
            "append_file".to_string(),
            "edit_notebook".to_string(),
            "record_note".to_string(),
            "respond".to_string(),
            "agent_spawn".to_string(),
        ],
        denied_tools: vec!["run_command".to_string()],
        required_mcp_servers: Vec::new(),
        max_steps: 12,
    }
}

fn slug(input: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use retina_types::{AgentId, AgentLifecycle};
    use tempfile::tempdir;

    #[test]
    fn research_definition_scopes_command_execution_off() {
        let now = Utc::now();
        let manifest = AgentManifest {
            agent_id: AgentId("specialist-research".to_string()),
            domain: "research".to_string(),
            status: AgentStatus::Spawned,
            description: "placeholder".to_string(),
            role_prompt: None,
            initial_prompt: None,
            model_id: None,
            created_at: now,
            updated_at: now,
            parent_agent_id: None,
            capabilities: Vec::new(),
            allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            required_mcp_servers: Vec::new(),
            authority: AgentAuthority::default(),
            lifecycle: AgentLifecycle::ready(),
            budget: AgentBudget::default(),
        };
        let definition = resolve_definition(Path::new("/tmp/does-not-exist"), "research")
            .unwrap_or_else(|error| panic!("research definition: {error}"));
        let manifest = apply_definition(manifest, &definition, &AgentAuthority::default());

        assert_eq!(
            manifest.role_prompt.as_deref(),
            Some(
                "You are a local research specialist. Focus on reading available sources, extracting grounded facts, and returning a concise synthesis for the parent worker."
            )
        );
        assert_eq!(
            manifest.initial_prompt.as_deref(),
            Some(
                "Read the available sources, extract the strongest grounded facts, and return a concise synthesis for the parent worker without unnecessary interpretation."
            )
        );
        assert_eq!(manifest.model_id, None);
        assert!(
            manifest
                .allowed_tools
                .iter()
                .any(|tool| tool == "read_file")
        );
        assert!(
            manifest
                .denied_tools
                .iter()
                .any(|tool| tool == "run_command")
        );
        assert!(!manifest.authority.allow_command_execution);
    }

    #[test]
    fn definition_path_uses_specialist_agent_directory() {
        let path = definition_path_for(Path::new("/tmp/agents"), "research");
        assert_eq!(
            path,
            Path::new("/tmp/agents/specialist-research/definition.toml")
        );
    }

    #[test]
    fn custom_definition_overrides_builtin() {
        let dir = tempdir().unwrap_or_else(|error| panic!("tempdir: {error}"));
        let definition_path = definition_path_for(dir.path(), "research");
        std::fs::create_dir_all(
            definition_path
                .parent()
                .unwrap_or_else(|| panic!("definition path missing parent")),
        )
        .unwrap_or_else(|error| panic!("mkdir: {error}"));
        std::fs::write(
            &definition_path,
            r#"description = "Custom research specialist"
role_prompt = "You are a custom research specialist."
initial_prompt = "Read the delegated sources and return only the answer."
model_id = "claude-sonnet-4-20250514"
capabilities = ["research", "custom"]
allowed_tools = ["read_file", "respond"]
denied_tools = ["run_command"]
required_mcp_servers = ["docs"]
max_steps = 9
"#,
        )
        .unwrap_or_else(|error| panic!("write definition: {error}"));

        let definition = resolve_definition(dir.path(), "research")
            .unwrap_or_else(|error| panic!("load custom definition: {error}"));
        assert_eq!(definition.description, "Custom research specialist");
        assert_eq!(
            definition.role_prompt,
            "You are a custom research specialist."
        );
        assert_eq!(
            definition.initial_prompt,
            "Read the delegated sources and return only the answer."
        );
        assert_eq!(
            definition.model_id.as_deref(),
            Some("claude-sonnet-4-20250514")
        );
        assert_eq!(definition.required_mcp_servers, vec!["docs".to_string()]);
        assert_eq!(definition.max_steps, 9);
    }
}
