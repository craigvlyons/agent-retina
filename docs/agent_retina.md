> Historical concept note. Do not implement from this file.
>
> Canonical implementation guidance lives in `README.md`, `docs/v1_plan.md`, `docs/architecture.md`, `docs/trait_contracts.md`, `docs/research_overview.md`, `docs/research_memory.md`, and `docs/research_memory_v2.md`.

This PRD outlines the architecture for **"Retina Kernel"**—a self-evolving, CLI-first agent harness. It is designed to solve the "Silent Failure" problem by giving the agent the ability to diagnose its own limitations and modify its own environment.

---

# PRD: Project Retina Kernel (2026)
**Target:** A "Cursor-style" infrastructure that treats the agent's tools and prompts as a dynamic, self-healing operating system.

## 1. Core Principles
* **Harness as Moat:** The model is a commodity; the harness (context, tools, recovery) is the intelligence.
* **Fail Loudly:** If an action results in a `null` state change, the agent must halt and perform a self-diagnostic.
* **Tool Fabrication:** The agent is authorized to write its own Python tools to bridge "Blindness Gaps."
* **Local-First Memory:** SQLite + `sqlite-vec` for 100% private, portable, and semantic context.

---

## 2. Technical Stack
* **Database:** SQLite 3 (Relational) + `sqlite-vec` (Vector Search).
* **Memory Architecture:** * **Factual:** Tool registry and project rules.
    * **Episodic:** Every CLI command, AX tree state, and state-hash.
    * **Semantic:** Embeddings of "Lessons Learned" (e.g., *"Why clicking (0,0) failed last time"*).
* **The Shell:** A persistent, stateful terminal emulator the agent "lives" in.

---

## 3. Key Feature Modules

### A. The Diagnostic "Self-Reflection" Loop
When the agent's action (e.g., a click) results in the same `state_hash` (no change), the harness triggers a **Reflection Step**:
* **Action:** Intercept the loop.
* **Prompt:** *"Your last click at (450, 220) resulted in zero UI change. Analyze the logs. Are you blind? Is the element hidden? Do you need a new tool?"*
* **Outcome:** The agent admits: *"I am blind to this Electron wrapper. I need an OCR-based coordinate finder."*

### B. The Tool Fabricator (Recursive Development)
The agent has a special tool called `design_tool()`.
* **Capability:** The agent can write a Python script into a `/.retina/tools/` directory.
* **Verification:** The harness runs a test script against the new tool.
* **Persistence:** Once verified, the tool is registered in SQLite and becomes part of the agent's permanent "Skill Set."

### C. Multimodal Grounding (The "Eyes")
To solve your specific "AX Tree Blindness":
* **Visual Check:** If AX coordinates are null, the harness captures a screenshot.
* **OmniParser Integration:** A local vision model (like Qwen-2-VL) identifies the bounding boxes.
* **State Alignment:** The harness maps the visual center back to the AX Tree ID, creating a "Self-Healed Coordinate."

---

## 4. User Interaction Model: "Architect Mode"
The CLI is not just for tasks; it’s for **Development**.

* **Human:** "Why did you fail to update the Excel file?"
* **Agent:** "The AX Tree didn't report the 'Save' button. I checked my semantic memory and realized this happens in Excel version 16.8. I've designed a `keyboard_shortcut_save` tool to bypass the UI entirely. Should I install it?"
* **Human:** "Yes."
* **Result:** Agent updates its own harness logic.

---

## 5. SQLite Memory Schema
| Table | Purpose |
| :--- | :--- |
| **`tool_registry`** | Name, Description, and Source Code of all Python tools. |
| **`ui_experience`** | Map of App Names -> Known UI quirks (e.g., "Chrome: Use Vision Mode"). |
| **`state_log`** | Sequential history of `state_hashes` and terminal outputs. |
| **`vec_knowledge`** | `sqlite-vec` virtual table for "Lessons Learned" and user preferences. |

---

## 6. The "Success" Metric: The 2026 "Self-Healing" Ratio
The goal is for **80% of tool failures** to be caught by the harness before the human notices, with the agent presenting a **"Solution Proposal"** (a new tool or a logic shift) rather than a "Generic Error."
