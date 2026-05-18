use crate::{
    Action, ActionResult, AgentId, ArtifactReference, CompactedResultReference, CompactionSnapshot,
    NextStepGuidance, RecentActionStatus, RecentActionSummary, StoredResultLedger,
    StoredResultReference, TaskGoal, TaskKind, TaskProgress, TaskState, TextSearchOutputMode,
    TranscriptLedger, TranscriptUnitKind, WorkingSource,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContinuationTransition {
    pub reason: String,
    #[serde(default)]
    pub attempt: Option<u64>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum CachedLineEndingStyle {
    #[default]
    Lf,
    Crlf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CachedFileVersionSnapshot {
    pub exists: bool,
    pub size: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
    pub content_hash: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CachedFileReadState {
    pub path: PathBuf,
    pub content: String,
    pub start_line: usize,
    pub line_count: usize,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub read_bytes: usize,
    pub was_partial: bool,
    pub read_at: Option<DateTime<Utc>>,
    pub version: CachedFileVersionSnapshot,
    pub line_endings: CachedLineEndingStyle,
}

impl CachedFileReadState {
    pub fn from_action_result(result: &ActionResult) -> Option<Self> {
        match result {
            ActionResult::FileRead {
                path,
                content,
                truncated,
                start_line,
                line_count,
                total_lines,
                total_bytes,
                read_bytes,
            } => Some(Self {
                path: path.clone(),
                content: content.clone(),
                start_line: *start_line,
                line_count: *line_count,
                total_lines: *total_lines,
                total_bytes: *total_bytes,
                read_bytes: *read_bytes,
                was_partial: *truncated || *start_line > 1 || *line_count < *total_lines,
                read_at: Some(Utc::now()),
                version: CachedFileVersionSnapshot {
                    exists: true,
                    size: Some(*total_bytes as u64),
                    modified_at: None,
                    content_hash: None,
                },
                line_endings: if content.contains("\r\n") {
                    CachedLineEndingStyle::Crlf
                } else {
                    CachedLineEndingStyle::Lf
                },
            }),
            ActionResult::FileWrite {
                path,
                bytes_written,
                artifact,
                ..
            } => {
                let content = artifact.final_content.clone();
                let line_count = if content.is_empty() {
                    0
                } else {
                    content.lines().count()
                };
                Some(Self {
                    path: path.clone(),
                    content: content.clone(),
                    start_line: 1,
                    line_count,
                    total_lines: line_count,
                    total_bytes: *bytes_written,
                    read_bytes: *bytes_written,
                    was_partial: false,
                    read_at: Some(Utc::now()),
                    version: CachedFileVersionSnapshot {
                        exists: true,
                        size: Some(*bytes_written as u64),
                        modified_at: None,
                        content_hash: None,
                    },
                    line_endings: if content.contains("\r\n") {
                        CachedLineEndingStyle::Crlf
                    } else {
                        CachedLineEndingStyle::Lf
                    },
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CachedSearchState {
    pub kind: String,
    pub root: PathBuf,
    pub query: String,
    pub applied_offset: usize,
    pub truncated: bool,
    pub result_count: usize,
    pub output_mode: Option<TextSearchOutputMode>,
    pub glob: Option<String>,
    pub case_insensitive: Option<bool>,
    pub samples: Vec<String>,
    pub recorded_at: Option<DateTime<Utc>>,
}

impl CachedSearchState {
    pub fn from_action_result(result: &ActionResult) -> Option<Self> {
        match result {
            ActionResult::FileMatches {
                root,
                pattern,
                matches,
                truncated,
                applied_offset,
            } => Some(Self {
                kind: "find_files".to_string(),
                root: root.clone(),
                query: pattern.clone(),
                applied_offset: *applied_offset,
                truncated: *truncated,
                result_count: matches.len(),
                output_mode: None,
                glob: None,
                case_insensitive: None,
                samples: matches
                    .iter()
                    .take(6)
                    .map(|path| path.display().to_string())
                    .collect(),
                recorded_at: Some(Utc::now()),
            }),
            ActionResult::TextSearch {
                root,
                query,
                output_mode,
                matches,
                filenames,
                num_files: _,
                num_matches,
                truncated,
                applied_offset,
                glob,
                case_insensitive,
                ..
            } => {
                let samples = match output_mode {
                    TextSearchOutputMode::Content => matches
                        .iter()
                        .take(6)
                        .map(|item| format!("{}:{}", item.path.display(), item.line_number))
                        .collect(),
                    TextSearchOutputMode::FilesWithMatches => filenames
                        .iter()
                        .take(6)
                        .map(|path| path.display().to_string())
                        .collect(),
                    TextSearchOutputMode::Count => filenames
                        .iter()
                        .take(6)
                        .map(|path| path.display().to_string())
                        .collect(),
                };
                Some(Self {
                    kind: "search_text".to_string(),
                    root: root.clone(),
                    query: query.clone(),
                    applied_offset: *applied_offset,
                    truncated: *truncated,
                    result_count: match output_mode {
                        TextSearchOutputMode::Content => matches.len(),
                        TextSearchOutputMode::FilesWithMatches => filenames.len(),
                        TextSearchOutputMode::Count => *num_matches,
                    },
                    output_mode: Some(output_mode.clone()),
                    glob: glob.clone(),
                    case_insensitive: Some(*case_insensitive),
                    samples,
                    recorded_at: Some(Utc::now()),
                })
            }
            _ => None,
        }
    }

    pub fn cache_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}",
            self.kind,
            self.root.display(),
            self.query,
            self.applied_offset,
            self.glob.as_deref().unwrap_or_default(),
            self.output_mode
                .as_ref()
                .map(|mode| match mode {
                    TextSearchOutputMode::Content => "content",
                    TextSearchOutputMode::FilesWithMatches => "files_with_matches",
                    TextSearchOutputMode::Count => "count",
                })
                .unwrap_or(""),
        )
    }

    pub fn render(&self) -> String {
        let mode = self
            .output_mode
            .as_ref()
            .map(|mode| match mode {
                TextSearchOutputMode::Content => " mode=content",
                TextSearchOutputMode::FilesWithMatches => " mode=files_with_matches",
                TextSearchOutputMode::Count => " mode=count",
            })
            .unwrap_or("");
        let glob = self
            .glob
            .as_deref()
            .map(|value| format!(" glob={value}"))
            .unwrap_or_default();
        let ci = self
            .case_insensitive
            .map(|value| format!(" case_insensitive={value}"))
            .unwrap_or_default();
        let truncated = if self.truncated {
            " truncated=true"
        } else {
            ""
        };
        let samples = if self.samples.is_empty() {
            String::new()
        } else {
            format!(" samples={}", self.samples.join(", "))
        };
        format!(
            "- {} root={} query=\"{}\" offset={} count={}{}{}{}{}{}",
            self.kind,
            self.root.display(),
            self.query,
            self.applied_offset,
            self.result_count,
            mode,
            glob,
            ci,
            truncated,
            samples
        )
    }
}

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
        let mut sections = vec![
            format!("Identity:\n{}", self.identity),
            format!("Task:\n{}", self.task),
        ];
        if let Some(recent_context) = self.recent_context.as_ref() {
            sections.push(format!("Recent context:\n{}", recent_context.render()));
        }
        sections.push(format!(
            "Active continuation window:\n{}",
            self.continuation_window.render()
        ));
        sections.push(format!("Tools:\n{}", tools));
        if let Some(operator_guidance) = self.operator_guidance.as_ref() {
            sections.push(format!("Operator guidance:\n{}", operator_guidance));
        }
        sections.join("\n\n")
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ActiveContinuationWindow {
    pub objective: String,
    pub current_step: usize,
    pub max_steps: usize,
    pub reasoner_tokens_used: u32,
    #[serde(default)]
    pub max_output_tokens_recovery_count: usize,
    #[serde(default)]
    pub has_attempted_prompt_too_long_compaction: bool,
    #[serde(default)]
    pub last_transition: Option<ContinuationTransition>,
    #[serde(default)]
    pub read_state_cache: Vec<CachedFileReadState>,
    #[serde(default)]
    pub search_state_cache: Vec<CachedSearchState>,
    pub transcript: TranscriptLedger,
    pub stored_results: StoredResultLedger,
    pub content_replacements: ContentReplacementState,
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
        let replacement_by_id = self
            .content_replacements
            .records
            .iter()
            .map(|record| (record.replacement_id.as_str(), record))
            .collect::<std::collections::BTreeMap<_, _>>();
        let transcript_lines = self
            .transcript
            .entries()
            .iter()
            .map(|item| {
                item.result_ref_id
                    .as_deref()
                    .and_then(|id| replacement_by_id.get(id).copied())
                    .map(|record| item.render_with_override(&record.replacement_text))
                    .unwrap_or_else(|| item.render())
            })
            .collect::<Vec<_>>();

        let transcript_units = if transcript_lines.is_empty() {
            "none".to_string()
        } else {
            transcript_lines.join("\n")
        };
        let search_state = if self.search_state_cache.is_empty() {
            "none".to_string()
        } else {
            self.search_state_cache
                .iter()
                .map(CachedSearchState::render)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let sections = vec![
            format!("- objective: {}", self.objective),
            format!("- step: {} / {}", self.current_step, self.max_steps),
            format!(
                "- read_state_cache_entries: {}",
                self.read_state_cache.len()
            ),
            format!(
                "- search_state_cache_entries: {}",
                self.search_state_cache.len()
            ),
            format!("- search_state_cache:\n{}", search_state),
            format!("- transcript_units:\n{}", transcript_units),
        ];

        sections.join("\n")
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentReplacementState {
    pub records: Vec<ContentReplacementRecord>,
}

impl ContentReplacementState {
    pub fn from_continuation(
        stored_results: &StoredResultLedger,
        compacted_results: &[CompactedResultReference],
    ) -> Self {
        let mut state = Self::default();
        state.extend_from_continuation(stored_results, compacted_results);
        state
    }

    pub fn extend_from_continuation(
        &mut self,
        stored_results: &StoredResultLedger,
        compacted_results: &[CompactedResultReference],
    ) {
        for reference in stored_results.entries() {
            self.record_stored_result(reference);
        }
        for reference in compacted_results {
            self.record_compacted_result(reference);
        }
    }

    pub fn record_stored_result(
        &mut self,
        reference: &StoredResultReference,
    ) -> Option<ContentReplacementRecord> {
        self.push_record(ContentReplacementRecord::for_stored_result(reference))
    }

    pub fn record_compacted_result(
        &mut self,
        reference: &CompactedResultReference,
    ) -> Option<ContentReplacementRecord> {
        self.push_record(ContentReplacementRecord::for_compacted_result(reference))
    }

    pub fn render(&self) -> String {
        if self.records.is_empty() {
            "none".to_string()
        } else {
            self.records
                .iter()
                .map(ContentReplacementRecord::render)
                .collect::<Vec<_>>()
                .join("\n")
        }
    }

    fn push_record(
        &mut self,
        record: ContentReplacementRecord,
    ) -> Option<ContentReplacementRecord> {
        const MAX_REPLACEMENT_RECORDS: usize = 10;
        if self
            .records
            .iter()
            .any(|existing| existing.replacement_id == record.replacement_id)
        {
            return None;
        }
        self.records.push(record.clone());
        if self.records.len() > MAX_REPLACEMENT_RECORDS {
            let excess = self.records.len() - MAX_REPLACEMENT_RECORDS;
            self.records.drain(0..excess);
        }
        Some(record)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentReplacementRecord {
    pub replacement_id: String,
    pub source_kind: String,
    pub result_type: String,
    pub locator: Option<String>,
    pub persisted_path: Option<String>,
    pub replacement_text: String,
}

impl ContentReplacementRecord {
    pub fn for_stored_result(reference: &StoredResultReference) -> Self {
        Self {
            replacement_id: reference.result_id.clone(),
            source_kind: "stored_result".to_string(),
            result_type: reference.result_type.clone(),
            locator: reference.primary_locator.clone(),
            persisted_path: Some(reference.persisted_path.clone()),
            replacement_text: format!(
                "[stored-result {}] type={} preview=\"{}\"{} persisted={}",
                reference.result_id,
                reference.result_type,
                reference.preview_excerpt,
                reference
                    .primary_locator
                    .as_ref()
                    .map(|locator| format!(" locator={locator}"))
                    .unwrap_or_default(),
                reference.persisted_path
            ),
        }
    }

    pub fn for_compacted_result(reference: &CompactedResultReference) -> Self {
        Self {
            replacement_id: format!("boundary-{}", reference.boundary_id),
            source_kind: "compacted_result".to_string(),
            result_type: reference.result_type.clone(),
            locator: reference.locator.clone(),
            persisted_path: reference.persisted_path.clone(),
            replacement_text: format!(
                "[compacted-result boundary={}] type={} preview=\"{}\"{}{}",
                reference.boundary_id,
                reference.result_type,
                reference.preview_excerpt,
                reference
                    .locator
                    .as_ref()
                    .map(|locator| format!(" locator={locator}"))
                    .unwrap_or_default(),
                reference
                    .persisted_path
                    .as_ref()
                    .map(|path| format!(" persisted={path}"))
                    .unwrap_or_default()
            ),
        }
    }

    pub fn render(&self) -> String {
        let locator = self
            .locator
            .as_ref()
            .map(|value| format!(" locator={value}"))
            .unwrap_or_default();
        let persisted_path = self
            .persisted_path
            .as_ref()
            .map(|value| format!(" persisted={value}"))
            .unwrap_or_default();
        format!(
            "- {} [{}:{}]{}{} text=\"{}\"",
            self.replacement_id,
            self.source_kind,
            self.result_type,
            locator,
            persisted_path,
            self.replacement_text
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
    pub sticky_constraints: Vec<String>,
    pub sources: Vec<WorkingSource>,
    pub artifacts: Vec<ArtifactReference>,
}

impl RecentContext {
    pub fn render(&self) -> String {
        let answer = self
            .prior_answer_summary
            .clone()
            .unwrap_or_else(|| "none".to_string());
        let sticky_constraints = if self.sticky_constraints.is_empty() {
            "  - none".to_string()
        } else {
            self.sticky_constraints
                .iter()
                .map(|item| format!("  - {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
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
            "- prior_objective: {}\n- prior_answer_summary: {}\n- sticky_constraints:\n{}\n- sources:\n{}\n- artifacts:\n{}",
            self.prior_objective, answer, sticky_constraints, sources, artifacts
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
pub struct ReasonerTransition {
    pub reason: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub attempt: Option<u64>,
    #[serde(default)]
    pub metadata: Value,
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
