# hive

A native **Rust + TUI** YOLO coding agent — like Claude Code / Codex, but yours.
No tool-call confirmations: you say it, hive does it. Beautiful terminal UI,
streaming responses, skills, web search, a subagent swarm, and a smart vision
fallback.

## Features

- **YOLO mode** — the agent uses tools immediately, never asking for permission.
- **Beautiful TUI** (ratatui) — live token streaming, braille spinners, collapsible
  tool cards, a swarm panel, markdown rendering, a Catppuccin-Mocha palette.
- **Tools** — `read_file`, `write_file`, `edit_file`, `list_dir`, `glob`, `grep`,
  `run_shell` (streamed output), `web_search` / `web_get_contents` (Exa),
  `read_skill`, `spawn_subagent`, `spawn_swarm`.
- **Skills** — drop a `SKILL.md` (with `name`/`description` frontmatter) into
  `~/.config/hive/skills/<name>/` or `./.hive/skills/<name>/`; hive advertises it
  and loads it on demand.
- **Subagent swarm** — run up to 200 subagents concurrently (configurable), with a
  recursion depth cap. Each picks a model role per task.
- **Smart vision fallback** — if the active model can't see images, hive routes them
  through a vision-capable model to get a detailed description, then feeds that back
  as text. Vision-capable models get the image directly.

## Architecture

A small, decoupled Cargo workspace built around a thin contract layer
(`hive-core`). Every crate depends only on `hive-core` (traits + domain types);
the `hive` binary is the sole composition root that wires concrete
implementations together. `hive-agent` is the brain: the loop plus everything
it needs to do its work (tools, swarm, skills, vision) as internal modules.

```
crates/
  hive-core     contracts: LlmProvider, Tool, SkillSource, SubagentSpawner,
                VisionDescriber + domain types (Message, AgentEvent, config)
  hive-llm      Fireworks (OpenAI-compatible) provider: wire/, stream/, provider/
  hive-agent    the brain:
                  agent / session / prompt   the YOLO loop + state
                  tools/                     fs, shell, web, skill, delegate
                                             (self-registered via `inventory`)
                  swarm                      concurrency (Semaphore + JoinSet, depth cap)
                  skills                     disk-backed SKILL.md loader
                  vision                     describe-and-inject fallback
  hive-tui      ratatui frontend: theme, render/ (per-widget), app/ (state+input)
  hive          composition root: config, driver, wire
```

### Extending it

- **New tool** → add it to a file under `crates/hive-agent/src/tools/`
  implementing `Tool` and `inventory::submit!` it. It appears automatically; no
  central edits.
- **New provider** → implement `LlmProvider`; select in `hive/src/wire.rs`.
- **New skill source** → implement `SkillSource`.
- **New frontend** → consume `AgentEvent`s; the agent is UI-agnostic.

## Setup

Requires a Rust toolchain.

```bash
export FIREWORKS_API_KEY=...     # required
export EXA_API_KEY=...           # optional, enables web search
cargo run --release
```

On first run, hive writes a default config to `~/.config/hive/config.toml` and
seeds a sample `git-commit` skill.

## Configuration (`~/.config/hive/config.toml`)

- `[provider]` — `base_url`, `api_key_env` (any OpenAI-compatible endpoint).
- `[models]` — model ids for the `default`, `smart`, `fast`, and `vision` roles.
- `[vision] capable` — models that natively accept images.
- `[exa]` — `api_key_env`, `base_url`.
- `[swarm]` — `max_concurrent` (default 200), `max_depth`.
- `[ui]` — `theme`.

Default model roles (Fireworks):

- `default` — `kimi-k2p6-fast` (vision-capable, great for frontend)
- `smart` — `glm-5p2` (backend / deep reasoning)
- `fast` — `deepseek-v4-flash` (commits, merges, simple ops)
- `vision` — `kimi-k2p6` (describes images for non-vision models)

## Keys & commands

- `Enter` send · `Alt+Enter` newline · `Ctrl+C` stop current turn · `Ctrl+Q` quit
- `PgUp`/`PgDn` scroll
- `/model <id-or-role>` · `/image <path>` · `/clear` · `/cost` · `/help` · `/quit`

Logs are written to `~/.cache/hive/hive.log` (never the TUI).
