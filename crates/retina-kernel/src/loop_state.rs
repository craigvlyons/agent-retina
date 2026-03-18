use crate::result_helpers::{
    artifact_references_for_result, compact_action_result_for_context,
    compact_last_result_for_compacted_context, prioritized_artifact_references,
    prioritized_working_sources, repeated_step_signature, summarize_action_result,
    working_sources_for_result,
};
use retina_types::*;
use std::collections::HashMap;

pub(crate) struct TaskLoopState {
    pub(crate) step_index: usize,
    pub(crate) last_result_json: Option<String>,
    pub(crate) last_result_summary: Option<String>,
    pub(crate) recent_steps: Vec<String>,
    pub(crate) recent_action_summaries: Vec<RecentActionSummary>,
    pub(crate) working_sources: Vec<WorkingSource>,
    pub(crate) artifact_references: Vec<ArtifactReference>,
    pub(crate) avoid_rules: Vec<AvoidRule>,
    pub(crate) last_reasoner_framing: Option<ReasonerTaskFraming>,
    pub(crate) compaction_count: usize,
    pub(crate) last_compaction_reason: Option<String>,
    pub(crate) last_compaction_snapshot: Option<CompactionSnapshot>,
    seen_signatures: HashMap<String, usize>,
}

impl TaskLoopState {
    pub(crate) fn new(_max_steps: usize) -> Self {
        Self {
            step_index: 0,
            last_result_json: None,
            last_result_summary: None,
            recent_steps: Vec::new(),
            recent_action_summaries: Vec::new(),
            working_sources: Vec::new(),
            artifact_references: Vec::new(),
            avoid_rules: Vec::new(),
            last_reasoner_framing: None,
            compaction_count: 0,
            last_compaction_reason: None,
            last_compaction_snapshot: None,
            seen_signatures: HashMap::new(),
        }
    }

    pub(crate) fn record_step(
        &mut self,
        action: &Action,
        outcome: &Outcome,
    ) -> Result<StepProgress> {
        self.step_index += 1;
        let mut repeated_without_progress = false;
        self.last_result_json = match outcome {
            Outcome::Success(result) if !matches!(action, Action::Respond { .. }) => {
                let summary = summarize_action_result(result);
                let artifact_refs = artifact_references_for_result(result);
                let working_sources =
                    working_sources_for_result(action, result, self.step_index + 1);
                self.last_result_summary = Some(summary.clone());
                self.recent_steps.push(format!(
                    "step {}: {} -> {}",
                    self.step_index,
                    action_label(action),
                    summary
                ));
                trim_recent_steps(&mut self.recent_steps);
                self.recent_action_summaries.push(RecentActionSummary {
                    step: self.step_index,
                    action: action_label(action),
                    outcome: summary.clone(),
                    artifact_refs: artifact_refs.clone(),
                });
                trim_recent_action_summaries(&mut self.recent_action_summaries);
                merge_working_sources(&mut self.working_sources, working_sources);
                merge_artifact_references(&mut self.artifact_references, artifact_refs);
                if let Some(signature) = repeated_step_signature(action, result) {
                    let count = self.seen_signatures.entry(signature).or_insert(0);
                    *count += 1;
                    repeated_without_progress = *count > 3;
                }
                Some(
                    compact_action_result_for_context(result)
                        .map_err(|error| KernelError::Reasoning(error.to_string()))?,
                )
            }
            Outcome::Success(_) => {
                self.last_result_summary = Some("responded to operator".to_string());
                self.recent_steps.push(format!(
                    "step {}: {} -> responded to operator",
                    self.step_index,
                    action_label(action)
                ));
                trim_recent_steps(&mut self.recent_steps);
                self.recent_action_summaries.push(RecentActionSummary {
                    step: self.step_index,
                    action: action_label(action),
                    outcome: "responded to operator".to_string(),
                    artifact_refs: Vec::new(),
                });
                trim_recent_action_summaries(&mut self.recent_action_summaries);
                None
            }
            Outcome::Failure(reason) | Outcome::Blocked(reason) => {
                self.last_result_summary = Some(reason.clone());
                self.recent_steps.push(format!(
                    "step {}: {} -> {}",
                    self.step_index,
                    action_label(action),
                    reason
                ));
                trim_recent_steps(&mut self.recent_steps);
                self.recent_action_summaries.push(RecentActionSummary {
                    step: self.step_index,
                    action: action_label(action),
                    outcome: reason.clone(),
                    artifact_refs: Vec::new(),
                });
                trim_recent_action_summaries(&mut self.recent_action_summaries);
                self.avoid_rules.push(AvoidRule {
                    label: action_label(action),
                    reason: reason.clone(),
                });
                trim_avoid_rules(&mut self.avoid_rules);
                None
            }
        };
        Ok(StepProgress {
            repeated_without_progress,
        })
    }

    pub(crate) fn record_retry_feedback(&mut self, failed_action_label: String, reason: String) {
        if let Some(existing) = self
            .avoid_rules
            .iter_mut()
            .find(|rule| rule.label == failed_action_label)
        {
            existing.reason = reason;
            return;
        }

        self.avoid_rules.push(AvoidRule {
            label: failed_action_label,
            reason,
        });
        trim_avoid_rules(&mut self.avoid_rules);
    }

    pub(crate) fn avoid_reason_for_action(&self, action: &Action) -> Option<&str> {
        let label = action_label(action);
        self.avoid_rules
            .iter()
            .rev()
            .find(|rule| rule.label == label)
            .map(|rule| rule.reason.as_str())
    }

    pub(crate) fn apply_live_compaction(&mut self) -> Option<CompactionDecision> {
        let mut reasons = Vec::new();

        if self.step_index >= 3 && self.recent_steps.len() > 3 {
            reasons.push("step threshold".to_string());
        }
        if self
            .last_result_json
            .as_ref()
            .map(|value| value.len() > 1400)
            .unwrap_or(false)
        {
            reasons.push("large tool result".to_string());
        }
        if self.working_sources.len() > 6 {
            reasons.push("source set growth".to_string());
        }

        if reasons.is_empty() {
            return None;
        }

        let reason = reasons.join(", ");
        let score_explanations = build_compaction_score_explanations(self);
        self.compaction_count += 1;
        self.last_compaction_reason = Some(reason.clone());
        self.last_compaction_snapshot = Some(CompactionSnapshot {
            reason: reason.clone(),
            score_explanations: score_explanations.clone(),
        });

        if let Some(last_result) = self.last_result_json.as_ref() {
            self.last_result_json = compact_last_result_for_compacted_context(last_result).ok();
        }

        trim_recent_steps_for_compacted_state(&mut self.recent_steps);
        trim_recent_action_summaries_for_compacted_state(&mut self.recent_action_summaries);
        trim_working_sources_for_compacted_state(&mut self.working_sources);
        trim_artifact_references_for_compacted_state(&mut self.artifact_references);

        Some(CompactionDecision {
            reason,
            score_explanations,
        })
    }
}

#[derive(Default)]
pub(crate) struct StepProgress {
    pub(crate) repeated_without_progress: bool,
}

pub(crate) struct CompactionDecision {
    pub(crate) reason: String,
    pub(crate) score_explanations: Vec<CompactionScoreExplanation>,
}

fn trim_recent_steps(recent_steps: &mut Vec<String>) {
    const MAX_RECENT_STEPS: usize = 6;
    if recent_steps.len() > MAX_RECENT_STEPS {
        let excess = recent_steps.len() - MAX_RECENT_STEPS;
        recent_steps.drain(0..excess);
    }
}

fn trim_recent_action_summaries(recent_actions: &mut Vec<RecentActionSummary>) {
    const MAX_RECENT_ACTION_SUMMARIES: usize = 6;
    if recent_actions.len() > MAX_RECENT_ACTION_SUMMARIES {
        let excess = recent_actions.len() - MAX_RECENT_ACTION_SUMMARIES;
        recent_actions.drain(0..excess);
    }
}

fn trim_avoid_rules(avoid_rules: &mut Vec<AvoidRule>) {
    const MAX_AVOID_RULES: usize = 6;
    if avoid_rules.len() > MAX_AVOID_RULES {
        let excess = avoid_rules.len() - MAX_AVOID_RULES;
        avoid_rules.drain(0..excess);
    }
}

fn merge_artifact_references(
    existing: &mut Vec<ArtifactReference>,
    candidates: Vec<ArtifactReference>,
) {
    const MAX_ARTIFACT_REFERENCES: usize = 12;

    for candidate in candidates {
        if let Some(position) = existing
            .iter()
            .position(|item| item.locator == candidate.locator && item.kind == candidate.kind)
        {
            existing[position] = candidate;
        } else {
            existing.push(candidate);
        }
    }

    if existing.len() > MAX_ARTIFACT_REFERENCES {
        let excess = existing.len() - MAX_ARTIFACT_REFERENCES;
        existing.drain(0..excess);
    }
}

fn merge_working_sources(existing: &mut Vec<WorkingSource>, candidates: Vec<WorkingSource>) {
    const MAX_WORKING_SOURCES: usize = 12;

    for candidate in candidates {
        if let Some(position) = existing
            .iter()
            .position(|item| item.locator == candidate.locator && item.kind == candidate.kind)
        {
            existing[position] = candidate;
        } else {
            existing.push(candidate);
        }
    }

    if existing.len() > MAX_WORKING_SOURCES {
        let excess = existing.len() - MAX_WORKING_SOURCES;
        existing.drain(0..excess);
    }
}

fn trim_recent_steps_for_compacted_state(recent_steps: &mut Vec<String>) {
    const MAX_COMPACTED_RECENT_STEPS: usize = 3;
    if recent_steps.len() > MAX_COMPACTED_RECENT_STEPS {
        let excess = recent_steps.len() - MAX_COMPACTED_RECENT_STEPS;
        recent_steps.drain(0..excess);
    }
}

fn trim_recent_action_summaries_for_compacted_state(recent_actions: &mut Vec<RecentActionSummary>) {
    const MAX_COMPACTED_RECENT_ACTIONS: usize = 3;
    if recent_actions.len() > MAX_COMPACTED_RECENT_ACTIONS {
        let excess = recent_actions.len() - MAX_COMPACTED_RECENT_ACTIONS;
        recent_actions.drain(0..excess);
    }
}

fn trim_working_sources_for_compacted_state(working_sources: &mut Vec<WorkingSource>) {
    const MAX_COMPACTED_WORKING_SOURCES: usize = 6;
    *working_sources = prioritized_working_sources(working_sources)
        .into_iter()
        .take(MAX_COMPACTED_WORKING_SOURCES)
        .collect();
}

fn trim_artifact_references_for_compacted_state(artifact_refs: &mut Vec<ArtifactReference>) {
    const MAX_COMPACTED_ARTIFACT_REFS: usize = 8;
    *artifact_refs = prioritized_artifact_references(artifact_refs)
        .into_iter()
        .take(MAX_COMPACTED_ARTIFACT_REFS)
        .collect();
}

fn build_compaction_score_explanations(state: &TaskLoopState) -> Vec<CompactionScoreExplanation> {
    let mut explanations = Vec::new();

    for source in &state.working_sources {
        let decision = if source.role == "authoritative" || source.role == "generated" {
            "keep"
        } else if source.status == "matched" || source.status == "listed" {
            "compact"
        } else {
            "keep"
        };
        let rationale = if source.role == "authoritative" {
            "high state dependency and recovery value".to_string()
        } else if source.role == "generated" {
            "captures produced artifact and recovery anchor".to_string()
        } else if source.status == "matched" || source.status == "listed" {
            "useful candidate context but lower forward utility than authoritative sources"
                .to_string()
        } else {
            "still relevant to current frontier".to_string()
        };
        explanations.push(CompactionScoreExplanation {
            item_kind: "source".to_string(),
            locator: source.locator.clone(),
            decision: decision.to_string(),
            rationale,
        });
    }

    for artifact in &state.artifact_references {
        explanations.push(CompactionScoreExplanation {
            item_kind: "artifact".to_string(),
            locator: artifact.locator.clone(),
            decision: "keep_ref".to_string(),
            rationale: "exact evidence reference preserved for recovery and re-open".to_string(),
        });
    }

    for avoid in &state.avoid_rules {
        explanations.push(CompactionScoreExplanation {
            item_kind: "avoid".to_string(),
            locator: avoid.label.clone(),
            decision: "keep".to_string(),
            rationale: "failed path preserved to avoid repeating harmful work".to_string(),
        });
    }

    explanations
}

pub(crate) fn action_label(action: &Action) -> String {
    match action {
        Action::RunCommand { command, .. } => format!("run_command:{command}"),
        Action::InspectPath { path, .. } => format!("inspect_path:{}", path.display()),
        Action::InspectWorkingDirectory { .. } => "inspect_working_directory".to_string(),
        Action::ListDirectory {
            path, recursive, ..
        } => {
            format!("list_directory:{}:recursive={recursive}", path.display())
        }
        Action::FindFiles { root, pattern, .. } => {
            format!("find_files:{}:{pattern}", root.display())
        }
        Action::SearchText { root, query, .. } => format!("search_text:{}:{query}", root.display()),
        Action::ReadFile { path, .. } => format!("read_file:{}", path.display()),
        Action::IngestStructuredData { path, .. } => {
            format!("ingest_structured_data:{}", path.display())
        }
        Action::ExtractDocumentText {
            path,
            page_start,
            page_end,
            ..
        } => {
            let page_suffix = match (page_start, page_end) {
                (Some(start), Some(end)) => format!(":pages={start}-{end}"),
                (Some(start), None) => format!(":pages={start}-{start}"),
                (None, Some(end)) => format!(":pages={end}-{end}"),
                (None, None) => String::new(),
            };
            format!("extract_document_text:{}{}", path.display(), page_suffix)
        }
        Action::WriteFile { path, .. } => format!("write_file:{}", path.display()),
        Action::AppendFile { path, .. } => format!("append_file:{}", path.display()),
        Action::RecordNote { note, .. } => format!("record_note:{note}"),
        Action::Respond { message, .. } => format!("respond:{message}"),
    }
}
