//! Loading `config.toml` and resolving secrets (env overrides file).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

use hive_core::config::{AppConfig, ConnectionProfile, SearchBackend};
use hive_core::event::ConnectionInfo;
use hive_llm::catalog::provider_label_for_base;

/// `~/.config/hive` (or platform equivalent).
pub fn config_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.config_dir().join("hive"))
        .unwrap_or_else(|| PathBuf::from(".hive"))
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

const DEFAULT_CONFIG: &str = r#"# hive configuration
#
# Put API keys here so you don't need to export them every session.
# Environment variables always win when set.

[provider]
# Any OpenAI-compatible endpoint. Default: Fireworks.
base_url = "https://api.fireworks.ai/inference/v1"
api_key_env = "FIREWORKS_API_KEY"
# api_key = "fw_..."

[models]
# Bare string = provider id (display = last path segment).
# Or table: id for the API, name for the TUI footer.
default = { id = "accounts/fireworks/routers/kimi-k2p6-fast", name = "Kimi Fast" }

# Future: multiple saved providers. Today only [provider] is active.
# [connections]
# active = "default"

[search]
backend = "exa"

[exa]
api_key_env = "EXA_API_KEY"
# api_key = "..."
base_url = "https://api.exa.ai"

[perplexity]
api_key_env = "PERPLEXITY_API_KEY"
# api_key = "..."
base_url = "https://api.perplexity.ai"
model = "sonar"

[swarm]
max_concurrent = 200
max_depth = 2

[ui]
theme = "gray"
setup_complete = false
# Chat / sidebar prefs (also editable via /settings in the TUI)
thoughts_always_open = false
sidebar_mode = "auto"          # auto | pinned | hidden
sidebar_collapse_sections = true
sidebar_width = 34             # panel width (min 24, max 56)

# Saved providers for /connect (seeded automatically from [provider] on first load).
# [connections]
# active = "fireworks"
"#;

/// One model slot written by the intro wizard.
#[derive(Debug, Clone)]
pub struct SetupModel {
    pub id: String,
    pub name: String,
}

/// Choices from the first-run intro, ready to write as `config.toml`.
#[derive(Debug, Clone)]
pub struct SetupChoices {
    pub provider_base_url: String,
    pub provider_api_key_env: String,
    /// Key explicitly entered by the user. Values sourced from env stay `None`.
    pub provider_api_key: Option<String>,
    pub default: SetupModel,
    pub search_backend: SearchBackend,
    /// Key for Exa or Perplexity when that backend is selected.
    pub search_api_key: Option<String>,
}

/// Env var first, then optional value from the config file.
fn resolve_secret(env_name: &str, file_key: Option<&str>) -> Option<String> {
    std::env::var(env_name)
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            file_key
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

fn secure_config_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("securing {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_config_file(path: &Path, content: &str) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    secure_config_permissions(path)?;
    file.write_all(content.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

/// Parse config from disk (writing a default file if missing) without requiring
/// a provider key. Secrets that resolve are filled in; missing provider key is ok.
pub fn try_load() -> Result<Arc<AppConfig>> {
    let dir = config_dir();
    let path = dir.join("config.toml");

    let mut cfg: AppConfig = if path.exists() {
        secure_config_permissions(&path)?;
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    } else {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        write_config_file(&path, DEFAULT_CONFIG)?;
        AppConfig::default()
    };

    let needs_seed = cfg.connections.profiles.is_empty();
    ensure_connections(&mut cfg);
    if needs_seed {
        // Persist the seeded profile so /connect can switch later.
        let _ = persist_connections_from_cfg(&cfg);
    }
    fill_secrets(&mut cfg);
    Ok(Arc::new(cfg))
}

fn persist_connections_from_cfg(cfg: &AppConfig) -> Result<()> {
    for (id, p) in &cfg.connections.profiles {
        upsert_connection(
            id,
            &p.label,
            &p.base_url,
            &p.api_key_env,
            p.api_key.as_deref(),
            p.model.id(),
            p.model.display_name(),
        )?;
    }
    Ok(())
}

/// Seed / sync `[connections]` with the active `[provider]` + model.
fn ensure_connections(cfg: &mut AppConfig) {
    if cfg.connections.profiles.is_empty() {
        let label = provider_label_for_base(&cfg.provider.base_url).to_string();
        let id = connection_slug(&label);
        cfg.connections.active = id.clone();
        cfg.connections.profiles.insert(
            id,
            ConnectionProfile {
                label,
                base_url: cfg.provider.base_url.clone(),
                api_key_env: cfg.provider.api_key_env.clone(),
                api_key: cfg.provider.api_key.clone(),
                model: cfg.models.default.clone(),
            },
        );
        return;
    }

    if cfg.connections.active.is_empty()
        || !cfg
            .connections
            .profiles
            .contains_key(&cfg.connections.active)
    {
        if let Some((id, _)) = cfg.connections.profiles.iter().next() {
            cfg.connections.active = id.clone();
        }
    }

    if let Some(profile) = cfg
        .connections
        .profiles
        .get(&cfg.connections.active)
        .cloned()
    {
        cfg.provider.base_url = profile.base_url;
        cfg.provider.api_key_env = profile.api_key_env;
        if cfg.provider.api_key.is_none() {
            cfg.provider.api_key = profile.api_key;
        }
        cfg.models.default = profile.model;
    }
}

fn connection_slug(label: &str) -> String {
    let mut s: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "provider".into()
    } else {
        s
    }
}

fn host_detail(base_url: &str) -> String {
    base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(base_url)
        .to_string()
}

/// Snapshot of saved providers for the TUI `/connect` picker.
pub fn connection_infos(cfg: &AppConfig) -> Vec<ConnectionInfo> {
    cfg.connections
        .profiles
        .iter()
        .map(|(id, p)| ConnectionInfo {
            id: id.clone(),
            label: if p.label.is_empty() {
                id.clone()
            } else {
                p.label.clone()
            },
            detail: host_detail(&p.base_url),
        })
        .collect()
}

fn fill_secrets(cfg: &mut AppConfig) {
    // Resolve every saved profile key before wiping plaintext from the struct.
    cfg.secrets.connection_keys.clear();
    for (id, p) in &cfg.connections.profiles {
        if let Some(key) = resolve_secret(&p.api_key_env, p.api_key.as_deref()) {
            cfg.secrets.connection_keys.insert(id.clone(), key);
        }
    }

    // Prefer the active profile's key when present.
    let profile_key = cfg
        .secrets
        .connection_keys
        .get(&cfg.connections.active)
        .cloned();
    let file_key = cfg.provider.api_key.clone().or(profile_key);
    cfg.secrets.provider_api_key =
        resolve_secret(&cfg.provider.api_key_env, file_key.as_deref()).unwrap_or_default();
    // Keep active key in the map too (env may only be on [provider]).
    if !cfg.secrets.provider_api_key.is_empty() && !cfg.connections.active.is_empty() {
        cfg.secrets
            .connection_keys
            .entry(cfg.connections.active.clone())
            .or_insert_with(|| cfg.secrets.provider_api_key.clone());
    }
    cfg.secrets.exa_api_key = resolve_secret(&cfg.exa.api_key_env, cfg.exa.api_key.as_deref());
    cfg.secrets.perplexity_api_key = resolve_secret(
        &cfg.perplexity.api_key_env,
        cfg.perplexity.api_key.as_deref(),
    );

    // Don't keep plaintext copies on the live config struct beyond secrets.
    cfg.provider.api_key = None;
    cfg.exa.api_key = None;
    cfg.perplexity.api_key = None;
    for p in cfg.connections.profiles.values_mut() {
        p.api_key = None;
    }
}

/// True when a non-empty provider API key is available (env or file).
pub fn has_provider_key(cfg: &AppConfig) -> bool {
    !cfg.secrets.provider_api_key.trim().is_empty()
}

/// Load config and require a provider key (post-intro / normal startup).
pub fn load() -> Result<Arc<AppConfig>> {
    let cfg = try_load()?;
    if !has_provider_key(&cfg) {
        let path = config_path();
        return Err(anyhow!(
            "no provider API key. Set it in {}:\n\
             \n\
             \t[provider]\n\
             \tapi_key = \"...\"\n\
             \n\
             or export {}=...",
            path.display(),
            cfg.provider.api_key_env
        ));
    }
    Ok(cfg)
}

fn toml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn backend_str(b: SearchBackend) -> &'static str {
    match b {
        SearchBackend::Exa => "exa",
        SearchBackend::Perplexity => "perplexity",
        SearchBackend::None => "none",
    }
}

fn format_setup_toml(choices: &SetupChoices) -> String {
    let provider_key_line = choices
        .provider_api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .map(|key| format!("api_key = \"{}\"\n", toml_escape(key)))
        .unwrap_or_default();
    let (exa_key_line, pplx_key_line) = match choices.search_backend {
        SearchBackend::Exa => (
            choices
                .search_api_key
                .as_deref()
                .filter(|k| !k.is_empty())
                .map(|k| format!("api_key = \"{}\"\n", toml_escape(k)))
                .unwrap_or_default(),
            String::new(),
        ),
        SearchBackend::Perplexity => (
            String::new(),
            choices
                .search_api_key
                .as_deref()
                .filter(|k| !k.is_empty())
                .map(|k| format!("api_key = \"{}\"\n", toml_escape(k)))
                .unwrap_or_default(),
        ),
        SearchBackend::None => (String::new(), String::new()),
    };

    format!(
        r#"# hive configuration
#
# Generated by first-run setup. Environment variables always win when set.

[provider]
base_url = "{base_url}"
api_key_env = "{api_key_env}"
{provider_key}

[models]
default = {{ id = "{def_id}", name = "{def_name}" }}

# Future: multiple saved providers. Today only [provider] is active.
# [connections]
# active = "default"

[search]
backend = "{backend}"

[exa]
api_key_env = "EXA_API_KEY"
{exa_key}base_url = "https://api.exa.ai"

[perplexity]
api_key_env = "PERPLEXITY_API_KEY"
{pplx_key}base_url = "https://api.perplexity.ai"
model = "sonar"

[swarm]
max_concurrent = 200
max_depth = 2

[ui]
theme = "gray"
setup_complete = true
thoughts_always_open = false
sidebar_mode = "auto"
sidebar_collapse_sections = true
sidebar_width = 34
"#,
        base_url = toml_escape(&choices.provider_base_url),
        api_key_env = toml_escape(&choices.provider_api_key_env),
        provider_key = provider_key_line,
        def_id = toml_escape(&choices.default.id),
        def_name = toml_escape(&choices.default.name),
        backend = backend_str(choices.search_backend),
        exa_key = exa_key_line,
        pplx_key = pplx_key_line,
    )
}

/// Write a clean `config.toml` from intro wizard choices.
pub fn write_setup(choices: &SetupChoices) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join("config.toml");
    let mut choices = choices.clone();
    if path.exists() {
        secure_config_permissions(&path)?;
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(existing) = toml::from_str::<AppConfig>(&text) {
                choices.provider_api_key = choices.provider_api_key.or(existing.provider.api_key);
                match choices.search_backend {
                    SearchBackend::Exa => {
                        choices.search_api_key = choices.search_api_key.or(existing.exa.api_key);
                    }
                    SearchBackend::Perplexity => {
                        choices.search_api_key =
                            choices.search_api_key.or(existing.perplexity.api_key);
                    }
                    SearchBackend::None => {}
                }
            }
        }
    }
    let text = format_setup_toml(&choices);
    write_config_file(&path, &text)
}

/// Merge UI preference fields into the existing `config.toml` `[ui]` table.
/// Write every `[ui]` value we own into the table. Split out so it can be
/// tested without touching the real config file.
fn apply_ui_fields(
    ui_table: &mut toml::map::Map<String, toml::Value>,
    ui: &hive_core::config::UiConfig,
) {
    ui_table.insert("theme".into(), toml::Value::String(ui.theme.clone()));
    ui_table.insert(
        "setup_complete".into(),
        toml::Value::Boolean(ui.setup_complete),
    );
    ui_table.insert(
        "thoughts_always_open".into(),
        toml::Value::Boolean(ui.thoughts_always_open),
    );
    ui_table.insert(
        "sidebar_mode".into(),
        toml::Value::String(ui.sidebar_mode.as_str().into()),
    );
    ui_table.insert(
        "sidebar_collapse_sections".into(),
        toml::Value::Boolean(ui.sidebar_collapse_sections),
    );
    ui_table.insert(
        "sidebar_width".into(),
        toml::Value::Integer(i64::from(ui.sidebar_width)),
    );
    ui_table.insert(
        "show_work_summary".into(),
        toml::Value::Boolean(ui.show_work_summary),
    );
    ui_table.insert("tool_revert".into(), toml::Value::Boolean(ui.tool_revert));
    ui_table.insert(
        "show_tool_cards".into(),
        toml::Value::Boolean(ui.show_tool_cards),
    );
}

pub fn patch_ui(ui: &hive_core::config::UiConfig) -> Result<()> {
    use hive_core::config::SidebarMode;

    let path = config_path();
    let text = if path.exists() {
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };
    let mut value: toml::Value = if text.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    };
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a table"))?;
    let ui_entry = root
        .entry("ui")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let ui_table = ui_entry
        .as_table_mut()
        .ok_or_else(|| anyhow!("[ui] must be a table"))?;

    apply_ui_fields(ui_table, ui);

    // Keep mode parseable even if an old build wrote an unknown string.
    let _ = SidebarMode::default();

    let out = toml::to_string_pretty(&value).context("serializing config")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    write_config_file(&path, &out)
}

/// Persist `[agent].context_window` so compaction matches the active model.
pub fn patch_context_window(window: u64) -> Result<()> {
    let mut value = read_toml_root()?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a table"))?;
    let agent = root
        .entry("agent")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let agent_table = agent
        .as_table_mut()
        .ok_or_else(|| anyhow!("[agent] must be a table"))?;
    agent_table.insert("context_window".into(), toml::Value::Integer(window as i64));
    write_toml_root(&value)
}

/// Persist the active model into `[models].default` (+ active connection profile).
pub fn patch_model(id: &str, name: &str) -> Result<()> {
    let mut value = read_toml_root()?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a table"))?;
    write_model_table(root, id, name)?;

    if let Some(active) = root
        .get("connections")
        .and_then(|c| c.get("active"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
    {
        if let Some(profiles) = root
            .entry("connections")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .and_then(|c| {
                c.entry("profiles")
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
                    .as_table_mut()
            })
        {
            if let Some(profile) = profiles.get_mut(&active).and_then(|v| v.as_table_mut()) {
                let mut model = toml::map::Map::new();
                model.insert("id".into(), toml::Value::String(id.to_string()));
                model.insert("name".into(), toml::Value::String(name.to_string()));
                profile.insert("model".into(), toml::Value::Table(model));
            }
        }
    }

    write_toml_root(&value)
}

/// Activate a saved connection profile and mirror it into `[provider]` / `[models]`.
pub fn activate_connection(id: &str) -> Result<()> {
    let mut value = read_toml_root()?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a table"))?;
    let connections = root
        .get_mut("connections")
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| anyhow!("no [connections] in config — add a provider via /connect"))?;
    let profiles = connections
        .get_mut("profiles")
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| anyhow!("no connection profiles"))?;
    let profile = profiles
        .get(id)
        .and_then(|v| v.as_table())
        .ok_or_else(|| anyhow!("unknown connection: {id}"))?
        .clone();

    connections.insert("active".into(), toml::Value::String(id.to_string()));
    mirror_profile_to_provider(root, &profile)?;
    write_toml_root(&value)
}

/// Add or update a connection profile, make it active, mirror to `[provider]`.
pub fn upsert_connection(
    id: &str,
    label: &str,
    base_url: &str,
    api_key_env: &str,
    api_key: Option<&str>,
    model_id: &str,
    model_name: &str,
) -> Result<()> {
    let mut value = read_toml_root()?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a table"))?;

    let connections = root
        .entry("connections")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let connections = connections
        .as_table_mut()
        .ok_or_else(|| anyhow!("[connections] must be a table"))?;
    let profiles = connections
        .entry("profiles")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let profiles = profiles
        .as_table_mut()
        .ok_or_else(|| anyhow!("[connections.profiles] must be a table"))?;

    let existing_key = profiles
        .get(id)
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("api_key"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut profile = toml::map::Map::new();
    profile.insert("label".into(), toml::Value::String(label.to_string()));
    profile.insert("base_url".into(), toml::Value::String(base_url.to_string()));
    profile.insert(
        "api_key_env".into(),
        toml::Value::String(api_key_env.to_string()),
    );
    let key = api_key
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .or(existing_key);
    if let Some(key) = key {
        profile.insert("api_key".into(), toml::Value::String(key));
    }
    let mut model = toml::map::Map::new();
    model.insert("id".into(), toml::Value::String(model_id.to_string()));
    model.insert("name".into(), toml::Value::String(model_name.to_string()));
    profile.insert("model".into(), toml::Value::Table(model));

    profiles.insert(id.to_string(), toml::Value::Table(profile.clone()));
    connections.insert("active".into(), toml::Value::String(id.to_string()));
    mirror_profile_to_provider(root, &profile)?;
    write_toml_root(&value)
}

/// Replace one saved provider's API key without activating that provider.
pub fn update_connection_key(id: &str, api_key: &str) -> Result<()> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(anyhow!("API key cannot be empty"));
    }

    let mut value = read_toml_root()?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a table"))?;
    let active = root
        .get("connections")
        .and_then(|v| v.get("active"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let profile = {
        let profiles = root
            .get_mut("connections")
            .and_then(|v| v.get_mut("profiles"))
            .and_then(|v| v.as_table_mut())
            .ok_or_else(|| anyhow!("no connection profiles"))?;
        let profile = profiles
            .get_mut(id)
            .and_then(|v| v.as_table_mut())
            .ok_or_else(|| anyhow!("unknown connection: {id}"))?;
        profile.insert("api_key".into(), toml::Value::String(api_key.to_string()));
        profile.clone()
    };

    if active == id {
        mirror_profile_to_provider(root, &profile)?;
    }
    write_toml_root(&value)
}

/// Remove a saved connection. Refuses to delete the last profile.
pub fn remove_connection(id: &str) -> Result<()> {
    let mut value = read_toml_root()?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a table"))?;

    let active = root
        .get("connections")
        .and_then(|v| v.get("active"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let was_active = active == id;

    {
        let connections = root
            .get_mut("connections")
            .and_then(|v| v.as_table_mut())
            .ok_or_else(|| anyhow!("no [connections]"))?;
        let profiles = connections
            .get_mut("profiles")
            .and_then(|v| v.as_table_mut())
            .ok_or_else(|| anyhow!("no profiles"))?;
        if profiles.len() <= 1 {
            return Err(anyhow!("cannot remove the last provider"));
        }
        if profiles.remove(id).is_none() {
            return Err(anyhow!("unknown connection: {id}"));
        }
    }

    if was_active {
        let (next_id, next_profile) = {
            let profiles = root
                .get("connections")
                .and_then(|v| v.get("profiles"))
                .and_then(|v| v.as_table())
                .ok_or_else(|| anyhow!("no profiles"))?;
            let next_id = profiles
                .keys()
                .next()
                .cloned()
                .ok_or_else(|| anyhow!("no providers left"))?;
            let next_profile = profiles
                .get(&next_id)
                .and_then(|v| v.as_table())
                .cloned()
                .ok_or_else(|| anyhow!("missing next profile"))?;
            (next_id, next_profile)
        };
        if let Some(connections) = root.get_mut("connections").and_then(|v| v.as_table_mut()) {
            connections.insert("active".into(), toml::Value::String(next_id));
        }
        mirror_profile_to_provider(root, &next_profile)?;
    }
    write_toml_root(&value)
}

fn read_toml_root() -> Result<toml::Value> {
    let path = config_path();
    let text = if path.exists() {
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };
    if text.trim().is_empty() {
        Ok(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

fn write_toml_root(value: &toml::Value) -> Result<()> {
    let path = config_path();
    let out = toml::to_string_pretty(value).context("serializing config")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    write_config_file(&path, &out)
}

fn write_model_table(
    root: &mut toml::map::Map<String, toml::Value>,
    id: &str,
    name: &str,
) -> Result<()> {
    let models = root
        .entry("models")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let models_table = models
        .as_table_mut()
        .ok_or_else(|| anyhow!("[models] must be a table"))?;
    let mut entry = toml::map::Map::new();
    entry.insert("id".into(), toml::Value::String(id.to_string()));
    entry.insert("name".into(), toml::Value::String(name.to_string()));
    models_table.insert("default".into(), toml::Value::Table(entry));
    Ok(())
}

fn mirror_profile_to_provider(
    root: &mut toml::map::Map<String, toml::Value>,
    profile: &toml::map::Map<String, toml::Value>,
) -> Result<()> {
    let provider = root
        .entry("provider")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let provider = provider
        .as_table_mut()
        .ok_or_else(|| anyhow!("[provider] must be a table"))?;

    if let Some(v) = profile.get("base_url").cloned() {
        provider.insert("base_url".into(), v);
    }
    if let Some(v) = profile.get("api_key_env").cloned() {
        provider.insert("api_key_env".into(), v);
    }
    if let Some(v) = profile.get("api_key").cloned() {
        provider.insert("api_key".into(), v);
    }

    if let Some(model) = profile.get("model").and_then(|v| v.as_table()) {
        let id = model
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = model
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !id.is_empty() {
            write_model_table(root, &id, &name)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_core::config::AppConfig;

    /// Every toggle in Settings has to survive a restart.
    #[test]
    fn every_ui_toggle_is_written_back() {
        use hive_core::config::{SidebarMode, UiConfig};

        let ui = UiConfig {
            theme: "gray".into(),
            setup_complete: true,
            thoughts_always_open: true,
            sidebar_mode: SidebarMode::Auto,
            sidebar_collapse_sections: false,
            sidebar_width: 40,
            show_work_summary: false,
            tool_revert: false,
            show_tool_cards: false,
        };

        let mut table = toml::map::Map::new();
        apply_ui_fields(&mut table, &ui);

        let round: UiConfig = toml::Value::Table(table.clone())
            .try_into()
            .expect("parse back");
        assert_eq!(round.sidebar_width, 40);
        assert!(round.thoughts_always_open);
        assert!(!round.sidebar_collapse_sections);
        assert!(!round.show_work_summary, "work summary must persist");
        assert!(!round.tool_revert, "revert toggle must persist");
        assert!(!round.show_tool_cards, "completed tools must persist");
    }

    #[test]
    fn resolve_prefers_non_empty_env() {
        assert_eq!(
            resolve_secret("HIVE_TEST_UNSET_ENV_VAR_XYZ", Some("  file-key  ")),
            Some("file-key".into())
        );
        assert_eq!(
            resolve_secret("HIVE_TEST_UNSET_ENV_VAR_XYZ", Some("")),
            None
        );
        assert_eq!(resolve_secret("HIVE_TEST_UNSET_ENV_VAR_XYZ", None), None);
    }

    #[test]
    fn write_setup_round_trips_parse() {
        let choices = SetupChoices {
            provider_base_url: "https://openrouter.ai/api/v1".into(),
            provider_api_key_env: "OPENROUTER_API_KEY".into(),
            provider_api_key: Some("sk-test".into()),
            default: SetupModel {
                id: "model/a".into(),
                name: "A".into(),
            },
            search_backend: SearchBackend::Perplexity,
            search_api_key: Some("pplx-key".into()),
        };
        let text = format_setup_toml(&choices);
        let cfg: AppConfig = toml::from_str(&text).expect("parse written config");
        assert_eq!(cfg.provider.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(cfg.provider.api_key.as_deref(), Some("sk-test"));
        assert_eq!(cfg.models.default.id(), "model/a");
        assert_eq!(cfg.models.default.display_name(), "A");
        assert_eq!(cfg.search.backend, SearchBackend::Perplexity);
        assert_eq!(cfg.perplexity.api_key.as_deref(), Some("pplx-key"));
        assert!(cfg.ui.setup_complete);
        assert_eq!(cfg.ui.theme, "gray");
    }

    #[test]
    fn setup_omits_keys_resolved_from_environment() {
        let choices = SetupChoices {
            provider_base_url: "https://example.test/v1".into(),
            provider_api_key_env: "PROVIDER_KEY".into(),
            provider_api_key: None,
            default: SetupModel {
                id: "model/a".into(),
                name: "A".into(),
            },
            search_backend: SearchBackend::Exa,
            search_api_key: None,
        };

        let text = format_setup_toml(&choices);
        assert!(!text
            .lines()
            .any(|line| line.trim_start().starts_with("api_key =")));
    }

    #[cfg(unix)]
    #[test]
    fn config_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "hive-config-permissions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_config_file(&path, "secret").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        std::fs::remove_file(path).unwrap();
    }
}
