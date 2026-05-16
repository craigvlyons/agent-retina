use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskState {
    pub goal: TaskGoal,
    pub progress: TaskProgress,
    pub transcript: TranscriptLedger,
    pub stored_results: StoredResultLedger,
    pub recent_actions: Vec<RecentActionSummary>,
    pub working_sources: Vec<WorkingSource>,
    pub artifact_references: Vec<ArtifactReference>,
    pub next_step_guidance: Option<NextStepGuidance>,
    pub compaction: Option<CompactionSnapshot>,
    pub compaction_history: Vec<CompactionSnapshot>,
    pub compacted_results: Vec<CompactedResultReference>,
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
        let transcript_units =
            render_tail_items(self.transcript.entries(), 6, TranscriptUnit::render);
        let stored_result_refs = render_tail_items(
            self.stored_results.entries(),
            6,
            StoredResultReference::render,
        );
        let recent_actions =
            render_tail_items(&self.recent_actions, 4, RecentActionSummary::render);
        let working_sources = render_tail_items(&self.working_sources, 4, WorkingSource::render);
        let artifacts = render_tail_items(&self.artifact_references, 4, ArtifactReference::render);
        let next_step_guidance = self
            .next_step_guidance
            .as_ref()
            .map(NextStepGuidance::render)
            .unwrap_or_else(|| "none".to_string());
        let compaction = self
            .compaction
            .as_ref()
            .map(CompactionSnapshot::render)
            .unwrap_or_else(|| "none".to_string());
        let compaction_history =
            render_tail_items(&self.compaction_history, 3, CompactionSnapshot::render);
        let compacted_results =
            render_tail_items(&self.compacted_results, 4, CompactedResultReference::render);

        format!(
            "Goal:\n- objective: {}\n- constraints:\n{}\n\nProgress:\n- phase: {}\n- step: {} / {}\n- recent_completed:\n{}\n- verified_facts:\n{}\n- output_written: {}\n- output_verified: {}\n\nTranscript units:\n{}\n\nStored result refs:\n{}\n\nRecent meaningful actions:\n{}\n\nWorking sources:\n{}\n\nArtifact references:\n{}\n\nNext step guidance:\n{}\n\nCompaction:\n{}\n\nRecent compaction boundaries:\n{}\n\nCompacted result refs:\n{}",
            self.goal.objective,
            constraints,
            self.progress.current_phase,
            self.progress.current_step,
            self.progress.max_steps,
            completed,
            verified,
            self.progress.output_written,
            self.progress.output_verified,
            transcript_units,
            stored_result_refs,
            recent_actions,
            working_sources,
            artifacts,
            next_step_guidance,
            compaction,
            compaction_history,
            compacted_results
        )
    }

    pub fn render_compact_summary(&self) -> String {
        let verified = render_compact_list(&self.progress.verified_facts, 4);
        let next_step_guidance = self
            .next_step_guidance
            .as_ref()
            .map(NextStepGuidance::render)
            .unwrap_or_else(|| "none".to_string());
        format!(
            "- objective: {}\n- phase: {}\n- step: {} / {}\n- output_written: {}\n- output_verified: {}\n- verified_facts:\n{}\n- next_step_guidance:\n{}",
            self.goal.objective,
            self.progress.current_phase,
            self.progress.current_step,
            self.progress.max_steps,
            self.progress.output_written,
            self.progress.output_verified,
            verified,
            next_step_guidance
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptUnitKind {
    TaskMessage,
    ReflexDecision,
    CircuitBreakerState,
    OperatorGuidance,
    GuidanceUpdate,
    ModelDecision,
    ToolInvocation,
    ToolResult,
    CompactBoundary,
    MicroCompactBoundary,
    CompactedResultRef,
    RestoredContinuation,
    FinalResponse,
    TerminalBlocked,
    TerminalFailure,
}

impl Display for TranscriptUnitKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::TaskMessage => "task_message",
            Self::ReflexDecision => "reflex_decision",
            Self::CircuitBreakerState => "circuit_breaker_state",
            Self::OperatorGuidance => "operator_guidance",
            Self::GuidanceUpdate => "guidance_update",
            Self::ModelDecision => "model_decision",
            Self::ToolInvocation => "tool_invocation",
            Self::ToolResult => "tool_result",
            Self::CompactBoundary => "compact_boundary",
            Self::MicroCompactBoundary => "microcompact_boundary",
            Self::CompactedResultRef => "compacted_result_ref",
            Self::RestoredContinuation => "restored_continuation",
            Self::FinalResponse => "final_response",
            Self::TerminalBlocked => "terminal_blocked",
            Self::TerminalFailure => "terminal_failure",
        };
        f.write_str(label)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptUnit {
    pub ordinal: usize,
    pub step: usize,
    pub kind: TranscriptUnitKind,
    pub summary: String,
    pub result_ref_id: Option<String>,
    pub primary_locator: Option<String>,
    pub evidence_refs: Vec<String>,
    pub working_sources: Vec<WorkingSource>,
    pub artifact_references: Vec<ArtifactReference>,
    pub next_step_guidance: Option<NextStepGuidance>,
    #[serde(default)]
    pub repetition_signature: Option<String>,
    #[serde(default)]
    pub avoid_label: Option<String>,
    #[serde(default)]
    pub compaction_snapshot: Option<CompactionSnapshot>,
}

impl TranscriptUnit {
    pub fn render(&self) -> String {
        let result_ref = self
            .result_ref_id
            .as_ref()
            .map(|value| format!(" result_ref={value}"))
            .unwrap_or_default();
        let locator = self
            .primary_locator
            .as_ref()
            .map(|value| format!(" locator={value}"))
            .unwrap_or_default();
        let refs = if self.evidence_refs.is_empty() {
            String::new()
        } else {
            format!(" refs={}", self.evidence_refs.join(", "))
        };
        format!(
            "- #{} step {} [{}] {}{}{}{}",
            self.ordinal, self.step, self.kind, self.summary, result_ref, locator, refs
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptLedger {
    pub entries: Vec<TranscriptUnit>,
}

impl TranscriptLedger {
    pub fn from_entries(entries: Vec<TranscriptUnit>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[TranscriptUnit] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn next_ordinal(&self) -> usize {
        self.entries
            .last()
            .map(|item| item.ordinal + 1)
            .unwrap_or(1)
    }

    pub fn append(
        &mut self,
        step: usize,
        kind: TranscriptUnitKind,
        summary: String,
        result_ref_id: Option<String>,
        primary_locator: Option<String>,
        evidence_refs: Vec<String>,
    ) {
        self.append_rich(
            step,
            kind,
            summary,
            result_ref_id,
            primary_locator,
            evidence_refs,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
        );
    }

    pub fn append_rich(
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
        const MAX_TRANSCRIPT_ENTRIES: usize = 32;
        let ordinal = self.next_ordinal();
        self.entries.push(TranscriptUnit {
            ordinal,
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
        });
        if self.entries.len() > MAX_TRANSCRIPT_ENTRIES {
            let excess = self.entries.len() - MAX_TRANSCRIPT_ENTRIES;
            self.entries.drain(0..excess);
        }
    }

    pub fn tail_from(&self, start: usize) -> Vec<TranscriptUnit> {
        self.entries[start..].to_vec()
    }

    pub fn latest_boundary_start(&self) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|item| matches!(item.kind, TranscriptUnitKind::CompactBoundary))
    }

    pub fn recent_step_summaries(&self, limit: usize) -> Vec<String> {
        self.entries
            .iter()
            .rev()
            .take(limit)
            .map(|item| format!("step {}: {} -> {}", item.step, item.kind, item.summary))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn reduced_working_sources(&self) -> Vec<WorkingSource> {
        const MAX_WORKING_SOURCES: usize = 12;
        let mut reduced = Vec::new();
        for entry in &self.entries {
            for candidate in &entry.working_sources {
                if let Some(position) = reduced.iter().position(|item: &WorkingSource| {
                    item.locator == candidate.locator && item.kind == candidate.kind
                }) {
                    reduced[position] = candidate.clone();
                } else {
                    reduced.push(candidate.clone());
                }
            }
        }
        if reduced.len() > MAX_WORKING_SOURCES {
            let excess = reduced.len() - MAX_WORKING_SOURCES;
            reduced.drain(0..excess);
        }
        reduced
    }

    pub fn reduced_artifact_references(&self) -> Vec<ArtifactReference> {
        const MAX_ARTIFACT_REFERENCES: usize = 12;
        let mut reduced = Vec::new();
        for entry in &self.entries {
            for candidate in &entry.artifact_references {
                if let Some(position) = reduced.iter().position(|item: &ArtifactReference| {
                    item.locator == candidate.locator && item.kind == candidate.kind
                }) {
                    reduced[position] = candidate.clone();
                } else {
                    reduced.push(candidate.clone());
                }
            }
        }
        if reduced.len() > MAX_ARTIFACT_REFERENCES {
            let excess = reduced.len() - MAX_ARTIFACT_REFERENCES;
            reduced.drain(0..excess);
        }
        reduced
    }

    pub fn latest_next_step_guidance(&self) -> Option<NextStepGuidance> {
        self.entries
            .iter()
            .rev()
            .find_map(|entry| entry.next_step_guidance.clone())
    }

    pub fn repetition_count(&self, signature: &str) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.repetition_signature.as_deref() == Some(signature))
            .count()
    }

    pub fn terminal_failure_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.kind, TranscriptUnitKind::TerminalFailure))
            .count()
    }

    pub fn compaction_snapshots(&self) -> Vec<CompactionSnapshot> {
        self.entries
            .iter()
            .filter_map(|entry| entry.compaction_snapshot.clone())
            .collect()
    }

    pub fn latest_compaction_snapshot(&self) -> Option<CompactionSnapshot> {
        self.entries
            .iter()
            .rev()
            .find_map(|entry| entry.compaction_snapshot.clone())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredResultReference {
    pub result_id: String,
    pub source_transcript_ordinal: usize,
    pub step: usize,
    pub result_type: String,
    pub primary_locator: Option<String>,
    pub preview_excerpt: String,
    pub persisted_path: String,
}

impl StoredResultReference {
    pub fn render(&self) -> String {
        let locator = self
            .primary_locator
            .as_ref()
            .map(|value| format!(" locator={value}"))
            .unwrap_or_default();
        format!(
            "- {} source=#{} step={} [{}]{} preview=\"{}\" persisted={}",
            self.result_id,
            self.source_transcript_ordinal,
            self.step,
            self.result_type,
            locator,
            self.preview_excerpt,
            self.persisted_path
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredResultLedger {
    pub entries: Vec<StoredResultReference>,
}

impl StoredResultLedger {
    pub fn from_entries(entries: Vec<StoredResultReference>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[StoredResultReference] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn append(&mut self, reference: StoredResultReference) {
        const MAX_STORED_RESULT_REFS: usize = 24;
        self.entries.push(reference);
        if self.entries.len() > MAX_STORED_RESULT_REFS {
            let excess = self.entries.len() - MAX_STORED_RESULT_REFS;
            self.entries.drain(0..excess);
        }
    }

    pub fn filter_by_result_ids(
        &self,
        result_ids: &std::collections::BTreeSet<String>,
    ) -> Vec<StoredResultReference> {
        self.entries
            .iter()
            .filter(|item| result_ids.contains(&item.result_id))
            .cloned()
            .collect()
    }

    pub fn supplemental_recent(
        &self,
        excluded_result_ids: &std::collections::BTreeSet<String>,
        limit: usize,
    ) -> Vec<StoredResultReference> {
        self.entries
            .iter()
            .rev()
            .filter(|item| !excluded_result_ids.contains(&item.result_id))
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NextStepDirective {
    AnswerFromEvidence,
    ReformulateSearch,
    GatherMissingFact,
    ReportLimitation,
}

impl Display for NextStepDirective {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::AnswerFromEvidence => "answer_from_evidence",
            Self::ReformulateSearch => "reformulate_search",
            Self::GatherMissingFact => "gather_missing_fact",
            Self::ReportLimitation => "report_limitation",
        };
        f.write_str(label)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchToolFamily {
    Web,
    Local,
    News,
}

impl Display for SearchToolFamily {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Web => "web_search",
            Self::Local => "local_search",
            Self::News => "news_search",
        };
        f.write_str(label)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NextStepGuidance {
    pub directive: NextStepDirective,
    pub reason: String,
    pub based_on_action: Option<String>,
    pub evidence_locator: Option<String>,
    pub preferred_search_family: Option<SearchToolFamily>,
    pub suggested_query: Option<String>,
}

impl NextStepGuidance {
    pub fn render(&self) -> String {
        let action = self
            .based_on_action
            .as_ref()
            .map(|value| format!("\n- based_on_action: {}", value))
            .unwrap_or_default();
        let evidence = self
            .evidence_locator
            .as_ref()
            .map(|value| format!("\n- evidence_locator: {}", value))
            .unwrap_or_default();
        let preferred_search_family = self
            .preferred_search_family
            .as_ref()
            .map(|value| format!("\n- preferred_search_family: {}", value))
            .unwrap_or_default();
        let suggested_query = self
            .suggested_query
            .as_ref()
            .map(|value| format!("\n- suggested_query: {}", value))
            .unwrap_or_default();
        format!(
            "- directive: {}\n- reason: {}{}{}{}{}",
            self.directive, self.reason, action, evidence, preferred_search_family, suggested_query
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecentActionStatus {
    Succeeded,
    Failed,
    Blocked,
    Responded,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentActionSummary {
    pub step: usize,
    pub action: String,
    pub status: RecentActionStatus,
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
pub struct CompactionSnapshot {
    pub boundary_id: usize,
    pub compacted_at_step: usize,
    pub reason: String,
    pub score_explanations: Vec<CompactionScoreExplanation>,
    pub preserved_locators: Vec<String>,
    pub active_window_summary: String,
    pub last_result_continuation: Option<String>,
    pub compacted_results: Vec<CompactedResultReference>,
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
        let preserved = if self.preserved_locators.is_empty() {
            "  - none".to_string()
        } else {
            self.preserved_locators
                .iter()
                .map(|item| format!("  - {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let continuation = self
            .last_result_continuation
            .as_ref()
            .map(|value| format!("\n- continuation: {}", value))
            .unwrap_or_default();
        let compacted_results = if self.compacted_results.is_empty() {
            "\n- compacted_results:\n  - none".to_string()
        } else {
            format!(
                "\n- compacted_results:\n{}",
                self.compacted_results
                    .iter()
                    .map(|item| format!("  {}", item.render()))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        format!(
            "- boundary_id: {}\n- compacted_at_step: {}\n- reason: {}\n- active_window: {}{}{}\n- preserved_locators:\n{}\n- ranking:\n{}",
            self.boundary_id,
            self.compacted_at_step,
            self.reason,
            self.active_window_summary,
            continuation,
            compacted_results,
            preserved,
            scores
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactedResultReference {
    pub boundary_id: usize,
    pub result_type: String,
    pub locator: Option<String>,
    pub preview_excerpt: String,
    pub continuation: Option<String>,
    pub persisted_path: Option<String>,
}

impl CompactedResultReference {
    pub fn render(&self) -> String {
        let locator = self
            .locator
            .as_ref()
            .map(|value| format!(" locator={value}"))
            .unwrap_or_default();
        let continuation = self
            .continuation
            .as_ref()
            .map(|value| format!(" continuation={value}"))
            .unwrap_or_default();
        let persisted_path = self
            .persisted_path
            .as_ref()
            .map(|value| format!(" persisted={value}"))
            .unwrap_or_default();
        format!(
            "- boundary={} [{}]{} preview=\"{}\"{}{}",
            self.boundary_id,
            self.result_type,
            locator,
            self.preview_excerpt,
            continuation,
            persisted_path
        )
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
