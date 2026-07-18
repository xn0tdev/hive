# hive

A native **Rust + TUI** YOLO coding agent — like Claude Code / Codex, but yours.
No tool-call confirmations: you say it, hive does it. Streaming responses, skills,
web search, a subagent swarm, and a smart vision fallback.

## Features

- **YOLO mode** — the agent uses tools immediately, never asking for permission.
- **Beautiful TUI** (`comb`) — live token streaming, braille spinners, collapsible
  tool cards, a swarm panel, markdown rendering, quiet grayscale theme.
- **Tools** — `read_file`, `write_file`, `edit_file`, `list_dir`, `glob`, `grep`,
  `run_shell` (streamed output), `web_search` / `web_get_contents` (Exa),
  `read_skill`, `spawn_subagent`, `spawn_swarm`.
- **Skills** — drop a `SKILL.md` into `~/.config/hive/skills/<name>/` or
  `./.hive/skills/<name>/`.
- **Subagent swarm** — up to 200 concurrent subagents with a depth cap.
- **Smart vision fallback** — routes images through a vision model when needed.

## Layout

```
lib/
  comb            native TUI engine (termios + ANSI, surfaces, widgets)

crates/
  hive-core       contracts + agent loop, tools, swarm, skills, vision
  hive-llm        Fireworks / OpenAI-compatible provider
  hive-tui        terminal frontend on comb
  hive            binary — composition root
```

### Extending

- **New tool** → `crates/hive-core/src/tools/` + `inventory::submit!`
- **New provider** → implement `LlmProvider` in `hive-llm` (or a new crate)
- **New skill source** → implement `SkillSource`

## Setup

Requires a Rust toolchain.

```bash
cargo run -p hive
```

Config lives at `~/.config/hive/config.toml` (created on first run). Put your key there:

```toml
[provider]
api_key = "fw_..."
```

Or export `FIREWORKS_API_KEY` instead — env always wins when set.
