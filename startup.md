# Retina Startup

## Folder

Run Retina from the project root:

```bash
cd /path/to/agent-retina
```

## Environment

Make sure your `.env` file is in the project root and includes your Claude key:

```env
ANTHROPIC_API_KEY=your_key_here
```

Retina loads `.env` automatically.

## First-time setup

Initialize the private runtime:

```bash
cargo run -p retina-cli -- init
```

This creates:

```text
~/.retina/
~/.retina/root/agent.db
~/.retina/root/manifest.toml
```

## Main commands

Start interactive mode:

```bash
cargo run -p retina-cli -- chat
```

Run a one-shot task:

```bash
cargo run -p retina-cli -- run "inspect working directory"
```

Inspect recent timeline events:

```bash
cargo run -p retina-cli -- inspect timeline
```

Inspect remembered knowledge and experiences:

```bash
cargo run -p retina-cli -- inspect memory "working directory"
```

See current memory counts:

```bash
cargo run -p retina-cli -- stats
```

## Recommended dev flow

```bash
cd /path/to/agent-retina
cargo test
cargo run -p retina-cli -- init
cargo run -p retina-cli -- chat
```

On Windows PowerShell, the same flow is:

```powershell
cd C:\path\to\agent-retina
cargo test
cargo run -p retina-cli -- init
cargo run -p retina-cli -- chat
```

## Notes

- Run commands from the repo root, not from inside `crates/`.
- `retina chat` is the best way to watch the agent work live.
- If the Claude key is missing, Retina falls back to the local heuristic reasoner behavior.
- Command execution uses the native shell for the platform: `sh` on Unix-like systems and PowerShell on Windows.
