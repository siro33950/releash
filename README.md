# Releash

Let your AI agent work — review from your phone.

## The Problem

CLI agents (Claude Code, Aider, etc.) are powerful. You give them a task, they edit files, run commands, iterate. But when they're done, reviewing 500 lines of changes in a terminal is painful.

You could open VS Code — but then why not just use Cursor?

## What Releash Does

Releash is a desktop app where you run your CLI agent, let it work, and review the results — from your desk or your phone.

1. **Launch** your agent in the built-in terminal
2. **Walk away** — go grab coffee, work on something else
3. **Review** the diff when you're ready — on desktop or on your phone via browser
4. **Fix** small issues inline, or send review comments back to the agent
5. **Commit** and push

## Key Features

**Review from anywhere (Remote Web UI)**
- Scan a QR code on your phone to connect
- Browse diffs, stage/unstage files, read terminal output
- Write review comments from your phone
- Auto-detects VPN (Tailscale, WireGuard, ZeroTier) for secure remote access

**Precision diff review**
- 3 view modes: gutter (compact), inline, split (side-by-side)
- Hunk and group-level staging — stage exactly the lines you want
- Inline comments on any line → send to agent in one click

**Full Git workflow**
- Stage / unstage / commit / push without leaving the app
- Hunk-level partial staging
- Branch management

**Built-in terminal**
- Full PTY terminal (not a toy)
- Shell integration detects when commands finish
- Review comments are sent directly to the terminal as text — works with any agent

**Editor**
- Monaco Editor (same engine as VS Code)
- Multi-tab, syntax highlighting, file explorer with git status

## How It Works

Releash doesn't parse agent output or use special protocols. It watches Git.

Your agent writes files → Git tracks the changes → Releash shows the diff. That's it. Works with any CLI agent that writes to the filesystem.

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

## Tech Stack

- **Frontend**: React 19 + Monaco Editor + xterm.js
- **Backend**: Tauri 2 (Rust) + git2 + portable-pty
- **Remote**: WebSocket server + Web UI + QR code auth

## License

MIT OR Apache-2.0
