# roctopai

A terminal UI for managing GitHub issues, worktrees, and AI-powered coding sessions.

Built with Rust and [Ratatui](https://github.com/ratatui/ratatui).

## What it does

Roctopai gives you a kanban-style board in your terminal with four columns: **Issues**, **Worktrees**, **Pull Requests**, and **Sessions**. Select a GitHub repo, browse its issues, and spin up a git worktree with a tmux session where Claude works on the issue autonomously — with neovim open alongside it.

## Prerequisites

- [gh](https://cli.github.com/) (GitHub CLI, authenticated)
- [git](https://git-scm.com/)
- [tmux](https://github.com/tmux/tmux)
- [neovim](https://neovim.io/)
- [claude](https://claude.ai/claude-code) (Claude Code CLI)

## Install

```sh
cargo install --path .
```

## Usage

```sh
roctopai
```

On first launch you'll be prompted to enter a GitHub user or org. Pick a repo and you're on the board.

## Keybindings

### Board (Normal mode)

| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `Tab` / `Shift+Tab` | Switch column |
| `j` / `k` / `Up` / `Down` | Navigate cards |
| `/` | Fuzzy filter |
| `Enter` | Change repo |
| `n` | New issue (Issues column) |
| `w` | Create worktree + Claude session (Issues column) |
| `d` | Close issue (Issues column) / Remove worktree (Worktrees column) |

### Worktree + Claude session

Pressing `w` on an issue:

1. Creates a git worktree at `../<repo>-issue-<number>` on branch `issue-<number>`
2. Opens a tmux session with neovim on the left and Claude on the right
3. Claude receives the issue title and body as a prompt and begins working

```
┌──────────────┬──────────────┐
│              │              │
│    neovim    │  claude -p   │
│              │              │
└──────────────┴──────────────┘
```

Attach to the session with `tmux attach -t issue-<number>`.

## License

MIT
