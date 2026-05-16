use crate::{Action, AgentId, ArtifactReference, TaskKind, TaskState, WorkingSource};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssembledContext {
    pub identity: String,
    pub task: String,
    pub task_state: TaskState,
    #[serde(default)]
    pub recent_context: Option<RecentContext>,
    pub tools: Vec<ToolDescriptor>,
    pub memory_slice: Vec<String>,
    pub last_result: Option<String>,
    pub operator_guidance: Option<String>,
    pub current_step: usize,
    pub max_steps: usize,
}

impl AssembledContext {
    pub fn render(&self) -> String {
        let tools = self
            .tools
            .iter()
            .map(ToolDescriptor::render)
            .collect::<Vec<_>>()
            .join("\n");
        let operator_guidance = self
            .operator_guidance
            .clone()
            .unwrap_or_else(|| "none".to_string());
        format!(
            "Identity:\n{}\n\nTask:\n{}\n\nTask state:\n{}\n\nTools:\n{}\n\nOperator guidance:\n{}",
            self.identity,
            self.task,
            self.task_state.render(),
            tools,
            operator_guidance,
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentContext {
    pub prior_objective: String,
    pub prior_answer_summary: Option<String>,
    pub sources: Vec<WorkingSource>,
    pub artifacts: Vec<ArtifactReference>,
}

impl RecentContext {
    pub fn render(&self) -> String {
        let answer = self
            .prior_answer_summary
            .clone()
            .unwrap_or_else(|| "none".to_string());
        let sources = if self.sources.is_empty() {
            "  - none".to_string()
        } else {
            self.sources
                .iter()
                .map(WorkingSource::render)
                .map(|item| format!("  {}", item))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let artifacts = if self.artifacts.is_empty() {
            "  - none".to_string()
        } else {
            self.artifacts
                .iter()
                .map(ArtifactReference::render)
                .map(|item| format!("  {}", item))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "- prior_objective: {}\n- prior_answer_summary: {}\n- sources:\n{}\n- artifacts:\n{}",
            self.prior_objective, answer, sources, artifacts
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub source: ToolSourceKind,
    #[serde(default)]
    pub concurrency: ToolConcurrencyClass,
    #[serde(default)]
    pub approval: ToolApprovalPolicy,
    #[serde(default)]
    pub required_authority: Vec<String>,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default = "default_tool_input_schema")]
    pub input_schema: Value,
}

impl ToolDescriptor {
    pub fn render(&self) -> String {
        let mut traits = vec![self.concurrency.label().to_string()];
        if self.streaming {
            traits.push("streaming".to_string());
        }
        match self.approval {
            ToolApprovalPolicy::None => {}
            ToolApprovalPolicy::ExplicitOperatorApproval => {
                traits.push("approval".to_string());
            }
            ToolApprovalPolicy::ToolDefined => {
                traits.push("conditional_approval".to_string());
            }
        }
        if !self.required_authority.is_empty() {
            traits.push(format!("requires {}", self.required_authority.join(",")));
        }
        let input_summary = render_input_schema_summary(&self.input_schema);
        format!(
            "- {} [{}]: {}{}",
            self.name,
            traits.join(", "),
            self.description,
            input_summary
        )
    }
}

fn default_tool_input_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "required": []
    })
}

fn render_input_schema_summary(schema: &Value) -> String {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return String::new();
    };
    if properties.is_empty() {
        return String::new();
    }

    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();

    let fields = properties
        .iter()
        .take(6)
        .map(|(name, value)| {
            let field_type = value.get("type").and_then(Value::as_str).unwrap_or("value");
            if required.contains(name.as_str()) {
                format!("{name}:{field_type}*")
            } else {
                format!("{name}:{field_type}")
            }
        })
        .collect::<Vec<_>>();

    if fields.is_empty() {
        String::new()
    } else {
        format!(" Input: {}.", fields.join(", "))
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolSourceKind {
    #[default]
    BuiltinShell,
    MemoryRecord,
    McpServer,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolConcurrencyClass {
    #[default]
    ReadOnly,
    Mutation,
    LongRunning,
    Streaming,
    Unknown,
}

impl ToolConcurrencyClass {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Mutation => "mutation",
            Self::LongRunning => "long_running",
            Self::Streaming => "streaming",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolApprovalPolicy {
    #[default]
    None,
    ExplicitOperatorApproval,
    ToolDefined,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReasonRequest {
    pub context: AssembledContext,
    pub tools: Vec<ToolDescriptor>,
    pub constraints: Vec<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReasonResponse {
    pub action: Action,
    pub task_complete: bool,
    #[serde(default)]
    pub framing: Option<ReasonerTaskFraming>,
    pub reasoning: Option<String>,
    pub tokens_used: TokenUsage,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReasonerTaskFraming {
    pub intent_kind: Option<TaskKind>,
    pub deliverable: Option<String>,
    pub completion_basis: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReasonerCapabilities {
    pub max_context_tokens: u32,
    pub supports_tool_use: bool,
    pub supports_vision: bool,
    pub supports_caching: bool,
    pub model_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShellCapabilities {
    pub can_execute_commands: bool,
    pub can_read_files: bool,
    pub can_write_files: bool,
    pub can_search_files: bool,
    pub can_extract_documents: bool,
    pub can_write_notes: bool,
    pub can_respond_text: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HardConstraint {
    DeleteOrKillRequireApproval,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RoutingDecision {
    HandleDirectly,
    RouteToExisting(AgentId),
    Reactivate(AgentId),
    SpawnSpecialist { domain: String, capability: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingCandidate {
    pub agent_id: AgentId,
    pub domain: String,
    pub status: crate::AgentStatus,
    pub capability_match: f64,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingAssessment {
    pub effective_decision: RoutingDecision,
    pub recommended_decision: RoutingDecision,
    pub candidates: Vec<RoutingCandidate>,
    pub rationale: String,
    pub network_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentManifest {
    pub agent_id: AgentId,
    pub domain: String,
    pub status: crate::AgentStatus,
    pub description: String,
    #[serde(default)]
    pub role_prompt: Option<String>,
    #[serde(default)]
    pub initial_prompt: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub parent_agent_id: Option<AgentId>,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default)]
    pub required_mcp_servers: Vec<String>,
    pub authority: crate::AgentAuthority,
    pub lifecycle: crate::AgentLifecycle,
    pub budget: crate::AgentBudget,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentAuthority {
    pub allow_command_execution: bool,
    pub allow_file_reads: bool,
    pub allow_file_writes: bool,
    pub allow_file_search: bool,
    pub allow_mcp: bool,
    pub allow_agent_delegation: bool,
    pub allow_notes: bool,
    pub allow_text_responses: bool,
    pub accessible_roots: Vec<PathBuf>,
}

impl Default for AgentAuthority {
    fn default() -> Self {
        Self {
            allow_command_execution: true,
            allow_file_reads: true,
            allow_file_writes: true,
            allow_file_search: true,
            allow_mcp: true,
            allow_agent_delegation: true,
            allow_notes: true,
            allow_text_responses: true,
            accessible_roots: Vec::new(),
        }
    }
}
