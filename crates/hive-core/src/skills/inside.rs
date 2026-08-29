//! Built-in "inside skills" — compiled into the binary, immutable, always
//! available. They describe hive's own configuration so the agent can pull them
//! with `read_skill` just like user skills, but they can't be edited or removed.

use crate::skill::{SkillMeta, SkillSource};

struct Inside {
    name: &'static str,
    description: &'static str,
    content: &'static str,
}

const HIVE_CONFIG: &str = r#"---
name: hive-config
description: Hive's own config.toml — add/change providers, models, keys, search, MCP, agent knobs.
---

# hive-config

Edit Hive itself: providers, models, API keys, search, MCP, agent limits.
This is `~/.config/hive/config.toml` (Windows: `%USERPROFILE%\.config\hive\config.toml`).
Not the project. Not `.hive/` in the repo.

## How to touch the file

Structured fs tools (`read_file`, `edit_file`, `write_file`) stay inside the
workspace by default (`[agent].workspace_only = true`). Config is outside.
Use `run_shell` to read/patch it.

- Read first. Patch surgically. Do not rewrite the whole file (comments + extra
  keys disappear).
- Env vars always win over `api_key` in the file. Prefer `api_key_env` when the
  user already exports a key; write `api_key` only when they pasted one.
- Never echo a full key back to the user. Last 4 chars is enough.
- Do not commit this file. Do not copy keys into the repo.
- The running Hive process does not hot-reload a hand-edited `config.toml`.
  After provider / model / MCP / key changes, tell the user to restart `hive`
  (or use `/connect` and `/model` in the TUI, which persist and reload).

## File shape

```toml
[provider]
base_url = "https://api.fireworks.ai/inference/v1"
api_key_env = "FIREWORKS_API_KEY"
# api_key = "fw_..."          # optional; env wins when set

[models]
default = { id = "accounts/fireworks/routers/kimi-k2p6-fast", name = "Kimi Fast" }
# Optional curated picker. Non-empty replaces GET /models for the active provider.
# catalog = [
#   { id = "my-local-model", name = "Local" },
# ]

[connections]
active = "fireworks"

[connections.profiles.fireworks]
label = "Fireworks"
base_url = "https://api.fireworks.ai/inference/v1"
api_key_env = "FIREWORKS_API_KEY"
# api_key = "fw_..."
model = { id = "accounts/fireworks/routers/kimi-k2p6-fast", name = "Kimi Fast" }
# Per-provider curated picker (wins over [models].catalog). Same effect:
# models = [
#   { id = "exp-1", name = "Experiment" },
# ]

[search]
backend = "exa"               # exa | perplexity | none

[exa]
api_key_env = "EXA_API_KEY"
base_url = "https://api.exa.ai"

[perplexity]
api_key_env = "PERPLEXITY_API_KEY"
base_url = "https://api.perplexity.ai"
model = "sonar"

[agents]
max_concurrent = 3            # occupied job slots; hard cap 6
max_depth = 1

[agent]
context_window = 256000       # auto-compact ~75% of this
workspace_only = true

[ui]
theme = "gray"
# also: setup_complete, thoughts_always_open, sidebar_mode (auto|pinned|hidden),
# sidebar_collapse_sections, sidebar_width, show_work_summary, tool_revert,
# show_tool_cards, logo_animation, sound

# [mcp_servers.github]
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-github"]
# timeout_secs = 60
# [mcp_servers.github.env]
# GITHUB_PERSONAL_ACCESS_TOKEN = "..."
```

`[provider]` + `[models]` are the *active* connection, mirrored from
`[connections.profiles.<active>]`. Keep them in sync when switching.

## Add a custom / extra provider

Any OpenAI-compatible chat endpoint works. `base_url` must include the API
prefix that `/chat/completions` and `/models` hang off (usually `.../v1`).

Known hosts (label is what `/connect` shows):

| host | label | typical base_url |
|------|-------|------------------|
| api.fireworks.ai | Fireworks | `https://api.fireworks.ai/inference/v1` |
| openrouter.ai | OpenRouter | `https://openrouter.ai/api/v1` |
| api.openai.com | OpenAI | `https://api.openai.com/v1` |
| api.anthropic.com | Anthropic | `https://api.anthropic.com/v1` |
| api.groq.com | Groq | `https://api.groq.com/openai/v1` |
| api.deepseek.com | DeepSeek | `https://api.deepseek.com/v1` |
| api.together.xyz | Together | `https://api.together.xyz/v1` |
| api.mistral.ai | Mistral | `https://api.mistral.ai/v1` |
| generativelanguage.googleapis.com | Google AI | `https://generativelanguage.googleapis.com/v1beta/openai` |
| api.x.ai | xAI | `https://api.x.ai/v1` |
| api.cerebras.ai | Cerebras | `https://api.cerebras.ai/v1` |
| 127.0.0.1:11434 | Ollama | `http://127.0.0.1:11434/v1` |
| 127.0.0.1:1234 | LM Studio | `http://127.0.0.1:1234/v1` |

Unknown host → still fine. Use a short id (slug: lowercase, digits, dashes).

Steps:

1. Pick `id` (e.g. `openrouter`, `local-ollama`) and a `label`.
2. Append a new table. Do **not** change `connections.active`, `[provider]`, or
   `[models]` unless the user asked to switch *now*. Adding must not yank the
   current provider out from under the session.

```toml
[connections.profiles.openrouter]
label = "OpenRouter"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
api_key = "sk-or-..."          # omit if they use the env var
# model = { id = "...", name = "..." }   # optional until they pick one
```

3. If they *did* ask to switch now: set `connections.active = "<id>"`, copy
   that profile's `base_url` / `api_key_env` / `api_key` into `[provider]`, and
   copy `model` into `[models].default` (skip model if the profile has none —
   they pick it in `/model` after restart).
4. Confirm the file still parses (`python3 -c 'import tomllib,pathlib; tomllib.loads(pathlib.Path.home().joinpath(".config/hive/config.toml").read_text())'` on 3.11+, or `hive` restart).
5. Tell them to restart, then `/model` if no model was set.

Do not delete the last profile. Do not wipe `api_key` on an existing profile
when you are only adding another one.

## Change the active model

If they only want a different model on the **current** provider: set both
`[models].default` and `connections.profiles.<active>.model` to
`{ id = "...", name = "..." }`. `id` is what the API gets; `name` is the TUI
label (last path segment if omitted).

## Curated model list (skip auto-detect)

By default `/model` calls `GET {base_url}/models` and shows everything the
endpoint returns. For a local/test API that dumps dozens of ids, pin the
picker instead.

- `[models].catalog` — curated list for the **active** provider.
- `connections.profiles.<id>.models` — curated list for that provider only
  (wins over `[models].catalog`).

Each entry is `{ id = "api-id", name = "Pretty name" }` or a bare `"api-id"`
(name = last path segment). Non-empty list **replaces** auto-detect for that
provider — other providers still list remotely. Empty / omitted = auto-detect.

```toml
[connections.profiles.local]
label = "Local"
base_url = "http://127.0.0.1:8000/v1"
api_key_env = "LOCAL_API_KEY"
model = { id = "hive-dev", name = "Hive Dev" }
models = [
  { id = "hive-dev", name = "Hive Dev" },
  { id = "hive-fast", name = "Hive Fast" },
]
```

Do not invent ids the endpoint cannot serve. Keep `model` / `[models].default`
as one of the curated ids.

## Search / MCP / knobs

- Search: `[search].backend`, plus `[exa]` / `[perplexity]` keys.
- MCP: `[mcp_servers.<id>]` with `command`, optional `args`, `timeout_secs`
  (default 60), optional `[mcp_servers.<id>.env]`. Empty `command` is ignored.
  Needs a Hive restart to spawn.
- Agent: `[agent]` (context window, workspace_only, turn caps) and `[agents]`
  (subagent slots). UI prefs are `[ui]` — `/settings` also writes these.

## If they can do it in the TUI

`/connect` adds/updates/removes providers and reloads live. `/model` switches
model (and activates that model's provider). Prefer telling them that when they
are sitting in Hive and just need a picker. Edit the file when they asked you
to configure it, or when they are not in the UI.
"#;

const INSIDE_SKILLS: &[Inside] = &[Inside {
    name: "hive-config",
    description:
        "Hive's own config.toml — add/change providers, models, keys, search, MCP, agent knobs.",
    content: HIVE_CONFIG,
}];

pub struct InsideSkills;

impl SkillSource for InsideSkills {
    fn list(&self) -> Vec<SkillMeta> {
        INSIDE_SKILLS
            .iter()
            .map(|s| SkillMeta {
                name: s.name.to_string(),
                description: s.description.to_string(),
                path: "(built-in)".to_string(),
            })
            .collect()
    }

    fn read(&self, name: &str) -> Option<String> {
        INSIDE_SKILLS
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.content.to_string())
    }
}

/// Merges multiple skill sources. Earlier sources win on name collisions
/// (inside skills take priority over disk skills with the same name).
pub struct CompositeSkills {
    sources: Vec<Box<dyn SkillSource>>,
}

impl CompositeSkills {
    pub fn new(sources: Vec<Box<dyn SkillSource>>) -> Self {
        CompositeSkills { sources }
    }
}

impl SkillSource for CompositeSkills {
    fn list(&self) -> Vec<SkillMeta> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for src in &self.sources {
            for meta in src.list() {
                if seen.insert(meta.name.clone()) {
                    out.push(meta);
                }
            }
        }
        out
    }

    fn read(&self, name: &str) -> Option<String> {
        for src in &self.sources {
            if let Some(content) = src.read(name) {
                return Some(content);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::SkillSource;

    #[test]
    fn inside_skills_include_hive_config() {
        let src = InsideSkills;
        let names: Vec<_> = src.list().into_iter().map(|m| m.name).collect();
        assert!(names.contains(&"hive-config".to_string()), "{names:?}");

        let cfg = src.read("hive-config").expect("hive-config body");
        assert!(cfg.contains("config.toml"));
        assert!(cfg.contains("[connections.profiles"));
        assert!(cfg.contains("workspace_only"));
        assert!(cfg.contains("run_shell"));
        assert!(cfg.contains("[models].catalog"));
        assert!(cfg.contains("Curated model list"));
        assert!(src.read("missing").is_none());
    }

    #[test]
    fn composite_prefers_inside_over_later_sources() {
        struct Override;
        impl SkillSource for Override {
            fn list(&self) -> Vec<SkillMeta> {
                vec![SkillMeta {
                    name: "hive-config".into(),
                    description: "disk".into(),
                    path: "disk".into(),
                }]
            }
            fn read(&self, name: &str) -> Option<String> {
                (name == "hive-config").then(|| "from disk".into())
            }
        }

        let composite = CompositeSkills::new(vec![Box::new(InsideSkills), Box::new(Override)]);
        let body = composite.read("hive-config").unwrap();
        assert!(body.contains("Hive's own config.toml"), "{body}");
        assert!(!body.contains("from disk"));
    }
}
