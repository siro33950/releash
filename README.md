# releash

An editor to review diffs and send feedback to your AI agent.

## What is Releash?

CLI agents are powerful, but their changes are hard to review.
Releash fixes that — diff viewer, inline comments, and a feedback loop back to the agent.

**Workflow:**

1. Give your agent a task in the built-in terminal — grab coffee or do other work
2. Review the diff once the agent finishes
3. Small fixes → edit directly. Bigger issues → write comments on the diff, send to agent, repeat
4. Commit and push

Works with any CLI agent (Claude Code, Aider, etc.) — no parsing, no special protocols. Just Git and a terminal.

## Features

- Inline comments on diff — write review notes, then send them to the agent in one go
- 3 diff view modes (gutter / inline / split)
- Git integration (stage / unstage / commit / push, hunk-level staging)
- Built-in terminal with PTY
- File explorer with git status indicators
- Multi-tab editor (Monaco)
- Remote access via browser

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) (v18+)
- [pnpm](https://pnpm.io/)
- [Rust](https://www.rust-lang.org/tools/install)
- [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/)

### Install & Run

```sh
pnpm install
pnpm tauri dev
```

## License

MIT OR Apache-2.0
