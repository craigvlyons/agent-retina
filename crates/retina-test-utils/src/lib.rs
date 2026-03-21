// File boundary: keep lib.rs limited to shared test harness surfaces and small
// cross-crate utilities. Move scenario-specific helpers into modules as it grows.
use chrono::Utc;
use retina_traits::{Memory, Reasoner, Shell};
use retina_types::*;
use serde_json::json;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

fn recover_mutex<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn current_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

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
        recover_mutex(&self.timeline).push(event.clone());
        Ok(())
    }

    fn record_experience(&self, exp: &Experience) -> Result<ExperienceId> {
        recover_mutex(&self.experiences).push(exp.clone());
        Ok(ExperienceId::new())
    }

    fn store_knowledge(&self, node: &KnowledgeNode) -> Result<KnowledgeId> {
        recover_mutex(&self.knowledge).push(node.clone());
        Ok(KnowledgeId::new())
    }

    fn link_knowledge(&self, _from: KnowledgeId, _to: KnowledgeId, _relation: &str) -> Result<()> {
        Ok(())
    }

    fn store_rule(&self, rule: &ReflexiveRule) -> Result<RuleId> {
        recover_mutex(&self.rules).push(rule.clone());
        Ok(RuleId::new())
    }

    fn register_tool(&self, tool: &ToolRecord) -> Result<ToolId> {
        recover_mutex(&self.tools).push(tool.clone());
        Ok(ToolId::new())
    }

    fn append_state(&self, entry: &TimelineEvent) -> Result<()> {
        self.append_timeline_event(entry)
    }

    fn recall_experiences(&self, _query: &str, limit: usize) -> Result<Vec<Experience>> {
        Ok(self
            .experiences
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    fn recall_knowledge(&self, _query: &str, limit: usize) -> Result<Vec<KnowledgeNode>> {
        Ok(self
            .knowledge
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    fn active_rules(&self) -> Result<Vec<ReflexiveRule>> {
        Ok(recover_mutex(&self.rules).clone())
    }

    fn find_tools(&self, _capability: &str) -> Result<Vec<ToolRecord>> {
        Ok(recover_mutex(&self.tools).clone())
    }

    fn recent_states(&self, limit: usize) -> Result<Vec<TimelineEvent>> {
        Ok(self
            .timeline
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    fn consolidate(&self, config: &ConsolidationConfig) -> Result<ConsolidationReport> {
        let experiences = recover_mutex(&self.experiences).clone();
        let mut grouped: HashMap<(String, String), (Action, usize, f64)> = HashMap::new();

        for experience in experiences {
            if experience.utility < config.min_success_utility {
                continue;
            }
            let Some(task) = experience
                .metadata
                .get("task")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(action) = experience
                .metadata
                .get("action")
                .cloned()
                .and_then(|value| serde_json::from_value::<Action>(value).ok())
            else {
                continue;
            };
            if matches!(
                action,
                Action::Respond { .. }
                    | Action::RecordNote { .. }
                    | Action::InspectWorkingDirectory { .. }
            ) {
                continue;
            }

            let key = (task.to_lowercase(), experience.action_summary.clone());
            let entry = grouped.entry(key).or_insert((action, 0, 0.0));
            entry.1 += 1;
            entry.2 += experience.utility;
        }

        let mut promoted = 0;
        let mut rules = recover_mutex(&self.rules);
        for ((task, action_summary), (action, count, utility_total)) in grouped {
            if count < config.min_successful_repeats {
                continue;
            }
            let confidence =
                ((utility_total / count as f64).clamp(0.0, 1.0) * 0.65) + (0.35 * 0.75);
            if confidence < config.min_rule_confidence {
                continue;
            }
            let name = format!("consolidated:{}:{}", task, action_summary);
            let exists = rules.iter().any(|rule| rule.name == name);
            if !exists {
                rules.push(ReflexiveRule {
                    id: Some(RuleId::new()),
                    name,
                    condition: RuleCondition::TaskContains(task),
                    action: RuleAction::UseAction(action),
                    confidence,
                    active: true,
                    last_fired: None,
                });
                promoted += 1;
            }
        }

        Ok(ConsolidationReport {
            merged_knowledge: 0,
            promoted_rules: promoted,
            compacted_events: 0,
            decayed_knowledge: 0,
            optimized: false,
        })
    }

    fn backup(&self, _path: &Path) -> Result<()> {
        Ok(())
    }
}

impl MockMemory {
    pub fn rule_count(&self) -> usize {
        recover_mutex(&self.rules).len()
    }

    pub fn experiences(&self) -> Vec<Experience> {
        recover_mutex(&self.experiences).clone()
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
            framing: None,
            reasoning: Some("mock reasoning".to_string()),
            tokens_used: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
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
        let mut responses = recover_mutex(&self.responses);
        let response = if responses.len() > 1 {
            responses.remove(0)
        } else {
            responses.first().cloned().unwrap_or(ReasonResponse {
                action: Action::Respond {
                    id: ActionId::new(),
                    message: "mock response".to_string(),
                },
                task_complete: true,
                framing: None,
                reasoning: Some("mock reasoning".to_string()),
                tokens_used: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
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
    files: Arc<Mutex<HashMap<PathBuf, String>>>,
    approvals: Arc<Mutex<Vec<ApprovalResponse>>>,
}

impl Default for MockShell {
    fn default() -> Self {
        Self {
            force_unchanged: false,
            inputs: Arc::new(Mutex::new(Vec::new())),
            files: Arc::new(Mutex::new(HashMap::new())),
            approvals: Arc::new(Mutex::new(vec![ApprovalResponse::Approved])),
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

    pub fn with_files<I, P, S>(mut self, files: I) -> Self
    where
        I: IntoIterator<Item = (P, S)>,
        P: Into<PathBuf>,
        S: Into<String>,
    {
        self.files = Arc::new(Mutex::new(
            files
                .into_iter()
                .map(|(path, content)| (path.into(), content.into()))
                .collect(),
        ));
        self
    }

    pub fn with_approvals(mut self, approvals: Vec<ApprovalResponse>) -> Self {
        self.approvals = Arc::new(Mutex::new(if approvals.is_empty() {
            vec![ApprovalResponse::Approved]
        } else {
            approvals
        }));
        self
    }
}

impl Shell for MockShell {
    fn observe(&self) -> Result<WorldState> {
        Ok(WorldState {
            cwd: current_cwd(),
            files: Vec::new(),
            last_command: None,
            notes: Vec::new(),
        })
    }

    fn capture_state(&self, scope: &HashScope) -> Result<StateSnapshot> {
        let files = recover_mutex(&self.files);
        Ok(StateSnapshot {
            scope: scope.clone(),
            cwd: current_cwd(),
            cwd_hash: if self.force_unchanged {
                "same".to_string()
            } else {
                Utc::now()
                    .timestamp_nanos_opt()
                    .unwrap_or_default()
                    .to_string()
            },
            files: scope
                .tracked_paths
                .iter()
                .map(|tracked| {
                    let content = files.get(&tracked.path).cloned();
                    PathState {
                        path: tracked.path.clone(),
                        exists: content.is_some(),
                        size: content.as_ref().map(|value| value.len() as u64),
                        modified_at: None,
                        content_hash: content
                            .as_ref()
                            .filter(|_| tracked.include_content)
                            .map(|value| {
                                let mut hasher = DefaultHasher::new();
                                value.hash(&mut hasher);
                                hasher.finish().to_string()
                            }),
                    }
                })
                .collect(),
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
        let changed_paths = after
            .files
            .iter()
            .filter(|after_path| {
                before
                    .files
                    .iter()
                    .find(|before_path| before_path.path == after_path.path)
                    .map(|before_path| {
                        before_path.exists != after_path.exists
                            || before_path.size != after_path.size
                            || before_path.content_hash != after_path.content_hash
                    })
                    .unwrap_or(true)
            })
            .map(|state| state.path.clone())
            .collect::<Vec<_>>();
        if before.cwd_hash == after.cwd_hash && changed_paths.is_empty() && expect_change {
            Ok(StateDelta::unchanged())
        } else {
            Ok(StateDelta {
                kind: if changed_paths.is_empty() {
                    StateDeltaKind::Unchanged
                } else {
                    StateDeltaKind::ChangedAsExpected
                },
                summary: if changed_paths.is_empty() {
                    "no state change detected".to_string()
                } else {
                    "changed".to_string()
                },
                changed_paths,
            })
        }
    }

    fn execute(&self, action: &Action) -> Result<ActionResult> {
        match action {
            Action::RunCommand {
                command,
                expect_change,
                state_scope,
                ..
            } => {
                if *expect_change {
                    if let Some(tracked) = state_scope.tracked_paths.first() {
                        let mut files = recover_mutex(&self.files);
                        let entry = files.entry(tracked.path.clone()).or_default();
                        if command.contains(">>") {
                            entry.push_str("command-output\n");
                        } else {
                            *entry = "command-output\n".to_string();
                        }
                    }
                }
                Ok(ActionResult::Command(CommandResult {
                    command: command.clone(),
                    cwd: current_cwd(),
                    stdout: "ok".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                    duration_ms: 1,
                    cancelled: false,
                    termination: None,
                    observed_paths: Vec::new(),
                }))
            }
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
                content: recover_mutex(&self.files)
                    .get(path)
                    .cloned()
                    .unwrap_or_else(|| "mock-content".to_string()),
                truncated: false,
            }),
            Action::IngestStructuredData { path, .. } => Ok(ActionResult::StructuredData {
                path: path.clone(),
                format: path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("csv")
                    .to_string(),
                headers: vec!["name".to_string(), "value".to_string()],
                rows: vec![StructuredDataRow {
                    row_number: 1,
                    values: vec!["mock".to_string(), "content".to_string()],
                }],
                total_rows: 1,
                truncated: false,
                extraction_method: "mock_structured_read".to_string(),
            }),
            Action::ExtractDocumentText {
                path,
                page_start,
                page_end,
                ..
            } => Ok(ActionResult::DocumentText {
                path: path.clone(),
                content: "mock-document-content".to_string(),
                truncated: false,
                format: path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("document")
                    .to_string(),
                extraction_method: "mock_extract".to_string(),
                page_range: page_start.map(|start_page| DocumentPageRange {
                    start_page,
                    end_page: page_end.unwrap_or(start_page),
                }),
                structured_rows_detected: false,
            }),
            Action::WriteFile { path, content, .. } => {
                let mut files = recover_mutex(&self.files);
                let existed_before = files.insert(path.clone(), content.clone()).is_some();
                Ok(ActionResult::FileWrite {
                    path: path.clone(),
                    bytes_written: content.len(),
                    created: !existed_before,
                    overwritten: existed_before,
                    appended: false,
                })
            }
            Action::AppendFile { path, content, .. } => {
                let mut files = recover_mutex(&self.files);
                let existed_before = files.contains_key(path);
                files.entry(path.clone())
                    .and_modify(|existing| existing.push_str(content))
                    .or_insert_with(|| content.clone());
                Ok(ActionResult::FileWrite {
                    path: path.clone(),
                    bytes_written: content.len(),
                    created: !existed_before,
                    overwritten: false,
                    appended: true,
                })
            }
            Action::RecordNote { note, .. } => {
                Ok(ActionResult::NoteRecorded { note: note.clone() })
            }
            Action::Respond { message, .. } => Ok(ActionResult::Response {
                message: message.clone(),
            }),
        }
    }

    fn constraints(&self) -> &[HardConstraint] {
        static CONSTRAINTS: [HardConstraint; 1] = [HardConstraint::DeleteOrKillRequireApproval];
        &CONSTRAINTS
    }

    fn capabilities(&self) -> ShellCapabilities {
        ShellCapabilities {
            can_execute_commands: true,
            can_read_files: true,
            can_write_files: true,
            can_search_files: true,
            can_extract_documents: true,
            can_write_notes: true,
            can_respond_text: true,
        }
    }

    fn request_approval(&self, _request: &ApprovalRequest) -> Result<ApprovalResponse> {
        let mut approvals = recover_mutex(&self.approvals);
        if approvals.len() > 1 {
            Ok(approvals.remove(0))
        } else {
            Ok(approvals
                .first()
                .cloned()
                .unwrap_or(ApprovalResponse::Approved))
        }
    }

    fn notify(&self, _message: &str) -> Result<()> {
        Ok(())
    }

    fn request_input(&self, _prompt: &str) -> Result<String> {
        let mut inputs = recover_mutex(&self.inputs);
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
