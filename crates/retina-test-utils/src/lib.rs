use chrono::Utc;
use retina_traits::{Memory, Reasoner, Shell};
use retina_types::*;
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct MockMemory {
    timeline: Arc<Mutex<Vec<TimelineEvent>>>,
    experiences: Arc<Mutex<Vec<Experience>>>,
    knowledge: Arc<Mutex<Vec<KnowledgeNode>>>,
    rules: Arc<Mutex<Vec<ReflexiveRule>>>,
    tools: Arc<Mutex<Vec<ToolRecord>>>,
}

impl Memory for MockMemory {
    fn append_timeline_event(&self, event: &TimelineEvent) -> Result<()> {
        self.timeline.lock().unwrap().push(event.clone());
        Ok(())
    }

    fn record_experience(&self, exp: &Experience) -> Result<ExperienceId> {
        self.experiences.lock().unwrap().push(exp.clone());
        Ok(ExperienceId::new())
    }

    fn store_knowledge(&self, node: &KnowledgeNode) -> Result<KnowledgeId> {
        self.knowledge.lock().unwrap().push(node.clone());
        Ok(KnowledgeId::new())
    }

    fn link_knowledge(&self, _from: KnowledgeId, _to: KnowledgeId, _relation: &str) -> Result<()> {
        Ok(())
    }

    fn store_rule(&self, rule: &ReflexiveRule) -> Result<RuleId> {
        self.rules.lock().unwrap().push(rule.clone());
        Ok(RuleId::new())
    }

    fn register_tool(&self, tool: &ToolRecord) -> Result<ToolId> {
        self.tools.lock().unwrap().push(tool.clone());
        Ok(ToolId::new())
    }

    fn append_state(&self, entry: &TimelineEvent) -> Result<()> {
        self.append_timeline_event(entry)
    }

    fn recall_experiences(&self, _query: &str, limit: usize) -> Result<Vec<Experience>> {
        Ok(self
            .experiences
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    fn recall_knowledge(&self, _query: &str, limit: usize) -> Result<Vec<KnowledgeNode>> {
        Ok(self
            .knowledge
            .lock()
            .unwrap()
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    fn active_rules(&self) -> Result<Vec<ReflexiveRule>> {
        Ok(self.rules.lock().unwrap().clone())
    }

    fn find_tools(&self, _capability: &str) -> Result<Vec<ToolRecord>> {
        Ok(self.tools.lock().unwrap().clone())
    }

    fn recent_states(&self, limit: usize) -> Result<Vec<TimelineEvent>> {
        Ok(self
            .timeline
            .lock()
            .unwrap()
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect())
    }

    fn update_utility(&self, _id: ExperienceId, _utility: f64) -> Result<()> {
        Ok(())
    }

    fn update_knowledge(&self, _id: KnowledgeId, _update: &KnowledgeUpdate) -> Result<()> {
        Ok(())
    }

    fn update_rule(&self, _id: RuleId, _update: &RuleUpdate) -> Result<()> {
        Ok(())
    }

    fn consolidate(&self, _config: &ConsolidationConfig) -> Result<ConsolidationReport> {
        Ok(ConsolidationReport::default())
    }

    fn backup(&self, _path: &Path) -> Result<()> {
        Ok(())
    }
}

impl MockMemory {
    pub fn rule_count(&self) -> usize {
        self.rules.lock().unwrap().len()
    }
}

#[derive(Clone)]
pub struct MockReasoner {
    responses: Arc<Mutex<Vec<ReasonResponse>>>,
}

impl MockReasoner {
    pub fn for_action(action: Action) -> Self {
        Self::for_response(ReasonResponse {
            action,
            task_complete: true,
            reasoning: Some("mock reasoning".to_string()),
            tokens_used: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        })
    }

    pub fn for_response(response: ReasonResponse) -> Self {
        Self {
            responses: Arc::new(Mutex::new(vec![response])),
        }
    }

    pub fn sequence(responses: Vec<ReasonResponse>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
        }
    }
}

impl Reasoner for MockReasoner {
    fn reason(&self, _request: &ReasonRequest) -> Result<ReasonResponse> {
        let mut responses = self.responses.lock().unwrap();
        let response = if responses.len() > 1 {
            responses.remove(0)
        } else {
            responses.first().cloned().unwrap_or(ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "mock response".to_string(),
                },
                task_complete: true,
                reasoning: Some("mock reasoning".to_string()),
                tokens_used: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                },
            })
        };
        Ok(response)
    }

    fn capabilities(&self) -> ReasonerCapabilities {
        ReasonerCapabilities {
            max_context_tokens: 1000,
            supports_tool_use: false,
            supports_vision: false,
            supports_caching: false,
            model_id: "mock".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct MockShell {
    force_unchanged: bool,
    inputs: Arc<Mutex<Vec<String>>>,
}

impl Default for MockShell {
    fn default() -> Self {
        Self {
            force_unchanged: false,
            inputs: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockShell {
    pub fn with_force_unchanged(mut self, force_unchanged: bool) -> Self {
        self.force_unchanged = force_unchanged;
        self
    }

    pub fn with_inputs(mut self, inputs: Vec<String>) -> Self {
        self.inputs = Arc::new(Mutex::new(inputs));
        self
    }
}

impl Shell for MockShell {
    fn observe(&self) -> Result<WorldState> {
        Ok(WorldState {
            cwd: std::env::current_dir().unwrap(),
            files: Vec::new(),
            last_command: None,
            notes: Vec::new(),
        })
    }

    fn capture_state(&self, scope: &HashScope) -> Result<StateSnapshot> {
        Ok(StateSnapshot {
            scope: scope.clone(),
            cwd: std::env::current_dir().unwrap(),
            cwd_hash: if self.force_unchanged {
                "same".to_string()
            } else {
                Utc::now()
                    .timestamp_nanos_opt()
                    .unwrap_or_default()
                    .to_string()
            },
            files: Vec::new(),
            last_command: None,
        })
    }

    fn compare_state(
        &self,
        before: &StateSnapshot,
        after: &StateSnapshot,
        action: Option<&Action>,
    ) -> Result<StateDelta> {
        let expect_change = action.map(Action::expects_change).unwrap_or(false);
        if before.cwd_hash == after.cwd_hash && expect_change {
            Ok(StateDelta::unchanged())
        } else {
            Ok(StateDelta {
                kind: StateDeltaKind::ChangedAsExpected,
                summary: "changed".to_string(),
                changed_paths: Vec::new(),
            })
        }
    }

    fn execute(&self, action: &Action) -> Result<ActionResult> {
        match action {
            Action::RunCommand { command, .. } => Ok(ActionResult::Command(CommandResult {
                command: command.clone(),
                cwd: std::env::current_dir().unwrap(),
                stdout: "ok".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
                duration_ms: 1,
            })),
            Action::InspectPath { path, .. } => Ok(ActionResult::Inspection(WorldState {
                cwd: path.clone(),
                files: Vec::new(),
                last_command: None,
                notes: Vec::new(),
            })),
            Action::InspectWorkingDirectory { .. } => self.observe().map(ActionResult::Inspection),
            Action::ListDirectory { path, .. } => Ok(ActionResult::DirectoryListing {
                root: path.clone(),
                entries: Vec::new(),
            }),
            Action::FindFiles { root, pattern, .. } => Ok(ActionResult::FileMatches {
                root: root.clone(),
                pattern: pattern.clone(),
                matches: Vec::new(),
            }),
            Action::SearchText { root, query, .. } => Ok(ActionResult::TextSearch {
                root: root.clone(),
                query: query.clone(),
                matches: Vec::new(),
            }),
            Action::ReadFile { path, .. } => Ok(ActionResult::FileRead {
                path: path.clone(),
                content: "mock-content".to_string(),
                truncated: false,
            }),
            Action::WriteFile { path, content, .. } => Ok(ActionResult::FileWrite {
                path: path.clone(),
                bytes_written: content.len(),
                appended: false,
            }),
            Action::AppendFile { path, content, .. } => Ok(ActionResult::FileWrite {
                path: path.clone(),
                bytes_written: content.len(),
                appended: true,
            }),
            Action::RecordNote { note, .. } => {
                Ok(ActionResult::NoteRecorded { note: note.clone() })
            }
            Action::Respond { message, .. } => Ok(ActionResult::Response {
                message: message.clone(),
            }),
        }
    }

    fn constraints(&self) -> &[HardConstraint] {
        static CONSTRAINTS: [HardConstraint; 2] = [
            HardConstraint::NoNetworkShellActions,
            HardConstraint::DestructiveOperationsRequireApproval,
        ];
        &CONSTRAINTS
    }

    fn capabilities(&self) -> ShellCapabilities {
        ShellCapabilities {
            can_execute_commands: true,
            can_read_files: true,
            can_write_files: true,
            can_search_files: true,
            can_write_notes: true,
            can_respond_text: true,
        }
    }

    fn request_approval(&self, _request: &ApprovalRequest) -> Result<ApprovalResponse> {
        Ok(ApprovalResponse::Approved)
    }

    fn notify(&self, _message: &str) -> Result<()> {
        Ok(())
    }

    fn request_input(&self, _prompt: &str) -> Result<String> {
        let mut inputs = self.inputs.lock().unwrap();
        if inputs.is_empty() {
            Ok("mock-input".to_string())
        } else {
            Ok(inputs.remove(0))
        }
    }
}

pub fn sample_knowledge() -> KnowledgeNode {
    KnowledgeNode {
        id: None,
        category: "lesson".to_string(),
        content: "Always verify file changes after writing.".to_string(),
        confidence: 0.8,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        metadata: json!({}),
    }
}
