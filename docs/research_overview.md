# Agent Retina — Architecture & Direction

## What We're Building

You deploy one agent. It grows into a network.

**Retina** is a self-evolving agent kernel written in Rust. When you deploy it, you deploy a **seed** — a single lightweight process that understands how to observe, act, learn, and most importantly, **replicate**. As the user gives it work, Retina spawns specialized copies of itself. Each copy builds the tools it needs, learns its domain, and persists. Over time, the user has a personal network of agents — each one an expert in its area — all grown from the same kernel.

```
Day 1:   retina run "help me manage my email"
         → Retina spawns an email agent
         → Email agent fabricates IMAP tools
         → Email agent learns the user's inbox patterns

Day 7:   retina run "research this topic and write a report"
         → Retina spawns a research agent
         → Research agent fabricates web scraping tools
         → Research agent learns the user's preferred sources

Day 30:  retina run "download invoices from email and update the spreadsheet"
         → Retina routes to existing email agent (already knows the inbox)
         → Spawns a data agent for the spreadsheet part
         → Email agent passes invoice data to data agent
         → Data agent fabricates spreadsheet tools
         → Task completes using two specialists that already exist

Day 90:  The user has a network:
         ├── email agent      (4 tools, 200+ experiences, handles inbox autonomously)
         ├── research agent   (6 tools, 150+ experiences, knows preferred sources)
         ├── data agent       (3 tools, 80+ experiences, knows the spreadsheet formats)
         ├── code agent       (5 tools, 300+ experiences, knows the user's repos)
         └── ops agent        (2 tools, 50+ experiences, monitors deploys)
```

The model is a commodity. **The harness is the intelligence. The network is the product.**

---

## Core Philosophy

### 0. 5 Traits, Not 50

The kernel talks to the world through 5 trait contracts: Shell, Reasoner, Memory, Fabricator (optional), Transport (optional). That's it. Retrieval strategy, state hashing, embeddings, human interaction — all implementation details hidden inside these 5 boundaries. No over-abstraction. See [trait_contracts.md](trait_contracts.md).

### 1. Deploy a Seed, Grow a Network

Retina is not a tool you configure. It's an organism you deploy. One binary grows into a network of specialists. The user doesn't decide what agents to create — the system figures out what it needs based on the work it's given.

### 2. The Harness Is the Brain, Not the Context Window

Agent intelligence lives in the Rust harness — reflexes, tool selection, circuit breakers, state verification — NOT in a giant system prompt stuffed with memories. The context window is a tiny scratchpad. The harness does the heavy lifting before the LLM ever sees the task.

### 3. Pull, Don't Push

Memory is never pre-loaded into context. Each agent starts near-empty (~500 tokens) and pulls specific memories on demand. Like a brain — you don't load all your memories into consciousness. You recall one thing when something triggers it.

### 4. Spawn, Don't Accumulate

Long-running tasks don't grow a single agent's context. They spawn or route to specialist agents with their own context. Each specialist handles its domain, returns results, and keeps learning. The orchestrator carries the plan, not the execution history.

### 5. Build, Don't Configure

Retina doesn't ship with adapters for email, browsers, databases, etc. When an agent needs to interact with something new, it fabricates the tool. The email agent builds IMAP tools. The data agent builds CSV parsers. The capability grows organically from the work.

### 6. Reflexes, Not Retrieval

Lessons learned become compiled behavioral rules in the harness — not text memories stuffed into prompts. The agent doesn't "remember" that Gmail marks messages read on FETCH. The harness already uses PEEK before the LLM is invoked.

---

## The Network Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                         HUMAN                                     │
│                    retina run "task"                               │
│                    retina chat "question"                          │
│                    or: another system via MCP/A2A                  │
└────────────────────────────┬─────────────────────────────────────┘
                             │
┌────────────────────────────▼─────────────────────────────────────┐
│                      ROOT AGENT                                   │
│                                                                   │
│  The router. The orchestrator. Knows what agents exist,           │
│  what they're good at, and when to spawn new ones.                │
│                                                                   │
│  Context: ~500 tokens (task + agent index + routing decision)     │
│  Memory: agent registry, task history, routing patterns           │
│  Job: decompose → route → spawn if needed → collect results       │
│                                                                   │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  AGENT REGISTRY (what exists in the network)               │  │
│  │                                                            │  │
│  │  email     │ 4 tools │ 200 exp │ running  │ "inbox, IMAP" │  │
│  │  research  │ 6 tools │ 150 exp │ idle     │ "web, papers" │  │
│  │  data      │ 3 tools │ 80 exp  │ idle     │ "csv, excel"  │  │
│  │  code      │ 5 tools │ 300 exp │ running  │ "git, rust"   │  │
│  └────────────────────────────────────────────────────────────┘  │
└───────────┬──────────┬──────────┬──────────┬─────────────────────┘
            │          │          │          │
   ┌────────▼───┐ ┌────▼────┐ ┌──▼─────┐ ┌─▼──────┐
   │ EMAIL      │ │RESEARCH │ │ DATA   │ │ CODE   │  ... grows over time
   │ AGENT      │ │ AGENT   │ │ AGENT  │ │ AGENT  │
   │            │ │         │ │        │ │        │
   │ Own kernel │ │ Own     │ │ Own    │ │ Own    │
   │ Own memory │ │ kernel  │ │ kernel │ │ kernel │
   │ Own tools  │ │ Own mem │ │ Own mem│ │ Own mem│
   │ Own reflex │ │ Own tool│ │ Own tl │ │ Own tl │
   │            │ │         │ │        │ │        │
   │ Persists   │ │Persists │ │Persist │ │Persist │
   │ across     │ │ across  │ │ across │ │ across │
   │ sessions   │ │sessions │ │sessions│ │sessions│
   └────────────┘ └─────────┘ └────────┘ └────────┘
```

### How the Network Grows

The root agent doesn't have a predefined list of specialist types. It grows them organically:

```
1. User: "check my email for invoices"

2. Root agent checks registry: any agent with email capability?
   → No agents exist yet.

3. Root agent decides: this is a distinct domain. Spawn a specialist.
   retina spawn --domain "email"
                --capability "email management, IMAP, inbox monitoring"
                --task "check inbox for invoices"

4. New email agent boots:
   - Fresh kernel (own reflex engine, fabricator, memory)
   - Task description: "check inbox for invoices"
   - No tools yet — it needs to figure out how to read email

5. Email agent's first action:
   - Realizes it needs IMAP access
   - Fabricator writes an IMAP client tool
   - Compiles to .wasm, tests, registers
   - Now it can read email

6. Email agent completes task, returns results to root agent.
   Agent stays alive (idle state) with its memory + tools intact.

7. Next time user mentions email:
   Root agent routes directly to existing email agent.
   No spawning. No tool fabrication. The specialist already knows how.
```

### Agent-to-Agent Communication

Agents in the network don't share context. They pass **messages**:

```
Root Agent → Email Agent:
  { task: "find invoices from last week",
    respond_to: "root",
    format: "structured" }

Email Agent → Root Agent:
  { status: "complete",
    result: [
      { sender: "vendor@co.com", subject: "Invoice #4521", amount: "$1,200", date: "2026-03-07" },
      { sender: "saas@tool.io", subject: "March Invoice", amount: "$49", date: "2026-03-10" }
    ],
    tools_used: ["imap_inbox_scan", "invoice_parser"],
    tokens_spent: 1200 }

Root Agent → Data Agent:
  { task: "add these rows to the Master spreadsheet",
    data: [... invoice results from email agent ...],
    respond_to: "root" }
```

Messages are small. Results are structured. No conversation history shipped between agents. Each agent's context stays clean.

### When Agents Collaborate

For tasks that span multiple domains, the root agent orchestrates a pipeline:

```
User: "Download invoices from email, extract the amounts,
       update the spreadsheet, and send a summary to my accountant"

Root Agent decomposes:
  Step 1: email agent    → find and download invoices
  Step 2: data agent     → extract amounts, update spreadsheet
  Step 3: email agent    → send summary to accountant

  Dependencies: step 2 needs output from step 1
                step 3 needs output from step 2

Root Agent executes:
  1. email agent completes → returns invoice files + metadata
  2. data agent receives invoice data → updates spreadsheet → returns summary
  3. email agent receives summary → composes email → HUMAN PULSE GATE → sends

Root Agent context throughout: ~800 tokens (plan + current step + last result)
No agent ever sees the full task history. Each sees only its step.
```

---

## The Kernel (Every Agent Runs This)

Every agent in the network — root, email, research, data — runs the same kernel code. The difference is what's in their memory, what tools they've built, and what implementations are plugged in for the 5 traits.

### The Execute Loop

```rust
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
```

3 traits in the hot path: Shell, Memory, Reasoner (only on failure). Steps 1-2 are pure kernel logic. A mature agent is almost pure harness — step 7 fires less and less as reflexes handle known patterns.

### Context Assembler

Each LLM call gets a minimal, assembled context:

```
┌─────────────────────────────────────────────────┐
│ WHAT THE LLM SEES (~500-1,000 tokens)           │
│                                                  │
│ Identity: "You are Retina/email. You handle      │
│ email tasks." (50 tokens)                        │
│                                                  │
│ Current task: "Find invoices from last week"     │
│ (100 tokens)                                     │
│                                                  │
│ Available tools: [imap_scan, invoice_parser,     │
│ send_email] — filtered for this task (200 tok)   │
│                                                  │
│ Memory index: "You have 200 past experiences,    │
│ 47 email-specific patterns. Use recall() to      │
│ access." (50 tokens)                             │
│                                                  │
│ Last result: "imap_scan returned 12 unread       │
│ messages" (200 tokens)                           │
│                                                  │
│ TOTAL: ~600 tokens. That's it.                   │
└─────────────────────────────────────────────────┘
```

Everything else lives in the harness or in SQLite, accessible via pull tools.

---

## Memory Architecture

### Where Memory Lives

```
IN THE HARNESS (Rust, zero tokens):
├── Reflex Engine       behavioral rules compiled from experience
├── Tool Weights        context-dependent preference scores
├── Circuit Breakers    "stop after N failures on same action"
├── Pre-action Checks   route around known failures
└── Agent Registry      what specialists exist and what they're good at

IN SQLITE (never in context, per-agent):
├── experiences         action-outcome pairs with Q-value utility
├── knowledge_graph     linked nodes with activation spreading
├── tool_registry       fabricated tools + source + test status
├── reflexive_rules     promoted from high-utility experiences
└── state_log           ring buffer of recent observations

IN CONTEXT (~500 tokens):
├── Identity            who am I, what's my domain
├── Task                what am I doing right now
├── Tools               filtered for current task
├── Memory index        what I COULD look up (not the data itself)
└── Last result         what just happened

PULL TOOLS (agent decides when):
├── recall(query)            → 1 relevant memory
├── remember(content)        → write to experience store
├── check_experience(ctx)    → known patterns for this situation
└── list_tools(capability)   → matching tools
```

### How Memory Becomes Behavior

```
EXPERIENCE → KNOWLEDGE → REFLEX

Step 1: Agent tries IMAP FETCH on Gmail → messages marked as read (unintended)
        Recorded: {action: "imap_fetch", outcome: "side_effect", utility: -0.7}

Step 2: Agent tries IMAP FETCH with PEEK → messages stay unread (correct)
        Recorded: {action: "imap_fetch_peek", outcome: "success", utility: +0.9}

Step 3: After N similar experiences, consolidation creates knowledge node:
        "Gmail: always use PEEK flag on FETCH to avoid marking messages read"

Step 4: Knowledge reaches high confidence → promoted to reflexive rule:
        condition: server="imap.gmail.com" AND command="FETCH"
        action: add_flag("PEEK")

Now the harness handles this in Rust. Zero tokens. Instant. The agent just "knows."
```

### Per-Agent Memory Isolation

```
~/.retina/
├── config.toml                    # API keys, global settings
├── root/
│   ├── agent.db                   # Root agent: routing patterns, agent registry
│   └── tools/                     # Unlikely to have many — root mainly routes
├── agents/
│   ├── email/
│   │   ├── agent.db               # IMAP quirks, sender patterns, inbox reflexes
│   │   └── tools/                 # imap_scan.wasm, invoice_parser.wasm, etc.
│   ├── research/
│   │   ├── agent.db               # Source preferences, search patterns
│   │   └── tools/                 # web_fetch.wasm, pdf_extract.wasm, etc.
│   ├── data/
│   │   ├── agent.db               # Schema knowledge, format patterns
│   │   └── tools/                 # csv_parse.wasm, xlsx_writer.wasm, etc.
│   └── code/
│       ├── agent.db               # Repo knowledge, build patterns
│       └── tools/                 # git_ops.wasm, test_runner.wasm, etc.
└── shared/
    └── promoted_tools/            # Tools promoted from any agent, available to all
```

Each agent's memory is private. No cross-contamination. The email agent's IMAP lessons don't pollute the data agent's retrieval. Tools that prove universally useful get promoted to shared (with human approval).

---

## Agent Lifecycle

### States

```
                spawn
    ┌──────────────────────────┐
    │                          ▼
    │    ┌─────────┐    ┌──────────┐    ┌─────────┐
    │    │ spawned  │───▶│ tooling  │───▶│ running │
    │    └─────────┘    └──────────┘    └────┬────┘
    │                                        │
    │         ┌──────────────────────────────┤
    │         │                              │
    │    ┌────▼────┐                   ┌─────▼──────┐
    │    │  idle   │                   │ persistent │
    │    └────┬────┘                   └─────┬──────┘
    │         │                              │
    │    ┌────▼─────┐                        │
    └────┤ archived │◀───────────────────────┘
         └──────────┘       (shutdown/timeout)
```

| State | Description |
|---|---|
| **spawned** | Kernel booted, task received, empty memory |
| **tooling** | Fabricating tools needed for its domain |
| **running** | Executing task, learning from outcomes |
| **idle** | Task complete, waiting for next assignment in its domain |
| **persistent** | Long-running agent (e.g., inbox monitor, deploy watcher) |
| **archived** | Shut down, memory preserved on disk for future reactivation |

### Auto-Spawn vs Manual Spawn

The root agent can spawn specialists automatically based on task analysis:

```rust
fn route_task(task: &Task) -> RoutingDecision {
    // Check if any existing agent matches this domain
    for agent in registry.active_agents() {
        if agent.capability_match(task) > 0.8 {
            return RoutingDecision::RouteToExisting(agent.id);
        }
    }

    // Check if any archived agent could handle this
    for agent in registry.archived_agents() {
        if agent.capability_match(task) > 0.8 {
            return RoutingDecision::Reactivate(agent.id);
        }
    }

    // No specialist exists — should we create one?
    if task.is_domain_specific() && task.likely_recurring() {
        return RoutingDecision::SpawnSpecialist {
            domain: task.inferred_domain(),
            capability: task.capability_description(),
        };
    }

    // One-off task — root agent handles it directly
    RoutingDecision::HandleDirectly
}
```

One-off tasks don't spawn agents. Recurring domain work does. The network grows only where it needs to.

---

## Interaction Models

### CLI

```bash
# The root agent handles routing automatically
retina run "check my email for invoices"
#  [routing] No email agent exists. Spawning...
#  [email] Booting. Fabricating IMAP tools...
#  [email] Connected to inbox. Found 3 invoices.
#  Done.

retina run "check my email for invoices"
#  [routing] → email agent (200 experiences, 4 tools)
#  [email] Found 2 new invoices since last check.
#  Done.

# Spawn a persistent agent explicitly
retina spawn --domain deploy-monitor \
             --task "watch CI/CD and alert me on failures" \
             --persistent

# See the network
retina agents
#  ID      DOMAIN          STATUS      TOOLS  EXP   UPTIME
#  root    orchestrator    running     0      45    12d
#  a1b2    email           idle        4      200   12d
#  c3d4    research        idle        6      150   8d
#  e5f6    data            idle        3      80    5d
#  g7h8    deploy-monitor  persistent  2      50    3d

# Talk to a specific agent
retina chat email "what patterns have you noticed in my invoices?"

# Approve a tool promotion
retina approve a1b2 --tool invoice_parser --promote-to-shared

# Archive an agent (preserves memory)
retina archive c3d4

# Budget and cost tracking
retina stats
#  AGENT           TOKENS/DAY   COST/DAY   REFLEXES   LLM CALLS/DAY
#  root            1,200        $0.02      12         ~40
#  email           800          $0.01      23         ~15 (mostly reflexes now)
#  deploy-monitor  2,400        $0.04      8          ~80
#  TOTAL           4,400        $0.07
```

### MCP / A2A (Other Systems Can Hire the Network)

The root agent exposes the entire network as MCP tools:

```json
{
  "tools": [
    {
      "name": "retina_email",
      "description": "Email operations — inbox scan, send, parse invoices",
      "inputSchema": { "task": "string" }
    },
    {
      "name": "retina_research",
      "description": "Web research — search, scrape, summarize",
      "inputSchema": { "task": "string", "sources": "array" }
    },
    {
      "name": "retina_execute",
      "description": "General task — will route to specialist or spawn one",
      "inputSchema": { "task": "string" }
    }
  ]
}
```

Another agent (Cursor, Claude, an external system) can call `retina_email` and get the full power of a specialist with 200 experiences and 4 fabricated tools — without knowing any of that exists. The network is the product, exposed through a clean interface.

### Agent Cards (A2A)

Each specialist broadcasts what it can do:

```json
{
  "agent_id": "retina-email-a1b2",
  "capabilities": ["imap_read", "email_send", "invoice_parse", "inbox_monitor"],
  "tools": 4,
  "experience_count": 200,
  "confidence_domains": ["gmail", "outlook"],
  "status": "idle",
  "cost_per_task": "$0.005 avg"
}
```

---

## Deployment Targets: Same Kernel, Different Shells

The kernel is the brain. The **shell** is the body. The same `retina-kernel` + `retina-memory` + `retina-fabricator` stack deploys into radically different contexts by swapping the shell layer.

### The Shell Abstraction

```rust
/// Every deployment target implements this trait.
/// The kernel doesn't know or care what world it's operating in.
trait Shell {
    /// What can this shell observe? (the agent's "senses")
    fn observe(&self) -> WorldState;

    /// What actions can this shell take? (the agent's "hands")
    fn execute(&self, action: Action) -> ActionResult;

    /// How does the user interact? (approval, input, feedback)
    fn human_interface(&self) -> HumanChannel;

    /// Hard constraints on this deployment (what the agent CANNOT do)
    fn constraints(&self) -> Vec<HardConstraint>;
}
```

### Deployment Examples

| Shell | Observes | Acts On | Human Interface | Hard Constraints |
|---|---|---|---|---|
| **CLI** | Terminal output, file system, process state | Shell commands, file writes, API calls | Terminal prompt, stdin/stdout | OS permissions |
| **Chrome Extension** | DOM, form fields, page content, user documents | Fill fields, click buttons, read DOM | Extension popup, overlay UI | **Never submit forms**, no navigation away |
| **Server/Daemon** | API responses, webhooks, cron triggers | API calls, database writes, notifications | Slack/webhook for approval | Rate limits, auth scopes |
| **Embedded (IoT)** | Sensor data, device state | Actuator commands, config changes | Mobile app for approval | Hardware safety limits |
| **MCP Client** | Other agent's tool outputs | Other agent's tool calls | Parent agent is the "human" | Parent agent's permission scope |

The kernel execute loop is identical in every case:
```
observe → reflex check → act → verify state change → record experience → reflect if needed
```

What changes is what "observe" sees and what "act" can do.

### Example: Medicare Form-Filling Agent (Chrome Extension)

This is a single specialist agent — no network needed — deployed inside a Chrome extension shell.

**What it does:**
- Reads the user's documents (uploaded PDFs, photos of cards, prior forms)
- Observes the current web form (DOM fields, labels, validation rules)
- Maps document data to form fields
- Fills fields for the user
- Highlights what it filled and why (transparency)
- **Never submits. Never clicks submit. This is a hard constraint in the shell, not a guideline.**

**How it works with the kernel:**

```
SHELL: Chrome Extension
  observe() → DOM snapshot (form fields, labels, validation state)
  execute() → set field values, scroll, highlight filled fields
  human_interface() → extension popup overlay
  constraints() → [NeverSubmit, NeverNavigateAway, NeverModifyNonFormElements]

KERNEL: Standard Retina kernel
  Same execute loop, same reflex engine, same memory, same fabricator

MEMORY: Single agent.db stored in extension storage
  Experiences: "Medicare Part A field 3 = SSN from card photo"
  Reflexes: "Date of Birth field → parse from driver's license → MM/DD/YYYY format"
  Knowledge: "CMS-1500 form: fields 1-13 are patient info, 14-33 are provider"

FABRICATED TOOLS:
  - pdf_field_extractor.wasm    (reads uploaded documents)
  - medicare_form_mapper.wasm   (maps CMS field IDs to standard data)
  - date_format_converter.wasm  (handles MM/DD/YYYY vs YYYY-MM-DD)
  - address_parser.wasm         (normalizes address formats)
```

**Day 1:** User uploads their Medicare card photo and a doctor's bill. Agent reads the form, figures out what fields it can fill, asks the user to confirm what it doesn't know.

**Day 30:** Agent has filled 20 Medicare forms. It knows:
- Where SSN goes on every CMS form variant (reflex, no LLM call)
- That field 21 needs ICD-10 codes and they come from the doctor's bill (reflex)
- That the date format is always MM/DD/YYYY on government forms (reflex)
- It built a specialized CMS-1500 mapper tool from experience

**The user reviews and clicks submit themselves.** The agent assists, never acts on submission. This isn't a policy — it's a hard constraint in the shell that the kernel physically cannot bypass.

```
┌──────────────────────────────────────────────┐
│  CHROME EXTENSION SHELL                       │
│                                               │
│  ┌─────────────────────────────────────────┐  │
│  │  Hard Constraints (enforced by shell)   │  │
│  │  ✗ Cannot click submit buttons          │  │
│  │  ✗ Cannot navigate to other pages       │  │
│  │  ✗ Cannot modify non-form elements      │  │
│  │  ✗ Cannot send data to external APIs    │  │
│  │  ✓ Can read DOM (form fields, labels)   │  │
│  │  ✓ Can set form field values            │  │
│  │  ✓ Can read user-provided documents     │  │
│  │  ✓ Can highlight/annotate what it did   │  │
│  └─────────────────────────────────────────┘  │
│                                               │
│  ┌─────────────────────────────────────────┐  │
│  │  RETINA KERNEL (identical to CLI)       │  │
│  │  execute loop → reflex → verify → learn │  │
│  └─────────────────────────────────────────┘  │
│                                               │
│  ┌─────────────────────────────────────────┐  │
│  │  MEMORY (extension local storage)       │  │
│  │  agent.db: form patterns, field maps    │  │
│  │  tools/: pdf_parser.wasm, mapper.wasm   │  │
│  └─────────────────────────────────────────┘  │
│                                               │
│  ┌─────────────────────────────────────────┐  │
│  │  USER OVERLAY                           │  │
│  │  "I filled 8 of 12 fields. Review:"    │  │
│  │  ☑ Name: John Smith (from Medicare card)│  │
│  │  ☑ SSN: •••-••-1234 (from Medicare card)│  │
│  │  ☑ DOB: 03/15/1952 (from license)      │  │
│  │  ☐ Provider NPI: [need this from user]  │  │
│  │                                         │  │
│  │  [Accept All] [Edit] [Clear]            │  │
│  └─────────────────────────────────────────┘  │
└──────────────────────────────────────────────┘
```

### How This Connects to the Network

The Chrome extension agent can be a standalone specialist OR part of the network:

**Standalone:** User installs the extension. Single agent, single domain. Learns form patterns over time. No network, no root agent.

**Part of network:** Root agent routes "fill out this Medicare form" to the form-filling specialist, which happens to live in a Chrome extension. The specialist returns "filled 8/12 fields, need user input on 4" to the root, which relays to the user.

**Tool promotion:** The form-filling agent's `pdf_field_extractor.wasm` gets promoted to shared tools — now the data agent can also parse PDFs.

The kernel doesn't care where it runs. The shell determines the boundaries.

---

## Human Pulse Gate

The network has tiered approval:

| Action Type | Approval |
|---|---|
| Read data (file, email, API) | Auto-approved |
| Fabricate + test tool | Auto-approved (sandboxed) |
| Write data (file, database) | Auto-approved for known patterns, confirm for new |
| **External action** (send email, API POST, deploy) | **Always requires human approval** |
| **Spawn new agent** | Auto if task warrants it, human notified |
| **Promote tool to shared** | **Always requires human approval** |
| **Modify reflexive rules** | Auto from consolidation, human can review |
| **Delete/archive agent** | **Always requires human approval** |

```
[!] email agent wants to: Send email
    To: accountant@firm.com
    Subject: "March Invoice Summary"
    Body: [142 words]

    This is an EXTERNAL ACTION.
    Approve? [y/n/edit/inspect]: _
```

---

## Project Structure

The crate layout follows the 5-trait architecture. `retina-traits` defines the 5 contracts. `retina-kernel` depends only on `retina-traits`. Implementation crates each implement one trait. The binary wires them together.

```
agent-retina/
├── Cargo.toml                        # Workspace root
├── crates/
│   │
│   ├── retina-traits/                # 5 trait contracts + shared types
│   │   ├── shell.rs                  # Shell (observe, act, verify state, ask user)
│   │   ├── reasoner.rs               # Reasoner (think, reflect)
│   │   ├── memory.rs                 # Memory (record, recall, learn)
│   │   ├── fabricator.rs             # Fabricator (compile + run tools)
│   │   ├── transport.rs              # Transport (agent-to-agent messages)
│   │   └── types.rs                  # Shared types (Action, Intent, Experience, etc.)
│   │
│   ├── retina-kernel/                # The agent. Depends ONLY on retina-traits.
│   │   ├── execute.rs                # Execute loop (act → verify → record → learn)
│   │   ├── reflex.rs                 # Reflex engine (pre/post action checks)
│   │   ├── circuit_breaker.rs        # Failure detection and blocking
│   │   ├── context.rs                # Minimal prompt assembler (~500 tokens)
│   │   ├── router.rs                 # Task routing / agent spawning
│   │   ├── consolidation.rs          # Experience → knowledge → reflex promotion
│   │   └── utility.rs                # Q-value scoring
│   │
│   ├── retina-memory-sqlite/         # Memory impl (rusqlite + sqlite-vec + FTS5 + fastembed)
│   ├── retina-memory-inmem/          # Memory impl (HashMap, for tests)
│   ├── retina-llm-claude/            # Reasoner impl (Anthropic API)
│   ├── retina-llm-ollama/            # Reasoner impl (local models)
│   ├── retina-fabricator-wasm/       # Fabricator impl (wasmtime + rustc)
│   ├── retina-transport-local/       # Transport impl (in-process channels)
│   ├── retina-transport-mcp/         # Transport impl (MCP protocol)
│   ├── retina-shell-cli/             # Shell impl (terminal, filesystem, stdin/stdout)
│   ├── retina-shell-browser/         # Shell impl (DOM, forms, extension popup)
│   │
│   └── retina-cli/                   # Binary: wires impls into kernel, runs
│       ├── main.rs
│       ├── commands.rs               # run, spawn, agents, approve, chat, stats
│       └── output.rs
│
├── docs/                             # Research and architecture docs
└── tests/                            # Integration tests
```

### Data Directory

```
~/.retina/
├── config.toml                       # Global: API keys, model preferences, approval rules
├── root/
│   ├── agent.db                      # Root agent memory (routing patterns, task decomposition)
│   └── tools/
├── agents/
│   ├── email-a1b2/
│   │   ├── agent.db                  # Email specialist memory
│   │   ├── tools/                    # Fabricated .wasm tools
│   │   └── manifest.toml             # Domain, capabilities, status, budget
│   ├── research-c3d4/
│   │   ├── agent.db
│   │   ├── tools/
│   │   └── manifest.toml
│   └── .../
└── shared/
    └── promoted_tools/               # Tools promoted from specialists
```

---

## Build Order

| Phase | What | Result |
|---|---|---|
| **1** | `retina-traits` + `retina-kernel` | 5 traits defined, kernel compiles, testable with mocks |
| **2** | `retina-memory-sqlite` + `retina-shell-cli` + `retina-llm-claude` | First runnable agent: can think, act, remember |
| **3** | Kernel: reflection, consolidation, reflex promotion | Agent can recover from failures and learn from experience |
| **4** | `retina-fabricator-wasm` | Agent can build its own tools |
| **5** | `retina-transport-local` + kernel router | Agent can spawn specialists, becomes a network |
| **6** | `retina-cli` | Packaged behind `retina run`, `retina agents`, `retina stats` |
| **7** | `retina-transport-mcp`, `retina-llm-ollama`, alternative impls | Proves the abstractions work with second implementations |

---

## Deep Research Areas

### 1. Minimal Context Engineering
How to assemble the smallest possible prompt that still lets the agent succeed. [Cursor's dynamic context discovery](https://cursor.com/blog/dynamic-context-discovery) showed 46.9% token reduction. [Phil Schmid's context engineering](https://www.philschmid.de/context-engineering): offloading, reduction, retrieval, isolation.

### 2. Reflex Engine Design
How to compile experiences into fast Rust-native checks. [ACT-R activation spreading](https://dl.acm.org/doi/10.1145/3765766.3765803), [MemRL Q-value utility](https://arxiv.org/abs/2601.03192), experience-to-rule promotion thresholds. This is the core learning algorithm.

### 3. Memory Retrieval Strategy (inside SQLite impl)
How `recall_experiences()` and `recall_knowledge()` work internally: sqlite-vec for semantic search, FTS5 for keywords, hybrid scoring (reciprocal rank fusion), utility-weighted ranking. All behind the Memory trait — kernel doesn't know.

### 4. Agent Network Orchestration
How to route tasks, spawn specialists, manage lifecycles. [Multi-agent memory consistency](https://arxiv.org/html/2603.10062), [Cursor background agents](https://docs.cursor.com/en/background-agent), capability matching.

### 5. Tool Fabrication Pipeline
How agents write, compile, test, and deploy tools via the Fabricator trait. Wasmtime as default impl, WASI capabilities, [Voyager skill library](https://arxiv.org/abs/2305.16291) pattern.

### 6. Memory Consolidation & Reflex Promotion
Experience → knowledge → reflex lifecycle. [A-MEM Zettelkasten](https://arxiv.org/abs/2502.12110) self-organizing memory, [KV compaction](https://arxiv.org/abs/2602.16284) for local models, forgetting/decay functions.

### 7. State Verification
How each Shell implementation captures and compares state. CLI: file checksums + process state. Browser: DOM hashing. Server: API response hashing. The `capture_state()` / `compare_state()` contract must work for any world.

### 8. Budget & Cost Management
Per-agent token budgets, cost tracking, auto-archival of expensive idle agents, reflex-to-LLM ratio as a maturity metric.

### 9. MCP / A2A Integration
Transport implementations for MCP and A2A protocols. Expose the network as callable tools. Agent Cards per specialist.

---

## Key Risks

1. **Agent sprawl** — Too many specialists, burning API quota on idle agents. Mitigation: auto-archive after idle timeout, budget caps per agent, maturity metrics (reflex ratio).
2. **Fabrication quality** — LLM-generated tools may be buggy. Mitigation: mandatory test pass in Fabricator sandbox, usage-gated promotion.
3. **Reflex poisoning** — Bad experience promoted to reflex. Mitigation: confidence thresholds, decay on failed reflexes, human override via `retina rules`.
4. **Routing errors** — Root agent sends task to wrong specialist. Mitigation: capability matching with confidence scores, fallback to spawn-new-agent.
5. **Context starvation** — Minimal context means agent might miss critical info. Mitigation: good memory recall, reflection catches "need more context" situations.
6. **Network coordination** — Multi-agent pipelines can fail mid-way. Mitigation: orchestrator tracks step completion, retry/reroute on failure, human escalation.
7. **Tool drift** — Agents fabricate redundant tools. Mitigation: tool registry deduplication, capability matching before fabrication.

---

## The Vision

Day 1, you deploy a seed. One Rust binary.

Day 90, you have a personal network of specialists — email, research, code, data, ops — each one growing smarter at its domain, each one cheaper to run as reflexes replace LLM calls, each one building tools you never asked for but exactly needed.

The user doesn't configure agents. They don't manage memory. They don't choose tools. They say what they want done, and the network figures out who does it, how to do it, and what to build if nothing exists yet.

**One kernel. Any backend. Many agents. The network is the product.**
