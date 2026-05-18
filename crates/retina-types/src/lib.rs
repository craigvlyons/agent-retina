// File boundary: keep lib.rs limited to module declarations and re-exports.
// New type families and helper logic should live in focused sibling modules.
mod actions;
mod agents;
mod core;
mod execution;
mod fabrication;
mod memory;
mod reasoning;
mod task_state;
mod tasking;

pub use actions::*;
pub use agents::*;
pub use core::*;
pub use execution::*;
pub use fabrication::*;
pub use memory::*;
pub use reasoning::*;
pub use task_state::*;
pub use tasking::*;

#[cfg(test)]
mod tests {
    use crate::{
        Action, ActionId, ActiveContinuationWindow, AgentId, CompactedResultReference,
        CompactionSnapshot, ContentReplacementRecord, ContentReplacementState, HashScope,
        PrivilegedCommandKind, StoredResultLedger, StoredResultReference, Task, TaskFollowUpSeed,
        TaskRecoverySnapshot, TranscriptLedger, TranscriptUnit, TranscriptUnitKind, WorkingSource,
        classify_privileged_command,
    };
    use std::path::PathBuf;

    #[test]
    fn privileged_command_classifier_only_flags_delete_and_kill_commands() {
        assert_eq!(
            classify_privileged_command("rm tmp/test.txt"),
            Some(PrivilegedCommandKind::Delete)
        );
        assert_eq!(
            classify_privileged_command("find . -name '*.tmp' -delete"),
            Some(PrivilegedCommandKind::Delete)
        );
        assert_eq!(
            classify_privileged_command("pkill retina"),
            Some(PrivilegedCommandKind::Kill)
        );
        assert_eq!(classify_privileged_command("mv a b"), None);
        assert_eq!(classify_privileged_command("chmod +x script.sh"), None);
        assert_eq!(classify_privileged_command("curl --version"), None);
    }

    #[test]
    fn only_delete_or_kill_commands_require_approval_by_policy() {
        let delete = Action::RunCommand {
            id: ActionId::new(),
            command: "rm tmp/test.txt".to_string(),
            cwd: None,
            require_approval: false,
            expect_change: true,
            state_scope: HashScope::default(),
        };
        let write = Action::WriteFile {
            id: ActionId::new(),
            path: PathBuf::from("tmp/test.txt"),
            content: "hello".to_string(),
            overwrite: true,
        };

        assert!(delete.approval_required_by_policy());
        assert!(!write.approval_required_by_policy());
    }

    #[test]
    fn task_state_projection_defaults_cleanly() {
        let state = crate::TaskState::default();
        assert!(state.goal.objective.is_empty());
        assert_eq!(state.progress.current_step, 0);
    }

    #[test]
    fn resumed_task_keeps_source_session_and_seeds_resume_context() {
        let source_task = Task::new(AgentId::new(), "finish bulk report");
        let source_task_id = source_task.id.clone();
        let source_session_id = source_task.session_id.clone();
        let snapshot = TaskRecoverySnapshot {
            source_task_id: source_task.id.clone(),
            source_session_id: source_task.session_id.clone(),
            source_agent_id: source_task.agent_id.clone(),
            objective: source_task.description.clone(),
            continuation_window: ActiveContinuationWindow {
                objective: source_task.description.clone(),
                current_step: 4,
                max_steps: 50,
                reasoner_tokens_used: 0,
                max_output_tokens_recovery_count: 0,
                has_attempted_prompt_too_long_compaction: false,
                last_transition: None,
                read_state_cache: Vec::new(),
                search_state_cache: Vec::new(),
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 4,
                    kind: TranscriptUnitKind::ToolResult,
                    summary: "read the first PDF".to_string(),
                    result_ref_id: None,
                    primary_locator: Some("/tmp/example.pdf".to_string()),
                    evidence_refs: vec!["/tmp/example.pdf".to_string()],
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                reannounced_sources: vec![WorkingSource {
                    kind: "document".to_string(),
                    locator: "/tmp/example.pdf".to_string(),
                    role: "primary".to_string(),
                    status: "active".to_string(),
                    why_it_matters: "contains the original PDF evidence".to_string(),
                    last_used_step: 4,
                    evidence_refs: vec!["/tmp/example.pdf".to_string()],
                    page_reference: None,
                    extraction_method: None,
                    structured_summary: None,
                    preview_excerpt: Some("example".to_string()),
                }],
                ..ActiveContinuationWindow::default()
            },
            resume_reason: "reasoning transport failed".to_string(),
        };

        let resumed = Task::resume_from_snapshot(AgentId::new(), snapshot, None);
        assert_eq!(resumed.parent_task_id, Some(source_task_id.clone()));
        assert_eq!(resumed.session_id, source_session_id);
        assert!(resumed.resume_context.is_some());
        assert_eq!(
            resumed
                .metadata
                .get("resumed_from_task_id")
                .map(String::as_str),
            Some(source_task_id.0.as_str())
        );
    }

    #[test]
    fn follow_up_task_keeps_source_session_and_seeds_resume_context() {
        let source_task = Task::new(AgentId::new(), "inspect gabactl");
        let source_task_id = source_task.id.clone();
        let source_session_id = source_task.session_id.clone();
        let follow_up = Task::follow_up_from_seed(
            AgentId::new(),
            TaskFollowUpSeed {
                source_task_id: source_task.id.clone(),
                source_session_id: source_task.session_id.clone(),
                source_agent_id: source_task.agent_id.clone(),
                objective: source_task.description.clone(),
                continuation_window: ActiveContinuationWindow {
                    objective: source_task.description.clone(),
                    current_step: 5,
                    max_steps: 50,
                    reasoner_tokens_used: 0,
                    max_output_tokens_recovery_count: 0,
                    has_attempted_prompt_too_long_compaction: false,
                    last_transition: None,
                    read_state_cache: Vec::new(),
                    search_state_cache: Vec::new(),
                    transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                        ordinal: 1,
                        step: 5,
                        kind: TranscriptUnitKind::ToolResult,
                        summary: "validated gabactl debug binary".to_string(),
                        result_ref_id: None,
                        primary_locator: Some(
                            "/Users/macc/projects/personal/gabanode-desktop/gabactl/.build/debug/gabactl"
                                .to_string(),
                        ),
                        evidence_refs: vec![
                            "/Users/macc/projects/personal/gabanode-desktop/gabactl/.build/debug/gabactl"
                                .to_string(),
                        ],
                        working_sources: Vec::new(),
                        artifact_references: Vec::new(),
                        next_step_guidance: None,
                        repetition_signature: None,
                        avoid_label: None,
                        compaction_snapshot: None,
                    }]),
                    ..ActiveContinuationWindow::default()
                },
                recent_context: None,
            },
            "use the library to open chrome",
        );

        assert_eq!(follow_up.parent_task_id, Some(source_task_id.clone()));
        assert_eq!(follow_up.session_id, source_session_id);
        assert!(follow_up.resume_context.is_some());
        assert!(
            follow_up
                .recent_context
                .as_ref()
                .expect("follow-up recent context")
                .sticky_constraints
                .iter()
                .any(|item| item.contains("Reuse previously validated tools"))
        );
        assert_eq!(
            follow_up
                .metadata
                .get("follow_up_from_task_id")
                .map(String::as_str),
            Some(source_task_id.0.as_str())
        );
        assert_eq!(follow_up.description, "use the library to open chrome");
    }

    #[test]
    fn assembled_context_render_surfaces_recent_context_for_follow_up_turns() {
        let rendered = crate::AssembledContext {
            identity: "Retina/root".to_string(),
            task: "use the library to open chrome".to_string(),
            continuation_window: ActiveContinuationWindow {
                reasoner_tokens_used: 0,
                max_output_tokens_recovery_count: 0,
                has_attempted_prompt_too_long_compaction: false,
                last_transition: None,
                read_state_cache: Vec::new(),
                    search_state_cache: Vec::new(),
                transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                    ordinal: 1,
                    step: 5,
                    kind: TranscriptUnitKind::ToolResult,
                    summary: "validated gabactl debug binary".to_string(),
                    result_ref_id: Some("result-5-1".to_string()),
                    primary_locator: Some(
                        "/Users/macc/projects/personal/gabanode-desktop/gabactl/.build/debug/gabactl"
                            .to_string(),
                    ),
                    evidence_refs: vec![
                        "/Users/macc/projects/personal/gabanode-desktop/gabactl/.build/debug/gabactl"
                            .to_string(),
                    ],
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                }]),
                stored_results: StoredResultLedger::from_entries(vec![StoredResultReference {
                    result_id: "result-5-1".to_string(),
                    source_transcript_ordinal: 1,
                    step: 5,
                    result_type: "run_command".to_string(),
                    primary_locator: Some(
                        "/Users/macc/projects/personal/gabanode-desktop/gabactl/.build/debug/gabactl"
                            .to_string(),
                    ),
                    preview_excerpt: "validated gabactl debug binary".to_string(),
                    persisted_path: "/tmp/result-5-1.json".to_string(),
                }]),
                content_replacements: crate::ContentReplacementState::from_continuation(
                    &StoredResultLedger::from_entries(vec![StoredResultReference {
                        result_id: "result-5-1".to_string(),
                        source_transcript_ordinal: 1,
                        step: 5,
                        result_type: "run_command".to_string(),
                        primary_locator: Some(
                            "/Users/macc/projects/personal/gabanode-desktop/gabactl/.build/debug/gabactl"
                                .to_string(),
                        ),
                        preview_excerpt: "validated gabactl debug binary".to_string(),
                        persisted_path: "/tmp/result-5-1.json".to_string(),
                    }]),
                    &[],
                ),
                ..ActiveContinuationWindow::default()
            },
            recent_context: Some(crate::RecentContext {
                prior_objective: "inspect gabactl".to_string(),
                prior_answer_summary: Some("validated gabactl debug binary".to_string()),
                sticky_constraints: vec![
                    "Reuse previously validated tools, library paths, and artifacts.".to_string(),
                ],
                sources: Vec::new(),
                artifacts: Vec::new(),
            }),
            tools: Vec::new(),
            memory_slice: Vec::new(),
            operator_guidance: None,
            current_step: 1,
            max_steps: 6,
        }
        .render();

        assert!(rendered.contains("Recent context:"));
        assert!(rendered.contains("inspect gabactl"));
        assert!(rendered.contains("sticky_constraints:"));
        assert!(rendered.contains("Reuse previously validated tools"));
        assert!(rendered.contains("[stored-result result-5-1]"));
        assert!(!rendered.contains("content_replacements:"));
        assert!(!rendered.contains("stored_result_refs:"));
    }

    #[test]
    fn content_replacement_extension_keeps_existing_exact_record_text() {
        let mut replacements = ContentReplacementState {
            records: vec![ContentReplacementRecord {
                replacement_id: "result-5-1".to_string(),
                source_kind: "stored_result".to_string(),
                result_type: "run_command".to_string(),
                locator: Some("/tmp/gabactl".to_string()),
                persisted_path: Some("/tmp/result-5-1.json".to_string()),
                replacement_text: "[stored-result result-5-1] frozen exact replacement".to_string(),
            }],
        };
        let stored_results = StoredResultLedger::from_entries(vec![StoredResultReference {
            result_id: "result-5-1".to_string(),
            source_transcript_ordinal: 1,
            step: 5,
            result_type: "run_command".to_string(),
            primary_locator: Some("/tmp/gabactl".to_string()),
            preview_excerpt: "validated gabactl debug binary".to_string(),
            persisted_path: "/tmp/result-5-1.json".to_string(),
        }]);

        replacements.extend_from_continuation(&stored_results, &[]);

        assert_eq!(replacements.records.len(), 1);
        assert_eq!(
            replacements.records[0].replacement_text,
            "[stored-result result-5-1] frozen exact replacement"
        );
    }

    #[test]
    fn continuation_render_hides_compacted_result_refs_when_replacements_exist() {
        let rendered = ActiveContinuationWindow {
            reasoner_tokens_used: 0,
            max_output_tokens_recovery_count: 0,
            has_attempted_prompt_too_long_compaction: false,
            last_transition: None,
            read_state_cache: Vec::new(),
            search_state_cache: Vec::new(),
            transcript: TranscriptLedger::from_entries(vec![TranscriptUnit {
                ordinal: 1,
                step: 1,
                kind: TranscriptUnitKind::ToolResult,
                summary: "large directory listing".to_string(),
                result_ref_id: Some("boundary-2".to_string()),
                primary_locator: Some("/tmp".to_string()),
                evidence_refs: vec!["/tmp".to_string()],
                working_sources: Vec::new(),
                artifact_references: Vec::new(),
                next_step_guidance: None,
                repetition_signature: None,
                avoid_label: None,
                compaction_snapshot: None,
            }]),
            content_replacements: ContentReplacementState::from_continuation(
                &StoredResultLedger::default(),
                &[CompactedResultReference {
                    boundary_id: 2,
                    result_type: "directory_listing".to_string(),
                    locator: Some("/tmp".to_string()),
                    preview_excerpt: "preview".to_string(),
                    continuation: None,
                    persisted_path: Some("/tmp/boundary-2.json".to_string()),
                }],
            ),
            reannounced_compacted_results: vec![CompactedResultReference {
                boundary_id: 2,
                result_type: "directory_listing".to_string(),
                locator: Some("/tmp".to_string()),
                preview_excerpt: "preview".to_string(),
                continuation: None,
                persisted_path: Some("/tmp/boundary-2.json".to_string()),
            }],
            ..ActiveContinuationWindow::default()
        }
        .render();

        assert!(rendered.contains("[compacted-result boundary=2]"));
        assert!(!rendered.contains("content_replacements:"));
        assert!(!rendered.contains("reannounced_compacted_results:"));
    }

    #[test]
    fn continuation_render_uses_compaction_carryover_summary_for_model_view() {
        let rendered = ActiveContinuationWindow {
            reasoner_tokens_used: 0,
            max_output_tokens_recovery_count: 0,
            has_attempted_prompt_too_long_compaction: false,
            last_transition: None,
            read_state_cache: Vec::new(),
                    search_state_cache: Vec::new(),
            transcript: TranscriptLedger::from_entries(vec![
                TranscriptUnit {
                    ordinal: 1,
                    step: 9,
                    kind: TranscriptUnitKind::CompactSummary,
                    summary: "compaction carried forward: reason=\"large tool result\" summary=\"Carry forward the saved report context\" preserved_locator_count=2 continuation=\"Continue from the saved report\"".to_string(),
                    result_ref_id: None,
                    primary_locator: None,
                    evidence_refs: vec!["a.md".to_string(), "b.md".to_string()],
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                },
                TranscriptUnit {
                    ordinal: 2,
                    step: 10,
                    kind: TranscriptUnitKind::ToolResult,
                    summary: "read preserved report".to_string(),
                    result_ref_id: None,
                    primary_locator: Some("report.md".to_string()),
                    evidence_refs: vec!["report.md".to_string()],
                    working_sources: Vec::new(),
                    artifact_references: Vec::new(),
                    next_step_guidance: None,
                    repetition_signature: None,
                    avoid_label: None,
                    compaction_snapshot: None,
                },
            ]),
            compaction_boundaries: vec![CompactionSnapshot {
                boundary_id: 7,
                compacted_at_step: 9,
                reason: "large tool result".to_string(),
                score_explanations: vec![crate::CompactionScoreExplanation {
                    item_kind: "source".to_string(),
                    locator: "a.md".to_string(),
                    decision: "keep".to_string(),
                    rationale: "important".to_string(),
                }],
                preserved_locators: vec!["a.md".to_string(), "b.md".to_string()],
                active_window_summary: "Carry forward the saved report context".to_string(),
                last_result_continuation: Some("Continue from the saved report".to_string()),
                compacted_results: Vec::new(),
            }],
            ..ActiveContinuationWindow::default()
        }
        .render();

        assert!(rendered.contains("compaction carried forward:"));
        assert!(rendered.contains("[compact_summary]"));
        assert!(rendered.contains("reason=\"large tool result\""));
        assert!(rendered.contains("summary=\"Carry forward the saved report context\""));
        assert!(rendered.contains("preserved_locator_count=2"));
        assert!(!rendered.contains("carried forward context:"));
        assert!(!rendered.contains("- carryover:"));
        assert!(!rendered.contains("compaction_boundaries:"));
        assert!(!rendered.contains("boundary_id: 7"));
        assert!(!rendered.contains("ranking:"));
        assert!(
            rendered
                .find("[compact_summary]")
                .expect("carryover note should render")
                < rendered
                    .find("read preserved report")
                    .expect("transcript entry should render")
        );
    }

    #[test]
    fn continuation_render_never_exposes_stored_result_ledger_section() {
        let rendered = ActiveContinuationWindow {
            reasoner_tokens_used: 0,
            max_output_tokens_recovery_count: 0,
            has_attempted_prompt_too_long_compaction: false,
            last_transition: None,
            read_state_cache: Vec::new(),
            search_state_cache: Vec::new(),
            stored_results: StoredResultLedger::from_entries(vec![StoredResultReference {
                result_id: "result-9-1".to_string(),
                source_transcript_ordinal: 1,
                step: 9,
                result_type: "run_command".to_string(),
                primary_locator: Some("/tmp/example.txt".to_string()),
                preview_excerpt: "preview".to_string(),
                persisted_path: "/tmp/result-9-1.json".to_string(),
            }]),
            ..ActiveContinuationWindow::default()
        }
        .render();

        assert!(!rendered.contains("stored_result_refs:"));
        assert!(!rendered.contains("result-9-1"));
    }
}
