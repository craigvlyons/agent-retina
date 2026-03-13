use crate::controller::{AgentController, InspectController};
use crate::output::{render_action_result, render_memory_inspection, render_timeline_event};
use retina_traits::Memory;
use retina_types::*;
use std::io::{self, Write};
use std::path::Path;

pub struct ChatSession {
    agent: AgentController,
    inspector: InspectController,
}

impl ChatSession {
    pub fn new() -> Result<Self> {
        Ok(Self {
            agent: AgentController::new(true)?,
            inspector: InspectController::new()?,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        println!(
            "Retina chat is live. Enter a task, or type /help, /exit, /timeline, /memory <query>."
        );
        loop {
            print!("retina> ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
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
                    println!("  any other text       Execute as a task");
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

            let outcome = self.agent.execute_task(line.to_string())?;
            match outcome {
                Outcome::Success(result) => println!("{}", render_action_result(&result)),
                Outcome::Failure(reason) => println!("Task failed: {reason}"),
                Outcome::Blocked(reason) => println!("Task blocked: {reason}"),
            }
        }
    }
}

pub struct StreamingMemory<M> {
    inner: M,
}

impl<M> StreamingMemory<M> {
    pub fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<M: Memory> Memory for StreamingMemory<M> {
    fn append_timeline_event(&self, event: &TimelineEvent) -> Result<()> {
        print!("{}", render_timeline_event(event));
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
