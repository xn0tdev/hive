# hive

A coding agent in the spirit of Claude Code and Codex — but written in **Rust**, with a native terminal UI.

You talk. Hive reads the repo, runs tools, edits files, and keeps going. No Electron shell, no “approve every `ls`” dance by default: it’s a YOLO harness that stays out of your way.

Hive wasn’t built as a product launch. It started as a personal agent — something to close real day-to-day needs for me and a few friends. No grand roadmap, no growth thesis. Somehow it became the harness a lot of us actually *like* opening. That’s still the bar: useful at 2am, not impressive in a demo reel.

## Why use it

- **Feels like a real coding agent** — plan / make / multitask, tools, subagents, skills, context compact when the window fills up.
- **Native TUI** — fast redraws, keyboard-first, runs where you already work.
- **Rust end-to-end** — one binary, low overhead, no Node runtime in the loop.
- **Your providers** — OpenAI-compatible endpoints (Fireworks, OpenRouter, …). First run walks you through setup; `/connect` and `/model` later.
- **Yours to bend** — MIT, small crates, clear seams. Fork it, patch it, keep the keys on your machine.

## Install

Package on crates.io is **`hive-agent`** (the name `hive` was taken). The command you run is still **`hive`**.

```bash
cargo install hive-agent
hive
```

From this repo:

```bash
cargo run -p hive-agent
```

Needs a recent Rust toolchain and a terminal that behaves (truecolor helps).

## Platforms

| OS | Status |
|----|--------|
| **Linux** | Supported |
| **macOS** | Supported (same Unix TUI + `sh -c` path) |
| **Windows** | Supported on **Windows 10+** with **Windows Terminal** (or conhost with VT). Shell tool uses `cmd.exe /C`. |

## Config

Config lives at `~/.config/hive/config.toml` (on Windows: `%USERPROFILE%\.config\hive\config.toml`). On first launch with no key, Hive opens a short setup wizard.

Minimal example:

```toml
[provider]
base_url = "https://api.fireworks.ai/inference/v1"
api_key_env = "FIREWORKS_API_KEY"
# api_key = "fw_..."   # optional; env wins when set

[models]
default = { id = "accounts/fireworks/routers/kimi-k2p6-fast", name = "Kimi Fast" }

[agent]
context_window = 128000   # auto-/compact around 75% of this
```

You can also just export `FIREWORKS_API_KEY` (or whatever `api_key_env` you set). Extra providers go in via `/connect`; switch models with `/model`. Skills live under `~/.config/hive/skills/` or `.hive/skills/` and show up as `/skill-name`.

## Under the hood (brief)

```
lib/comb          → comb-tui — terminal engine
crates/hive-core  → agent loop, tools, swarm, skills, compact
crates/hive-llm   → OpenAI-compatible client + model catalog
crates/hive-tui   → the UI you stare at
crates/hive       → binary (package name: hive-agent)
```

One process, streaming completions, tools in a tight loop. Multitask can fan out into git worktrees; `/compact` (and auto-compact) keeps the task while shrinking history.

## License

MIT — use it, break it, make it yours.
