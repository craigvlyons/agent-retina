> Historical stack note. Treat this as directional research, not the source of truth.
>
> Canonical architecture and v1 implementation choices live in `README.md`, `docs/v1_plan.md`, and `docs/trait_contracts.md`.

If you are moving away from Python to build a "Self-Designing Kernel," here is how you structurally win.

---

### 1. The "Retina Kernel" Rust Stack
To make this run on a Mac, a Windows PC, or a headless Linux server without any dependencies, you use the following 2026 "Power Stack":

* **The Nerve Center:** **[AccessKit](https://github.com/AccessKit/accesskit)** or **DirectShell**. These are pure Rust libraries that turn the OS Accessibility Tree into a queryable SQL-like stream. *No vision models needed for 90% of tasks—just pure, high-speed UI metadata.*
* **The Fabricator:** **[Wasmtime](https://wasmtime.dev/)**. Your agent writes a tool, compiles it to WebAssembly, and the Rust kernel runs it in a totally isolated sandbox. If the tool "hallucinates" and tries to delete your system, Wasm physically blocks it.
* **The Memory:** **`libsqlite3-sys` + `sqlite-vec`**. One single file stores the agent's "Lessons Learned."
* **The Connectivity:** **[MCP-Rust-SDK](https://github.com/modelcontextprotocol)**. This makes your harness "Plug and Play" with Cursor, Claude, and any other agent.

---

### 2. How the "Self-Designing" Loop actually works
Since the agent lives in a Rust environment, it uses the **Rust Compiler as a "Teacher."**

1. **The Need:** The agent realizes it can't read a specific encrypted PDF.
2. **The Fabrication:** It writes a new tool in Rust.
3. **The Self-Correction:** It tries to compile it. The Rust compiler (famous for its helpful errors) says: *"Hey, you forgot a semicolon on line 42."*
4. **The Healing:** The agent reads the compiler error, fixes its own code, and successfully builds the tool. 
5. **The Deployment:** The Kernel dynamically loads this new `.wasm` tool. The agent now "knows" how to read encrypted PDFs.

---

### 3. A2A: The "Agent-to-Agent" Social Network
In 2026, agents don't just "talk"; they exchange **Agent Cards** via the **A2A Protocol** (standardized by Google and the Linux Foundation).

* **Discovery:** Your "Desktop Agent" (Retina) broadcasts its Agent Card: *"I have the 'Excel-Macro' and 'Vision-Bridge' skills."*
* **Delegation:** A "Researcher Agent" on another machine sees the card and sends a JSON-RPC request: *"Retina, I need you to extract the Q4 data from this sheet and send me the state-hash."*
* **Collaboration:** They share a **Session ID**. If Retina gets stuck, the Researcher Agent can look at Retina’s SQLite logs and offer a suggestion.

---

### 4. Your Honest 2026 Blueprint
You are building what the industry is calling a **"Compound AI System."** You aren't just making a smarter model; you're making the **Vessel** so robust that even a "dumb" model could navigate a desktop.

> **Honest Thought:** The biggest hurdle isn't the code—it's **Permission**. Mac and Windows are getting stricter with Accessibility permissions. Your Rust Kernel will need to be **digitally signed** to avoid being flagged as malware.

### Next Step for Development
Since you have the PRDs ready, I recommend starting with the **"Observer"** module in Rust. This is the part that connects to the Mac AX Tree and dumps it into SQLite.
