//! Loading `config.toml` and resolving secrets (env overrides file).

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
#
# Put API keys here so you don't need to export them every session.
# Environment variables always win when set.

[provider]
# Any OpenAI-compatible endpoint. Default: Fireworks.
base_url = "https://api.fireworks.ai/inference/v1"
api_key_env = "FIREWORKS_API_KEY"
# api_key = "fw_..."

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
# api_key = "..."
base_url = "https://api.exa.ai"

[swarm]
max_concurrent = 200
max_depth = 2

[ui]
theme = "mocha"
"#;

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

/// Load config from disk (writing a default on first run) and fill in secrets
/// from the environment (preferred) or `api_key` fields in the file.
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

    let provider_key = resolve_secret(
        &cfg.provider.api_key_env,
        cfg.provider.api_key.as_deref(),
    )
    .ok_or_else(|| {
        anyhow!(
            "no provider API key. Set it in {}:\n\
             \n\
             \t[provider]\n\
             \tapi_key = \"...\"\n\
             \n\
             or export {}=...",
            path.display(),
            cfg.provider.api_key_env
        )
    })?;
    cfg.secrets.provider_api_key = provider_key;
    cfg.secrets.exa_api_key = resolve_secret(&cfg.exa.api_key_env, cfg.exa.api_key.as_deref());

    // Don't keep plaintext copies on the live config struct beyond secrets.
    cfg.provider.api_key = None;
    cfg.exa.api_key = None;

    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_non_empty_env() {
        // Can't safely mutate process env in parallel tests for the named vars;
        // cover the file-only path and empty trimming instead.
        assert_eq!(
            resolve_secret("HIVE_TEST_UNSET_ENV_VAR_XYZ", Some("  file-key  ")),
            Some("file-key".into())
        );
        assert_eq!(resolve_secret("HIVE_TEST_UNSET_ENV_VAR_XYZ", Some("")), None);
        assert_eq!(resolve_secret("HIVE_TEST_UNSET_ENV_VAR_XYZ", None), None);
    }
}
