use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

/// Legacy role name kept so older tool calls / configs still parse.
/// Everything resolves to the single configured model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    #[default]
    Default,
}

impl ModelRole {
    pub fn parse(s: &str) -> Option<ModelRole> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" | "main" | "smart" | "deep" | "backend" | "fast" | "simple" | "quick"
            | "vision" | "image" => Some(ModelRole::Default),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub base_url: String,
    /// Env var name checked first (overrides [`Self::api_key`] when set).
    pub api_key_env: String,
    /// Optional key from `config.toml`. Prefer env for CI; file for local use.
    pub api_key: Option<String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        ProviderConfig {
            base_url: "https://api.fireworks.ai/inference/v1".to_string(),
            api_key_env: "FIREWORKS_API_KEY".to_string(),
            api_key: None,
        }
    }
}

/// One model slot: provider `id` plus optional pretty `name` for the UI.
///
/// TOML accepts either a bare string (id only) or a table:
/// ```toml
/// default = "accounts/…/kimi-k2p6-fast"
/// # or
/// default = { id = "accounts/…/kimi-k2p6-fast", name = "Kimi Fast" }
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ModelRef {
    Id(String),
    Named {
        id: String,
        #[serde(default, alias = "display_name", alias = "label")]
        name: Option<String>,
    },
}

impl ModelRef {
    pub fn id(&self) -> &str {
        match self {
            ModelRef::Id(id) => id,
            ModelRef::Named { id, .. } => id,
        }
    }

    /// Pretty label for the TUI; falls back to the last path segment of `id`.
    pub fn display_name(&self) -> &str {
        match self {
            ModelRef::Named { name: Some(n), .. } if !n.trim().is_empty() => n.trim(),
            _ => short_model_id(self.id()),
        }
    }
}

impl Default for ModelRef {
    fn default() -> Self {
        ModelRef::Id(String::new())
    }
}

fn short_model_id(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ModelsConfig {
    /// The one model Hive uses for chat, tools, and subagents.
    pub default: ModelRef,
    /// Optional curated picker (`id` + pretty `name`). Non-empty replaces
    /// `GET /models` for the active provider when that profile has no
    /// `models` list of its own.
    #[serde(default)]
    pub catalog: Vec<ModelRef>,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        ModelsConfig {
            default: ModelRef::Named {
                id: "accounts/fireworks/routers/kimi-k2p6-fast".into(),
                name: Some("Kimi Fast".into()),
            },
            catalog: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchBackend {
    #[default]
    Exa,
    Perplexity,
    None,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub backend: SearchBackend,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            backend: SearchBackend::Exa,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExaConfig {
    pub api_key_env: String,
    /// Optional key from `config.toml` (env wins when set).
    pub api_key: Option<String>,
    pub base_url: String,
}

impl Default for ExaConfig {
    fn default() -> Self {
        ExaConfig {
            api_key_env: "EXA_API_KEY".to_string(),
            api_key: None,
            base_url: "https://api.exa.ai".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PerplexityConfig {
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
}

impl Default for PerplexityConfig {
    fn default() -> Self {
        PerplexityConfig {
            api_key_env: "PERPLEXITY_API_KEY".to_string(),
            api_key: None,
            base_url: "https://api.perplexity.ai".to_string(),
            model: "sonar".to_string(),
        }
    }
}

/// Hard ceiling on occupied subagent slots (running or finished-not-closed).
pub const MAX_AGENT_SLOTS: usize = 6;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    /// Occupied job slots (running + finished until `agent_close`). Capped at [`MAX_AGENT_SLOTS`].
    pub max_concurrent: usize,
    /// Nested spawn depth. Workers are depth 1 and cannot spawn further when this is 1.
    pub max_depth: usize,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        AgentsConfig {
            max_concurrent: 3,
            max_depth: 1,
        }
    }
}

impl AgentsConfig {
    pub fn slot_limit(&self) -> usize {
        self.max_concurrent.clamp(1, MAX_AGENT_SLOTS)
    }
}

/// How the right project panel behaves on wide terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidebarMode {
    /// User can show/hide with the › control (default).
    #[default]
    Auto,
    /// Always open; hide control disabled.
    Pinned,
    /// Always hidden (even on wide terminals).
    Hidden,
}

impl SidebarMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SidebarMode::Auto => "auto",
            SidebarMode::Pinned => "pinned",
            SidebarMode::Hidden => "hidden",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SidebarMode::Auto => "auto",
            SidebarMode::Pinned => "pinned",
            SidebarMode::Hidden => "hidden",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            SidebarMode::Auto => SidebarMode::Pinned,
            SidebarMode::Pinned => SidebarMode::Hidden,
            SidebarMode::Hidden => SidebarMode::Auto,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
    /// Set true after first-run intro (or when the user skips an existing config).
    #[serde(default)]
    pub setup_complete: bool,
    /// Keep reasoning/thought blocks expanded in the transcript.
    #[serde(default)]
    pub thoughts_always_open: bool,
    /// Right panel: auto / pinned / hidden.
    #[serde(default)]
    pub sidebar_mode: SidebarMode,
    /// Allow ▾/▸ collapse on Context / Sub agents / Changes.
    #[serde(default = "default_true")]
    pub sidebar_collapse_sections: bool,
    /// Right panel width in columns (clamped in the TUI to min/max).
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: u16,
    /// Show a "Worked for Nm" summary line at the end of each turn.
    #[serde(default = "default_true")]
    pub show_work_summary: bool,
    /// Offer "Revert <file>" on tool cards. Off hides it entirely, so a stray
    /// click can't roll a file back.
    #[serde(default = "default_true")]
    pub tool_revert: bool,
    /// Keep successful tool-call cards in the transcript. Active and failed
    /// tools stay visible even when this is off.
    #[serde(default = "default_true")]
    pub show_tool_cards: bool,
    /// Ripple on the landing HIVE wordmark when you click it.
    #[serde(default = "default_true")]
    pub logo_animation: bool,
    /// Play the click sample with the logo bonk.
    #[serde(default = "default_true")]
    pub sound: bool,
}

fn default_true() -> bool {
    true
}

fn default_sidebar_width() -> u16 {
    34
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            theme: "gray".to_string(),
            setup_complete: false,
            thoughts_always_open: false,
            sidebar_mode: SidebarMode::Auto,
            sidebar_collapse_sections: true,
            sidebar_width: default_sidebar_width(),
            show_work_summary: true,
            tool_revert: true,
            show_tool_cards: true,
            logo_animation: true,
            sound: true,
        }
    }
}

/// Secrets resolved from the environment at startup. Never serialized.
#[derive(Debug, Clone, Default)]
pub struct Secrets {
    pub provider_api_key: String,
    /// Per-profile keys (`/connect` id → API key) for multi-provider `/model`.
    pub connection_keys: std::collections::HashMap<String, String>,
    pub exa_api_key: Option<String>,
    pub perplexity_api_key: Option<String>,
}

/// One saved provider profile (`/connect`).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ConnectionProfile {
    pub label: String,
    pub base_url: String,
    pub api_key_env: String,
    pub api_key: Option<String>,
    pub model: ModelRef,
    /// Curated picker for this provider. Non-empty replaces `GET /models`
    /// so a local/test API can show only the ids you named.
    #[serde(default)]
    pub models: Vec<ModelRef>,
}

/// Saved providers + which one is active. Mirrored into `[provider]` / `[models]`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ConnectionsConfig {
    /// Id key in `profiles`.
    #[serde(default)]
    pub active: String,
    #[serde(default)]
    pub profiles: BTreeMap<String, ConnectionProfile>,
}

/// Default model context window (tokens) when unset in config.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 256_000;

/// Agent runtime knobs (context window, compaction, filesystem scope).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Model context window in tokens. Auto-compact fires at 75% of this.
    pub context_window: u64,
    /// Keep filesystem tools inside the agent's current workspace by default.
    ///
    /// `run_shell` is intentionally still a direct user-shell capability. This
    /// flag protects the structured filesystem tools and makes their boundary
    /// explicit; it is not a substitute for an OS-level shell sandbox.
    #[serde(default = "default_true")]
    pub workspace_only: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            context_window: DEFAULT_CONTEXT_WINDOW,
            workspace_only: true,
        }
    }
}

/// One stdio MCP server: a process speaking JSON-RPC over stdin/stdout.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct McpServerConfig {
    /// Executable to spawn.
    pub command: String,
    /// Arguments passed to `command`.
    pub args: Vec<String>,
    /// Extra environment variables for the server process.
    pub env: std::collections::HashMap<String, String>,
    /// Per-request timeout in seconds (default 60).
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub provider: ProviderConfig,
    pub models: ModelsConfig,
    pub search: SearchConfig,
    pub exa: ExaConfig,
    pub perplexity: PerplexityConfig,
    /// Subagent job slots. TOML `[agents]` or legacy `[swarm]`.
    #[serde(default, alias = "swarm")]
    pub agents: AgentsConfig,
    pub ui: UiConfig,
    /// Agent loop settings (context window / compact).
    #[serde(default)]
    pub agent: AgentConfig,
    /// Reserved for multi-provider support.
    #[serde(default)]
    pub connections: ConnectionsConfig,
    /// External MCP tools via stdio servers: `[mcp_servers.<id>]`.
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    #[serde(skip)]
    pub secrets: Secrets,
}

impl AppConfig {
    fn model_ref(&self, _role: ModelRole) -> &ModelRef {
        &self.models.default
    }

    /// Resolve the configured model id (`role` is ignored — one model only).
    pub fn model(&self, role: ModelRole) -> &str {
        self.model_ref(role).id()
    }

    /// Pretty display name for the configured model.
    pub fn model_display(&self, role: ModelRole) -> &str {
        self.model_ref(role).display_name()
    }

    /// Look up a display name for a concrete id (configured model or short id).
    pub fn display_for_model_id(&self, id: &str) -> String {
        let r = &self.models.default;
        if r.id() == id {
            return r.display_name().to_string();
        }
        for m in self.curated_models_for(&self.connections.active) {
            if m.id() == id {
                return m.display_name().to_string();
            }
        }
        short_model_id(id).to_string()
    }

    /// Curated picker for a connection. Non-empty means skip auto-detect.
    ///
    /// Profile `models` wins; `[models].catalog` is the fallback for the
    /// active provider so a simple config doesn't have to touch `[connections]`.
    pub fn curated_models_for(&self, connection_id: &str) -> &[ModelRef] {
        if let Some(profile) = self.connections.profiles.get(connection_id) {
            if profile.models.iter().any(|m| !m.id().trim().is_empty()) {
                return &profile.models;
            }
        }
        if !self
            .models
            .catalog
            .iter()
            .any(|m| !m.id().trim().is_empty())
        {
            return &[];
        }
        let active = self.connections.active.as_str();
        let is_active = connection_id == active
            || self.connections.profiles.is_empty()
            || (active.is_empty() && (connection_id.is_empty() || connection_id == "default"));
        if is_active {
            &self.models.catalog
        } else {
            &[]
        }
    }

    pub fn has_curated_models(&self, connection_id: &str) -> bool {
        self.curated_models_for(connection_id)
            .iter()
            .any(|m| !m.id().trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ref_display_falls_back_to_short_id() {
        let bare = ModelRef::Id("accounts/fireworks/models/foo".into());
        assert_eq!(bare.id(), "accounts/fireworks/models/foo");
        assert_eq!(bare.display_name(), "foo");

        let named = ModelRef::Named {
            id: "accounts/fireworks/models/bar".into(),
            name: Some("Bar".into()),
        };
        assert_eq!(named.display_name(), "Bar");
    }

    #[test]
    fn curated_models_parse_on_profile_and_top_level_catalog() {
        let mut cfg = AppConfig::default();
        cfg.models.default = ModelRef::Named {
            id: "local-a".into(),
            name: Some("A".into()),
        };
        cfg.models.catalog = vec![
            ModelRef::Named {
                id: "local-a".into(),
                name: Some("A".into()),
            },
            ModelRef::Id("local-b".into()),
        ];
        cfg.connections.active = "local".into();
        cfg.connections.profiles.insert(
            "local".into(),
            ConnectionProfile {
                label: "Local".into(),
                base_url: "http://127.0.0.1:8000/v1".into(),
                api_key_env: "LOCAL_API_KEY".into(),
                model: ModelRef::Named {
                    id: "local-a".into(),
                    name: Some("A".into()),
                },
                ..Default::default()
            },
        );
        cfg.connections.profiles.insert(
            "lab".into(),
            ConnectionProfile {
                label: "Lab".into(),
                base_url: "http://127.0.0.1:9000/v1".into(),
                api_key_env: "LAB_API_KEY".into(),
                models: vec![ModelRef::Named {
                    id: "exp-1".into(),
                    name: Some("Experiment".into()),
                }],
                ..Default::default()
            },
        );

        assert_eq!(cfg.curated_models_for("lab").len(), 1);
        assert_eq!(cfg.curated_models_for("lab")[0].id(), "exp-1");
        assert_eq!(
            cfg.curated_models_for("lab")[0].display_name(),
            "Experiment"
        );
        // Active profile has no `models` list → top-level catalog.
        let active = cfg.curated_models_for("local");
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].display_name(), "A");
        assert_eq!(active[1].id(), "local-b");
        assert!(cfg.has_curated_models("local"));
        assert!(cfg.has_curated_models("lab"));
        assert!(!cfg.has_curated_models("missing"));
    }
}
