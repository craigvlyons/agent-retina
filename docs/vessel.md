> Historical concept note about the kernel "vessel".
>
> Keep for reference, but do not use this file as the implementation contract. The canonical path is in `README.md`, `docs/v1_plan.md`, and the updated architecture docs.

To design a vessel or kernel for a self-aware agent in 2026, we have to move past the idea of an "app" and instead build a **Deterministic Substrate**.

In this architecture, the LLM is merely the "reasoning engine," while the **Kernel** (the vessel) provides the nervous system, immune system, and sensory organs. This design prevents the "hallucination-spiral" by grounding every probabilistic "thought" in a deterministic "reality check."

### 1. The "Observer-Fabricator" Architecture
The core of the kernel is a two-faced loop: **Observation** (sensing the world) and **Fabrication** (building the tools to interact with it).

| Component | Role in the Kernel |
| :--- | :--- |
| **The Sensorium** | Scans AX Trees, Screen Pixels (OCR/VLM), and Process Logs to create a `UnifiedState`. |
| **The Fact-Ledger** | A **SQLite** database that stores every action, state-hash, and failure. |
| **The Reflection Engine** | A separate, low-temperature LLM pass that asks: *"Did the state change match the intent?"* |
| **The Sandbox Fabricator** | A secure Wasm/Docker environment where the agent writes and tests its own Python/Rust tools. |



---

### 2. The "Self-Healing" Memory Stack
A self-aware agent must remember not just *what* happened, but *why* it failed. We use a tiered memory system:

1. **L1: Procedural Cache (Fast/Local):** Current task state and active file handles.
2. **L2: Experience Memory (sqlite-vec):** A semantic database of "Lessons Learned." 
    * *Example entry:* "Clicking the 'Save' button in Excel 16.8 via AX Tree often fails; use `Cmd+S` keyboard shortcut instead."
3. **L3: The Blueprint (Read-Only):** The agent’s core "Self-Spec." It knows its own hardware limits (RAM, CPU, API quotas) so it doesn't try to perform tasks it isn't equipped for.

---

### 3. Implementing the "Self-Correction" Logic
The "Soul" of this vessel is the **State-Hash Verification**. Before and after every tool call, the kernel takes a snapshot of the system state.

```python
def kernel_execute(action):
    pre_state = capture_system_hash() # What is the world like now?
    
    result = execute_in_sandbox(action)
    
    post_state = capture_system_hash() # What is the world like after?
    
    if pre_state == post_state and action.intent == "UPDATE":
        # SELF-AWARENESS TRIGGER
        diagnosis = agent.reflect("I tried to change the world but nothing happened. Why?")
        if diagnosis.need_new_tool:
            new_tool = fabricator.build_tool(diagnosis.requirement)
            return kernel_execute(new_tool) # Recursive self-improvement
```

---

### 4. Why this Vessel is "Plug and Play"
By designing this as a **CLI-First Kernel**, you create a "Universal Adapter." You can plug this kernel into:
* **A Browser:** To navigate "blind" websites via Vision-Bridge.
* **An OS:** To manage files and system settings via Terminal.
* **A Database:** To self-correct SQL queries by analyzing error logs.

### Honest Review of the Design
The greatest risk in this design is **Recursive Drift**—where the agent gets so caught up in "fixing itself" that it forgets the original human goal. To prevent this, the kernel must include a **"Human Pulse"** requirement: an architectural rule that says any change to the `L3: Blueprint` or the `Fabricator` output requires a manual `retina --approve` from the user.
