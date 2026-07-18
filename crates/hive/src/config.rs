//! Loading `config.toml` and resolving secrets from the environment.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

use hive_core::config::AppConfig;

/// `~/.config/hive` (or platform equivalent).
pub fn config_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.config_dir().join("hive"))
        .unwrap_or_else(|| PathBuf::from(".hive"))
}

const DEFAULT_CONFIG: &str = r#"# hive configuration

[provider]
# Any OpenAI-compatible endpoint. Default: Fireworks.
base_url = "https://api.fireworks.ai/inference/v1"
api_key_env = "FIREWORKS_API_KEY"

[models]
default = "accounts/fireworks/routers/kimi-k2p6-fast"  # main (vision-capable)
smart   = "accounts/fireworks/models/glm-5p2"          # backend / deep reasoning
fast    = "accounts/fireworks/models/deepseek-v4-flash" # commits / merges / simple
vision  = "accounts/fireworks/models/kimi-k2p6"         # describes images for non-vision models

[vision]
# Models that natively accept images. Everything else uses the vision fallback.
capable = [
    "accounts/fireworks/models/kimi-k2p6",
    "accounts/fireworks/routers/kimi-k2p6-fast",
]

[exa]
api_key_env = "EXA_API_KEY"
base_url = "https://api.exa.ai"

[swarm]
max_concurrent = 200
max_depth = 2

[ui]
theme = "mocha"
"#;

/// Load config from disk (writing a default on first run) and fill in secrets
/// from the environment.
pub fn load() -> Result<Arc<AppConfig>> {
    let dir = config_dir();
    let path = dir.join("config.toml");

    let mut cfg: AppConfig = if path.exists() {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    } else {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(&path, DEFAULT_CONFIG);
        AppConfig::default()
    };

    let provider_key = std::env::var(&cfg.provider.api_key_env)
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "{} is not set. Export your provider API key, e.g.\n    export {}=...",
                cfg.provider.api_key_env,
                cfg.provider.api_key_env
            )
        })?;
    cfg.secrets.provider_api_key = provider_key;
    cfg.secrets.exa_api_key = std::env::var(&cfg.exa.api_key_env)
        .ok()
        .filter(|s| !s.is_empty());

    Ok(Arc::new(cfg))
}
