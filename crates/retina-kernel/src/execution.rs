use crate::result_helpers::summarize_verified_facts;
use crate::support::{
    ActionExecution, ContextAssemblyInput, EventSpec, StepDecision, StepSelectionContext,
    action_failure_reason, action_requires_approval, action_utility, approval_reason,
};
use crate::{Kernel, TaskLoopState, action_label};
use chrono::Utc;
use retina_tools::{ToolExecutor, ToolRegistry};
use retina_types::*;
use serde_json::json;

impl Kernel {
    pub(crate) fn select_action(
        &self,
        selection: StepSelectionContext<'_>,
        reflex_action: Option<Action>,
    ) -> Result<StepDecision> {
        let StepSelectionContext {
            task,
            intent,
            state,
            control,
            current_step,
            max_steps,
        } = selection;
        if let Some(action) = reflex_action {
            self.emit_event(EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::ReflexSelected,
                json!({ "action": action_label(&action) }),
            ))?;
            return Ok(StepDecision {
                action,
                task_complete: false,
                framing: None,
            });
        }

        let context = self.assemble_context(ContextAssemblyInput {
            task,
            state,
            last_result: state.last_result_json.clone(),
            operator_guidance: control.and_then(ExecutionControlHandle::take_guidance),
            current_step,
            max_steps,
        })?;
        let response = self.reasoner.reason(&ReasonRequest {
            tools: context.tools.clone(),
            context,
            constraints: self
                .shell
                .constraints()
                .iter()
                .map(|constraint| format!("{constraint:?}"))
                .collect(),
            max_tokens: Some(1536),
        })?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&response.action),
            TimelineEventType::ReasonerCalled,
            json!({
                "action": action_label(&response.action),
                "reasoning": response.reasoning,
                "framing": response.framing,
                "tokens": response.tokens_used,
                "task_complete": response.task_complete
            }),
        ))?;
        Ok(StepDecision {
            action: response.action,
            task_complete: response.task_complete,
            framing: response.framing,
        })
    }

    pub(crate) fn execute_action(
        &self,
        task: &Task,
        intent: &mut Intent,
        state: &TaskLoopState,
        step: &StepDecision,
        control: Option<&ExecutionControlHandle>,
    ) -> Result<ActionExecution> {
        let mut action = step.action.clone();
        intent.action = Some(action.clone());
        intent.expects_change = action.expects_change();
        intent.hash_scope = action.hash_scope();

        if let Some(outcome) = self.check_cancellation(
            task,
            Some(intent),
            Some(&action),
            control,
            "before pre-state capture",
        )? {
            return Ok(ActionExecution::Outcome(outcome));
        }

        let pre = self.shell.capture_state(&intent.hash_scope)?;
        self.emit_event(
            EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::PreStateCaptured,
                json!({ "scope": intent.hash_scope }),
            )
            .with_pre_hash(pre.cwd_hash.clone()),
        )?;

        if action_requires_approval(&action) {
            if let Some(outcome) = self.check_cancellation(
                task,
                Some(intent),
                Some(&action),
                control,
                "before approval prompt",
            )? {
                return Ok(ActionExecution::Outcome(outcome));
            }
            let response = self.shell.request_approval(&ApprovalRequest {
                action: action_label(&action),
                reason: approval_reason(&action),
            })?;
            if matches!(response, ApprovalResponse::Cancelled) {
                return self
                    .cancel_outcome(
                        task,
                        Some(intent),
                        Some(&action),
                        "task cancelled by operator during approval",
                    )
                    .map(ActionExecution::Outcome);
            }
            if matches!(response, ApprovalResponse::Denied) {
                return Ok(ActionExecution::Outcome(Outcome::Blocked(
                    synthesize_approval_denied_blocker(task, state, &action),
                )));
            }
            action.mark_approval_granted();
            intent.action = Some(action.clone());
        }

        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ActionDispatched,
            json!({ "action": action_label(&action) }),
        ))?;

        if let Some(outcome) = self.check_cancellation(
            task,
            Some(intent),
            Some(&action),
            control,
            "before action execution",
        )? {
            return Ok(ActionExecution::Outcome(outcome));
        }

        if let Some(reason) = self.invalid_mcp_fileish_reference_reason(&action)? {
            self.circuit_breaker.record_failure(intent);
            return Ok(ActionExecution::Outcome(Outcome::Failure(reason)));
        }

        let mut result = match &action {
            Action::SpawnAgent {
                prompt,
                allowed_tools,
                denied_tools,
                ..
            } => {
                let Some(runtime) = &self.agent_runtime else {
                    return Ok(ActionExecution::Outcome(Outcome::Blocked(
                        "local agent delegation is not available in this runtime".to_string(),
                    )));
                };
                ActionResult::DelegatedTask(runtime.spawn_local_agent(
                    &SpawnAgentRequest {
                        parent_task: task.clone(),
                        prompt: prompt.clone(),
                        allowed_tools: allowed_tools.clone(),
                        denied_tools: denied_tools.clone(),
                    },
                    control,
                )?)
            }
            Action::ListMcpResources { server, .. } => {
                let Some(runtime) = &self.mcp_runtime else {
                    return Ok(ActionExecution::Outcome(Outcome::Blocked(
                        "MCP runtime is not available in this runtime".to_string(),
                    )));
                };
                ActionResult::McpResources {
                    server: server.clone(),
                    resources: runtime.list_resources(server.as_deref())?,
                }
            }
            Action::ReadMcpResource { server, uri, .. } => {
                let Some(runtime) = &self.mcp_runtime else {
                    return Ok(ActionExecution::Outcome(Outcome::Blocked(
                        "MCP runtime is not available in this runtime".to_string(),
                    )));
                };
                ActionResult::McpResourceRead(runtime.read_resource(server, uri)?)
            }
            Action::CallMcpTool {
                server,
                tool,
                input_json,
                ..
            } => {
                let Some(runtime) = &self.mcp_runtime else {
                    return Ok(ActionExecution::Outcome(Outcome::Blocked(
                        "MCP runtime is not available in this runtime".to_string(),
                    )));
                };
                ActionResult::McpToolCall(runtime.call_tool(server, tool, input_json)?)
            }
            _ => match self.shell.execute_controlled(&action, control) {
                Ok(result) => result,
                Err(error) => {
                    self.circuit_breaker.record_failure(intent);
                    return Ok(ActionExecution::Outcome(Outcome::Failure(
                        error.to_string(),
                    )));
                }
            },
        };

        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ActionResultReceived,
            json!({ "result": result }),
        ))?;

        if let ActionResult::Command(command) = &result {
            if command.cancelled {
                return self
                    .cancel_outcome(
                        task,
                        Some(intent),
                        Some(&action),
                        command
                            .termination
                            .clone()
                            .unwrap_or_else(|| "task cancelled by operator".to_string()),
                    )
                    .map(ActionExecution::Outcome);
            }
        }

        if let Some(outcome) = self.check_cancellation(
            task,
            Some(intent),
            Some(&action),
            control,
            "after action result",
        )? {
            return Ok(ActionExecution::Outcome(outcome));
        }

        let post = self.shell.capture_state(&intent.hash_scope)?;
        self.emit_event(
            EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::PostStateCaptured,
                json!({ "scope": intent.hash_scope }),
            )
            .with_post_hash(post.cwd_hash.clone()),
        )?;

        let delta = self.shell.compare_state(&pre, &post, Some(&action))?;
        if let ActionResult::Command(command) = &mut result {
            if !delta.changed_paths.is_empty() {
                command.observed_paths = delta.changed_paths.clone();
            }
        }
        self.emit_event(
            EventSpec::new(
                task,
                Some(intent),
                Some(&action),
                TimelineEventType::StateDeltaComputed,
                json!({ "summary": delta.summary, "kind": delta.kind }),
            )
            .with_delta(delta.summary.clone()),
        )?;

        let utility = action_utility(&action, &result, &delta);
        let experience_id =
            self.record_experience(task, intent, &action, &result, &delta, utility)?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ExperiencePersisted,
            json!({ "experience_id": experience_id }),
        ))?;
        self.memory.update_utility(experience_id.clone(), utility)?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::UtilityScored,
            json!({ "experience_id": experience_id, "utility": utility }),
        ))?;
        let consolidation = self.memory.consolidate(&ConsolidationConfig::default())?;
        self.emit_event(EventSpec::new(
            task,
            Some(intent),
            Some(&action),
            TimelineEventType::ConsolidationCompleted,
            json!({
                "merged_knowledge": consolidation.merged_knowledge,
                "promoted_rules": consolidation.promoted_rules,
                "compacted_events": consolidation.compacted_events
            }),
        ))?;
        if consolidation.promoted_rules > 0 {
            self.reflex_engine.sync(self.memory.active_rules()?);
        }

        if let Some(reason) = action_failure_reason(&result, &delta, &action) {
            self.circuit_breaker.record_failure(intent);
            return Ok(ActionExecution::Outcome(Outcome::Failure(reason)));
        }
        Ok(ActionExecution::Outcome(Outcome::Success(result)))
    }

    pub(crate) fn record_experience(
        &self,
        task: &Task,
        intent: &Intent,
        action: &Action,
        result: &ActionResult,
        delta: &StateDelta,
        utility: f64,
    ) -> Result<ExperienceId> {
        let experience = Experience {
            id: None,
            session_id: task.session_id.clone(),
            task_id: task.id.clone(),
            intent_id: intent.id.clone(),
            action_summary: action_label(action),
            outcome: format!("{:?}", delta.kind),
            utility,
            created_at: Utc::now(),
            metadata: json!({
                "task": task.description,
                "action": action,
                "delta_kind": delta.kind,
                "delta": delta.summary,
                "result": result,
                "utility": utility,
            }),
        };
        self.memory.record_experience(&experience)
    }

    pub(crate) fn assemble_context(
        &self,
        input: ContextAssemblyInput<'_>,
    ) -> Result<AssembledContext> {
        let ContextAssemblyInput {
            task,
            state,
            last_result,
            operator_guidance,
            current_step,
            max_steps,
        } = input;
        let shell_constraints = self
            .shell
            .constraints()
            .iter()
            .map(|constraint| format!("{constraint:?}"))
            .collect::<Vec<_>>();
        let tool_policy = self.tool_policy.clone().with_task_metadata(&task.metadata);
        let mut registry = ToolRegistry::for_shell_capabilities(
            self.shell.capabilities(),
            self.agent_runtime.is_some(),
        );
        if let Some(runtime) = &self.mcp_runtime {
            registry = registry.with_mcp_snapshot(&runtime.snapshot()?);
        }
        let tools = ToolExecutor::new(registry)
            .with_policy(tool_policy)
            .available_tools();
        let identity = task
            .metadata
            .get("agent_role_prompt")
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                format!(
                    "You are Retina/{}. {}\nExecute tasks through the CLI shell.",
                    task.agent_id, value
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "You are Retina/{}. Execute tasks through the CLI shell.",
                    task.agent_id
                )
            });
        let task_text = task
            .metadata
            .get("agent_initial_prompt")
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("{value}\n\nCurrent task:\n{}", task.description))
            .unwrap_or_else(|| task.description.clone());

        Ok(AssembledContext {
            identity,
            task: task_text,
            task_state: self
                .build_task_state(task, state, current_step, max_steps)
                .with_constraints(shell_constraints),
            recent_context: task.recent_context.clone(),
            tools,
            memory_slice: Vec::new(),
            last_result,
            operator_guidance,
            current_step,
            max_steps,
        })
    }

    pub(crate) fn build_task_state(
        &self,
        task: &Task,
        state: &TaskLoopState,
        current_step: usize,
        max_steps: usize,
    ) -> TaskState {
        let output_written = state.artifact_references.iter().any(|artifact| {
            matches!(
                artifact.status.as_str(),
                "created" | "written" | "overwritten" | "appended" | "command_changed"
            )
        });
        let output_verified = state.working_sources.iter().any(|source| {
            source.role == "generated"
                && matches!(
                    source.status.as_str(),
                    "created" | "written" | "overwritten" | "appended" | "command_changed"
                )
                && source.preview_excerpt.is_some()
        }) || state.artifact_references.iter().any(|artifact| {
            matches!(
                artifact.status.as_str(),
                "read" | "structured_read" | "extracted"
            )
        });
        TaskState {
            goal: TaskGoal {
                objective: task.description.clone(),
                constraints: Vec::new(),
            },
            progress: TaskProgress {
                current_phase: describe_task_phase(state, current_step, max_steps),
                current_step,
                max_steps,
                completed_checkpoints: state.recent_steps.clone(),
                verified_facts: summarize_verified_facts(
                    &state.working_sources,
                    &state.artifact_references,
                ),
                output_written,
                output_verified,
            },
            recent_actions: state.recent_action_summaries.clone(),
            working_sources: state.working_sources.clone(),
            artifact_references: state.artifact_references.clone(),
            compaction: state.last_compaction_snapshot.clone(),
        }
    }

    pub(crate) fn check_cancellation(
        &self,
        task: &Task,
        intent: Option<&Intent>,
        action: Option<&Action>,
        control: Option<&ExecutionControlHandle>,
        checkpoint: &str,
    ) -> Result<Option<Outcome>> {
        if control
            .map(ExecutionControlHandle::is_cancel_requested)
            .unwrap_or(false)
        {
            return Ok(Some(self.cancel_outcome(task, intent, action, checkpoint)?));
        }
        Ok(None)
    }

    pub(crate) fn cancel_outcome(
        &self,
        task: &Task,
        intent: Option<&Intent>,
        action: Option<&Action>,
        reason: impl Into<String>,
    ) -> Result<Outcome> {
        let reason = reason.into();
        self.emit_event(EventSpec::new(
            task,
            intent,
            action,
            TimelineEventType::TaskCancelRequested,
            json!({ "reason": reason }),
        ))?;
        self.emit_event(EventSpec::new(
            task,
            intent,
            action,
            TimelineEventType::TaskCancelled,
            json!({ "reason": reason }),
        ))?;
        Ok(Outcome::Blocked("task cancelled by operator".to_string()))
    }

    pub(crate) fn emit_event(&self, spec: EventSpec<'_>) -> Result<()> {
        let event = TimelineEvent {
            event_id: EventId::new(),
            session_id: spec.task.session_id.clone(),
            task_id: spec.task.id.clone(),
            agent_id: spec.task.agent_id.clone(),
            timestamp: Utc::now(),
            event_type: spec.event_type,
            intent_id: spec.intent.map(|intent| intent.id.clone()),
            action_id: spec.action.map(Action::id),
            pre_state_hash: spec.pre_state_hash,
            post_state_hash: spec.post_state_hash,
            delta_summary: spec.delta_summary,
            duration_ms: spec.duration_ms,
            payload_json: spec.payload_json,
        };
        self.memory.append_timeline_event(&event)
    }

    fn invalid_mcp_fileish_reference_reason(&self, action: &Action) -> Result<Option<String>> {
        let Some(runtime) = &self.mcp_runtime else {
            return Ok(None);
        };
        let snapshot = runtime.snapshot()?;

        let target = match action {
            Action::ReadFile { path, .. }
            | Action::InspectPath { path, .. }
            | Action::ListDirectory { path, .. }
            | Action::ExtractDocumentText { path, .. } => path.to_str().map(str::to_string),
            Action::FindFiles { root, .. } | Action::SearchText { root, .. } => {
                root.to_str().map(str::to_string)
            }
            _ => None,
        };

        let Some(target) = target else {
            return Ok(None);
        };
        if let Some(locator) = resolve_mcp_locator_reference(&snapshot, &target) {
            return Ok(Some(format!(
                "{locator} is MCP output, not a filesystem path; use the MCP result directly or call another MCP tool instead"
            )));
        }
        Ok(None)
    }
}

fn describe_task_phase(state: &TaskLoopState, current_step: usize, max_steps: usize) -> String {
    if state.step_index == 0 {
        "starting".to_string()
    } else if current_step >= max_steps {
        "final step".to_string()
    } else {
        format!("working through step {} of {}", current_step, max_steps)
    }
}

fn synthesize_approval_denied_blocker(
    task: &Task,
    state: &TaskLoopState,
    action: &Action,
) -> String {
    let attempted = state
        .recent_action_summaries
        .iter()
        .rev()
        .take(3)
        .map(|summary| summary.action.clone())
        .collect::<Vec<_>>();
    let recent_attempts = if attempted.is_empty() {
        "no prior control steps were recorded".to_string()
    } else {
        attempted.join(", ")
    };
    let latest_status = state
        .working_sources
        .iter()
        .rev()
        .find(|source| source.kind == "command")
        .and_then(|source| source.preview_excerpt.clone())
        .unwrap_or_else(|| {
            "the latest command evidence still indicates the task is unresolved".to_string()
        });

    format!(
        "Automatic completion is blocked for '{}'. Earlier steps already attempted: {}. Latest command evidence: {}. The stronger step '{}' requires approval and was denied, so Retina cannot continue automatically.",
        task.description,
        recent_attempts,
        latest_status,
        action_label(action)
    )
}

fn resolve_mcp_locator_reference(snapshot: &McpRegistrySnapshot, target: &str) -> Option<String> {
    if target.starts_with("mcp-tool://") || target.starts_with("mcp-resource://") {
        return Some(target.to_string());
    }

    let (server, remainder) = target.split_once('/')?;
    let server_snapshot = snapshot.servers.iter().find(|entry| entry.name == server)?;

    if server_snapshot.tools.iter().any(|tool| tool.name == remainder) {
        return Some(format!("mcp-tool://{server}/{remainder}"));
    }

    if server_snapshot
        .resources
        .iter()
        .any(|resource| resource.uri == remainder)
    {
        return Some(format!("mcp-resource://{server}/{remainder}"));
    }

    None
}
