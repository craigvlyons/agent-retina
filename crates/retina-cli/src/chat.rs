use crate::controller::{AgentController, InspectController};
use crate::output::{
    render_action_result, render_chat_event, render_memory_inspection, render_timeline_event,
};
use crate::runtime::root_manifest;
use retina_shell_cli::{CliShell, ScopedShell};
use retina_traits::{Memory, Shell};
use retina_types::*;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
    Arc,
    Mutex,
};
use std::thread;

pub struct ChatSession {
    agent: AgentController,
    inspector: InspectController,
    debug_events: Arc<AtomicBool>,
    terminal: Arc<ChatTerminal>,
}

impl ChatSession {
    pub fn new() -> Result<Self> {
        let debug_events = Arc::new(AtomicBool::new(false));
        let terminal = ChatTerminal::new();
        let shell = Box::new(InteractiveShell::new(
            ScopedShell::new(CliShell::new(), root_manifest()?.authority),
            terminal.clone(),
        ));
        Ok(Self {
            agent: AgentController::new_with_streaming_and_shell(shell, debug_events.clone())?,
            inspector: InspectController::new()?,
            debug_events,
            terminal,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        println!(
            "Retina chat is live. Enter a task, or type /help, /exit, /timeline, /memory <query>, /debug. Type /s while a task is running to stop after the current step."
        );
        loop {
            let input = self.terminal.read_line("retina> ")?;
            let line = input.trim();
            if line.is_empty() {
                continue;
            }
            match line {
                "/exit" | "/quit" => {
                    println!("Ending chat session.");
                    return Ok(());
                }
                "/help" => {
                    println!("Commands:");
                    println!("  /help                Show this help");
                    println!("  /exit                Exit chat");
                    println!("  /timeline            Show recent timeline events");
                    println!("  /memory <query>      Show recalled memory");
                    println!("  /debug               Toggle verbose internal event output");
                    println!("  /s                   Stop the current task after the current step");
                    println!("  any other text       Execute as a task");
                    continue;
                }
                "/s" | "/stop" => {
                    println!("No task is currently running.");
                    continue;
                }
                "/debug" => {
                    let next = !self.debug_events.load(Ordering::Relaxed);
                    self.debug_events.store(next, Ordering::Relaxed);
                    println!(
                        "Debug event output {}.",
                        if next { "enabled" } else { "disabled" }
                    );
                    continue;
                }
                "/timeline" => {
                    for event in self.inspector.recent_timeline(20)? {
                        print!("{}", render_timeline_event(&event));
                    }
                    continue;
                }
                _ if line.starts_with("/memory") => {
                    let query = line.trim_start_matches("/memory").trim();
                    let (knowledge, experiences) = self.inspector.memory_lookup(query, 5)?;
                    print!("{}", render_memory_inspection(&knowledge, &experiences));
                    continue;
                }
                _ => {}
            }

            self.terminal.begin_task();
            let outcome = match self.agent.execute_task_with_config(
                line.to_string(),
                ExecutionConfig {
                    max_steps: 6,
                    pause_before_continuation: true,
                },
            ) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.terminal.end_task();
                    println!("Task error: {error}");
                    continue;
                }
            };
            self.terminal.end_task();
            match outcome {
                Outcome::Success(result) => println!("{}", render_action_result(&result)),
                Outcome::Failure(reason) => println!("Task failed: {reason}"),
                Outcome::Blocked(reason) => println!("Task blocked: {reason}"),
            }
        }
    }
}

struct ChatTerminal {
    receiver: Mutex<Receiver<String>>,
    task_running: AtomicBool,
    cancel_requested: AtomicBool,
}

impl ChatTerminal {
    fn new() -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let terminal = Arc::new(Self {
            receiver: Mutex::new(receiver),
            task_running: AtomicBool::new(false),
            cancel_requested: AtomicBool::new(false),
        });
        let task_running = Arc::clone(&terminal);
        thread::spawn(move || loop {
            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                break;
            }
            let trimmed = input.trim().to_string();
            if task_running.task_running.load(Ordering::Relaxed)
                && matches!(trimmed.as_str(), "/s" | "/stop")
            {
                task_running.cancel_requested.store(true, Ordering::Relaxed);
                continue;
            }
            if sender.send(trimmed).is_err() {
                break;
            }
        });
        terminal
    }

    fn read_line(&self, prompt: &str) -> Result<String> {
        print!("{prompt}");
        io::stdout().flush()?;
        self.receiver
            .lock()
            .expect("chat receiver mutex poisoned")
            .recv()
            .map_err(|error| KernelError::Execution(error.to_string()))
    }

    fn begin_task(&self) {
        self.cancel_requested.store(false, Ordering::Relaxed);
        self.task_running.store(true, Ordering::Relaxed);
    }

    fn end_task(&self) {
        self.task_running.store(false, Ordering::Relaxed);
        self.cancel_requested.store(false, Ordering::Relaxed);
    }

    fn take_cancel_request(&self) -> bool {
        self.cancel_requested.swap(false, Ordering::Relaxed)
    }
}

struct InteractiveShell<S> {
    inner: S,
    terminal: Arc<ChatTerminal>,
}

impl<S> InteractiveShell<S> {
    fn new(inner: S, terminal: Arc<ChatTerminal>) -> Self {
        Self { inner, terminal }
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

    fn constraints(&self) -> &[HardConstraint] {
        self.inner.constraints()
    }

    fn capabilities(&self) -> ShellCapabilities {
        self.inner.capabilities()
    }

    fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalResponse> {
        let input = self.terminal.read_line(&format!(
            "Approve action '{}' because {}? [y/N]: ",
            request.action, request.reason
        ))?;
        let normalized = input.trim().to_lowercase();
        if normalized == "y" || normalized == "yes" {
            Ok(ApprovalResponse::Approved)
        } else {
            Ok(ApprovalResponse::Denied)
        }
    }

    fn notify(&self, message: &str) -> Result<()> {
        self.inner.notify(message)
    }

    fn request_input(&self, prompt: &str) -> Result<String> {
        if prompt.contains("type /stop to cancel") {
            if self.terminal.take_cancel_request() {
                return Ok("/stop".to_string());
            }
            return Ok(String::new());
        }
        self.terminal.read_line(&format!("{prompt}: "))
    }
}

pub struct StreamingMemory<M> {
    inner: M,
    debug_events: Arc<AtomicBool>,
}

impl<M> StreamingMemory<M> {
    pub fn new(inner: M, debug_events: Arc<AtomicBool>) -> Self {
        Self { inner, debug_events }
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

    fn backup(&self, path: &Path) -> Result<()> {
        self.inner.backup(path)
    }
}
