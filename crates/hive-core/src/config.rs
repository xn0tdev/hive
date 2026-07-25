use std::collections::BTreeMap;

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
}

impl Default for ModelsConfig {
    fn default() -> Self {
        ModelsConfig {
            default: ModelRef::Named {
                id: "accounts/fireworks/routers/kimi-k2p6-fast".into(),
                name: Some("Kimi Fast".into()),
            },
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SwarmConfig {
    pub max_concurrent: usize,
    pub max_depth: usize,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        SwarmConfig {
            max_concurrent: 200,
            max_depth: 2,
        }
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

/// Agent runtime knobs (context window, compaction).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// Model context window in tokens. Auto-compact fires at 75% of this.
    pub context_window: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            context_window: DEFAULT_CONTEXT_WINDOW,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub provider: ProviderConfig,
    pub models: ModelsConfig,
    pub search: SearchConfig,
    pub exa: ExaConfig,
    pub perplexity: PerplexityConfig,
    pub swarm: SwarmConfig,
    pub ui: UiConfig,
    /// Agent loop settings (context window / compact).
    #[serde(default)]
    pub agent: AgentConfig,
    /// Reserved for multi-provider support.
    #[serde(default)]
    pub connections: ConnectionsConfig,
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
        short_model_id(id).to_string()
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
}
