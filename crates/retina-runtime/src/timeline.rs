use crate::{RuntimeTask, RuntimeTaskKind, RuntimeTaskRegistry, RuntimeTaskStatus};
use retina_types::{TimelineEvent, TimelineEventType};

impl RuntimeTaskRegistry {
    pub fn from_timeline(events: &[TimelineEvent]) -> Self {
        let registry = Self::default();
        let mut ordered = events.to_vec();
        ordered.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
        for event in ordered {
            registry.apply_timeline_event(&event);
        }
        registry
    }

    fn apply_timeline_event(&self, event: &TimelineEvent) {
        match event.event_type {
            TimelineEventType::TaskReceived => {
                let description = event
                    .payload_json
                    .get("task")
                    .and_then(|value| value.as_str())
                    .unwrap_or("task")
                    .to_string();
                self.register(RuntimeTask {
                    task_id: event.task_id.clone(),
                    parent_task_id: None,
                    task_kind: RuntimeTaskKind::Session,
                    owner_agent_id: event.agent_id.clone(),
                    status: RuntimeTaskStatus::Pending,
                    started_at: event.timestamp,
                    ended_at: None,
                    description: description.clone(),
                    prompt_or_objective: description,
                    output_path: None,
                    output_offset: 0,
                    progress_summary: None,
                    last_activity: event.timestamp,
                    notified: false,
                });
            }
            TimelineEventType::ActionDispatched
            | TimelineEventType::ActionResultReceived
            | TimelineEventType::ReasonerCalled
            | TimelineEventType::TaskContinued
            | TimelineEventType::TaskRecoveryContinued => {
                self.mark_running_at(&event.task_id, event_summary(event), event.timestamp);
            }
            TimelineEventType::TaskCompleted => {
                self.mark_terminal_at(
                    &event.task_id,
                    RuntimeTaskStatus::Completed,
                    event_summary(event).unwrap_or_else(|| "completed".to_string()),
                    event.timestamp,
                );
            }
            TimelineEventType::TaskFailed => {
                self.mark_terminal_at(
                    &event.task_id,
                    RuntimeTaskStatus::Failed,
                    event_summary(event).unwrap_or_else(|| "failed".to_string()),
                    event.timestamp,
                );
            }
            TimelineEventType::TaskBlocked => {
                self.mark_terminal_at(
                    &event.task_id,
                    RuntimeTaskStatus::Blocked,
                    event_summary(event).unwrap_or_else(|| "blocked".to_string()),
                    event.timestamp,
                );
            }
            TimelineEventType::TaskCancelled => {
                self.mark_terminal_at(
                    &event.task_id,
                    RuntimeTaskStatus::Killed,
                    event_summary(event).unwrap_or_else(|| "stopped".to_string()),
                    event.timestamp,
                );
            }
            _ => {}
        }
    }

    fn mark_running_at(
        &self,
        task_id: &retina_types::TaskId,
        progress: Option<String>,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) {
        if let Some(mut task) = self.snapshot(task_id) {
            if task.status.is_terminal() {
                return;
            }
            task.status = RuntimeTaskStatus::Running;
            task.progress_summary = progress;
            task.last_activity = timestamp;
            self.register(task);
        }
    }
}

fn event_summary(event: &TimelineEvent) -> Option<String> {
    if matches!(event.event_type, TimelineEventType::TaskContinued) {
        return continuation_summary(event);
    }
    if matches!(event.event_type, TimelineEventType::TaskRecoveryContinued) {
        return recovery_summary(event);
    }
    event
        .payload_json
        .get("reason")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            event
                .payload_json
                .get("action")
                .and_then(|value| value.as_str())
                .map(|action| format!("action: {action}"))
        })
        .or_else(|| {
            event
                .payload_json
                .get("final_answer_summary")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
}

fn recovery_summary(event: &TimelineEvent) -> Option<String> {
    let reason = event
        .payload_json
        .get("reason")
        .and_then(|value| value.as_str());
    let attempt = event
        .payload_json
        .get("attempt")
        .and_then(|value| value.as_u64());
    let message = event
        .payload_json
        .get("message")
        .and_then(|value| value.as_str());

    match (reason, attempt, message) {
        (Some("max_output_tokens_escalate"), _, _) => {
            let max_tokens = event
                .payload_json
                .get("metadata")
                .and_then(|value| value.get("max_tokens"))
                .and_then(|value| value.as_u64());
            Some(match max_tokens {
                Some(max_tokens) => {
                    format!("recovering with larger output budget ({max_tokens} tokens)")
                }
                None => "recovering with larger output budget".to_string(),
            })
        }
        (Some("max_output_tokens_recovery"), Some(attempt), Some(_)) => Some(format!(
            "recovering after output truncation (attempt {attempt})"
        )),
        (Some("prompt_too_long_compaction"), _, Some(_)) => {
            Some("recovering from prompt-too-long via compaction".to_string())
        }
        (_, _, Some(message)) => Some(message.to_string()),
        (Some(reason), _, _) => Some(reason.to_string()),
        _ => None,
    }
}

fn continuation_summary(event: &TimelineEvent) -> Option<String> {
    let reason = event
        .payload_json
        .get("reason")
        .and_then(|value| value.as_str());
    let message = event
        .payload_json
        .get("message")
        .and_then(|value| value.as_str());
    match (reason, message) {
        (Some("completion_blocker"), Some(message)) => Some(format!("continuing: {message}")),
        (Some("next_turn"), Some(message)) => Some(message.to_string()),
        (Some(reason), Some(message)) => Some(format!("{reason}: {message}")),
        (Some(reason), None) => Some(reason.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use retina_types::{AgentId, EventId, SessionId, TaskId};
    use serde_json::json;

    #[test]
    fn registry_rebuilds_recent_task_projection_from_timeline() {
        let task_id = TaskId::new();
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let received = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({ "task": "summarize file" }),
        };
        let completed = TimelineEvent {
            timestamp: received.timestamp + chrono::Duration::seconds(1),
            event_type: TimelineEventType::TaskCompleted,
            payload_json: json!({ "final_answer_summary": "done" }),
            ..received.clone()
        };

        let registry = RuntimeTaskRegistry::from_timeline(&[completed, received]);
        let task = registry.snapshot(&task_id).unwrap();
        assert_eq!(task.status, RuntimeTaskStatus::Completed);
        assert_eq!(task.description, "summarize file");
        assert_eq!(task.progress_summary.as_deref(), Some("done"));
    }

    #[test]
    fn registry_marks_blocked_task_from_timeline() {
        let task_id = TaskId::new();
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let received = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({ "task": "search current events" }),
        };
        let blocked = TimelineEvent {
            timestamp: received.timestamp + chrono::Duration::seconds(1),
            event_type: TimelineEventType::TaskBlocked,
            payload_json: json!({ "reason": "repeated the same step without new evidence" }),
            ..received.clone()
        };

        let registry = RuntimeTaskRegistry::from_timeline(&[blocked, received]);
        let task = registry.snapshot(&task_id).unwrap();
        assert_eq!(task.status, RuntimeTaskStatus::Blocked);
        assert_eq!(
            task.progress_summary.as_deref(),
            Some("repeated the same step without new evidence")
        );
    }

    #[test]
    fn registry_humanizes_recovery_transition_progress_from_timeline() {
        let task_id = TaskId::new();
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let received = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({ "task": "write report" }),
        };
        let recovery = TimelineEvent {
            timestamp: received.timestamp + chrono::Duration::seconds(1),
            event_type: TimelineEventType::TaskRecoveryContinued,
            payload_json: json!({
                "reason": "max_output_tokens_recovery",
                "attempt": 2,
                "message": "Output token limit hit. Resume directly."
            }),
            ..received.clone()
        };

        let registry = RuntimeTaskRegistry::from_timeline(&[recovery, received]);
        let task = registry.snapshot(&task_id).unwrap();
        assert_eq!(task.status, RuntimeTaskStatus::Running);
        assert_eq!(
            task.progress_summary.as_deref(),
            Some("recovering after output truncation (attempt 2)")
        );
    }

    #[test]
    fn registry_humanizes_max_output_tokens_escalation_transition() {
        let task_id = TaskId::new();
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let received = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({ "task": "write report" }),
        };
        let recovery = TimelineEvent {
            timestamp: received.timestamp + chrono::Duration::seconds(1),
            event_type: TimelineEventType::TaskRecoveryContinued,
            payload_json: json!({
                "reason": "max_output_tokens_escalate",
                "attempt": 1,
                "message": "Retrying the same request with a larger output token budget.",
                "metadata": { "max_tokens": 64000 }
            }),
            ..received.clone()
        };

        let registry = RuntimeTaskRegistry::from_timeline(&[recovery, received]);
        let task = registry.snapshot(&task_id).unwrap();
        assert_eq!(task.status, RuntimeTaskStatus::Running);
        assert_eq!(
            task.progress_summary.as_deref(),
            Some("recovering with larger output budget (64000 tokens)")
        );
    }

    #[test]
    fn registry_humanizes_next_turn_continuation_transition() {
        let task_id = TaskId::new();
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let received = TimelineEvent {
            event_id: EventId::new(),
            session_id,
            task_id: task_id.clone(),
            agent_id,
            timestamp: Utc::now(),
            event_type: TimelineEventType::TaskReceived,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json: json!({ "task": "write report" }),
        };
        let continued = TimelineEvent {
            timestamp: received.timestamp + chrono::Duration::seconds(1),
            event_type: TimelineEventType::TaskContinued,
            payload_json: json!({
                "reason": "next_turn",
                "message": "continuing after non-terminal tool progress"
            }),
            ..received.clone()
        };

        let registry = RuntimeTaskRegistry::from_timeline(&[continued, received]);
        let task = registry.snapshot(&task_id).unwrap();
        assert_eq!(task.status, RuntimeTaskStatus::Running);
        assert_eq!(
            task.progress_summary.as_deref(),
            Some("continuing after non-terminal tool progress")
        );
    }
}
