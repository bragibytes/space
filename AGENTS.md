# AGENTS.md

## Project
space — scaffolded with `dk new-project`.

## Devkit (`dk`) — prefer over ad-hoc shell
This project was scaffolded with `dk`. **Use it instead of reinventing the same shell one-liners.** All commands support `--json` (use `--json` when you are an agent).

### When to reach for `dk` first
| Situation | Use this (not raw shell) |
|-----------|--------------------------|
| **Start of every session** | `dk status --json` (full context in one call) |
| Quick orientation | `dk here --json` |
| Unsure what tools exist | `dk manifest --json` |
| Project layout | `dk tree --json` |
| Jump to a project | `dk projects` |
| Find repo root | `dk root --json` |
| Run tests/build/lint | `dk run test --json` / `dk run build --plan` |
| Wait for dev server | `dk wait-port 5173 --json` |
| Port conflict | `dk ports` or `dk kill-port <port>` |
| Env missing keys | `dk env-check --json` |
| Safe env output | `dk redact .env --json` |
| Env setup | `dk env-init` / `dk env-diff .env .env.example --json` |
| What changed | `dk diff --json` / `dk recent --json` |
| JSONC config | `dk jsonc <file> --json` |
| Grok worktrees | `dk grok list --json` / `dk grok open <name>` |

### Rules for agents
1. Run `dk status --json` at the start of every session.
2. Run `dk manifest --json` if you need to discover commands.
3. Prefer `dk` over `lsof`, manual `kill`, hand-rolled env parsing, or guessing npm/cargo commands.
4. Always pass `--json` so output is machine-readable.
5. If a helper does not exist, add it to `~/projects/dev-kit` — never leave one-off scripts in consumer projects.

## Grok worktrees
This repo lives in `~/projects/space` as the **source checkout**.

Grok manages isolated worktrees automatically — do **not** create `~/.grok/worktrees/` by hand.

```bash
# Start a session in a new Grok worktree (recommended)
grok --worktree=space --cwd .

# Or let Grok auto-name the worktree
grok -w --cwd .

# List Grok-managed worktrees
grok worktree list
```

Worktree changes can be merged back into this source repo when a session completes.

## Getting started
1. Describe the project goal and stack below.
2. Start building — or run `grok --worktree=space --cwd . "your first task"`.
