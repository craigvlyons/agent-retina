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
    if !allowed_tools.is_empty() && !uses_full_parent_toolset(allowed_tools) {
        scoped.allow_command_execution = allowed_tools.iter().any(|tool| tool == "run_command");
        scoped.allow_file_reads = allowed_tools.iter().any(|tool| {
            matches!(
                tool.as_str(),
                "inspect_path" | "read_file" | "ingest_structured_data" | "extract_document_text"
            )
        });
        scoped.allow_mcp = allowed_tools.iter().any(|tool| {
            matches!(tool.as_str(), "list_mcp_resources" | "read_mcp_resource")
                || retina_types::parse_mcp_tool_name(tool).is_some()
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
            "list_mcp_resources" | "read_mcp_resource" => scoped.allow_mcp = false,
            "list_directory" | "find_files" | "search_text" => scoped.allow_file_search = false,
            "edit_file" | "write_file" | "append_file" | "edit_notebook" => {
                scoped.allow_file_writes = false
            }
            "record_note" => scoped.allow_notes = false,
            "respond" => scoped.allow_text_responses = false,
            "agent_spawn" => scoped.allow_agent_delegation = false,
            _ => {
                if retina_types::parse_mcp_tool_name(tool).is_some() {
                    scoped.allow_mcp = false;
                }
            }
        }
    }
    scoped
}

fn uses_full_parent_toolset(allowed_tools: &[String]) -> bool {
    allowed_tools.iter().any(|tool| tool.trim() == "*")
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

fn general_purpose_role_prompt() -> String {
    "You are a local general-purpose specialist. Use the available tools to complete the delegated task fully. Search broadly when you do not yet know where something lives, stay grounded in observed evidence, and return only the essential result for the parent worker.".to_string()
}

fn general_purpose_initial_prompt() -> String {
    "Complete the delegated task fully. Search broadly when the location or answer is not yet clear, narrow down from the strongest evidence, and once you have enough grounded information, return a concise factual result for the parent worker. For simple directory or file inventory tasks, one grounded listing is usually enough; do not repeat the same listing or open child files unless the task actually needs file contents. Treat requests for files on or in a folder as top-level scope by default; only descend into nested folders when the task says under, recursively, across subfolders, or otherwise clearly asks for nested contents. When matching files for a top-level folder request, keep the file-match scope top-level as well instead of mixing nested results into the same answer. Keep simple inventory and summary answers short: counts, notable items, and brief content summaries are enough unless the task asks for full detail.".to_string()
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
        description: "Local general-purpose research specialist for complex questions, file search, and multi-step delegated tasks.".to_string(),
        role_prompt: general_purpose_role_prompt(),
        initial_prompt: general_purpose_initial_prompt(),
        model_id: None,
        capabilities: vec![
            "research".to_string(),
            "documents".to_string(),
            "synthesis".to_string(),
            "search".to_string(),
        ],
        allowed_tools: vec!["*".to_string()],
        denied_tools: Vec::new(),
        required_mcp_servers: Vec::new(),
        max_steps: 14,
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
        description: "Local general-purpose specialist for reusable multi-step work.".to_string(),
        role_prompt: general_purpose_role_prompt(),
        initial_prompt: general_purpose_initial_prompt(),
        model_id: None,
        capabilities: vec![
            "generalist".to_string(),
            "filesystem".to_string(),
            "search".to_string(),
            "notes".to_string(),
        ],
        allowed_tools: vec!["*".to_string()],
        denied_tools: Vec::new(),
        required_mcp_servers: Vec::new(),
        max_steps: 14,
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
    fn research_definition_inherits_full_parent_tool_surface() {
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
                "You are a local general-purpose specialist. Use the available tools to complete the delegated task fully. Search broadly when you do not yet know where something lives, stay grounded in observed evidence, and return only the essential result for the parent worker."
            )
        );
        assert_eq!(
            manifest.initial_prompt.as_deref(),
            Some(
                "Complete the delegated task fully. Search broadly when the location or answer is not yet clear, narrow down from the strongest evidence, and once you have enough grounded information, return a concise factual result for the parent worker. For simple directory or file inventory tasks, one grounded listing is usually enough; do not repeat the same listing or open child files unless the task actually needs file contents. Treat requests for files on or in a folder as top-level scope by default; only descend into nested folders when the task says under, recursively, across subfolders, or otherwise clearly asks for nested contents. When matching files for a top-level folder request, keep the file-match scope top-level as well instead of mixing nested results into the same answer. Keep simple inventory and summary answers short: counts, notable items, and brief content summaries are enough unless the task asks for full detail."
            )
        );
        assert_eq!(manifest.model_id, None);
        assert_eq!(manifest.allowed_tools, vec!["*".to_string()]);
        assert!(manifest.denied_tools.is_empty());
        assert!(manifest.authority.allow_command_execution);
        assert!(manifest.authority.allow_file_reads);
        assert!(manifest.authority.allow_file_search);
    }

    #[test]
    fn wildcard_allowed_tools_preserves_parent_authority() {
        let authority = scoped_authority(
            &AgentAuthority {
                allow_command_execution: true,
                allow_file_reads: true,
                allow_file_writes: false,
                allow_file_search: true,
                allow_mcp: false,
                allow_agent_delegation: true,
                allow_notes: false,
                allow_text_responses: true,
                accessible_roots: vec![],
            },
            &["*".to_string()],
            &[],
            true,
        );

        assert!(authority.allow_command_execution);
        assert!(authority.allow_file_reads);
        assert!(authority.allow_file_search);
        assert!(!authority.allow_file_writes);
        assert!(authority.allow_agent_delegation);
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
