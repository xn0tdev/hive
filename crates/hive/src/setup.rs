//! First-run gate: intro wizard → write config → load.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use hive_core::config::AppConfig;
use hive_tui::{run_intro, IntroOpts, IntroPrefill, IntroResult, SetupDraft};

use crate::config::{self, has_provider_key, write_setup, SetupChoices, SetupModel};

/// Run setup when needed (or when `force` / `--intro`).
pub async fn ensure_ready(cfg: Arc<AppConfig>, force: bool) -> Result<Arc<AppConfig>> {
    if !force && has_provider_key(&cfg) && cfg.ui.setup_complete {
        return Ok(cfg);
    }

    let version = env!("CARGO_PKG_VERSION").to_string();
    let existing = has_provider_key(&cfg);

    // Prefill when a key already exists so step-level “use existing” can keep values
    // without treating stock defaults as "configured" on a fresh install.
    let prefill = if existing {
        Some(prefill_from_config(&cfg))
    } else {
        None
    };

    let result = run_intro(IntroOpts { version, prefill })
        .await
        .context("intro")?;
    apply_intro(result).await
}

fn prefill_from_config(cfg: &AppConfig) -> IntroPrefill {
    let search_api_key = match cfg.search.backend {
        hive_core::config::SearchBackend::Exa => cfg.secrets.exa_api_key.clone(),
        hive_core::config::SearchBackend::Perplexity => cfg.secrets.perplexity_api_key.clone(),
        hive_core::config::SearchBackend::None => None,
    };
    IntroPrefill {
        provider_base_url: cfg.provider.base_url.clone(),
        provider_api_key_env: cfg.provider.api_key_env.clone(),
        provider_api_key: cfg.secrets.provider_api_key.clone(),
        default_id: cfg.models.default.id().to_string(),
        default_name: cfg.models.default.display_name().to_string(),
        search_backend: cfg.search.backend,
        search_api_key,
    }
}

async fn apply_intro(result: IntroResult) -> Result<Arc<AppConfig>> {
    match result {
        IntroResult::Completed(draft) => {
            write_setup(&choices_from_draft(*draft)).context("writing setup")?;
            config::load().context("loading config after setup")
        }
        IntroResult::Abort => {
            bail!("setup cancelled");
        }
    }
}

fn choices_from_draft(d: SetupDraft) -> SetupChoices {
    let provider_api_key = key_for_file(&d.provider_api_key_env, Some(d.provider_api_key));
    let search_api_key = match d.search_backend {
        hive_core::config::SearchBackend::Exa => key_for_file("EXA_API_KEY", d.search_api_key),
        hive_core::config::SearchBackend::Perplexity => {
            key_for_file("PERPLEXITY_API_KEY", d.search_api_key)
        }
        hive_core::config::SearchBackend::None => None,
    };
    SetupChoices {
        provider_base_url: d.provider_base_url,
        provider_api_key_env: d.provider_api_key_env,
        provider_api_key,
        default: SetupModel {
            id: d.default_id,
            name: d.default_name,
        },
        search_backend: d.search_backend,
        search_api_key,
    }
}

fn key_for_file(env_name: &str, candidate: Option<String>) -> Option<String> {
    let env_value = std::env::var(env_name).ok().filter(|key| !key.is_empty());
    key_for_file_value(env_value.as_deref(), candidate)
}

fn key_for_file_value(env_value: Option<&str>, candidate: Option<String>) -> Option<String> {
    let candidate = candidate.filter(|key| !key.trim().is_empty())?;
    (env_value != Some(candidate.as_str())).then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_env_key_is_not_persisted() {
        assert_eq!(
            key_for_file_value(Some("from-env"), Some("from-env".into())),
            None
        );
        assert_eq!(
            key_for_file_value(Some("from-env"), Some("typed".into())),
            Some("typed".into())
        );
    }
}
