//! Agent driver: consumes `InputCommand`s from the TUI and runs turns.

use std::collections::{hash_map::DefaultHasher, VecDeque};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedReceiver;

use hive_core::agent::session_store;
use hive_core::config::{AppConfig, ModelRef, ModelRole};
use hive_core::event::{AgentEvent, CatalogModel, EventSender};
use hive_core::message::Message;
use hive_core::provider::{ChatRequest, Delta, LlmProvider};
use hive_core::{Agent, FollowUpSlot, TerminalHandle, UserInput};
use hive_llm::catalog::{
    enrich_models, fetch_models_dev, list_provider_models, merge_catalog_models,
    models_dev_hint_for_base, provider_label_for_base, ModelCard,
};
use hive_llm::FireworksProvider;
use hive_tui::InputCommand;

/// Id of the session the driver is writing to. The shell reads it after the
/// TUI exits to print "resume with this id"; `None` until the first save.
pub type SessionSlot = Arc<Mutex<Option<String>>>;

/// Handles the shell, the TUI, and the driver all hold.
pub struct DriverShared {
    pub interrupt: Arc<AtomicBool>,
    pub follow_up: FollowUpSlot,
    pub session_id: SessionSlot,
}

pub async fn run(
    mut agent: Agent,
    mut input_rx: UnboundedReceiver<InputCommand>,
    events: EventSender,
    shared: DriverShared,
    terminal: TerminalHandle,
    mut cfg: Arc<AppConfig>,
) {
    // Populate every configured provider before the user opens /models. The
    // refresh is detached so startup and the command loop stay responsive.
    spawn_model_refresh(cfg.clone(), events.clone());
    let DriverShared {
        interrupt,
        follow_up,
        session_id: session_slot,
    } = shared;
    let mut pending = VecDeque::new();
    let mut session: Option<ActiveSession> = None;
    // Autosave failures repeat every turn; say it once instead of nagging.
    let mut autosave_warned = false;
    'commands: loop {
        let cmd = match pending.pop_front() {
            Some(command) => command,
            None => match input_rx.recv().await {
                Some(command) => command,
                None => break,
            },
        };
        let cmd = match handle_terminal_command(cmd, &terminal, &events).await {
            Ok(()) => continue,
            Err(command) => command,
        };
        match cmd {
            InputCommand::User { text, images, mode } => {
                interrupt.store(false, Ordering::Relaxed);
                // Drop any stale mid-turn inject from a previous turn.
                if let Ok(mut g) = follow_up.lock() {
                    *g = None;
                }
                let turn = agent.run_turn(
                    UserInput { text, images, mode },
                    interrupt.clone(),
                    follow_up.clone(),
                );
                if drive_turn(
                    turn,
                    &mut input_rx,
                    &terminal,
                    &events,
                    &mut pending,
                    interrupt.as_ref(),
                )
                .await
                .is_none()
                {
                    break 'commands;
                }
                // Auto-save after each turn so a crash never loses a transcript.
                autosave(
                    &agent,
                    &mut session,
                    &session_slot,
                    &events,
                    &mut autosave_warned,
                )
                .await;
            }
            InputCommand::SetModel {
                id,
                display,
                connection_id,
                vision,
                context,
                cost_input,
                cost_output,
            } => {
                if let Some(cid) = connection_id.as_deref().filter(|c| !c.is_empty()) {
                    if cid != cfg.connections.active {
                        if let Err(e) = activate_provider(&mut agent, &mut cfg, &events, cid) {
                            let _ = events.send(AgentEvent::Notice(format!("connect failed: {e}")));
                            continue;
                        }
                    }
                }
                let (id, display) = resolve_model(&cfg, &id, &display);
                agent.set_model(id.clone());
                agent.set_vision_capable(vision);
                if context > 0 {
                    let cfg_mut = Arc::make_mut(&mut cfg);
                    cfg_mut.agent.context_window = context;
                    agent.set_context_window(context);
                    if let Err(e) = crate::config::patch_context_window(context) {
                        let _ = events.send(AgentEvent::Notice(format!(
                            "could not save context window: {e}"
                        )));
                    }
                }
                if let Err(e) = crate::config::patch_model(&id, &display) {
                    let _ = events.send(AgentEvent::Notice(format!("could not save model: {e}")));
                }
                let model_ref = ModelRef::Named {
                    id: id.clone(),
                    name: Some(display.clone()),
                };
                let cfg_mut = Arc::make_mut(&mut cfg);
                cfg_mut.models.default = model_ref.clone();
                let active_connection = cfg_mut.connections.active.clone();
                if let Some(profile) = cfg_mut.connections.profiles.get_mut(&active_connection) {
                    profile.model = model_ref;
                }
                let _ = events.send(AgentEvent::ModelChanged {
                    id: id.clone(),
                    display: display.clone(),
                    context,
                    cost_input,
                    cost_output,
                });
                let _ = events.send(AgentEvent::Notice(format!("Model set to {display}")));
            }
            InputCommand::UpdateContextWindow { context } => {
                if context > 0 {
                    let cfg_mut = Arc::make_mut(&mut cfg);
                    cfg_mut.agent.context_window = context;
                    agent.set_context_window(context);
                    if let Err(e) = crate::config::patch_context_window(context) {
                        let _ = events.send(AgentEvent::Notice(format!(
                            "could not save context window: {e}"
                        )));
                    }
                }
            }
            InputCommand::SetGoal {
                objective,
                duration,
            } => {
                let deadline = duration.map(|d| std::time::Instant::now() + d);
                agent.set_goal(objective.clone(), deadline);
                let _ = events.send(AgentEvent::GoalSet {
                    objective: objective.clone(),
                    deadline,
                });
                // Start the first turn with the objective as the user prompt.
                interrupt.store(false, Ordering::Relaxed);
                if let Ok(mut g) = follow_up.lock() {
                    *g = None;
                }
                let turn = agent.run_turn(
                    UserInput::from(objective),
                    interrupt.clone(),
                    follow_up.clone(),
                );
                if drive_turn(
                    turn,
                    &mut input_rx,
                    &terminal,
                    &events,
                    &mut pending,
                    interrupt.as_ref(),
                )
                .await
                .is_none()
                {
                    break 'commands;
                }
                // Goal continuation loop.
                loop {
                    drain_goal_commands(&agent, &events, &mut pending);
                    if !agent.goal_active() {
                        break;
                    }
                    let snap = match agent.goal_snapshot() {
                        Some(g) => g,
                        None => break,
                    };
                    let prompt = build_goal_continuation(&snap.objective, snap.remaining_secs());
                    interrupt.store(false, Ordering::Relaxed);
                    if let Ok(mut g) = follow_up.lock() {
                        *g = None;
                    }
                    let turn = agent.run_turn(
                        UserInput::from(prompt),
                        interrupt.clone(),
                        follow_up.clone(),
                    );
                    if drive_turn(
                        turn,
                        &mut input_rx,
                        &terminal,
                        &events,
                        &mut pending,
                        interrupt.as_ref(),
                    )
                    .await
                    .is_none()
                    {
                        break 'commands;
                    }
                }
            }
            InputCommand::StopGoal => {
                agent.stop_goal();
                let _ = events.send(AgentEvent::GoalStopped);
            }
            InputCommand::PauseGoal => {
                agent.pause_goal();
                let _ = events.send(AgentEvent::GoalPaused);
            }
            InputCommand::ResumeGoal => {
                agent.resume_goal();
                let _ = events.send(AgentEvent::GoalResumed);
                if !agent.goal_active() {
                    continue;
                }
                let snap = match agent.goal_snapshot() {
                    Some(g) => g,
                    None => continue,
                };
                let prompt = build_goal_continuation(&snap.objective, snap.remaining_secs());
                interrupt.store(false, Ordering::Relaxed);
                if let Ok(mut g) = follow_up.lock() {
                    *g = None;
                }
                let turn = agent.run_turn(
                    UserInput::from(prompt),
                    interrupt.clone(),
                    follow_up.clone(),
                );
                if drive_turn(
                    turn,
                    &mut input_rx,
                    &terminal,
                    &events,
                    &mut pending,
                    interrupt.as_ref(),
                )
                .await
                .is_none()
                {
                    break 'commands;
                }
                // Goal continuation loop.
                loop {
                    drain_goal_commands(&agent, &events, &mut pending);
                    if !agent.goal_active() {
                        break;
                    }
                    let snap = match agent.goal_snapshot() {
                        Some(g) => g,
                        None => break,
                    };
                    let prompt = build_goal_continuation(&snap.objective, snap.remaining_secs());
                    interrupt.store(false, Ordering::Relaxed);
                    if let Ok(mut g) = follow_up.lock() {
                        *g = None;
                    }
                    let turn = agent.run_turn(
                        UserInput::from(prompt),
                        interrupt.clone(),
                        follow_up.clone(),
                    );
                    if drive_turn(
                        turn,
                        &mut input_rx,
                        &terminal,
                        &events,
                        &mut pending,
                        interrupt.as_ref(),
                    )
                    .await
                    .is_none()
                    {
                        break 'commands;
                    }
                }
            }
            InputCommand::FetchModels => {
                spawn_model_refresh(cfg.clone(), events.clone());
            }
            InputCommand::UpsertConnection {
                id,
                label,
                base_url,
                api_key_env,
                api_key,
            } => {
                let (model_id, model_name) = if cfg.connections.profiles.is_empty() {
                    (
                        cfg.models.default.id().to_string(),
                        cfg.models.default.display_name().to_string(),
                    )
                } else {
                    (String::new(), String::new())
                };
                if let Err(e) = crate::config::upsert_connection(
                    &id,
                    &label,
                    &base_url,
                    &api_key_env,
                    Some(&api_key),
                    &model_id,
                    &model_name,
                ) {
                    let _ =
                        events.send(AgentEvent::Notice(format!("could not save provider: {e}")));
                    continue;
                }
                match crate::config::load() {
                    Ok(new_cfg) => {
                        cfg = new_cfg;
                        emit_connections(&cfg, &events);
                        spawn_model_refresh(cfg.clone(), events.clone());
                        let _ = events.send(AgentEvent::Notice(format!("Provider added: {label}")));
                    }
                    Err(e) => {
                        let _ = events.send(AgentEvent::Notice(format!("reload failed: {e}")));
                    }
                }
            }
            InputCommand::UpdateConnectionKey { id, api_key } => {
                if let Err(e) = crate::config::update_connection_key(&id, &api_key) {
                    let _ = events.send(AgentEvent::Notice(format!("could not update key: {e}")));
                    continue;
                }
                match crate::config::load() {
                    Ok(new_cfg) => {
                        cfg = new_cfg;
                        if cfg.connections.active == id {
                            let provider: Arc<dyn LlmProvider> = Arc::new(FireworksProvider::new(
                                cfg.provider.base_url.clone(),
                                cfg.secrets.provider_api_key.clone(),
                            ));
                            agent.set_provider(provider);
                        }
                        emit_connections(&cfg, &events);
                        spawn_model_refresh(cfg.clone(), events.clone());
                        let label = cfg
                            .connections
                            .profiles
                            .get(&id)
                            .map(|p| p.label.as_str())
                            .filter(|label| !label.is_empty())
                            .unwrap_or(&id);
                        let _ =
                            events.send(AgentEvent::Notice(format!("API key updated: {label}")));
                    }
                    Err(e) => {
                        let _ = events.send(AgentEvent::Notice(format!("reload failed: {e}")));
                    }
                }
            }
            InputCommand::RemoveConnection { id } => {
                let removed_was_active = cfg.connections.active == id;
                if let Err(e) = crate::config::remove_connection(&id) {
                    let _ = events.send(AgentEvent::Notice(format!("could not remove: {e}")));
                    continue;
                }
                match crate::config::load() {
                    Ok(new_cfg) => {
                        cfg = new_cfg;
                        emit_connections(&cfg, &events);
                        if removed_was_active {
                            sync_agent_to_active_config(&mut agent, &cfg, &events);
                        }
                        spawn_model_refresh(cfg.clone(), events.clone());
                    }
                    Err(e) => {
                        let _ = events.send(AgentEvent::Notice(format!("reload failed: {e}")));
                    }
                }
            }
            InputCommand::Clear => {
                terminal.shutdown();
                agent.reset();
                // A cleared transcript starts a new session file, not an
                // overwrite of the one we were just appending to.
                session = None;
                publish_session(&session_slot, None);
            }
            InputCommand::Compact => {
                if let Err(e) = agent.compact().await {
                    let _ = events.send(AgentEvent::Notice(e));
                }
            }
            InputCommand::LoadSession { id } => {
                let snap = match blocking_io(move || session_store::load(&id)).await {
                    Ok(snap) => snap,
                    Err(e) => {
                        let _ = events.send(AgentEvent::Notice(format!("load failed: {e}")));
                        continue;
                    }
                };

                // The model id in a snapshot is meaningless on another
                // provider, so the connection has to come back first.
                let mut model = snap.model.clone();
                let mut context = snap.context_window;
                let mut vision = snap.vision;
                if !snap.connection_id.is_empty() && snap.connection_id != cfg.connections.active {
                    if let Err(e) =
                        activate_provider(&mut agent, &mut cfg, &events, &snap.connection_id)
                    {
                        // Restoring the transcript is still useful; keep the
                        // live model rather than firing a foreign id at it.
                        let _ = events.send(AgentEvent::Notice(format!(
                            "resume: staying on the current model — could not switch to '{}': {e}",
                            snap.connection_id
                        )));
                        model = agent.model().to_string();
                        context = 0;
                        vision = agent.vision_capable();
                    }
                }

                agent.set_model(model.clone());
                agent.set_vision_capable(vision);
                if context > 0 {
                    let cfg_mut = Arc::make_mut(&mut cfg);
                    cfg_mut.agent.context_window = context;
                    agent.set_context_window(context);
                }

                terminal.shutdown();
                let title = snap.title.clone();
                let messages = snap.messages.clone();
                let usage = snap.usage;
                agent.restore_session(snap.messages, usage);
                publish_session(&session_slot, Some(&snap.id));
                session = Some(ActiveSession {
                    id: snap.id,
                    created_at: snap.created_at,
                });
                let _ = events.send(AgentEvent::SessionLoaded {
                    title,
                    model,
                    context_window: context,
                    messages,
                    usage,
                });
                // Real prompt tokens land after the next turn; until then the
                // gauge shows the agent's estimate of the restored context.
                let _ = events.send(AgentEvent::ContextTokens(agent.context_tokens()));
            }
            InputCommand::ListSessions => {
                match tokio::task::spawn_blocking(session_store::list).await {
                    Ok(metas) => {
                        let _ = events.send(AgentEvent::SessionsListed(metas));
                    }
                    Err(e) => {
                        let _ = events.send(AgentEvent::Notice(format!("sessions failed: {e}")));
                    }
                }
            }
            InputCommand::SaveUi(ui) => {
                if let Err(e) = crate::config::patch_ui(&ui) {
                    let _ =
                        events.send(AgentEvent::Notice(format!("could not save settings: {e}")));
                }
            }
            InputCommand::Recap { id, context } => {
                spawn_recap(
                    agent.provider_clone(),
                    agent.model().to_string(),
                    id,
                    context,
                    events.clone(),
                );
            }
            InputCommand::TerminalAttach { .. }
            | InputCommand::TerminalDetach { .. }
            | InputCommand::TerminalInput { .. }
            | InputCommand::TerminalResize { .. }
            | InputCommand::TerminalStop { .. } => {
                unreachable!("terminal command was not consumed by the router")
            }
        }
    }
    terminal.shutdown();
}

async fn handle_terminal_command(
    command: InputCommand,
    terminal: &TerminalHandle,
    events: &EventSender,
) -> Result<(), InputCommand> {
    match command {
        InputCommand::TerminalAttach { id } => {
            report_terminal_result(&id, terminal.request_attach(&id), events);
            Ok(())
        }
        InputCommand::TerminalDetach { id } => {
            report_terminal_result(&id, terminal.request_detach(&id), events);
            // Hand geometry back to the agent at a stable default size.
            let _ = terminal
                .resize(&id, hive_core::DEFAULT_ROWS, hive_core::DEFAULT_COLS)
                .await;
            Ok(())
        }
        InputCommand::TerminalInput { id, input } => {
            let result = terminal.write_user(&id, input.into_bytes()).await;
            report_terminal_result(&id, result, events);
            Ok(())
        }
        InputCommand::TerminalResize { id, rows, cols } => {
            let result = terminal.resize(&id, rows, cols).await;
            report_terminal_result(&id, result, events);
            Ok(())
        }
        InputCommand::TerminalStop { id } => {
            let manager = terminal.clone();
            let stop_id = id.clone();
            let result = tokio::task::spawn_blocking(move || manager.stop(&stop_id))
                .await
                .map_err(|error| {
                    hive_core::TerminalError::Io(format!("terminal stop task failed: {error}"))
                })
                .and_then(|result| result);
            report_terminal_result(&id, result, events);
            Ok(())
        }
        other => Err(other),
    }
}

fn report_terminal_result<T>(
    id: &str,
    result: Result<T, hive_core::TerminalError>,
    events: &EventSender,
) {
    if let Err(error) = result {
        let _ = events.send(AgentEvent::TerminalError {
            id: id.to_string(),
            message: error.to_string(),
        });
    }
}

async fn drive_turn<F, T>(
    turn: F,
    input_rx: &mut UnboundedReceiver<InputCommand>,
    terminal: &TerminalHandle,
    events: &EventSender,
    pending: &mut VecDeque<InputCommand>,
    interrupt: &AtomicBool,
) -> Option<T>
where
    F: Future<Output = T>,
{
    tokio::pin!(turn);
    loop {
        tokio::select! {
            biased;
            result = &mut turn => return Some(result),
            command = input_rx.recv() => {
                let Some(command) = command else {
                    interrupt.store(true, Ordering::Relaxed);
                    terminal.shutdown();
                    let _ = turn.await;
                    return None;
                };
                if let Err(command) =
                    handle_terminal_command(command, terminal, events).await
                {
                    pending.push_back(command);
                }
            }
        }
    }
}

/// The session file the current conversation is being appended to.
struct ActiveSession {
    id: String,
    /// Kept across saves — `snapshot` would otherwise restamp it every turn.
    created_at: u64,
}

/// Persist the transcript after a turn. A snapshot is the whole history, so
/// serializing and writing it runs on a blocking thread, not a runtime worker.
async fn autosave(
    agent: &Agent,
    session: &mut Option<ActiveSession>,
    slot: &SessionSlot,
    events: &EventSender,
    warned: &mut bool,
) {
    let (id, created_at) = match session.as_ref() {
        Some(s) => (s.id.clone(), Some(s.created_at)),
        None => (session_store::new_id(), None),
    };
    let snap = agent.session_snapshot(&id, created_at);
    publish_session(slot, Some(&snap.id));
    *session = Some(ActiveSession {
        id: snap.id.clone(),
        created_at: snap.created_at,
    });
    if let Err(e) = blocking_io(move || session_store::save(&snap)).await {
        if !*warned {
            *warned = true;
            let _ = events.send(AgentEvent::Notice(format!("session autosave failed: {e}")));
        }
    }
}

/// Hand the current session id to the shell. A poisoned lock just means no
/// resume hint at exit — never a reason to disturb the session.
fn publish_session(slot: &SessionSlot, id: Option<&str>) {
    if let Ok(mut current) = slot.lock() {
        *current = id.map(str::to_string);
    }
}

/// Run blocking file I/O off the runtime, folding a join failure into the
/// error so callers have one failure mode to handle.
async fn blocking_io<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Activate the provider behind a model choice without temporarily applying
/// that profile's previous model. The caller sets the final model immediately
/// afterwards, so emitting an intermediate model change only causes flicker.
fn activate_provider(
    agent: &mut Agent,
    cfg: &mut Arc<AppConfig>,
    events: &EventSender,
    id: &str,
) -> anyhow::Result<()> {
    crate::config::activate_connection(id)?;
    let new_cfg = crate::config::load()?;
    *cfg = new_cfg;

    install_provider(agent, cfg);
    emit_connections(cfg, events);
    Ok(())
}

fn install_provider(agent: &mut Agent, cfg: &AppConfig) {
    let provider: Arc<dyn LlmProvider> = Arc::new(FireworksProvider::new(
        cfg.provider.base_url.clone(),
        cfg.secrets.provider_api_key.clone(),
    ));
    agent.set_provider(provider);
}

/// Removing the provider used by the current model is the one management
/// action that needs a runtime fallback. Inactive removals leave the agent
/// untouched.
fn sync_agent_to_active_config(agent: &mut Agent, cfg: &AppConfig, events: &EventSender) {
    install_provider(agent, cfg);
    let model_id = cfg.models.default.id().to_string();
    let model_display = cfg.models.default.display_name().to_string();
    agent.set_model(model_id.clone());
    agent.set_vision_capable(false);
    let _ = events.send(AgentEvent::ModelChanged {
        id: model_id,
        display: model_display,
        context: 0,
        cost_input: 0.0,
        cost_output: 0.0,
    });
}

fn emit_connections(cfg: &AppConfig, events: &EventSender) {
    let _ = events.send(AgentEvent::ConnectionsUpdated {
        active: cfg.connections.active.clone(),
        profiles: crate::config::connection_infos(cfg),
    });
}

fn resolve_model(cfg: &AppConfig, id: &str, display: &str) -> (String, String) {
    if let Some(role) = ModelRole::parse(id) {
        return (
            cfg.model(role).to_string(),
            cfg.model_display(role).to_string(),
        );
    }
    let display = if display.trim().is_empty() {
        cfg.display_for_model_id(id)
    } else {
        display.to_string()
    };
    (id.to_string(), display)
}

const LISTING_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
struct ListingTarget {
    connection_id: String,
    label: String,
    base_url: String,
    api_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListingSource {
    connection_id: String,
    label: String,
    base_url: String,
    key_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct ListingCacheEntry {
    sources: Vec<ListingSource>,
    fetched_at: Instant,
    models: Vec<CatalogModel>,
}

static LISTING_CACHE: Mutex<Option<ListingCacheEntry>> = Mutex::new(None);
static LISTING_FETCH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn spawn_model_refresh(cfg: Arc<AppConfig>, events: EventSender) {
    tokio::spawn(async move {
        fetch_and_emit_models(&cfg, &events).await;
    });
}

fn listing_targets(cfg: &AppConfig) -> Result<Vec<ListingTarget>, String> {
    let mut targets: Vec<ListingTarget> = cfg
        .connections
        .profiles
        .iter()
        .filter_map(|(id, p)| {
            let key = cfg
                .secrets
                .connection_keys
                .get(id)
                .cloned()
                .filter(|k| !k.is_empty())?;
            let label = if p.label.trim().is_empty() {
                provider_label_for_base(&p.base_url).to_string()
            } else {
                p.label.clone()
            };
            Some(ListingTarget {
                connection_id: id.clone(),
                label,
                base_url: p.base_url.clone(),
                api_key: key,
            })
        })
        .collect();

    // No profiles / no keys on profiles — fall back to active [provider].
    if targets.is_empty() {
        let key = cfg.secrets.provider_api_key.clone();
        if key.is_empty() {
            return Err("no provider API key — add one with /connect".into());
        }
        let base = cfg.provider.base_url.clone();
        let label = active_provider_label(cfg, &base);
        let id = if cfg.connections.active.is_empty() {
            "default".into()
        } else {
            cfg.connections.active.clone()
        };
        targets.push(ListingTarget {
            connection_id: id,
            label,
            base_url: base,
            api_key: key,
        });
    }
    Ok(targets)
}

fn listing_sources(targets: &[ListingTarget]) -> Vec<ListingSource> {
    targets
        .iter()
        .map(|target| {
            let mut hasher = DefaultHasher::new();
            target.api_key.hash(&mut hasher);
            ListingSource {
                connection_id: target.connection_id.clone(),
                label: target.label.clone(),
                base_url: target.base_url.clone(),
                key_fingerprint: hasher.finish(),
            }
        })
        .collect()
}

fn cached_listing(sources: &[ListingSource]) -> Option<Vec<CatalogModel>> {
    let cache = LISTING_CACHE.lock().ok()?;
    let entry = cache.as_ref()?;
    (entry.sources == sources && entry.fetched_at.elapsed() < LISTING_CACHE_TTL)
        .then(|| entry.models.clone())
}

fn emit_models(events: &EventSender, models: Vec<CatalogModel>) {
    let _ = events.send(AgentEvent::ModelsListed { models });
}

async fn fetch_and_emit_models(cfg: &AppConfig, events: &EventSender) {
    let targets = match listing_targets(cfg) {
        Ok(targets) => targets,
        Err(message) => {
            let _ = events.send(AgentEvent::ModelsListFailed(message));
            return;
        }
    };
    let sources = listing_sources(&targets);
    if let Some(models) = cached_listing(&sources) {
        emit_models(events, models);
        return;
    }

    // Startup prefetch and an immediate /models open can race. One network
    // fan-out does the work; the follower reuses the populated cache.
    let _fetch_guard = LISTING_FETCH_LOCK.lock().await;
    if let Some(models) = cached_listing(&sources) {
        emit_models(events, models);
        return;
    }

    let mut models = Vec::new();
    let mut errors = Vec::new();

    // Every listing is independent of the others and of the models.dev catalog,
    // so the whole fan-out flies at once: one round trip instead of N + 1.
    let listings = futures_util::future::join_all(
        targets
            .iter()
            .map(|target| list_provider_models(&target.base_url, &target.api_key)),
    );
    let (listings, catalog) = tokio::join!(listings, fetch_models_dev());
    let catalog = catalog.ok();

    for (target, listed) in targets.iter().zip(listings) {
        match listed {
            Ok(remote) => {
                let hint = models_dev_hint_for_base(&target.base_url);
                let remote = merge_catalog_models(remote, catalog.as_ref(), hint);
                let cards = enrich_models(&remote, catalog.as_ref(), hint);
                models.extend(catalog_rows(&target.label, &target.connection_id, &cards));
            }
            Err(e) => errors.push(format!("{}: {e}", target.label)),
        }
    }

    if models.is_empty() {
        let msg = if errors.is_empty() {
            "no models returned".into()
        } else {
            errors.join("; ")
        };
        let _ = events.send(AgentEvent::ModelsListFailed(msg));
        return;
    }

    models.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    // A partial list is still useful now, but must not hide a temporarily
    // failing provider for the full cache TTL.
    if errors.is_empty() {
        if let Ok(mut g) = LISTING_CACHE.lock() {
            *g = Some(ListingCacheEntry {
                sources,
                fetched_at: Instant::now(),
                models: models.clone(),
            });
        }
    } else {
        let _ = events.send(AgentEvent::Notice(format!(
            "couldn't list {}",
            errors.join("; ")
        )));
    }
    emit_models(events, models);
}

/// One section header for `/model`: active connection label, else host → known name.
fn active_provider_label(cfg: &AppConfig, base_url: &str) -> String {
    cfg.connections
        .profiles
        .get(&cfg.connections.active)
        .map(|p| p.label.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| provider_label_for_base(base_url).to_string())
}

/// Apply any goal commands (pause/stop/resume) that queued up during a turn,
/// so the continuation loop sees the updated state before deciding to go again.
fn drain_goal_commands(agent: &Agent, events: &EventSender, pending: &mut VecDeque<InputCommand>) {
    let mut i = 0;
    while i < pending.len() {
        let is_goal_cmd = matches!(
            pending[i],
            InputCommand::StopGoal | InputCommand::PauseGoal | InputCommand::ResumeGoal
        );
        if !is_goal_cmd {
            i += 1;
            continue;
        }
        let cmd = pending.remove(i).unwrap();
        match cmd {
            InputCommand::StopGoal => {
                agent.stop_goal();
                let _ = events.send(AgentEvent::GoalStopped);
            }
            InputCommand::PauseGoal => {
                agent.pause_goal();
                let _ = events.send(AgentEvent::GoalPaused);
            }
            InputCommand::ResumeGoal => {
                agent.resume_goal();
                let _ = events.send(AgentEvent::GoalResumed);
            }
            _ => unreachable!(),
        }
    }
}

/// Build the continuation prompt for the goal loop.
fn build_goal_continuation(objective: &str, remaining_secs: u64) -> String {
    let time_line = if remaining_secs > 0 {
        if remaining_secs >= 3600 {
            format!(
                " ({}h {}m left)",
                remaining_secs / 3600,
                (remaining_secs % 3600) / 60
            )
        } else if remaining_secs >= 60 {
            format!(" ({}m left)", remaining_secs / 60)
        } else {
            format!(" ({}s left)", remaining_secs)
        }
    } else {
        " (no time limit — keep working until stopped)".to_string()
    };
    format!("Continue: {objective}{time_line}")
}

fn catalog_rows(group: &str, connection_id: &str, cards: &[ModelCard]) -> Vec<CatalogModel> {
    cards
        .iter()
        .map(|c| CatalogModel {
            id: c.id.clone(),
            name: c.name.clone(),
            detail: c.badges(),
            group: group.to_string(),
            connection_id: connection_id.to_string(),
            vision: c.vision,
            context: c.context,
            cost_input: c.cost_input,
            cost_output: c.cost_output,
        })
        .collect()
}

const RECAP_SYSTEM: &str = "\
Write a short recap of this one coding-agent turn for the person who asked. \
Cover what they wanted, what changed, and anything left unfinished. \
Use 3–8 short sentences or a tight bullet list. \
No title, no greeting, no 'here's a recap'. Keep it under 120 words.";

fn spawn_recap(
    provider: Arc<dyn LlmProvider>,
    model: String,
    id: u64,
    context: String,
    events: EventSender,
) {
    tokio::spawn(async move {
        let messages = [Message::system(RECAP_SYSTEM), Message::user(context)];
        let mut on_delta = |delta: Delta| {
            if let Delta::Text(chunk) = delta {
                if !chunk.is_empty() {
                    let _ = events.send(AgentEvent::RecapDelta { id, chunk });
                }
            }
        };
        let req = ChatRequest {
            model: &model,
            messages: &messages,
            tools: &[],
            temperature: Some(0.3),
            max_tokens: Some(280),
        };
        match provider.chat_stream(req, &mut on_delta).await {
            Ok(out) => {
                let _ = events.send(AgentEvent::RecapFinished {
                    id,
                    text: out.message.text(),
                });
            }
            Err(e) => {
                let _ = events.send(AgentEvent::RecapFailed {
                    id,
                    error: e.to_string(),
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn multi_provider_config() -> AppConfig {
        use hive_core::config::{ConnectionProfile, ModelRef};

        let mut cfg = AppConfig::default();
        cfg.connections.active = "groq".into();
        cfg.connections.profiles.clear();
        cfg.connections.profiles.insert(
            "groq".into(),
            ConnectionProfile {
                label: "Groq".into(),
                base_url: "https://api.groq.com/openai/v1".into(),
                api_key_env: "GROQ_API_KEY".into(),
                api_key: None,
                model: ModelRef::Id("llama".into()),
            },
        );
        cfg.connections.profiles.insert(
            "openai".into(),
            ConnectionProfile {
                label: "OpenAI".into(),
                base_url: "https://api.openai.com/v1".into(),
                api_key_env: "OPENAI_API_KEY".into(),
                api_key: None,
                model: ModelRef::Id("gpt-5".into()),
            },
        );
        cfg.secrets
            .connection_keys
            .insert("groq".into(), "gsk-one".into());
        cfg.secrets
            .connection_keys
            .insert("openai".into(), "sk-one".into());
        cfg
    }

    #[test]
    fn model_listing_targets_every_configured_provider() {
        let targets = listing_targets(&multi_provider_config()).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].connection_id, "groq");
        assert_eq!(targets[0].label, "Groq");
        assert_eq!(targets[1].connection_id, "openai");
        assert_eq!(targets[1].label, "OpenAI");
    }

    #[test]
    fn listing_cache_identity_changes_with_credentials() {
        let mut cfg = multi_provider_config();
        let before = listing_sources(&listing_targets(&cfg).unwrap());
        cfg.secrets
            .connection_keys
            .insert("openai".into(), "sk-two".into());
        let after = listing_sources(&listing_targets(&cfg).unwrap());
        assert_ne!(before, after);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_command_is_handled_while_turn_runs() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let terminal = hive_core::TerminalManager::new(events.clone());
        let started = terminal
            .start("cat", "test", std::path::Path::new("."))
            .await
            .unwrap();
        let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
        input_tx
            .send(InputCommand::TerminalAttach {
                id: started.id.clone(),
            })
            .unwrap();
        let mut pending = std::collections::VecDeque::new();
        let interrupt = AtomicBool::new(false);

        let completed = drive_turn(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                "done"
            },
            &mut input_rx,
            &terminal,
            &events,
            &mut pending,
            &interrupt,
        )
        .await;

        assert_eq!(completed, Some("done"));
        let read = terminal.read(&started.id, None, None).await.unwrap();
        assert_eq!(read.session.controller, hive_core::TerminalController::User);
        terminal.shutdown();
    }
}
