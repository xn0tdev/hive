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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ModelsConfig {
    pub default: String,
    pub smart: String,
    pub fast: String,
    pub vision: String,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        ModelsConfig {
            default: "accounts/fireworks/routers/kimi-k2p6-fast".to_string(),
            smart: "accounts/fireworks/models/glm-5p2".to_string(),
            fast: "accounts/fireworks/models/deepseek-v4-flash".to_string(),
            vision: "accounts/fireworks/models/kimi-k2p6".to_string(),
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
    /// Resolve the concrete model id for a role.
    pub fn model(&self, role: ModelRole) -> &str {
        match role {
            ModelRole::Default => &self.models.default,
            ModelRole::Smart => &self.models.smart,
            ModelRole::Fast => &self.models.fast,
            ModelRole::Vision => &self.models.vision,
        }
    }

    pub fn is_vision_capable(&self, model: &str) -> bool {
        self.vision.capable.iter().any(|m| m == model)
    }
}
