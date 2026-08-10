//! Composition root: builds concrete implementations, injects them through
//! core traits, and starts the driver + TUI.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use hive_core::config::AppConfig;
use hive_core::provider::LlmProvider;
use hive_core::skill::SkillSource;
use hive_core::{
    all_tools, new_spawner_in, Agent, AgentBuilder, CompositeSkills, DiskSkills, FollowUpSlot,
    InsideSkills,
};
use hive_llm::FireworksProvider;
use hive_tui::{ModelChoice, SkillChoice, TuiInit};

use crate::config;
use crate::driver;

pub fn build_agent(cfg: &Arc<AppConfig>, event_tx: hive_core::EventSender) -> Agent {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let global_skills = config::config_dir().join("skills");
    seed_sample_skill(&global_skills);
    let skill_dirs = vec![global_skills, cwd.join(".hive").join("skills")];

    let provider: Arc<dyn LlmProvider> = Arc::new(FireworksProvider::new(
        cfg.provider.base_url.clone(),
        cfg.secrets.provider_api_key.clone(),
    ));
    let skills: Arc<dyn SkillSource> = Arc::new(CompositeSkills::new(vec![
        Box::new(InsideSkills),
        Box::new(DiskSkills::load(skill_dirs)),
    ]));
    let tools = all_tools();

    let builder = AgentBuilder {
        provider,
        tools,
        skills,
        config: cfg.clone(),
    };

    let spawner = new_spawner_in(
        builder.clone(),
        event_tx.clone(),
        cfg.swarm.max_concurrent,
        cfg.swarm.max_depth,
        cwd.clone(),
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

/// Which saved session to reopen at startup.
#[derive(Debug, Clone)]
pub enum Resume {
    /// The most recently updated session.
    Last,
    Id(String),
}

impl Resume {
    /// Resolve to a concrete session id, or `None` when there's nothing to open.
    fn resolve(self) -> Option<String> {
        match self {
            Resume::Id(id) => Some(id),
            Resume::Last => hive_core::agent::session_store::list()
                .first()
                .map(|m| m.id.clone()),
        }
    }
}

pub async fn run(cfg: Arc<AppConfig>, resume: Option<Resume>) -> Result<()> {
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
    let skills: Arc<dyn SkillSource> = Arc::new(CompositeSkills::new(vec![
        Box::new(InsideSkills),
        Box::new(DiskSkills::load(skill_dirs)),
    ]));
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

    let spawner = new_spawner_in(
        builder.clone(),
        event_tx.clone(),
        cfg.swarm.max_concurrent,
        cfg.swarm.max_depth,
        cwd.clone(),
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

    // Show the last model used with every saved provider immediately; the
    // background catalog refresh expands each group without a blank/loading
    // detour on startup.
    let mut model_choices: Vec<ModelChoice> = cfg
        .connections
        .profiles
        .iter()
        .filter(|(_, profile)| !profile.model.id().trim().is_empty())
        .map(|(connection_id, profile)| ModelChoice {
            key: profile.model.id().to_string(),
            display: profile.model.display_name().to_string(),
            detail: String::new(),
            group: if profile.label.trim().is_empty() {
                hive_llm::catalog::provider_label_for_base(&profile.base_url).to_string()
            } else {
                profile.label.clone()
            },
            connection_id: connection_id.clone(),
            vision: false,
            context: 0,
            cost_input: 0.0,
            cost_output: 0.0,
        })
        .collect();
    if model_choices.is_empty() {
        model_choices.push(ModelChoice {
            key: default_model.clone(),
            display: default_display.clone(),
            detail: String::new(),
            group: hive_llm::catalog::provider_label_for_base(&cfg.provider.base_url).to_string(),
            connection_id: cfg.connections.active.clone(),
            vision: false,
            context: 0,
            cost_input: 0.0,
            cost_output: 0.0,
        });
    }
    model_choices.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then_with(|| a.display.cmp(&b.display))
    });

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

    // Queued before the driver starts, so the session is already loading while
    // the TUI paints its first frame.
    if let Some(target) = resume {
        match target.resolve() {
            Some(id) => {
                let _ = input_tx.send(hive_tui::InputCommand::LoadSession { id });
            }
            None => {
                let _ = event_tx.send(hive_core::event::AgentEvent::Notice(
                    "no saved sessions yet — starting a new one".into(),
                ));
            }
        }
    }

    let session_id: driver::SessionSlot = Arc::new(Mutex::new(None));
    let shared = driver::DriverShared {
        interrupt: interrupt.clone(),
        follow_up: follow_up.clone(),
        session_id: session_id.clone(),
    };
    let driver_cfg = cfg.clone();
    let driver_events = event_tx.clone();
    tokio::spawn(async move {
        driver::run(agent, input_rx, driver_events, shared, terminal, driver_cfg).await;
    });

    tokio::task::spawn_blocking(move || hive_tui::run(tui_init, event_rx, input_tx, interrupt))
        .await??;

    // The TUI has restored the terminal — leave the wordmark and a way back in.
    let last_session = session_id.lock().ok().and_then(|id| id.clone());
    hive_tui::print_farewell(last_session.as_deref());

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
