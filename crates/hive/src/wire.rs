//! Composition root: builds concrete implementations, injects them through
//! core traits, and starts the driver + TUI.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use hive_core::config::AppConfig;
use hive_core::provider::LlmProvider;
use hive_core::skill::SkillSource;
use hive_core::{all_tools, new_spawner, Agent, AgentBuilder, DiskSkills, FollowUpSlot};
use hive_llm::FireworksProvider;
use hive_tui::{ModelChoice, SkillChoice, TuiInit};

use crate::config;
use crate::driver;

pub fn build_agent(
    cfg: &Arc<AppConfig>,
    event_tx: hive_core::EventSender,
) -> Agent {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let global_skills = config::config_dir().join("skills");
    seed_sample_skill(&global_skills);
    let skill_dirs = vec![global_skills, cwd.join(".hive").join("skills")];

    let provider: Arc<dyn LlmProvider> = Arc::new(FireworksProvider::new(
        cfg.provider.base_url.clone(),
        cfg.secrets.provider_api_key.clone(),
    ));
    let skills: Arc<dyn SkillSource> = Arc::new(DiskSkills::load(skill_dirs));
    let tools = all_tools();

    let builder = AgentBuilder {
        provider,
        tools,
        skills,
        config: cfg.clone(),
    };

    let spawner = new_spawner(
        builder.clone(),
        event_tx.clone(),
        cfg.swarm.max_concurrent,
        cfg.swarm.max_depth,
    );

    let default_model = cfg.models.default.id().to_string();
    builder.build(event_tx, default_model, 0, spawner)
}

pub fn rebuild_agent_in(agent: &Agent, cwd: &std::path::Path) -> Option<Agent> {
    let provider = agent.provider_clone();
    let tools = agent.tools_clone();
    let skills = agent.skills_clone();
    let config = agent.config_clone();
    let spawner = agent.spawner_clone();
    let events = agent.events_clone();
    let model = agent.model().to_string();

    let builder = AgentBuilder {
        provider,
        tools,
        skills,
        config,
    };

    Some(builder.build_in(events, model, 0, spawner, cwd.to_path_buf()))
}

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

    let global_skills = config::config_dir().join("skills");
    seed_sample_skill(&global_skills);
    let skill_dirs = vec![global_skills, cwd.join(".hive").join("skills")];

    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
    let terminal = hive_core::TerminalManager::new(event_tx.clone());
    let interrupt = Arc::new(AtomicBool::new(false));
    let follow_up: FollowUpSlot = Arc::new(Mutex::new(None));

    let provider: Arc<dyn LlmProvider> = Arc::new(FireworksProvider::new(
        cfg.provider.base_url.clone(),
        cfg.secrets.provider_api_key.clone(),
    ));
    let skills: Arc<dyn SkillSource> = Arc::new(DiskSkills::load(skill_dirs));
    let skill_choices: Vec<SkillChoice> = skills
        .list()
        .into_iter()
        .filter_map(|m| {
            let content = skills.read(&m.name)?;
            Some(SkillChoice {
                name: m.name,
                description: m.description,
                content,
            })
        })
        .collect();
    let tools = all_tools();

    let builder = AgentBuilder {
        provider: provider.clone(),
        tools,
        skills: skills.clone(),
        config: cfg.clone(),
    };

    let spawner = new_spawner(
        builder.clone(),
        event_tx.clone(),
        cfg.swarm.max_concurrent,
        cfg.swarm.max_depth,
    );

    let default_model = cfg.models.default.id().to_string();
    let default_display = cfg.models.default.display_name().to_string();
    let agent = builder.build_with_terminal(
        event_tx.clone(),
        default_model.clone(),
        0,
        spawner,
        terminal.clone(),
    );

    let model_choices = vec![ModelChoice {
        key: default_model.clone(),
        display: default_display.clone(),
        detail: String::new(),
        group: "Configured".into(),
        connection_id: cfg.connections.active.clone(),
        vision: false,
        cost_input: 0.0,
        cost_output: 0.0,
    }];

    let tui_init = TuiInit {
        model: default_model,
        model_display: default_display,
        model_choices,
        skills: skill_choices,
        connections: config::connection_infos(&cfg),
        active_connection: cfg.connections.active.clone(),
        cwd: cwd.display().to_string(),
        theme: cfg.ui.theme.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ui: cfg.ui.clone(),
        context_window: cfg.agent.context_window,
        cost_input: 0.0,
        cost_output: 0.0,
    };

    install_panic_hook();

    let driver_interrupt = interrupt.clone();
    let driver_follow_up = follow_up.clone();
    let driver_cfg = cfg.clone();
    let driver_events = event_tx.clone();
    tokio::spawn(async move {
        driver::run(
            agent,
            input_rx,
            driver_events,
            driver_interrupt,
            driver_follow_up,
            terminal,
            driver_cfg,
        )
        .await;
    });

    tokio::task::spawn_blocking(move || {
        hive_tui::run(tui_init, event_rx, input_tx, interrupt, follow_up)
    })
    .await??;

    Ok(())
}

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

"#;
