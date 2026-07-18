use serde::Deserialize;

/// Named model roles the agent can route work to. The model behind each role is
/// configurable; the agent only ever refers to roles, never hardcoded ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Default,
    Smart,
    Fast,
    Vision,
}

impl ModelRole {
    pub fn parse(s: &str) -> Option<ModelRole> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" | "main" => Some(ModelRole::Default),
            "smart" | "deep" | "backend" => Some(ModelRole::Smart),
            "fast" | "simple" | "quick" => Some(ModelRole::Fast),
            "vision" | "image" => Some(ModelRole::Vision),
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
    pub default: ModelRef,
    pub smart: ModelRef,
    pub fast: ModelRef,
    pub vision: ModelRef,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        ModelsConfig {
            default: ModelRef::Named {
                id: "accounts/fireworks/routers/kimi-k2p6-fast".into(),
                name: Some("Kimi Fast".into()),
            },
            smart: ModelRef::Named {
                id: "accounts/fireworks/models/glm-5p2".into(),
                name: Some("GLM 5.2".into()),
            },
            fast: ModelRef::Named {
                id: "accounts/fireworks/models/deepseek-v4-flash".into(),
                name: Some("DeepSeek Flash".into()),
            },
            vision: ModelRef::Named {
                id: "accounts/fireworks/models/kimi-k2p6".into(),
                name: Some("Kimi".into()),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VisionConfig {
    /// Models that natively accept image inputs. Anything not listed here goes
    /// through the describe-and-inject fallback.
    pub capable: Vec<String>,
}

impl Default for VisionConfig {
    fn default() -> Self {
        VisionConfig {
            capable: vec![
                "accounts/fireworks/models/kimi-k2p6".to_string(),
                "accounts/fireworks/routers/kimi-k2p6-fast".to_string(),
            ],
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            theme: "mocha".to_string(),
        }
    }
}

/// Secrets resolved from the environment at startup. Never serialized.
#[derive(Debug, Clone, Default)]
pub struct Secrets {
    pub provider_api_key: String,
    pub exa_api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AppConfig {
    pub provider: ProviderConfig,
    pub models: ModelsConfig,
    pub vision: VisionConfig,
    pub exa: ExaConfig,
    pub swarm: SwarmConfig,
    pub ui: UiConfig,
    #[serde(skip)]
    pub secrets: Secrets,
}

impl AppConfig {
    fn model_ref(&self, role: ModelRole) -> &ModelRef {
        match role {
            ModelRole::Default => &self.models.default,
            ModelRole::Smart => &self.models.smart,
            ModelRole::Fast => &self.models.fast,
            ModelRole::Vision => &self.models.vision,
        }
    }

    /// Resolve the concrete provider model id for a role.
    pub fn model(&self, role: ModelRole) -> &str {
        self.model_ref(role).id()
    }

    /// Pretty display name for a role (UI).
    pub fn model_display(&self, role: ModelRole) -> &str {
        self.model_ref(role).display_name()
    }

    /// Look up a display name for a concrete id (role match or short id).
    pub fn display_for_model_id(&self, id: &str) -> String {
        for role in [
            ModelRole::Default,
            ModelRole::Smart,
            ModelRole::Fast,
            ModelRole::Vision,
        ] {
            let r = self.model_ref(role);
            if r.id() == id {
                return r.display_name().to_string();
            }
        }
        short_model_id(id).to_string()
    }

    pub fn is_vision_capable(&self, model: &str) -> bool {
        self.vision.capable.iter().any(|m| m == model)
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
