use crate::controller::{AgentController, InspectController, RunningTask};
use crate::output::{
    render_action_result, render_chat_event, render_memory_inspection, render_timeline_event,
};
use crate::runtime::root_manifest;
use retina_shell_cli::{CliShell, ScopedShell};
use retina_traits::{Memory, Shell};
use retina_types::*;
use std::io::{self, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender, TryRecvError},
};
use std::thread;
use std::time::Duration;

pub struct ChatSession {
    agent: AgentController,
    inspector: InspectController,
    debug_events: Arc<AtomicBool>,
    input_rx: Receiver<String>,
    prompt_rx: Receiver<PromptRequestEvent>,
}

impl ChatSession {
    pub fn new() -> Result<Self> {
        let debug_events = Arc::new(AtomicBool::new(false));
        let (input_tx, input_rx) = mpsc::channel();
        let (prompt_tx, prompt_rx) = mpsc::channel();
        spawn_input_reader(input_tx);
        let shell = Box::new(InteractiveShell::new(
            ScopedShell::new(CliShell::new(), root_manifest()?.authority),
            PromptBridge::new(prompt_tx),
        ));
        Ok(Self {
            agent: AgentController::new_with_streaming_and_shell(shell, debug_events.clone())?,
            inspector: InspectController::new()?,
            debug_events,
            input_rx,
            prompt_rx,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        println!(
            "Retina chat is live. Enter a task, or type /help, /exit, /timeline, /memory <query>, /debug. Type /s while a task is running to stop it. Use /guide <text> to steer the next step."
        );

        let mut active_task: Option<RunningTask> = None;
        let mut pending_prompt: Option<PendingPrompt> = None;
        let mut needs_prompt = true;

        loop {
            if let Some(prompt_event) = self.drain_prompt_requests(&active_task)? {
                pending_prompt = Some(prompt_event);
                needs_prompt = true;
            }

            if self.poll_task_completion(&mut active_task)? {
                pending_prompt = None;
                needs_prompt = true;
            }

            if needs_prompt {
                print_prompt(pending_prompt.as_ref());
                needs_prompt = false;
            }

            match self.input_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    let should_exit =
                        self.handle_input(line, &mut active_task, &mut pending_prompt)?;
                    needs_prompt = true;
                    if should_exit {
                        println!("Ending chat session.");
                        return Ok(());
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(KernelError::Execution(
                        "chat input channel disconnected".to_string(),
                    ));
                }
            }
        }
    }

    fn handle_input(
        &mut self,
        input: String,
        active_task: &mut Option<RunningTask>,
        pending_prompt: &mut Option<PendingPrompt>,
    ) -> Result<bool> {
        let line = input.trim();
        if line.is_empty() {
            return Ok(false);
        }

        if pending_prompt.is_some() {
            self.handle_prompt_input(line, active_task, pending_prompt)?;
            return Ok(false);
        }

        match line {
            "/exit" | "/quit" => return Ok(true),
            "/help" => {
                self.print_help();
                return Ok(false);
            }
            "/debug" => {
                let next = !self.debug_events.load(Ordering::Relaxed);
                self.debug_events.store(next, Ordering::Relaxed);
                println!(
                    "Debug event output {}.",
                    if next { "enabled" } else { "disabled" }
                );
                return Ok(false);
            }
            "/timeline" => {
                for event in self.inspector.recent_timeline(20)? {
                    print!("{}", render_timeline_event(&event));
                }
                io::stdout().flush()?;
                return Ok(false);
            }
            "/s" | "/stop" => {
                if let Some(task) = active_task.as_ref() {
                    task.control.request_cancel();
                    println!("Stop requested for the current task.");
                } else {
                    println!("No task is currently running.");
                }
                return Ok(false);
            }
            _ if line.starts_with("/guide") => {
                self.handle_guidance(line, active_task.as_ref())?;
                return Ok(false);
            }
            _ if line.starts_with("/memory") => {
                let query = line.trim_start_matches("/memory").trim();
                let (knowledge, experiences) = self.inspector.memory_lookup(query, 5)?;
                print!("{}", render_memory_inspection(&knowledge, &experiences));
                io::stdout().flush()?;
                return Ok(false);
            }
            _ => {}
        }

        if active_task.is_some() {
            println!(
                "A task is already running. Use /guide <text> to steer the next step or /s to stop it."
            );
            return Ok(false);
        }

        let task = self.agent.spawn_task(
            line.to_string(),
            ExecutionConfig {
                max_steps: 50,
                control: None,
            },
        );
        *active_task = Some(task);
        Ok(false)
    }

    fn handle_prompt_input(
        &mut self,
        line: &str,
        active_task: &mut Option<RunningTask>,
        pending_prompt: &mut Option<PendingPrompt>,
    ) -> Result<()> {
        let Some(prompt) = pending_prompt.take() else {
            return Ok(());
        };

        match line {
            "/s" | "/stop" => {
                if let Some(task) = active_task.as_ref() {
                    task.control.request_cancel();
                    self.log_control_event(
                        &task.task,
                        TimelineEventType::ApprovalPromptResolved,
                        serde_json::json!({ "resolution": "cancelled" }),
                    )?;
                }
                let _ = prompt
                    .response_tx
                    .send(PromptResponse::Approval(ApprovalResponse::Cancelled));
                println!("Task cancellation requested during prompt.");
                return Ok(());
            }
            _ if line.starts_with("/guide") => {
                println!("Guidance is not accepted while an approval prompt is active.");
                *pending_prompt = Some(prompt);
                return Ok(());
            }
            _ => {}
        }

        match prompt.kind {
            PromptKind::Approval => {
                let normalized = line.trim().to_lowercase();
                let response = match normalized.as_str() {
                    "y" | "yes" => ApprovalResponse::Approved,
                    "n" | "no" | "" => ApprovalResponse::Denied,
                    _ => {
                        println!("Respond with y/yes or n/no. Use /s to cancel the task.");
                        *pending_prompt = Some(prompt);
                        return Ok(());
                    }
                };
                if let Some(task) = active_task.as_ref() {
                    self.log_control_event(
                        &task.task,
                        TimelineEventType::ApprovalPromptResolved,
                        serde_json::json!({ "resolution": format!("{response:?}") }),
                    )?;
                }
                let _ = prompt.response_tx.send(PromptResponse::Approval(response));
            }
            PromptKind::TextInput => {
                if let Some(task) = active_task.as_ref() {
                    self.log_control_event(
                        &task.task,
                        TimelineEventType::ApprovalPromptResolved,
                        serde_json::json!({ "resolution": "text_input_received" }),
                    )?;
                }
                let _ = prompt
                    .response_tx
                    .send(PromptResponse::Text(line.to_string()));
            }
        }

        Ok(())
    }

    fn handle_guidance(&mut self, line: &str, active_task: Option<&RunningTask>) -> Result<()> {
        let Some(task) = active_task else {
            println!("No task is currently running.");
            return Ok(());
        };
        let guidance = line.trim_start_matches("/guide").trim();
        if guidance.is_empty() {
            println!("Usage: /guide <text>");
            return Ok(());
        }
        task.control.queue_guidance(guidance.to_string());
        self.log_control_event(
            &task.task,
            TimelineEventType::OperatorGuidanceQueued,
            serde_json::json!({ "guidance": guidance }),
        )?;
        println!("Queued guidance for the next planning step.");
        Ok(())
    }

    fn drain_prompt_requests(
        &mut self,
        active_task: &Option<RunningTask>,
    ) -> Result<Option<PendingPrompt>> {
        match self.prompt_rx.try_recv() {
            Ok(request) => {
                if let Some(task) = active_task.as_ref() {
                    self.log_control_event(
                        &task.task,
                        TimelineEventType::ApprovalPromptShown,
                        serde_json::json!({
                            "action": request
                                .approval_action_label()
                                .unwrap_or_else(|| "prompt".to_string())
                        }),
                    )?;
                }
                println!("{}", request.display_text);
                Ok(Some(PendingPrompt {
                    kind: request.kind,
                    response_tx: request.response_tx,
                }))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(KernelError::Execution(
                "prompt channel disconnected".to_string(),
            )),
        }
    }

    fn poll_task_completion(&mut self, active_task: &mut Option<RunningTask>) -> Result<bool> {
        let Some(task) = active_task.as_ref() else {
            return Ok(false);
        };

        match task.try_recv() {
            Ok(Ok(outcome)) => {
                match outcome {
                    Outcome::Success(result) => println!("{}", render_action_result(&result)),
                    Outcome::Failure(reason) => println!("Task failed: {reason}"),
                    Outcome::Blocked(reason) => println!("Task blocked: {reason}"),
                }
                *active_task = None;
                Ok(true)
            }
            Ok(Err(error)) => {
                println!("Task error: {error}");
                *active_task = None;
                Ok(true)
            }
            Err(TryRecvError::Empty) => Ok(false),
            Err(TryRecvError::Disconnected) => {
                println!("Task error: execution channel disconnected.");
                *active_task = None;
                Ok(true)
            }
        }
    }

    fn print_help(&self) {
        println!("Commands:");
        println!("  /help                Show this help");
        println!("  /exit                Exit chat");
        println!("  /timeline            Show recent timeline events");
        println!("  /memory <query>      Show recalled memory");
        println!("  /debug               Toggle verbose internal event output");
        println!("  /s                   Stop the current task");
        println!("  /guide <text>        Add one hint for the next planning step");
        println!("  any other text       Execute as a task");
    }

    fn log_control_event(
        &self,
        task: &Task,
        event_type: TimelineEventType,
        payload_json: serde_json::Value,
    ) -> Result<()> {
        self.inspector.append_timeline_event(&TimelineEvent {
            event_id: EventId::new(),
            session_id: task.session_id.clone(),
            task_id: task.id.clone(),
            agent_id: task.agent_id.clone(),
            timestamp: chrono::Utc::now(),
            event_type,
            intent_id: None,
            action_id: None,
            pre_state_hash: None,
            post_state_hash: None,
            delta_summary: None,
            duration_ms: None,
            payload_json,
        })
    }
}

fn print_prompt(pending_prompt: Option<&PendingPrompt>) {
    let prompt = if pending_prompt.is_some() {
        "approval> "
    } else {
        "retina> "
    };
    print!("{prompt}");
    let _ = io::stdout().flush();
}

fn spawn_input_reader(sender: Sender<String>) {
    thread::spawn(move || {
        loop {
            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                break;
            }
            let trimmed = input.trim().to_string();
            if sender.send(trimmed).is_err() {
                break;
            }
        }
    });
}

struct PendingPrompt {
    kind: PromptKind,
    response_tx: Sender<PromptResponse>,
}

#[derive(Clone, Copy)]
enum PromptKind {
    Approval,
    TextInput,
}

enum PromptResponse {
    Approval(ApprovalResponse),
    Text(String),
}

struct PromptRequestEvent {
    kind: PromptKind,
    display_text: String,
    response_tx: Sender<PromptResponse>,
}

impl PromptRequestEvent {
    fn approval_action_label(&self) -> Option<String> {
        if matches!(self.kind, PromptKind::Approval) {
            self.display_text
                .strip_prefix("Approve action '")
                .and_then(|value| value.split('\'').next())
                .map(ToOwned::to_owned)
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct PromptBridge {
    prompt_tx: Sender<PromptRequestEvent>,
}

impl PromptBridge {
    fn new(prompt_tx: Sender<PromptRequestEvent>) -> Self {
        Self { prompt_tx }
    }

    fn request(&self, kind: PromptKind, display_text: String) -> Result<PromptResponse> {
        let (response_tx, response_rx) = mpsc::channel();
        self.prompt_tx
            .send(PromptRequestEvent {
                kind,
                display_text,
                response_tx,
            })
            .map_err(|error| KernelError::Execution(error.to_string()))?;
        response_rx
            .recv()
            .map_err(|error| KernelError::Execution(error.to_string()))
    }
}

struct InteractiveShell<S> {
    inner: S,
    prompts: PromptBridge,
}

impl<S> InteractiveShell<S> {
    fn new(inner: S, prompts: PromptBridge) -> Self {
        Self { inner, prompts }
    }
}

impl<S: Shell> Shell for InteractiveShell<S> {
    fn observe(&self) -> Result<WorldState> {
        self.inner.observe()
    }

    fn capture_state(&self, scope: &HashScope) -> Result<StateSnapshot> {
        self.inner.capture_state(scope)
    }

    fn compare_state(
        &self,
        before: &StateSnapshot,
        after: &StateSnapshot,
        action: Option<&Action>,
    ) -> Result<StateDelta> {
        self.inner.compare_state(before, after, action)
    }

    fn execute(&self, action: &Action) -> Result<ActionResult> {
        self.inner.execute(action)
    }

    fn execute_controlled(
        &self,
        action: &Action,
        control: Option<&ExecutionControlHandle>,
    ) -> Result<ActionResult> {
        self.inner.execute_controlled(action, control)
    }

    fn constraints(&self) -> &[HardConstraint] {
        self.inner.constraints()
    }

    fn capabilities(&self) -> ShellCapabilities {
        self.inner.capabilities()
    }

    fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalResponse> {
        match self.prompts.request(
            PromptKind::Approval,
            format!(
                "Approve action '{}' because {}? [y/N]",
                request.action, request.reason
            ),
        )? {
            PromptResponse::Approval(response) => Ok(response),
            PromptResponse::Text(_) => Err(KernelError::Execution(
                "approval prompt received text input response".to_string(),
            )),
        }
    }

    fn notify(&self, message: &str) -> Result<()> {
        self.inner.notify(message)
    }

    fn request_input(&self, prompt: &str) -> Result<String> {
        match self
            .prompts
            .request(PromptKind::TextInput, format!("{prompt}:"))?
        {
            PromptResponse::Text(value) => Ok(value),
            PromptResponse::Approval(_) => Err(KernelError::Execution(
                "text prompt received approval response".to_string(),
            )),
        }
    }
}

pub struct StreamingMemory<M> {
    inner: M,
    debug_events: Arc<AtomicBool>,
}

impl<M> StreamingMemory<M> {
    pub fn new(inner: M, debug_events: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            debug_events,
        }
    }
}

impl<M: Memory> Memory for StreamingMemory<M> {
    fn append_timeline_event(&self, event: &TimelineEvent) -> Result<()> {
        print!(
            "{}",
            render_chat_event(event, self.debug_events.load(Ordering::Relaxed))
        );
        io::stdout().flush()?;
        self.inner.append_timeline_event(event)
    }

    fn record_experience(&self, exp: &Experience) -> Result<ExperienceId> {
        self.inner.record_experience(exp)
    }

    fn store_knowledge(&self, node: &KnowledgeNode) -> Result<KnowledgeId> {
        self.inner.store_knowledge(node)
    }

    fn link_knowledge(&self, from: KnowledgeId, to: KnowledgeId, relation: &str) -> Result<()> {
        self.inner.link_knowledge(from, to, relation)
    }

    fn store_rule(&self, rule: &ReflexiveRule) -> Result<RuleId> {
        self.inner.store_rule(rule)
    }

    fn register_tool(&self, tool: &ToolRecord) -> Result<ToolId> {
        self.inner.register_tool(tool)
    }

    fn append_state(&self, entry: &TimelineEvent) -> Result<()> {
        self.inner.append_state(entry)
    }

    fn recall_experiences(&self, query: &str, limit: usize) -> Result<Vec<Experience>> {
        self.inner.recall_experiences(query, limit)
    }

    fn recall_knowledge(&self, query: &str, limit: usize) -> Result<Vec<KnowledgeNode>> {
        self.inner.recall_knowledge(query, limit)
    }

    fn active_rules(&self) -> Result<Vec<ReflexiveRule>> {
        self.inner.active_rules()
    }

    fn find_tools(&self, capability: &str) -> Result<Vec<ToolRecord>> {
        self.inner.find_tools(capability)
    }

    fn recent_states(&self, limit: usize) -> Result<Vec<TimelineEvent>> {
        self.inner.recent_states(limit)
    }

    fn update_utility(&self, id: ExperienceId, utility: f64) -> Result<()> {
        self.inner.update_utility(id, utility)
    }

    fn update_knowledge(&self, id: KnowledgeId, update: &KnowledgeUpdate) -> Result<()> {
        self.inner.update_knowledge(id, update)
    }

    fn update_rule(&self, id: RuleId, update: &RuleUpdate) -> Result<()> {
        self.inner.update_rule(id, update)
    }

    fn consolidate(&self, config: &ConsolidationConfig) -> Result<ConsolidationReport> {
        self.inner.consolidate(config)
    }

    fn backup(&self, path: &std::path::Path) -> Result<()> {
        self.inner.backup(path)
    }
}
