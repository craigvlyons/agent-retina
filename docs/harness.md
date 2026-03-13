> Historical concept note focused on desktop blindness and multimodal recovery.
>
> This is not the canonical v1 implementation spec. Build from `README.md` and `docs/v1_plan.md` first, then use this file later when browser/vision shells are added.

This is the "Last Mile" problem of desktop automation. When the AX (Accessibility) Tree gives you the **What** (element name/role) but fails on the **Where** (coordinates), your agent becomes a "Brain without Eyes."

In 2026, the solution isn't to wait for a better AX Tree; it's to build a **Hybrid Multimodal Harness** that uses **OmniParser** or **Vision-Language Models (VLMs)** to "Self-Heal" missing metadata.

Below is a PRD (Product Requirements Document) for a CLI-based, self-aware Agent Harness designed specifically to handle this "Blindness" and self-improve.

---

# PRD: "Project Retina" – Self-Aware Agentic Harness

## 1. Executive Summary
**Retina** is a CLI-based agent harness that wraps any LLM (Llama 4, Claude 3.5/4, etc.) to automate desktop environments. Its primary innovation is **Multi-Source Spatial Grounding**, which allows it to resolve clickable coordinates even when the OS Accessibility Tree fails. It features a **Self-Improvement Loop** that detects "coordinate blindness" and automatically triggers an "Optical Recovery" mode.

## 2. The Core Problem & The "Blindness" Solution
* **The Issue:** Mac AX Tree often returns `(0,0)` or null coordinates for web-elements inside wrappers (Electron, Flutter, or complex React apps).
* **The Solution:** The **Retina Verification Loop**. If `AX_Coordinate == Null`, the harness automatically takes a screenshot, passes it to a local **Vision-Encoder (like OmniParser or Moondream 2.5)**, and maps the visual bounding box back to the AX Tree element ID.

---

## 3. Technical Architecture (The Harness)

### A. The "Vision-AX" Bridge
When the agent selects an element that lacks coordinates, the harness intercepts the command:
1. **Detection:** Harness sees `Action: Click(ID:405)` but `ID:405.coords == None`.
2. **Optical Overlay:** Harness captures a high-res screenshot and runs a fast local "Detection Model."
3. **Cross-Reference:** It matches the text/role of the visual button to the text/role in the AX Tree.
4. **Action:** It executes the click at the **Visual Center** of the detected object.

### B. The Self-Awareness Module (Self-Critique)
The harness maintains a **"Health Score"** for its own tooling.
* **The Check:** After every click, it calculates a **State-Delta**. If the screen didn't change, the harness logs a "Blindness Incident."
* **The Fix:** The harness creates a `RETINA_LOG.md` (Self-Observation Memory). Next time it encounters that specific app, it doesn't even try to use the AX Tree; it defaults to Vision Mode immediately.

---

## 4. Feature Requirements

| Feature | Requirement |
| :--- | :--- |
| **CLI-First** | Must be a terminal-based binary (`retina run "task"`) for easy piping. |
| **Hybrid Grounding** | Merge AX Tree data + Vision coordinates into a single "Unified UI Map." |
| **Plugin System** | "Plug and Play" adapters for Excel, Chrome, SAP, and Terminal. |
| **Self-Healing** | If an action fails twice, the harness must "Backtrack" and try a different tool (e.g., Tab-navigation vs. Clicking). |
| **Experience Cache** | A local JSON/Neo4j store of successful "Click-Maps" for common apps. |

---

## 5. User Workflow (The CLI Experience)
```bash
# Initialize the harness with a specific persona
retina init --persona "accounting-clerk"

# Execute a task
retina run "Download all invoices from the portal and update the Master Excel"

# The Self-Awareness Trigger
# Output: [!] Warning: Chrome AX Tree missing coordinates for "Download" button.
# Output: [i] Triggering Optical Recovery...
# Output: [i] Bounding box found at (450, 220). Click successful.
# Output: [✓] Experience updated: "Chrome > Portal" now uses Visual Grounding.
```

---

## 6. Implementation Roadmap

### Phase 1: The "Observer" (Months 1-2)
Build the CLI tool that pulls the Mac AX Tree and screenshots simultaneously. Create the `state_hash` logic to detect if a click "worked."

### Phase 2: The "Vision Bridge" (Months 3-4)
Integrate a local, small-parameter vision model (like **Qwen-2-VL** or **OmniParser**) to convert images into coordinate lists. Build the "Matcher" that links AX text to Image pixels.

### Phase 3: The "Hierarchy" (Months 5-6)
Implement the **Orchestrator-Worker** pattern. The Orchestrator plans the high-level task; the Worker uses the "Vision-AX" bridge to execute.

---

## 7. Strategic Advantage: Why This is "Next"
Traditional agents (like OpenClaw or OpenDevin) rely too heavily on the model's intelligence. **Retina** focuses on **Harness Intelligence**. It assumes the model might be "blind" and builds the sensory infrastructure to compensate. 

By making it a **Plug-and-Play CLI**, you allow any other service (a web-app, a cron job, or another agent) to "hire" Retina to do the dirty work of desktop navigation.
