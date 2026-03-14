// File boundary: keep lib.rs limited to public trait surfaces and top-level API
// wiring. Move helper implementations and feature-specific logic into modules.
use retina_types::*;
use serde_json::Value;
use std::path::Path;

pub trait Shell: Send + Sync {
    fn observe(&self) -> Result<WorldState>;
    fn capture_state(&self, scope: &HashScope) -> Result<StateSnapshot>;
    fn compare_state(
        &self,
        before: &StateSnapshot,
        after: &StateSnapshot,
        action: Option<&Action>,
    ) -> Result<StateDelta>;
    fn execute(&self, action: &Action) -> Result<ActionResult>;
    fn execute_controlled(
        &self,
        action: &Action,
        control: Option<&ExecutionControlHandle>,
    ) -> Result<ActionResult> {
        let _ = control;
        self.execute(action)
    }
    fn constraints(&self) -> &[HardConstraint];
    fn capabilities(&self) -> ShellCapabilities;
    fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalResponse>;
    fn notify(&self, message: &str) -> Result<()>;
    fn request_input(&self, prompt: &str) -> Result<String>;
}

pub trait Reasoner: Send + Sync {
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse>;

    fn reflect(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        self.reason(request)
    }

    fn capabilities(&self) -> ReasonerCapabilities;
}

pub trait Memory: Send + Sync {
    fn append_timeline_event(&self, event: &TimelineEvent) -> Result<()>;
    fn record_experience(&self, exp: &Experience) -> Result<ExperienceId>;
    fn store_knowledge(&self, node: &KnowledgeNode) -> Result<KnowledgeId>;
    fn link_knowledge(&self, from: KnowledgeId, to: KnowledgeId, relation: &str) -> Result<()>;
    fn store_rule(&self, rule: &ReflexiveRule) -> Result<RuleId>;
    fn register_tool(&self, tool: &ToolRecord) -> Result<ToolId>;
    fn append_state(&self, entry: &TimelineEvent) -> Result<()>;
    fn recall_experiences(&self, query: &str, limit: usize) -> Result<Vec<Experience>>;
    fn recall_knowledge(&self, query: &str, limit: usize) -> Result<Vec<KnowledgeNode>>;
    fn active_rules(&self) -> Result<Vec<ReflexiveRule>>;
    fn find_tools(&self, capability: &str) -> Result<Vec<ToolRecord>>;
    fn recent_states(&self, limit: usize) -> Result<Vec<TimelineEvent>>;
    fn update_utility(&self, id: ExperienceId, utility: f64) -> Result<()>;
    fn update_knowledge(&self, id: KnowledgeId, update: &KnowledgeUpdate) -> Result<()>;
    fn update_rule(&self, id: RuleId, update: &RuleUpdate) -> Result<()>;
    fn consolidate(&self, config: &ConsolidationConfig) -> Result<ConsolidationReport>;
    fn backup(&self, path: &Path) -> Result<()>;
}

pub trait Fabricator: Send + Sync {
    fn compile(&self, source: &ToolSource) -> Result<CompiledTool>;
    fn execute_tool(&self, tool: &CompiledTool, input: &Value) -> Result<Value>;
    fn test_tool(&self, tool: &CompiledTool, tests: &[ToolTest]) -> Result<TestReport>;
    fn supported_languages(&self) -> &[SourceLanguage];
    fn capabilities(&self) -> FabricatorCapabilities;
}

pub trait Transport: Send + Sync {
    fn send(&self, to: &AgentId, message: &AgentMessage) -> Result<()>;
    fn recv(&self) -> Result<Option<AgentMessage>>;
    fn advertise(&self, card: &AgentCard) -> Result<()>;
    fn discover(&self, query: &str) -> Result<Vec<AgentCard>>;
}
