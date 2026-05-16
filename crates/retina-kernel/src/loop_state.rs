use crate::result_helpers::{
    artifact_references_for_result, compact_action_result_for_context,
    compact_last_result_for_compacted_context, derive_next_step_guidance,
    prioritized_artifact_references, prioritized_working_sources, repeated_step_limit,
    repeated_step_signature, summarize_action_result, working_sources_for_result,
};
use retina_types::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct TaskLoopState {
    pub(crate) transcript: TranscriptLedger,
    pub(crate) stored_results: StoredResultLedger,
}

impl TaskLoopState {
    pub(crate) fn new(_max_steps: usize) -> Self {
        Self {
            transcript: TranscriptLedger::default(),
            stored_results: StoredResultLedger::default(),
        }
    }

    pub(crate) fn from_resume_context(context: &TaskResumeContext) -> Self {
        let mut state = Self::new(context.continuation_window.max_steps);
        state.transcript = context.continuation_window.transcript.clone();
        state.stored_results = context.continuation_window.stored_results.clone();
        state.push_transcript_unit(
            context.continuation_window.current_step,
            TranscriptUnitKind::RestoredContinuation,
            format!(
                "resumed from task {}: {}",
                context.source_task_id, context.resume_reason
            ),
            None,
            None,
            context
                .continuation_window
                .reannounced_sources
                .iter()
                .map(|item| item.locator.clone())
                .take(4)
                .collect(),
            context.continuation_window.reannounced_sources.clone(),
            context.continuation_window.reannounced_artifacts.clone(),
            context.continuation_window.next_step_guidance.clone(),
            None,
            None,
            None,
        );
        state
    }

    pub(crate) fn record_task_message(&mut self, objective: &str) {
        self.push_transcript_unit(
            0,
            TranscriptUnitKind::TaskMessage,
            objective.to_string(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        );
    }

    pub(crate) fn record_reflex_decision(&mut self, step: usize, action: Option<&Action>) {
        let summary = action
            .map(|item| format!("matched {}", action_label(item)))
            .unwrap_or_else(|| "no reflex match".to_string());
        self.push_transcript_unit(
            step,
            TranscriptUnitKind::ReflexDecision,
            summary,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        );
    }

    pub(crate) fn record_circuit_breaker_state(
        &mut self,
        step: usize,
        failure_count: usize,
        tripped: bool,
    ) {
        self.push_transcript_unit(
            step,
            TranscriptUnitKind::CircuitBreakerState,
            format!("failures={} tripped={}", failure_count, tripped),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        );
    }

    pub(crate) fn record_operator_guidance(&mut self, step: usize, guidance: &str) {
        self.push_transcript_unit(
            step,
            TranscriptUnitKind::OperatorGuidance,
            guidance.to_string(),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        );
    }

    pub(crate) fn record_guidance_update(&mut self, step: usize, guidance: NextStepGuidance) {
        self.push_transcript_unit(
            step,
            TranscriptUnitKind::GuidanceUpdate,
            guidance.reason.clone(),
            None,
            guidance.evidence_locator.clone(),
            guidance
                .evidence_locator
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            Vec::new(),
            Vec::new(),
            Some(guidance),
            None,
            None,
            None,
        );
    }

    pub(crate) fn record_model_decision(
        &mut self,
        step: usize,
        action: &Action,
        reasoning: Option<&str>,
        task_complete: bool,
    ) {
        let completion = if task_complete {
            " task_complete=true"
        } else {
            ""
        };
        let summary = reasoning
            .map(|value| format!("{}{} | {}", action_label(action), completion, value))
            .unwrap_or_else(|| format!("{}{}", action_label(action), completion));
        self.push_transcript_unit(
            step,
            TranscriptUnitKind::ModelDecision,
            summary,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        );
    }

    pub(crate) fn record_step(
        &mut self,
        task_id: &TaskId,
        action: &Action,
        outcome: &Outcome,
    ) -> Result<StepProgress> {
        let step_index = self.current_step() + 1;
        let mut repeated_without_progress = false;
        self.push_transcript_unit(
            step_index,
            TranscriptUnitKind::ToolInvocation,
            action_label(action),
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        );
        match outcome {
            Outcome::Success(result) if !matches!(action, Action::Respond { .. }) => {
                let summary = summarize_action_result(result);
                let artifact_refs = artifact_references_for_result(result);
                let transcript_locator = artifact_refs.first().map(|item| item.locator.clone());
                let transcript_refs = artifact_refs
                    .iter()
                    .map(|item| item.locator.clone())
                    .take(4)
                    .collect::<Vec<_>>();
                let compacted_result = compact_action_result_for_context(result)
                    .map_err(|error| KernelError::Reasoning(error.to_string()))?;
                let working_sources = working_sources_for_result(action, result, step_index + 1);
                let repetition_signature = repeated_step_signature(action, result);
                let guidance = if let Some(signature) = repetition_signature.as_deref() {
                    let count = self
                        .active_projection_transcript()
                        .repetition_count(signature)
                        + 1;
                    repeated_without_progress = count > repeated_step_limit(action, result);
                    derive_next_step_guidance(action, result, count)
                } else {
                    derive_next_step_guidance(action, result, 1)
                };
                let stored_result_ref = self.persist_result_reference(
                    task_id,
                    step_index,
                    result,
                    &compacted_result,
                    transcript_locator.as_deref(),
                    self.transcript.next_ordinal(),
                )?;
                self.push_transcript_unit(
                    step_index,
                    TranscriptUnitKind::ToolResult,
                    summary.clone(),
                    stored_result_ref
                        .as_ref()
                        .map(|item| item.result_id.clone()),
                    transcript_locator,
                    transcript_refs,
                    working_sources,
                    artifact_refs,
                    guidance,
                    repetition_signature,
                    None,
                    None,
                );
                Some(compacted_result)
            }
            Outcome::Success(_) => {
                if let Action::Respond { message, .. } = action {
                    self.push_transcript_unit(
                        step_index,
                        TranscriptUnitKind::FinalResponse,
                        preview_transcript_text(message, 240),
                        None,
                        None,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        None,
                        None,
                        None,
                        None,
                    );
                } else {
                    self.push_transcript_unit(
                        step_index,
                        TranscriptUnitKind::ToolResult,
                        "responded to operator".to_string(),
                        None,
                        None,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        None,
                        None,
                        None,
                        None,
                    );
                }
                None
            }
            Outcome::Failure(reason) | Outcome::Blocked(reason) => {
                let guidance = Some(NextStepGuidance {
                    directive: NextStepDirective::ReportLimitation,
                    reason: reason.clone(),
                    based_on_action: Some(action_label(action)),
                    evidence_locator: None,
                    preferred_search_family: None,
                    suggested_query: None,
                });
                self.push_transcript_unit(
                    step_index,
                    match outcome {
                        Outcome::Failure(_) => TranscriptUnitKind::TerminalFailure,
                        Outcome::Blocked(_) => TranscriptUnitKind::TerminalBlocked,
                        Outcome::Success(_) => TranscriptUnitKind::ToolResult,
                    },
                    reason.clone(),
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    guidance,
                    None,
                    Some(action_label(action)),
                    None,
                );
                None
            }
        };
        Ok(StepProgress {
            repeated_without_progress,
        })
    }

    pub(crate) fn apply_live_compaction(&mut self, task_id: &TaskId) -> Option<CompactionDecision> {
        let step_index = self.current_step();
        let mut reasons = Vec::new();

        if step_index >= 3 && retained_transcript_step_count(&self.transcript) > 3 {
            reasons.push("step threshold".to_string());
        }
        let latest_result_json = self.latest_stored_result_json();
        if latest_result_json
            .as_ref()
            .map(|value| value.len() > 1400)
            .unwrap_or(false)
        {
            reasons.push("large tool result".to_string());
        }
        if self.working_sources().len() > 6 {
            reasons.push("source set growth".to_string());
        }

        if reasons.is_empty() {
            return None;
        }

        let reason = reasons.join(", ");
        let score_explanations = build_compaction_score_explanations(self);
        let boundary_id = self
            .compaction_history()
            .last()
            .map(|snapshot| snapshot.boundary_id + 1)
            .unwrap_or(1);
        let preserved_locators = collect_preserved_locators(self);
        let compacted_last_result = latest_result_json
            .as_deref()
            .and_then(|last_result| compact_last_result_for_compacted_context(last_result).ok());

        let working_sources = trim_working_sources_for_compacted_state(&self.working_sources());
        let artifact_references =
            trim_artifact_references_for_compacted_state(&self.artifact_references());

        let persisted_result_path = compacted_last_result
            .as_deref()
            .and_then(|value| persist_compacted_result(task_id, boundary_id, value).ok())
            .map(|path| path.display().to_string());
        let compacted_results = compacted_last_result
            .as_deref()
            .and_then(|value| {
                compacted_result_reference_from_json(
                    boundary_id,
                    value,
                    persisted_result_path.clone(),
                )
            })
            .into_iter()
            .collect::<Vec<_>>();
        let snapshot = CompactionSnapshot {
            boundary_id,
            compacted_at_step: step_index,
            reason: reason.clone(),
            score_explanations: score_explanations.clone(),
            preserved_locators: preserved_locators.clone(),
            active_window_summary: summarize_active_window(&self.transcript),
            last_result_continuation: compacted_last_result
                .as_deref()
                .and_then(extract_compaction_continuation),
            compacted_results,
        };
        let existing_boundary_count = self.compaction_history().len();
        self.push_transcript_unit(
            step_index,
            TranscriptUnitKind::CompactBoundary,
            reason.clone(),
            None,
            None,
            preserved_locators.iter().take(4).cloned().collect(),
            working_sources,
            artifact_references,
            self.next_step_guidance(),
            None,
            None,
            Some(snapshot),
        );
        self.trim_compaction_history(existing_boundary_count + 1);
        if let Some(reference) = self
            .latest_compaction_snapshot()
            .and_then(|snapshot| snapshot.compacted_results.last().cloned())
        {
            self.push_transcript_unit(
                step_index,
                TranscriptUnitKind::CompactedResultRef,
                format!(
                    "boundary {} [{}] {}",
                    reference.boundary_id, reference.result_type, reference.preview_excerpt
                ),
                None,
                reference.locator.clone(),
                reference
                    .locator
                    .iter()
                    .cloned()
                    .chain(reference.persisted_path.iter().cloned())
                    .take(4)
                    .collect(),
                Vec::new(),
                Vec::new(),
                None,
                None,
                None,
                None,
            );
        }
        if compacted_last_result.is_some() {
            self.push_transcript_unit(
                step_index,
                TranscriptUnitKind::MicroCompactBoundary,
                format!(
                    "microcompacted surviving result context for boundary {}",
                    boundary_id
                ),
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
                None,
                None,
                None,
            );
        }
        if let Some(continuation) = self
            .latest_compaction_snapshot()
            .and_then(|snapshot| snapshot.last_result_continuation.clone())
        {
            self.push_transcript_unit(
                step_index,
                TranscriptUnitKind::RestoredContinuation,
                continuation,
                None,
                None,
                preserved_locators.into_iter().take(4).collect(),
                Vec::new(),
                Vec::new(),
                self.next_step_guidance(),
                None,
                None,
                None,
            );
        }

        Some(CompactionDecision {
            reason,
            score_explanations,
        })
    }
}

#[cfg(test)]
impl TaskLoopState {
    pub(crate) fn seed_working_source(&mut self, source: WorkingSource) {
        let step = source.last_used_step.max(1);
        self.push_transcript_unit(
            step,
            TranscriptUnitKind::ToolResult,
            format!("seed source {}", source.locator),
            None,
            Some(source.locator.clone()),
            source.evidence_refs.clone(),
            vec![source],
            Vec::new(),
            None,
            None,
            None,
            None,
        );
    }

    pub(crate) fn seed_artifact_reference(&mut self, artifact: ArtifactReference) {
        let step = self.current_step().max(1);
        self.push_transcript_unit(
            step,
            TranscriptUnitKind::ToolResult,
            format!("seed artifact {}", artifact.locator),
            None,
            Some(artifact.locator.clone()),
            vec![artifact.locator.clone()],
            Vec::new(),
            vec![artifact],
            None,
            None,
            None,
            None,
        );
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

fn preview_transcript_text(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(max_chars).collect::<String>();
    if normalized.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

impl TaskLoopState {
    pub(crate) fn current_step(&self) -> usize {
        self.transcript
            .entries()
            .iter()
            .filter(|item| {
                matches!(
                    item.kind,
                    TranscriptUnitKind::ToolInvocation
                        | TranscriptUnitKind::ToolResult
                        | TranscriptUnitKind::CompactBoundary
                        | TranscriptUnitKind::MicroCompactBoundary
                        | TranscriptUnitKind::CompactedResultRef
                        | TranscriptUnitKind::RestoredContinuation
                        | TranscriptUnitKind::FinalResponse
                        | TranscriptUnitKind::TerminalBlocked
                        | TranscriptUnitKind::TerminalFailure
                )
            })
            .map(|item| item.step)
            .max()
            .unwrap_or(0)
    }

    fn push_transcript_unit(
        &mut self,
        step: usize,
        kind: TranscriptUnitKind,
        summary: String,
        result_ref_id: Option<String>,
        primary_locator: Option<String>,
        evidence_refs: Vec<String>,
        working_sources: Vec<WorkingSource>,
        artifact_references: Vec<ArtifactReference>,
        next_step_guidance: Option<NextStepGuidance>,
        repetition_signature: Option<String>,
        avoid_label: Option<String>,
        compaction_snapshot: Option<CompactionSnapshot>,
    ) {
        self.transcript.append_rich(
            step,
            kind,
            summary,
            result_ref_id,
            primary_locator,
            evidence_refs,
            working_sources,
            artifact_references,
            next_step_guidance,
            repetition_signature,
            avoid_label,
            compaction_snapshot,
        );
    }

    pub(crate) fn working_sources(&self) -> Vec<WorkingSource> {
        self.active_projection_transcript()
            .reduced_working_sources()
    }

    pub(crate) fn artifact_references(&self) -> Vec<ArtifactReference> {
        self.active_projection_transcript()
            .reduced_artifact_references()
    }

    pub(crate) fn next_step_guidance(&self) -> Option<NextStepGuidance> {
        self.active_projection_transcript()
            .latest_next_step_guidance()
    }

    pub(crate) fn circuit_breaker_failure_count(&self) -> usize {
        self.active_projection_transcript().terminal_failure_count()
    }

    pub(crate) fn circuit_breaker_tripped(&self) -> bool {
        self.circuit_breaker_failure_count() >= 3
    }

    fn active_projection_transcript(&self) -> TranscriptLedger {
        let start = self.transcript.latest_boundary_start().unwrap_or(0);
        TranscriptLedger::from_entries(self.transcript.tail_from(start))
    }

    fn active_avoid_labels(&self) -> Vec<String> {
        derive_active_avoid_labels(
            &self.active_projection_transcript(),
            &self.artifact_references(),
            &self.working_sources(),
        )
    }

    fn latest_stored_result_json(&self) -> Option<String> {
        self.stored_results
            .entries()
            .iter()
            .rev()
            .find_map(|reference| fs::read_to_string(&reference.persisted_path).ok())
    }

    pub(crate) fn compaction_history(&self) -> Vec<CompactionSnapshot> {
        self.transcript.compaction_snapshots()
    }

    pub(crate) fn latest_compaction_snapshot(&self) -> Option<CompactionSnapshot> {
        self.transcript.latest_compaction_snapshot()
    }

    fn trim_compaction_history(&mut self, total_boundaries: usize) {
        const MAX_COMPACTION_HISTORY: usize = 3;
        if total_boundaries <= MAX_COMPACTION_HISTORY {
            return;
        }
        let excess = total_boundaries - MAX_COMPACTION_HISTORY;
        let mut remaining = excess;
        self.transcript.entries.retain(|entry| {
            if remaining == 0 {
                return true;
            }
            if entry.compaction_snapshot.is_some() {
                remaining -= 1;
                return false;
            }
            true
        });
    }

    fn persist_result_reference(
        &mut self,
        task_id: &TaskId,
        step: usize,
        result: &ActionResult,
        compacted_result: &str,
        primary_locator: Option<&str>,
        source_transcript_ordinal: usize,
    ) -> Result<Option<StoredResultReference>> {
        if matches!(result, ActionResult::Response { .. }) {
            return Ok(None);
        }
        let result_id = format!("result-{step}-{}", self.stored_results.len() + 1);
        let path = persist_tool_result(task_id, &result_id, compacted_result)?;
        let reference = StoredResultReference {
            result_id,
            source_transcript_ordinal,
            step,
            result_type: stored_result_type(result).to_string(),
            primary_locator: primary_locator.map(ToString::to_string),
            preview_excerpt: summarize_action_result(result),
            persisted_path: path.display().to_string(),
        };
        self.stored_results.append(reference.clone());
        Ok(Some(reference))
    }
}

fn derive_active_avoid_labels(
    transcript: &TranscriptLedger,
    artifacts: &[ArtifactReference],
    sources: &[WorkingSource],
) -> Vec<String> {
    let resolved_locators = artifacts
        .iter()
        .filter(|artifact| {
            matches!(
                artifact.status.as_str(),
                "created" | "written" | "overwritten" | "appended" | "command_changed"
            )
        })
        .map(|artifact| artifact.locator.clone())
        .chain(
            sources
                .iter()
                .filter(|source| {
                    matches!(
                        source.status.as_str(),
                        "created" | "written" | "overwritten" | "appended" | "command_changed"
                    )
                })
                .map(|source| source.locator.clone()),
        )
        .collect::<Vec<_>>();

    let mut active = Vec::new();
    for entry in transcript.entries() {
        let Some(label) = entry.avoid_label.as_ref() else {
            continue;
        };
        let is_resolved = avoid_rule_locator(label)
            .map(|blocked_locator| {
                resolved_locators
                    .iter()
                    .any(|resolved| locator_resolves_blocker(resolved, &blocked_locator))
            })
            .unwrap_or(false);
        if is_resolved {
            continue;
        }
        if let Some(position) = active.iter().position(|item| item == label) {
            active.remove(position);
        }
        active.push(label.clone());
    }

    const MAX_AVOID_RULES: usize = 6;
    if active.len() > MAX_AVOID_RULES {
        let excess = active.len() - MAX_AVOID_RULES;
        active.drain(0..excess);
    }
    active
}

fn avoid_rule_locator(label: &str) -> Option<String> {
    let raw = if let Some(value) = label.strip_prefix("list_directory:") {
        value.split(":recursive=").next().unwrap_or(value)
    } else if let Some(value) = label.strip_prefix("inspect_path:") {
        value
    } else if let Some(value) = label.strip_prefix("read_file:") {
        value
    } else if let Some(value) = label.strip_prefix("write_file:") {
        value
    } else if let Some(value) = label.strip_prefix("edit_file:") {
        value
    } else if let Some(value) = label.strip_prefix("append_file:") {
        value
    } else if let Some(value) = label.strip_prefix("edit_notebook:") {
        value
    } else {
        return None;
    };
    Some(raw.to_string())
}

fn locator_resolves_blocker(resolved: &str, blocked: &str) -> bool {
    let resolved_path = Path::new(resolved);
    let blocked_path = Path::new(blocked);
    resolved_path == blocked_path
        || resolved_path.starts_with(blocked_path)
        || blocked_path.starts_with(resolved_path)
}

fn trim_working_sources_for_compacted_state(
    working_sources: &[WorkingSource],
) -> Vec<WorkingSource> {
    const MAX_COMPACTED_WORKING_SOURCES: usize = 6;
    prioritized_working_sources(working_sources)
        .into_iter()
        .take(MAX_COMPACTED_WORKING_SOURCES)
        .collect()
}

fn trim_artifact_references_for_compacted_state(
    artifact_refs: &[ArtifactReference],
) -> Vec<ArtifactReference> {
    const MAX_COMPACTED_ARTIFACT_REFS: usize = 8;
    prioritized_artifact_references(artifact_refs)
        .into_iter()
        .take(MAX_COMPACTED_ARTIFACT_REFS)
        .collect()
}

fn build_compaction_score_explanations(state: &TaskLoopState) -> Vec<CompactionScoreExplanation> {
    let mut explanations = Vec::new();
    let working_sources = state.working_sources();
    let artifact_references = state.artifact_references();

    for source in &working_sources {
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

    for artifact in &artifact_references {
        explanations.push(CompactionScoreExplanation {
            item_kind: "artifact".to_string(),
            locator: artifact.locator.clone(),
            decision: "keep_ref".to_string(),
            rationale: "exact evidence reference preserved for recovery and re-open".to_string(),
        });
    }

    for avoid in state.active_avoid_labels() {
        explanations.push(CompactionScoreExplanation {
            item_kind: "avoid".to_string(),
            locator: avoid,
            decision: "keep".to_string(),
            rationale: "failed path preserved to avoid repeating harmful work".to_string(),
        });
    }

    explanations
}

fn collect_preserved_locators(state: &TaskLoopState) -> Vec<String> {
    let working_sources = state.working_sources();
    let artifact_references = state.artifact_references();
    let mut locators = prioritized_working_sources(&working_sources)
        .into_iter()
        .take(4)
        .map(|source| source.locator)
        .collect::<Vec<_>>();
    for artifact in prioritized_artifact_references(&artifact_references)
        .into_iter()
        .take(4)
    {
        if !locators.contains(&artifact.locator) {
            locators.push(artifact.locator);
        }
    }
    locators
}

fn retained_transcript_step_count(transcript: &TranscriptLedger) -> usize {
    transcript
        .entries()
        .iter()
        .filter(|item| item.step > 0)
        .map(|item| item.step)
        .collect::<BTreeSet<_>>()
        .len()
}

fn summarize_active_window(transcript: &TranscriptLedger) -> String {
    let actions = transcript_recent_action_statuses(transcript, 3)
        .into_iter()
        .map(|(step, action, status)| format!("step {} {} {:?}", step, action, status))
        .collect::<Vec<_>>();
    if actions.is_empty() {
        "no retained actions".to_string()
    } else {
        actions.join(" | ")
    }
}

fn transcript_recent_action_statuses(
    transcript: &TranscriptLedger,
    limit: usize,
) -> Vec<(usize, String, RecentActionStatus)> {
    let entries = transcript.entries();
    let mut derived = Vec::new();

    for (index, item) in entries.iter().enumerate().rev() {
        let Some(status) = transcript_status_for_kind(&item.kind) else {
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
        derived.push((item.step, action, status));
        if derived.len() >= limit {
            break;
        }
    }

    derived.into_iter().rev().collect()
}

fn transcript_status_for_kind(kind: &TranscriptUnitKind) -> Option<RecentActionStatus> {
    match kind {
        TranscriptUnitKind::ToolResult => Some(RecentActionStatus::Succeeded),
        TranscriptUnitKind::FinalResponse => Some(RecentActionStatus::Responded),
        TranscriptUnitKind::TerminalFailure => Some(RecentActionStatus::Failed),
        TranscriptUnitKind::TerminalBlocked => Some(RecentActionStatus::Blocked),
        _ => None,
    }
}

fn extract_compaction_continuation(last_result_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(last_result_json).ok()?;
    value
        .get("continuation")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
}

fn compacted_result_reference_from_json(
    boundary_id: usize,
    last_result_json: &str,
    persisted_path: Option<String>,
) -> Option<CompactedResultReference> {
    let value: serde_json::Value = serde_json::from_str(last_result_json).ok()?;
    let result_type = value.get("type")?.as_str()?.to_string();
    let locator = value
        .get("path")
        .or_else(|| value.get("root"))
        .or_else(|| value.get("uri"))
        .or_else(|| value.get("primary_locator"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    let preview_excerpt = value
        .get("content_preview")
        .or_else(|| value.get("evidence_summary"))
        .or_else(|| value.get("message"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let mut preview = value.chars().take(180).collect::<String>();
            if value.chars().count() > 180 {
                preview.push_str("...");
            }
            preview
        })
        .unwrap_or_else(|| result_type.clone());
    let continuation = value
        .get("continuation")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);
    Some(CompactedResultReference {
        boundary_id,
        result_type,
        locator,
        preview_excerpt,
        continuation,
        persisted_path,
    })
}

fn persist_compacted_result(
    task_id: &TaskId,
    boundary_id: usize,
    compacted_result_json: &str,
) -> std::io::Result<PathBuf> {
    let dir = compacted_results_dir(task_id)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("boundary-{boundary_id}.json"));
    fs::write(&path, compacted_result_json)?;
    Ok(path)
}

fn persist_tool_result(
    task_id: &TaskId,
    result_id: &str,
    compacted_result_json: &str,
) -> std::io::Result<PathBuf> {
    let dir = tool_results_dir(task_id)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{result_id}.json"));
    fs::write(&path, compacted_result_json)?;
    Ok(path)
}

fn compacted_results_dir(task_id: &TaskId) -> std::io::Result<PathBuf> {
    let retina_home = std::env::var("RETINA_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|home| home.join(".retina")))
        .ok_or_else(|| std::io::Error::other("could not determine RETINA_HOME"))?;
    Ok(retina_home
        .join("root")
        .join("runtime")
        .join("tasks")
        .join(task_id.to_string())
        .join("compacted-results"))
}

fn tool_results_dir(task_id: &TaskId) -> std::io::Result<PathBuf> {
    let retina_home = std::env::var("RETINA_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|home| home.join(".retina")))
        .ok_or_else(|| std::io::Error::other("could not determine RETINA_HOME"))?;
    Ok(retina_home
        .join("root")
        .join("runtime")
        .join("tasks")
        .join(task_id.to_string())
        .join("tool-results"))
}

fn stored_result_type(result: &ActionResult) -> &'static str {
    match result {
        ActionResult::Command(_) => "command",
        ActionResult::Inspection(_) => "inspection",
        ActionResult::DirectoryListing { .. } => "directory_listing",
        ActionResult::FileMatches { .. } => "file_matches",
        ActionResult::FileRead { .. } => "file_read",
        ActionResult::StructuredData { .. } => "structured_data",
        ActionResult::DocumentText { .. } => "document_text",
        ActionResult::TextSearch { .. } => "text_search",
        ActionResult::McpResources { .. } => "mcp_resources",
        ActionResult::McpResourceRead(_) => "mcp_resource_read",
        ActionResult::McpToolCall(_) => "mcp_tool_call",
        ActionResult::FileWrite { .. } => "file_write",
        ActionResult::DelegatedTask(_) => "delegated_task",
        ActionResult::NoteRecorded { .. } => "note_recorded",
        ActionResult::Response { .. } => "response",
    }
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
        Action::FindFiles {
            root,
            pattern,
            recursive,
            ..
        } => {
            format!(
                "find_files:{}:{pattern}:recursive={recursive}",
                root.display()
            )
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
        Action::ListMcpResources { server, .. } => format!(
            "list_mcp_resources:{}",
            server.clone().unwrap_or_else(|| "*".to_string())
        ),
        Action::ReadMcpResource { server, uri, .. } => {
            format!("read_mcp_resource:{server}:{uri}")
        }
        Action::CallMcpTool {
            server,
            tool,
            input_json,
            resolved_tool_name,
            ..
        } => {
            let base = resolved_tool_name
                .clone()
                .unwrap_or_else(|| format!("mcp_tool:{server}:{tool}"));
            let query_suffix = input_json
                .get("query")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|query| {
                    let mut preview = query.chars().take(80).collect::<String>();
                    if query.chars().count() > 80 {
                        preview.push_str("...");
                    }
                    format!(":query={preview}")
                })
                .unwrap_or_default();
            format!("{base}{query_suffix}")
        }
        Action::WriteFile { path, .. } => format!("write_file:{}", path.display()),
        Action::EditFile { path, .. } => format!("edit_file:{}", path.display()),
        Action::AppendFile { path, .. } => format!("append_file:{}", path.display()),
        Action::EditNotebook { path, .. } => format!("edit_notebook:{}", path.display()),
        Action::SpawnAgent { prompt, .. } => format!("agent_spawn:{prompt}"),
        Action::RecordNote { note, .. } => format!("record_note:{note}"),
        Action::Respond { message, .. } => format!("respond:{message}"),
    }
}
