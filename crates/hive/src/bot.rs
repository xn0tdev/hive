//! `hive bot` — the persona hub entry point. Collects persona scopes and the
//! provider, then hands over to the bot TUI in `hive-tui`.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use hive_core::config::AppConfig;
use hive_core::provider::LlmProvider;
use hive_llm::FireworksProvider;

pub async fn run(cfg: Arc<AppConfig>) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let project_agents = cwd.join(".hive").join("agents");
    let global_agents = directories::BaseDirs::new()
        .map(|b| b.home_dir().join(".hive").join("agents"))
        .unwrap_or_else(|| PathBuf::from(".hive/agents"));

    // Project scope wins when the project actually uses hive (.hive present);
    // otherwise personas are created globally so they follow you everywhere.
    let (save_root, scope) = if cwd.join(".hive").exists() {
        (project_agents.clone(), "project")
    } else {
        (global_agents.clone(), "global")
    };

    let provider: Arc<dyn LlmProvider> = Arc::new(FireworksProvider::new(
        cfg.provider.base_url.clone(),
        cfg.secrets.provider_api_key.clone(),
    ));

    let init = hive_tui::bot::BotInit {
        model: cfg.models.default.id().to_string(),
        provider,
        roots: vec![project_agents, global_agents],
        save_root,
        scope_label: scope.to_string(),
    };

    tokio::task::spawn_blocking(move || hive_tui::bot::run(init)).await??;
    Ok(())
}
