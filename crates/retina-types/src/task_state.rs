use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskState {
    pub goal: TaskGoal,
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
        let constraints = render_list(&self.goal.constraints);
        let completed = render_compact_list(&self.progress.completed_checkpoints, 4);
        let verified = render_compact_list(&self.progress.verified_facts, 4);
        let blockers = render_list(&self.frontier.blockers);
        let recent_actions =
            render_tail_items(&self.recent_actions, 4, RecentActionSummary::render);
        let working_sources = render_tail_items(&self.working_sources, 4, WorkingSource::render);
        let artifacts =
            render_tail_items(&self.artifact_references, 4, ArtifactReference::render);
        let avoid = render_tail_items(&self.avoid, 3, AvoidRule::render);
        let compaction = self
            .compaction
            .as_ref()
            .map(CompactionSnapshot::render)
            .unwrap_or_else(|| "none".to_string());

        format!(
            "Goal:\n- objective: {}\n- constraints:\n{}\n\nProgress:\n- phase: {}\n- step: {} / {}\n- recent_completed:\n{}\n- verified_facts:\n{}\n- output_written: {}\n- output_verified: {}\n\nBlockers:\n{}\n\nRecent meaningful actions:\n{}\n\nWorking sources:\n{}\n\nArtifact references:\n{}\n\nAvoid:\n{}\n\nCompaction:\n{}",
            self.goal.objective,
            constraints,
            self.progress.current_phase,
            self.progress.current_step,
            self.progress.max_steps,
            completed,
            verified,
            self.progress.output_written,
            self.progress.output_verified,
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
    pub preview_excerpt: Option<String>,
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
        let preview = self
            .preview_excerpt
            .as_ref()
            .map(|value| format!(" preview=\"{}\"", value))
            .unwrap_or_default();
        format!(
            "- {} [{}|{}] {}{}{}{}{}",
            self.locator, self.kind, self.role, self.status, scope, method, structured, preview
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

fn render_compact_list(items: &[String], limit: usize) -> String {
    if items.is_empty() {
        return "  - none".to_string();
    }

    let shown = items
        .iter()
        .take(limit)
        .map(|item| format!("  - {item}"))
        .collect::<Vec<_>>();
    let remaining = items.len().saturating_sub(limit);
    if remaining == 0 {
        shown.join("\n")
    } else {
        let mut lines = shown;
        lines.push(format!("  - ... {} more", remaining));
        lines.join("\n")
    }
}

fn render_tail_items<T, F>(items: &[T], limit: usize, render: F) -> String
where
    F: Fn(&T) -> String,
{
    if items.is_empty() {
        return "none".to_string();
    }

    let start = items.len().saturating_sub(limit);
    let mut lines = if start > 0 {
        vec![format!("... {} earlier items omitted", start)]
    } else {
        Vec::new()
    };
    lines.extend(items[start..].iter().map(render));
    lines.join("\n")
}
