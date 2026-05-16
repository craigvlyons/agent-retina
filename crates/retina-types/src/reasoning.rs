use crate::{
    Action, AgentId, ArtifactReference, CompactedResultReference, CompactionSnapshot,
    NextStepGuidance, RecentActionStatus, RecentActionSummary, StoredResultLedger, TaskGoal,
    TaskKind, TaskProgress, TaskState, TranscriptLedger, TranscriptUnitKind, WorkingSource,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssembledContext {
    pub identity: String,
    pub task: String,
    pub continuation_window: ActiveContinuationWindow,
    #[serde(default)]
    pub recent_context: Option<RecentContext>,
    pub tools: Vec<ToolDescriptor>,
    pub memory_slice: Vec<String>,
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
            "Identity:\n{}\n\nTask:\n{}\n\nActive continuation window:\n{}\n\nTools:\n{}\n\nOperator guidance:\n{}",
            self.identity,
            self.task,
            self.continuation_window.render(),
            tools,
            operator_guidance,
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ActiveContinuationWindow {
    pub objective: String,
    pub current_step: usize,
    pub max_steps: usize,
    pub transcript: TranscriptLedger,
    pub stored_results: StoredResultLedger,
    pub reannounced_sources: Vec<WorkingSource>,
    pub reannounced_artifacts: Vec<ArtifactReference>,
    pub next_step_guidance: Option<NextStepGuidance>,
    pub compaction_boundaries: Vec<CompactionSnapshot>,
    pub reannounced_compacted_results: Vec<CompactedResultReference>,
}

impl ActiveContinuationWindow {
    pub fn project_task_state(&self) -> TaskState {
        let working_sources = self.transcript.reduced_working_sources();
        let artifact_references = self.transcript.reduced_artifact_references();
        let output_written = artifact_references.iter().any(|artifact| {
            matches!(
                artifact.status.as_str(),
                "created" | "written" | "overwritten" | "appended" | "command_changed"
            )
        });
        let output_verified = working_sources.iter().any(|source| {
            source.role == "generated"
                && matches!(
                    source.status.as_str(),
                    "created" | "written" | "overwritten" | "appended" | "command_changed"
                )
                && source.preview_excerpt.is_some()
        }) || artifact_references.iter().any(|artifact| {
            matches!(
                artifact.status.as_str(),
                "read" | "structured_read" | "extracted"
            )
        });
        TaskState {
            goal: TaskGoal {
                objective: self.objective.clone(),
                constraints: Vec::new(),
            },
            progress: TaskProgress {
                current_phase: projected_task_phase(self.current_step, self.max_steps),
                current_step: self.current_step,
                max_steps: self.max_steps,
                completed_checkpoints: projected_completed_checkpoints(&self.transcript, 4),
                verified_facts: projected_verified_facts(&working_sources, &artifact_references),
                output_written,
                output_verified,
            },
            transcript: self.transcript.clone(),
            stored_results: self.stored_results.clone(),
            recent_actions: projected_recent_actions(&self.transcript, 4),
            working_sources,
            artifact_references,
            next_step_guidance: self.next_step_guidance.clone(),
            compaction: self.compaction_boundaries.last().cloned(),
            compaction_history: self.compaction_boundaries.clone(),
            compacted_results: flatten_compacted_results(&self.compaction_boundaries),
        }
    }

    pub fn render(&self) -> String {
        let transcript_units = if self.transcript.is_empty() {
            "none".to_string()
        } else {
            self.transcript
                .entries()
                .iter()
                .map(|item| item.render())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let stored_result_refs = if self.stored_results.is_empty() {
            "none".to_string()
        } else {
            self.stored_results
                .entries()
                .iter()
                .map(|item| item.render())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let reannounced_sources = if self.reannounced_sources.is_empty() {
            "none".to_string()
        } else {
            self.reannounced_sources
                .iter()
                .map(WorkingSource::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let reannounced_artifacts = if self.reannounced_artifacts.is_empty() {
            "none".to_string()
        } else {
            self.reannounced_artifacts
                .iter()
                .map(ArtifactReference::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let next_step_guidance = self
            .next_step_guidance
            .as_ref()
            .map(NextStepGuidance::render)
            .unwrap_or_else(|| "none".to_string());
        let compaction_boundaries = if self.compaction_boundaries.is_empty() {
            "none".to_string()
        } else {
            self.compaction_boundaries
                .iter()
                .map(CompactionSnapshot::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let reannounced_compacted_results = if self.reannounced_compacted_results.is_empty() {
            "none".to_string()
        } else {
            self.reannounced_compacted_results
                .iter()
                .map(CompactedResultReference::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "- objective: {}\n- step: {} / {}\n- transcript_units:\n{}\n- stored_result_refs:\n{}\n- reannounced_sources:\n{}\n- reannounced_artifacts:\n{}\n- next_step_guidance:\n{}\n- compaction_boundaries:\n{}\n- reannounced_compacted_results:\n{}",
            self.objective,
            self.current_step,
            self.max_steps,
            transcript_units,
            stored_result_refs,
            reannounced_sources,
            reannounced_artifacts,
            next_step_guidance,
            compaction_boundaries,
            reannounced_compacted_results
        )
    }
}

fn projected_completed_checkpoints(transcript: &TranscriptLedger, limit: usize) -> Vec<String> {
    transcript
        .entries()
        .iter()
        .filter(|item| {
            matches!(
                item.kind,
                TranscriptUnitKind::ToolResult
                    | TranscriptUnitKind::FinalResponse
                    | TranscriptUnitKind::TerminalBlocked
                    | TranscriptUnitKind::TerminalFailure
                    | TranscriptUnitKind::RestoredContinuation
                    | TranscriptUnitKind::CompactBoundary
            )
        })
        .rev()
        .take(limit)
        .map(|item| format!("step {}: {} -> {}", item.step, item.kind, item.summary))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn flatten_compacted_results(history: &[CompactionSnapshot]) -> Vec<CompactedResultReference> {
    history
        .iter()
        .flat_map(|snapshot| snapshot.compacted_results.iter().cloned())
        .collect()
}

fn projected_recent_actions(
    transcript: &TranscriptLedger,
    limit: usize,
) -> Vec<RecentActionSummary> {
    let entries = transcript.entries();
    let mut derived = Vec::new();

    for (index, item) in entries.iter().enumerate().rev() {
        let Some(status) = projected_status_for_kind(&item.kind) else {
            continue;
        };
        let action = entries[..=index]
            .iter()
            .rev()
            .find(|candidate| {
                candidate.step == item.step
                    && matches!(candidate.kind, TranscriptUnitKind::ToolInvocation)
            })
            .map(|candidate| candidate.summary.clone())
            .unwrap_or_else(|| item.kind.to_string());
        let artifact_refs = if item.evidence_refs.is_empty() {
            item.primary_locator
                .as_ref()
                .map(|locator| {
                    vec![ArtifactReference {
                        kind: "evidence".to_string(),
                        locator: locator.clone(),
                        status: "referenced".to_string(),
                    }]
                })
                .unwrap_or_default()
        } else {
            item.evidence_refs
                .iter()
                .map(|locator| ArtifactReference {
                    kind: "evidence".to_string(),
                    locator: locator.clone(),
                    status: "referenced".to_string(),
                })
                .collect()
        };
        derived.push(RecentActionSummary {
            step: item.step,
            action,
            status,
            outcome: item.summary.clone(),
            artifact_refs,
        });
        if derived.len() >= limit {
            break;
        }
    }

    derived.into_iter().rev().collect()
}

fn projected_status_for_kind(kind: &TranscriptUnitKind) -> Option<RecentActionStatus> {
    match kind {
        TranscriptUnitKind::ToolResult => Some(RecentActionStatus::Succeeded),
        TranscriptUnitKind::FinalResponse => Some(RecentActionStatus::Responded),
        TranscriptUnitKind::TerminalFailure => Some(RecentActionStatus::Failed),
        TranscriptUnitKind::TerminalBlocked => Some(RecentActionStatus::Blocked),
        _ => None,
    }
}

fn projected_task_phase(current_step: usize, max_steps: usize) -> String {
    if current_step == 0 {
        "starting".to_string()
    } else if current_step >= max_steps {
        "final step".to_string()
    } else {
        format!("working through step {} of {}", current_step, max_steps)
    }
}

fn projected_verified_facts(
    working_sources: &[WorkingSource],
    references: &[ArtifactReference],
) -> Vec<String> {
    let mut facts = Vec::new();

    for source in working_sources.iter().rev().take(5).rev() {
        let fact = match (source.role.as_str(), source.status.as_str()) {
            ("authoritative", "read") => format!("authoritative file read from {}", source.locator),
            ("authoritative", "excerpted") => {
                format!(
                    "authoritative document text extracted from {}",
                    source.locator
                )
            }
            ("authoritative", "ingested") => {
                format!(
                    "authoritative structured data ingested from {}",
                    source.locator
                )
            }
            ("generated", status) => format!("produced artifact {} ({status})", source.locator),
            (_, "matched") => format!("candidate source identified at {}", source.locator),
            (_, "matched_text") => format!("text evidence identified in {}", source.locator),
            (_, "listed") => format!("directory explored at {}", source.locator),
            (_, "inspected") => format!("path inspected at {}", source.locator),
            _ => format!("{} {} [{}]", source.status, source.locator, source.role),
        };
        push_unique_fact(&mut facts, fact);
    }

    for reference in references.iter().rev().take(5).rev() {
        let fact = match reference.status.as_str() {
            "read" => format!("exact evidence kept for {}", reference.locator),
            "structured_read" => {
                format!("exact structured evidence kept for {}", reference.locator)
            }
            "extracted" => format!("exact extracted evidence kept for {}", reference.locator),
            "matched" => format!(
                "candidate artifact reference kept for {}",
                reference.locator
            ),
            "searched" => format!("search evidence kept for {}", reference.locator),
            "created" | "written" | "overwritten" | "appended" | "command_changed" => {
                format!(
                    "output or changed artifact tracked at {}",
                    reference.locator
                )
            }
            _ => format!("{} {}", reference.status, reference.locator),
        };
        push_unique_fact(&mut facts, fact);
    }

    facts.into_iter().take(8).collect()
}

fn push_unique_fact(facts: &mut Vec<String>, fact: String) {
    if !facts.contains(&fact) {
        facts.push(fact);
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
