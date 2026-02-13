<!-- TODO: ロゴ画像 -->
<!-- <p align="center"><img src="docs/assets/logo.svg" alt="Releash" width="200" /></p> -->

# Releash

**Unleash your AI agent. Take the leash back from anywhere.**

A desktop app to manage multiple AI coding agents running in parallel — Kanban board, diff review, terminal, and remote access in one place.

<p align="center"><img src="docs/assets/hero.png" alt="Releash screenshot" width="800" /></p>

## Why Releash?

Running multiple CLI agents in parallel is easy — open a few terminals and go. The hard part is managing them. Which branch is still in progress? Which agent finished? Which one has a PR open? Which worktree is ready to merge?

Releash integrates worktrees, agents, and Git status into a single management layer. **A Kanban board shows all your parallel tasks — each backed by a worktree with its own terminal, editor, and source control. Manage them from your desktop, or from your phone.**

1. **Organize** tasks on the Kanban board — each card is a worktree with its own agent
2. **Track** agent status in real time — running, waiting, done
3. **Check in** from your phone — scan a QR code, see everything
4. **Review** diffs, leave comments, send feedback to agents
5. **Ship** — stage, commit, push, all without switching tools

No cloud, no IDE lock-in. Releash runs on your machine, watches Git, and works with any CLI agent that writes files.

## Features

### Kanban for parallel agents

Each worktree is a card on a Kanban board — Todo, In Progress, Review (PR open), Done (merged). See all your agents and their tasks at a glance. Click a card to jump into the worktree with the editor, terminal, and source control ready to go. PR detection uses the [GitHub CLI](https://cli.github.com/) (`gh`) to automatically move cards to Review and Done columns.

<p align="center"><img src="docs/assets/kanban.png" alt="Kanban board" width="600" /></p>

### Agent status tracking

Know whether your agent is running, waiting for input, or done. Get notified via webhook (compatible with Slack Incoming Webhooks) when it needs your attention.

Agent state detection connects through [Claude Code Hooks](https://docs.anthropic.com/en/docs/claude-code/hooks). Other agents are fully supported for file editing, diff review, and Git workflows — they just won't report granular run states.

### Monitor and operate from your phone

Scan a QR code to open Releash in your phone's browser. Check agent status, browse diffs, stage files, operate the terminal, leave review comments — all without being at your desk.

- Works on the same LAN out of the box
- Auto-detects VPN interfaces (Tailscale, WireGuard, ZeroTier) for access outside your local network
- All traffic is authenticated with HMAC-SHA256 challenge-response — no cloud relay, everything stays on your network

<p align="center"><img src="docs/assets/remote-ui.png" alt="Remote Web UI" width="600" /></p>

### Precision diff review

Three view modes — gutter (compact), inline, and split (side-by-side). Image diffs show before/after side by side. Stage exactly the lines you want with hunk-level and group-level staging. Leave inline comments on any line and send them to the agent in one click.

<p align="center"><img src="docs/assets/diff-review.png" alt="Diff review" width="600" /></p>

### Full Git workflow

Stage, unstage, commit, and push without leaving the app. Partial staging at the hunk level. Branch management with worktree support.

### Project-wide search

Regex-powered file search across the entire project. Results are grouped by file with match highlighting — click to jump straight into the editor.

### Built-in terminal

A real PTY terminal, not a toy. Shell integration (Bash, Zsh) detects when commands finish. Review comments are sent directly as text input, so it works with any agent — no plugins or integrations required.

## Getting Started

> **Early development** — pre-built binaries are not available yet. Build from source for now.

**Platforms:** macOS, Linux (Windows is untested)

**Prerequisites:** [Node.js](https://nodejs.org/) (v18+), [pnpm](https://pnpm.io/), [Rust](https://www.rust-lang.org/tools/install), [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/)

**Optional:** [GitHub CLI](https://cli.github.com/) (`gh`) — enables PR detection for Kanban board

```sh
pnpm install
pnpm tauri dev
```

## How It Works

```
Create worktrees for each task
        ↓
Launch agents in their terminals
        ↓
Agents edit files — Releash watches Git for changes
        ↓
Kanban board tracks status: Todo → In Progress → Review → Done
        ↓
Check in from desktop or phone — review diffs, leave comments
        ↓
Comments go straight to the agent as terminal input
        ↓
Stage, commit, push — done
```

Releash doesn't parse agent output or depend on any specific agent's API. It watches the filesystem and reads Git. That's why it works with Claude Code, Aider, Cline, or any tool that writes files. (Real-time agent state tracking requires [Claude Code Hooks](https://docs.anthropic.com/en/docs/claude-code/hooks); other agents get full Git and terminal support without run-state detection.)

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop | [Tauri 2](https://v2.tauri.app/) (Rust) |
| Frontend | React 19, Monaco Editor, xterm.js |
| Git | git2 crate |
| Remote | WebSocket server, QR code auth, HMAC-SHA256 |
| Terminal | portable-pty |

## License

MIT OR Apache-2.0
