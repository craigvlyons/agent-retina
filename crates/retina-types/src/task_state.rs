use crate::ReasonerTaskFraming;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskState {
    pub goal: TaskGoal,
    pub intent_hint: Option<TaskKind>,
    pub reasoner_framing: Option<ReasonerTaskFraming>,
    pub progress: TaskProgress,
    pub frontier: TaskFrontier,
    pub recent_actions: Vec<RecentActionSummary>,
    pub working_sources: Vec<WorkingSource>,
    pub artifact_references: Vec<ArtifactReference>,
    pub avoid: Vec<AvoidRule>,
    pub compaction: Option<CompactionSnapshot>,
}

impl TaskState {
    pub fn with_constraints(mut self, constraints: Vec<String>) -> Self {
        self.goal.constraints = constraints;
        self
    }

    pub fn render(&self) -> String {
        let success_criteria = render_list(&self.goal.success_criteria);
        let constraints = render_list(&self.goal.constraints);
        let completed = render_list(&self.progress.completed_checkpoints);
        let verified = render_list(&self.progress.verified_facts);
        let open_questions = render_list(&self.frontier.open_questions);
        let blockers = render_list(&self.frontier.blockers);
        let framing_hint = self
            .intent_hint
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "none".to_string());
        let reasoner_framing = self
            .reasoner_framing
            .as_ref()
            .map(|framing| {
                format!(
                    "- intent_kind: {}\n- deliverable: {}\n- completion_basis: {}",
                    framing
                        .intent_kind
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "none".to_string()),
                    framing
                        .deliverable
                        .clone()
                        .unwrap_or_else(|| "none".to_string()),
                    framing
                        .completion_basis
                        .clone()
                        .unwrap_or_else(|| "none".to_string())
                )
            })
            .unwrap_or_else(|| "none".to_string());
        let next_action = self
            .frontier
            .next_action_hint
            .clone()
            .unwrap_or_else(|| "none".to_string());
        let recent_actions = if self.recent_actions.is_empty() {
            "none".to_string()
        } else {
            self.recent_actions
                .iter()
                .map(RecentActionSummary::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let working_sources = if self.working_sources.is_empty() {
            "none".to_string()
        } else {
            self.working_sources
                .iter()
                .map(WorkingSource::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let artifacts = if self.artifact_references.is_empty() {
            "none".to_string()
        } else {
            self.artifact_references
                .iter()
                .map(ArtifactReference::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let avoid = if self.avoid.is_empty() {
            "none".to_string()
        } else {
            self.avoid
                .iter()
                .map(AvoidRule::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let compaction = self
            .compaction
            .as_ref()
            .map(CompactionSnapshot::render)
            .unwrap_or_else(|| "none".to_string());

        format!(
            "Goal:\n- objective: {}\n- success_criteria:\n{}\n- constraints:\n{}\n\nIntent hint:\n- {}\n\nReasoner framing:\n{}\n\nProgress:\n- phase: {}\n- step: {} / {}\n- completed:\n{}\n- verified_facts:\n{}\n- output_written: {}\n- output_verified: {}\n\nFrontier:\n- next_action_hint: {}\n- open_questions:\n{}\n- blockers:\n{}\n\nRecent meaningful actions:\n{}\n\nWorking sources:\n{}\n\nArtifact references:\n{}\n\nAvoid:\n{}\n\nCompaction:\n{}",
            self.goal.objective,
            success_criteria,
            constraints,
            framing_hint,
            reasoner_framing,
            self.progress.current_phase,
            self.progress.current_step,
            self.progress.max_steps,
            completed,
            verified,
            self.progress.output_written,
            self.progress.output_verified,
            next_action,
            open_questions,
            blockers,
            recent_actions,
            working_sources,
            artifacts,
            avoid,
            compaction
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskGoal {
    pub objective: String,
    pub success_criteria: Vec<String>,
    pub constraints: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskKind {
    #[default]
    Unknown,
    Answer,
    Output,
}

impl Display for TaskKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Unknown => "unknown",
            Self::Answer => "answer",
            Self::Output => "output",
        };
        f.write_str(label)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskProgress {
    pub current_phase: String,
    pub current_step: usize,
    pub max_steps: usize,
    pub completed_checkpoints: Vec<String>,
    pub verified_facts: Vec<String>,
    pub output_written: bool,
    pub output_verified: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskFrontier {
    pub next_action_hint: Option<String>,
    pub open_questions: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentActionSummary {
    pub step: usize,
    pub action: String,
    pub outcome: String,
    pub artifact_refs: Vec<ArtifactReference>,
}

impl RecentActionSummary {
    pub fn render(&self) -> String {
        let refs = if self.artifact_refs.is_empty() {
            String::new()
        } else {
            format!(
                " [{}]",
                self.artifact_refs
                    .iter()
                    .map(|item| item.locator.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "- step {}: {} -> {}{}",
            self.step, self.action, self.outcome, refs
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkingSource {
    pub kind: String,
    pub locator: String,
    pub role: String,
    pub status: String,
    pub why_it_matters: String,
    pub last_used_step: usize,
    pub evidence_refs: Vec<String>,
    pub page_reference: Option<String>,
    pub extraction_method: Option<String>,
    pub structured_summary: Option<StructuredSourceSummary>,
}

impl WorkingSource {
    pub fn render(&self) -> String {
        let scope = self
            .page_reference
            .as_ref()
            .map(|value| format!(" {value}"))
            .unwrap_or_default();
        let method = self
            .extraction_method
            .as_ref()
            .map(|value| format!(" via {value}"))
            .unwrap_or_default();
        let structured = self
            .structured_summary
            .as_ref()
            .map(|value| format!(" {}", value.render()))
            .unwrap_or_default();
        format!(
            "- {} [{}|{}] {}{}{}{}",
            self.locator, self.kind, self.role, self.status, scope, method, structured
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructuredSourceSummary {
    pub headers: Vec<String>,
    pub sample_rows: usize,
    pub total_rows: usize,
}

impl StructuredSourceSummary {
    pub fn render(&self) -> String {
        let headers = if self.headers.is_empty() {
            "headers=none".to_string()
        } else {
            format!("headers={}", self.headers.join(", "))
        };
        format!(
            "[{}; sample_rows={}; total_rows={}]",
            headers, self.sample_rows, self.total_rows
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactReference {
    pub kind: String,
    pub locator: String,
    pub status: String,
}

impl ArtifactReference {
    pub fn render(&self) -> String {
        format!("- {} [{}|{}]", self.locator, self.kind, self.status)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AvoidRule {
    pub label: String,
    pub reason: String,
}

impl AvoidRule {
    pub fn render(&self) -> String {
        format!("- {}: {}", self.label, self.reason)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionSnapshot {
    pub reason: String,
    pub score_explanations: Vec<CompactionScoreExplanation>,
}

impl CompactionSnapshot {
    pub fn render(&self) -> String {
        let scores = if self.score_explanations.is_empty() {
            "  - none".to_string()
        } else {
            self.score_explanations
                .iter()
                .map(CompactionScoreExplanation::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!("- reason: {}\n- ranking:\n{}", self.reason, scores)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionScoreExplanation {
    pub item_kind: String,
    pub locator: String,
    pub decision: String,
    pub rationale: String,
}

impl CompactionScoreExplanation {
    pub fn render(&self) -> String {
        format!(
            "  - {} {} => {} ({})",
            self.item_kind, self.locator, self.decision, self.rationale
        )
    }
}

fn render_list(items: &[String]) -> String {
    if items.is_empty() {
        "  - none".to_string()
    } else {
        items
            .iter()
            .map(|item| format!("  - {item}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
