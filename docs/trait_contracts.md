# Retina — Trait Contracts

> 5 boundaries. That's it. The kernel talks to the world through 5 traits.

---

## The Rule

The kernel depends on traits, not concrete types. But we don't abstract everything — only the boundaries that actually change between deployments. Retrieval strategy, state verification, human interaction — these are implementation details inside the 5 core traits, not separate contracts.

```
retina-kernel depends on retina-traits.
retina-kernel never imports rusqlite, wasmtime, fastembed, or any LLM SDK.
Concrete implementations live in their own crates and are injected at startup.
```

---

## The 5 Traits

```
┌─────────────────────────────────────────────────────────────┐
│                       retina-kernel                          │
│                                                             │
│  Reflex engine, execute loop, circuit breaker,              │
│  context assembler, router, utility scoring,                │
│  consolidation, promotion — this IS the agent.              │
│                                                             │
│  Talks to the outside world through:                        │
│                                                             │
│  ┌─────────┐ ┌──────────┐ ┌────────┐ ┌───────────┐ ┌─────────────┐
│  │  Shell   │ │ Reasoner │ │ Memory │ │Fabricator │ │  Transport  │
│  │         │ │          │ │        │ │           │ │             │
│  │ sense   │ │ think    │ │ record │ │ build     │ │ talk to     │
│  │ act     │ │ reflect  │ │ recall │ │ tools     │ │ other       │
│  │ verify  │ │          │ │ learn  │ │           │ │ agents      │
│  │ ask user│ │          │ │        │ │           │ │             │
│  └─────────┘ └──────────┘ └────────┘ └───────────┘ └─────────────┘
│       │            │           │           │              │
│    required     required    required    optional       optional
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

3 required, 2 optional. A minimal agent needs Shell + Reasoner + Memory. That's it.

---

## 1. Shell — "What world am I in?"

The Shell is the agent's body. It observes, acts, verifies state changes, and provides the human interface. State hashing and human interaction live INSIDE the shell — they're part of sensing and acting in a specific world, not independent concerns.

```rust
trait Shell: Send + Sync {
    // --- Senses ---
    /// What does the world look like right now?
    fn observe(&self) -> Result<WorldState>;

    /// Capture a hashable snapshot (for pre/post action verification).
    fn capture_state(&self, scope: &HashScope) -> Result<StateSnapshot>;

    /// Compare two snapshots — did the action change anything?
    fn compare_state(&self, before: &StateSnapshot, after: &StateSnapshot) -> Result<StateDelta>;

    // --- Hands ---
    /// Do something in the world.
    fn execute(&self, action: &Action) -> Result<ActionResult>;

    // --- Constraints ---
    /// What this shell physically cannot do (hard limits, not guidelines).
    fn constraints(&self) -> &[HardConstraint];

    /// What kinds of observations and actions are possible.
    fn capabilities(&self) -> ShellCapabilities;

    // --- Human interface ---
    /// Ask the user for approval (blocking).
    fn request_approval(&self, request: &ApprovalRequest) -> Result<ApprovalResponse>;

    /// Show the user a status update (non-blocking).
    fn notify(&self, message: &str) -> Result<()>;

    /// Ask the user for input.
    fn request_input(&self, prompt: &str) -> Result<String>;
}
```

| Implementation | Crate | Wraps |
|---|---|---|
| CLI | `retina-shell-cli` | Terminal, filesystem, processes, stdin/stdout |
| Browser | `retina-shell-browser` | DOM, forms, extension popup |
| Server | `retina-shell-server` | HTTP, webhooks, Slack for approval |

State hashing is the shell's job because only the shell knows what "state" means in its world. Human interaction is the shell's job because only the shell knows how to reach the user (terminal prompt, browser popup, Slack message).

---

## 2. Reasoner — "How do I think?"

The LLM abstraction. Context in, action decision out.

```rust
trait Reasoner: Send + Sync {
    /// Given context and tools, decide the next action.
    fn reason(&self, request: &ReasonRequest) -> Result<ReasonResponse>;

    /// Cheaper/faster reasoning for self-diagnosis on failure.
    /// Default: falls back to reason().
    fn reflect(&self, request: &ReasonRequest) -> Result<ReasonResponse> {
        self.reason(request)
    }

    /// Model metadata — context assembler uses this for token budgeting.
    fn capabilities(&self) -> ReasonerCapabilities;
}

struct ReasonRequest {
    context: AssembledContext,   // ~500 tokens: identity + task + tools + memory index + last result
    tools: Vec<ToolSchema>,
    constraints: Vec<String>,
    max_tokens: Option<u32>,
}

struct ReasonResponse {
    action: Action,
    reasoning: Option<String>,
    tokens_used: TokenUsage,
}

struct ReasonerCapabilities {
    max_context_tokens: u32,
    supports_tool_use: bool,
    supports_vision: bool,
    supports_caching: bool,
    cost_per_input_token: f64,
    cost_per_output_token: f64,
    model_id: String,
}
```

| Implementation | Crate | Wraps |
|---|---|---|
| Claude | `retina-llm-claude` | Anthropic SDK, prompt caching |
| Ollama | `retina-llm-ollama` | Local models via HTTP |
| OpenAI-compatible | `retina-llm-openai` | Any OpenAI-format endpoint |
| Mock | `retina-llm-mock` | Scripted responses for testing |

---

## 3. Memory — "What do I know?"

Storage AND retrieval. The Memory trait owns the full pipeline: persist, search, recall. How it searches internally (vectors, FTS5, hybrid, graph traversal) is the implementation's business. The kernel just says "record this" and "recall something relevant to this."

```rust
trait Memory: Send + Sync {
    // --- Record (write path) ---
    /// Store an action-outcome pair with initial utility.
    fn record_experience(&self, exp: &Experience) -> Result<ExperienceId>;

    /// Store a knowledge node (fact, lesson, pattern).
    fn store_knowledge(&self, node: &KnowledgeNode) -> Result<KnowledgeId>;

    /// Link two knowledge nodes.
    fn link_knowledge(&self, from: KnowledgeId, to: KnowledgeId, relation: &str) -> Result<()>;

    /// Store or update a reflexive rule.
    fn store_rule(&self, rule: &ReflexiveRule) -> Result<RuleId>;

    /// Register a fabricated tool.
    fn register_tool(&self, tool: &ToolRecord) -> Result<ToolId>;

    /// Append to the observation log.
    fn append_state(&self, entry: &StateEntry) -> Result<()>;

    // --- Recall (read path) ---
    /// Find relevant experiences for this situation.
    /// Implementation decides HOW (vectors, keywords, utility ranking, hybrid).
    fn recall_experiences(&self, query: &str, limit: usize) -> Result<Vec<Experience>>;

    /// Find relevant knowledge for this context.
    fn recall_knowledge(&self, query: &str, limit: usize) -> Result<Vec<KnowledgeNode>>;

    /// Get all active reflexive rules (loaded at boot into reflex engine).
    fn active_rules(&self) -> Result<Vec<ReflexiveRule>>;

    /// Find tools matching a capability description.
    fn find_tools(&self, capability: &str) -> Result<Vec<ToolRecord>>;

    /// Get recent observations.
    fn recent_states(&self, limit: usize) -> Result<Vec<StateEntry>>;

    // --- Learn (update path) ---
    /// Update utility score after an action's outcome is known.
    fn update_utility(&self, id: ExperienceId, utility: f64) -> Result<()>;

    /// Update confidence/activation on a knowledge node.
    fn update_knowledge(&self, id: KnowledgeId, update: &KnowledgeUpdate) -> Result<()>;

    /// Update a rule (e.g., after it fires and succeeds/fails).
    fn update_rule(&self, id: RuleId, update: &RuleUpdate) -> Result<()>;

    // --- Lifecycle ---
    /// Run consolidation: decay, dedupe, promote experiences to rules.
    fn consolidate(&self, config: &ConsolidationConfig) -> Result<ConsolidationReport>;

    /// Backup the memory store.
    fn backup(&self, path: &Path) -> Result<()>;
}
```

| Implementation | Crate | Wraps |
|---|---|---|
| SQLite (default) | `retina-memory-sqlite` | rusqlite + sqlite-vec + FTS5 + fastembed, all in one |
| In-memory | `retina-memory-inmem` | HashMap-based, for tests and ephemeral agents |

The SQLite implementation internally uses vectors, FTS5, embeddings, hybrid retrieval — but that's all behind `recall_experiences()` and `recall_knowledge()`. The kernel never knows. A different implementation could use pure keyword search, or a graph database, or nothing at all.

This is the key simplification: **the kernel says WHAT to remember and recall, the implementation decides HOW.**

---

## 4. Fabricator — "How do I build tools?" (Optional)

Compilation and sandboxed execution as one thing. You never compile to Wasm and run in Docker — they're always paired.

```rust
trait Fabricator: Send + Sync {
    /// Compile source code into a runnable tool.
    fn compile(&self, source: &ToolSource) -> Result<CompiledTool>;

    /// Execute a compiled tool with input, return output.
    fn execute_tool(&self, tool: &CompiledTool, input: &Value) -> Result<Value>;

    /// Test a compiled tool against its spec.
    fn test_tool(&self, tool: &CompiledTool, tests: &[ToolTest]) -> Result<TestReport>;

    /// What source languages can this fabricator handle?
    fn supported_languages(&self) -> &[SourceLanguage];

    /// Safety limits for tool execution.
    fn capabilities(&self) -> FabricatorCapabilities;
}

struct ToolSource {
    language: SourceLanguage,
    code: String,
    dependencies: Vec<Dependency>,
}

struct CompiledTool {
    binary: Vec<u8>,
    source_hash: String,
}

struct FabricatorCapabilities {
    allows_filesystem: bool,
    allows_network: bool,
    memory_limit_bytes: u64,
    timeout_ms: u64,
}
```

| Implementation | Crate | Wraps |
|---|---|---|
| Wasm | `retina-fabricator-wasm` | Wasmtime for sandbox + rustc/wasm32 for compilation |
| Process | `retina-fabricator-process` | Subprocess with resource limits |
| Mock | `retina-fabricator-mock` | Scripted tool outputs for testing |

Optional — a Tier 1 agent has `fabricator: None`. It works fine, it just can't build new tools.

---

## 5. Transport — "How do I talk to other agents?" (Optional)

Message passing between agents. Also how external systems (MCP, A2A) send tasks.

```rust
trait Transport: Send + Sync {
    /// Send a message to another agent.
    fn send(&self, to: &AgentId, message: &AgentMessage) -> Result<()>;

    /// Receive the next pending message (non-blocking).
    fn recv(&self) -> Result<Option<AgentMessage>>;

    /// Advertise this agent's capabilities.
    fn advertise(&self, card: &AgentCard) -> Result<()>;

    /// Discover other agents by capability.
    fn discover(&self, query: &str) -> Result<Vec<AgentCard>>;
}

struct AgentMessage {
    from: AgentId,
    to: AgentId,
    kind: MessageKind,      // TaskRequest, TaskResult, DataHandoff
    payload: Value,
    correlation_id: String,
}
```

| Implementation | Crate | Wraps |
|---|---|---|
| Local | `retina-transport-local` | mpsc channels, single machine |
| MCP | `retina-transport-mcp` | MCP protocol for Cursor/Claude |
| A2A | `retina-transport-a2a` | Google A2A protocol |
| gRPC | `retina-transport-grpc` | Distributed networks |
| Noop | `retina-transport-noop` | Independent agent, no network |

Optional — a standalone agent has `transport: None` (equivalent to Noop). Swap to Local and it becomes a network node.

---

## How the Kernel Wires Together

```rust
struct Kernel {
    // Required — every agent needs these
    shell: Box<dyn Shell>,
    reasoner: Box<dyn Reasoner>,
    memory: Box<dyn Memory>,

    // Optional — Tier 2+ capabilities
    fabricator: Option<Box<dyn Fabricator>>,
    transport: Option<Box<dyn Transport>>,

    // Kernel-owned (not traits, pure Rust logic)
    reflex_engine: ReflexEngine,
    circuit_breaker: CircuitBreaker,
    context_assembler: ContextAssembler,
    router: Router,
}
```

The execute loop:

```rust
impl Kernel {
    fn execute(&self, intent: Intent) -> Result<Outcome> {
        // 1. REFLEX CHECK (pure Rust, microseconds)
        let intent = self.reflex_engine.check(intent)?;

        // 2. CIRCUIT BREAKER (pure Rust, microseconds)
        if self.circuit_breaker.is_tripped(&intent) {
            return Ok(Outcome::Blocked { .. });
        }

        // 3. CAPTURE STATE (Shell — it knows what "state" means in its world)
        let pre = self.shell.capture_state(&intent.hash_scope())?;

        // 4. ACT (Shell)
        let result = self.shell.execute(&intent.action)?;

        // 5. VERIFY (Shell)
        let post = self.shell.capture_state(&intent.hash_scope())?;
        let delta = self.shell.compare_state(&pre, &post)?;

        // 6. RECORD (Memory)
        self.memory.record_experience(&Experience::from(&intent, &result, &delta))?;

        // 7. REFLECT (Reasoner — only on unexpected failure)
        if delta.is_unchanged() && intent.expects_change() {
            return self.handle_failure(&intent, &result);
        }

        // 8. UPDATE UTILITY (Memory)
        self.memory.update_utility(intent.experience_id, delta.utility_score())?;

        Ok(Outcome::Success(result))
    }
}
```

3 traits touched in the hot path: Shell, Memory, Reasoner (only on failure). That's the core loop. Fabricator only activates when the agent needs a new tool. Transport only activates in network mode.

---

## Wiring Examples

### Tier 1 — Minimal agent

```rust
fn main() {
    let shell = CliShell::new()?;
    let reasoner = ClaudeReasoner::new(&api_key)?;
    let memory = SqliteMemory::open("~/.retina/root/agent.db")?;

    let kernel = Kernel::new(shell, reasoner, memory, None, None);
    kernel.run()?;
}
```

### Tier 2 — Agent with tool fabrication

```rust
fn main() {
    let shell = CliShell::new()?;
    let reasoner = ClaudeReasoner::new(&api_key)?;
    let memory = SqliteMemory::open("~/.retina/root/agent.db")?;
    let fabricator = WasmFabricator::new()?;

    let kernel = Kernel::new(shell, reasoner, memory, Some(fabricator), None);
    kernel.run()?;
}
```

### Tier 3 — Network root

```rust
fn main() {
    let shell = CliShell::new()?;
    let reasoner = ClaudeReasoner::new(&api_key)?;
    let memory = SqliteMemory::open("~/.retina/root/agent.db")?;
    let fabricator = WasmFabricator::new()?;
    let transport = LocalTransport::new()?;

    let kernel = Kernel::new(shell, reasoner, memory, Some(fabricator), Some(transport));
    kernel.run()?;
}
```

### Browser extension — Minimal specialist

```rust
fn init() {
    let shell = BrowserShell::new()?;
    let reasoner = OpenAIReasoner::new(&key)?;
    let memory = InMemoryStore::new();

    let kernel = Kernel::new(shell, reasoner, memory, None, None);
    kernel.run()?;
}
```

---

## What Stays in the Kernel (Not Pluggable)

| Component | Why it's fixed |
|---|---|
| **Reflex engine** | The kernel's behavioral intelligence. Pure Rust condition matching. This is what makes Retina different. |
| **Circuit breaker** | Failure detection and blocking. Pure logic. |
| **Context assembler** | Builds ~500 token prompts. Core strategy, not an impl detail. |
| **Router** | Task decomposition, capability matching, spawn decisions. |
| **Consolidation** | Experience → knowledge → reflex promotion. The learning algorithm. |
| **Utility scoring** | Q-value updates. The kernel's value system. |

These are the agent. Everything else is plumbing.

---

## Crate Layout

```
retina-traits            Trait definitions only. No deps beyond serde.
retina-kernel            Depends only on retina-traits. Pure logic.

retina-memory-sqlite     Memory impl (rusqlite + sqlite-vec + FTS5 + fastembed inside)
retina-memory-inmem      Memory impl (HashMap, for tests)
retina-llm-claude        Reasoner impl
retina-llm-ollama        Reasoner impl
retina-llm-openai        Reasoner impl
retina-fabricator-wasm   Fabricator impl (wasmtime + rustc)
retina-transport-local   Transport impl
retina-transport-mcp     Transport impl
retina-shell-cli         Shell impl
retina-shell-browser     Shell impl
retina-shell-server      Shell impl

retina-cli               Binary: wires CLI shell + SQLite + Claude + optional fabricator/transport
```

~12 crates total, down from ~25. Each one has a clear reason to exist.

---

## Build Order

| Phase | What | Result |
|---|---|---|
| **1** | `retina-traits` + `retina-kernel` | Traits defined, kernel compiles against them, testable with mocks |
| **2** | `retina-memory-sqlite` + `retina-shell-cli` + `retina-llm-claude` | First runnable agent: can think, act, remember |
| **3** | Kernel: reflection, consolidation, reflex promotion | Agent can recover from failures and learn from experience |
| **4** | `retina-fabricator-wasm` | Agent can build its own tools |
| **5** | `retina-transport-local` + kernel router | Agent can spawn specialists, becomes a network |
| **6** | `retina-cli` | Packaged behind `retina run`, `retina agents`, `retina stats` |
| **7** | `retina-transport-mcp`, `retina-llm-ollama`, alternative impls | Proves the abstractions work with second implementations |
