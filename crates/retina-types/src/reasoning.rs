use crate::{Action, AgentId, ArtifactReference, TaskKind, TaskState, WorkingSource};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
    pub last_result_summary: Option<String>,
    pub recent_steps: Vec<String>,
    pub operator_guidance: Option<String>,
    pub current_step: usize,
    pub max_steps: usize,
}

impl AssembledContext {
    pub fn render(&self) -> String {
        let tools = self
            .tools
            .iter()
            .map(|tool| format!("- {}: {}", tool.name, tool.description))
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub parent_agent_id: Option<AgentId>,
    pub capabilities: Vec<String>,
    pub authority: crate::AgentAuthority,
    pub lifecycle: crate::AgentLifecycle,
    pub budget: crate::AgentBudget,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentAuthority {
    pub allow_command_execution: bool,
    pub allow_file_reads: bool,
    pub allow_file_writes: bool,
    pub allow_file_search: bool,
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
            allow_notes: true,
            allow_text_responses: true,
            accessible_roots: Vec::new(),
        }
    }
}
