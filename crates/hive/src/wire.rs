//! The composition root. This is the ONLY place that knows every concrete
//! implementation: it builds them, injects them through core traits, and starts
//! the driver + TUI. Concrete pieces come from just a few crates now —
//! `hive-llm` (provider) and `hive-agent` (loop + tools + swarm + skills +
//! vision) — so wiring a new capability is usually a one-line change here.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;

use hive_agent::{AgentBuilder, DescribeVision, DiskSkills};
use hive_core::config::AppConfig;
use hive_core::provider::LlmProvider;
use hive_core::skill::SkillSource;
use hive_core::vision::VisionDescriber;
use hive_llm::FireworksProvider;
use hive_tui::TuiInit;

use crate::config;
use crate::driver;

/// Set up file logging (never touches the TUI). Returns a guard that must live
/// for the program's lifetime.
pub fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = directories::BaseDirs::new()
        .map(|b| b.cache_dir().join("hive"))
        .unwrap_or_else(|| std::path::PathBuf::from(".hive-cache"));
    std::fs::create_dir_all(&dir).ok()?;
    let file = tracing_appender::rolling::never(&dir, "hive.log");
    let (nb, guard) = tracing_appender::non_blocking(file);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_writer(nb)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();
    Some(guard)
}

pub async fn run(cfg: Arc<AppConfig>) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    // Seed a sample skill on first run so the feature is discoverable.
    let global_skills = config::config_dir().join("skills");
    seed_sample_skill(&global_skills);
    let skill_dirs = vec![global_skills, cwd.join(".hive").join("skills")];

    // Channels between agent driver and TUI.
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
    let interrupt = Arc::new(AtomicBool::new(false));

    // --- concrete implementations, injected via core traits ---
    let provider: Arc<dyn LlmProvider> = Arc::new(FireworksProvider::new(
        cfg.provider.base_url.clone(),
        cfg.secrets.provider_api_key.clone(),
    ));
    let skills: Arc<dyn SkillSource> = Arc::new(DiskSkills::load(skill_dirs));
    let vision: Arc<dyn VisionDescriber> =
        Arc::new(DescribeVision::new(provider.clone(), cfg.models.vision.clone()));
    let tools = hive_agent::all_tools();

    let builder = AgentBuilder {
        provider: provider.clone(),
        tools,
        skills: skills.clone(),
        vision: vision.clone(),
        config: cfg.clone(),
    };

    // The swarm is both built from the builder and injected back as the main
    // agent's spawner.
    let spawner = hive_agent::new_spawner(
        builder.clone(),
        event_tx.clone(),
        cfg.swarm.max_concurrent,
        cfg.swarm.max_depth,
    );

    let default_model = cfg.models.default.clone();
    let agent = builder.build(event_tx.clone(), default_model.clone(), 0, spawner);

    let tui_init = TuiInit {
        model: default_model,
        cwd: cwd.display().to_string(),
        theme: cfg.ui.theme.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    install_panic_hook();

    // Run the agent driver on the async runtime; run the TUI on a blocking task.
    let driver_interrupt = interrupt.clone();
    let driver_cfg = cfg.clone();
    let driver_events = event_tx.clone();
    tokio::spawn(async move {
        driver::run(agent, input_rx, driver_events, driver_interrupt, driver_cfg).await;
    });

    tokio::task::spawn_blocking(move || hive_tui::run(tui_init, event_rx, input_tx, interrupt))
        .await??;

    Ok(())
}

/// Make sure the terminal is restored if a panic escapes the TUI. `comb` saves
/// the original termios when it enters raw mode, so `restore` can undo raw mode,
/// leave the alternate screen, and stop mouse reporting from here.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        comb::restore();
        default_hook(info);
    }));
}

fn seed_sample_skill(skills_dir: &Path) {
    if skills_dir.exists() {
        return;
    }
    let dir = skills_dir.join("git-commit");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join("SKILL.md"), SAMPLE_SKILL);
}

const SAMPLE_SKILL: &str = r#"---
name: git-commit
description: Create clean, conventional git commits from the current changes.
---

# git-commit

When asked to commit changes:

1. Run `git status` and `git diff --staged` (and `git diff`) to understand what changed.
2. Stage the relevant files with `git add`.
3. Write a concise Conventional Commits message: `type(scope): summary`.
   - types: feat, fix, docs, refactor, test, chore, perf.
   - Keep the summary under ~72 chars; add a body only if it adds value.
4. Commit with `git commit -m "..."`.
5. Show the resulting `git log -1 --stat`.

Prefer the `fast` model role for this kind of work.
"#;
