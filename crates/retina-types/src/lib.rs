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
        Action, ActionId, ActiveContinuationWindow, AgentId, HashScope, PrivilegedCommandKind,
        Task, TaskRecoverySnapshot, TranscriptLedger, TranscriptUnit, TranscriptUnitKind,
        WorkingSource, classify_privileged_command,
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
            recent_context: None,
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
}
