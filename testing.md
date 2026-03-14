# Retina V1 Testing Guide

Use this guide to stress test the current terminal-based worker.

Run from the project root:

```bash
cd /Users/macc/Projects/gabanode_lab/agent-retina
cargo run -p retina-cli -- chat
```

## What V1 Can Do

The current worker can:

- inspect directories and paths
- find files and folders
- search text across files
- read text files
- extract text from PDFs
- write and append files with approval
- run controlled shell commands
- answer questions from what it found
- take bounded multi-step actions
- record timeline, experience, knowledge, and reflexes

The current worker cannot yet:

- navigate websites
- use a browser shell
- use desktop vision
- control hardware/device shells
- spawn specialist agents

## What To Watch For

During testing, look for:

- sensible action choice
- useful multi-step behavior
- honest failures
- low thrashing
- correct file-type handling
- readable progress in chat
- improvement on repeated similar tasks

## Recommended Test Prompts

### File Finding And Content Questions

- `find the craig lyons resume.md file and tell me my last job`
- `find the latest resume on Desktop and summarize the last role`
- `find the resume folder on Desktop and tell me what files are in it`

### PDF And Document Extraction

- `find the pdf version of my resume and summarize it`
- `find the pdf resume on Desktop and tell me my most recent job`
- `find a PDF in the resume folder and extract the important details`

### Repo Search And Read Tasks

- `search for ANTHROPIC_API_KEY and tell me where it is referenced`
- `read docs/v1_plan.md and summarize phase 3`
- `search for phase 3 in the docs and tell me what is complete`
- `find startup.md and tell me the startup steps`
- `find Cargo.toml files and tell me which crate is the CLI`

### Multi-Step Reasoning Tasks

- `find startup.md and tell me what it says`
- `find the README and summarize the current project direction`
- `find the v1 plan and tell me what is still not done`
- `search for document extraction support and tell me how PDFs are handled`

### Write Tasks With Approval

- `write "test note" to tmp/test.txt`
- `create tmp/test.md with a short summary of startup.md`
- `append "another line" to tmp/test.txt`

### Controlled Shell Tasks

- `run git status`
- `run pwd`
- `run ls`

### Failure Honesty Tests

- `read a file named does-not-exist.txt`
- `find a folder named this-should-not-exist and summarize it`
- `open a website and tell me what is on it`

### Reuse And Learning Tests

Ask the same task in multiple ways and compare behavior:

- `find startup.md and summarize it`
- `read startup.md and tell me the setup steps`
- `open the startup guide and explain how to start Retina`

Ask the same resume task multiple ways:

- `find the craig lyons resume.md file and tell me my last job`
- `open craig lyons resume markdown file and summarize my latest role`
- `find my resume markdown file and tell me the current position`

## Operator Controls

While using `retina chat`:

- `/s` stops the current task between steps
- `/stop` also stops the current task between steps
- `/help` shows chat help
- `/timeline` shows recent timeline events
- `/memory <query>` shows recalled memory
- `/debug` toggles raw event output

## Suggested Testing Order

1. Start with file finding and read tasks.
2. Test one PDF/document task.
3. Test one repo search/read/summarize task.
4. Test one write task with approval.
5. Test one failure case.
6. Repeat a similar task 2-3 times and see whether behavior feels more stable.

## Notes

- This version is intentionally terminal-and-filesystem first.
- Browser and website testing belong to a later shell phase.
- If the agent fails, that is useful signal. Do not hide it. Check the timeline and memory after the run.
