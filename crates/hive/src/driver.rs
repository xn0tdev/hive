//! Agent driver: consumes `InputCommand`s from the TUI and runs turns.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedReceiver;

use hive_core::config::{AppConfig, ModelRole};
use hive_core::event::{AgentEvent, CatalogModel, EventSender};
use hive_core::provider::LlmProvider;
use hive_core::vision::VisionDescriber;
use hive_core::{Agent, DescribeVision, FollowUpSlot, UserInput};
use hive_llm::catalog::{
    enrich_models, fetch_models_dev, list_provider_models, models_dev_hint_for_base,
    provider_label_for_base, ModelCard,
};
use hive_llm::FireworksProvider;
use hive_tui::InputCommand;

pub async fn run(
    mut agent: Agent,
    mut input_rx: UnboundedReceiver<InputCommand>,
    events: EventSender,
    interrupt: Arc<AtomicBool>,
    follow_up: FollowUpSlot,
    mut cfg: Arc<AppConfig>,
) {
    while let Some(cmd) = input_rx.recv().await {
        match cmd {
            InputCommand::User { text, images, mode } => {
                interrupt.store(false, Ordering::Relaxed);
                // Drop any stale mid-turn inject from a previous turn.
                if let Ok(mut g) = follow_up.lock() {
                    *g = None;
                }
                agent
                    .run_turn(
                        UserInput { text, images, mode },
                        interrupt.clone(),
                        follow_up.clone(),
                    )
                    .await;
            }
            InputCommand::SetModel {
                id,
                display,
                connection_id,
            } => {
                if let Some(cid) = connection_id.as_deref().filter(|c| !c.is_empty()) {
                    if cid != cfg.connections.active {
                        if let Err(e) = apply_connection(&mut agent, &mut cfg, &events, cid).await {
                            let _ =
                                events.send(AgentEvent::Notice(format!("connect failed: {e}")));
                            continue;
                        }
                    }
                }
                let (id, display) = resolve_model(&cfg, &id, &display);
                agent.set_model(id.clone());
                if let Err(e) = crate::config::patch_model(&id, &display) {
                    let _ =
                        events.send(AgentEvent::Notice(format!("could not save model: {e}")));
                }
                let _ = events.send(AgentEvent::ModelChanged {
                    id: id.clone(),
                    display: display.clone(),
                });
                let _ = events.send(AgentEvent::Notice(format!("Model set to {display}")));
            }
            InputCommand::FetchModels => {
                fetch_and_emit_models(&cfg, &events).await;
            }
            InputCommand::SetConnection { id } => {
                if let Err(e) = apply_connection(&mut agent, &mut cfg, &events, &id).await {
                    let _ = events.send(AgentEvent::Notice(format!("connect failed: {e}")));
                }
            }
            InputCommand::UpsertConnection {
                id,
                label,
                base_url,
                api_key_env,
                api_key,
                model_id,
                model_name,
            } => {
                if let Err(e) = crate::config::upsert_connection(
                    &id,
                    &label,
                    &base_url,
                    &api_key_env,
                    Some(&api_key),
                    &model_id,
                    &model_name,
                ) {
                    let _ = events.send(AgentEvent::Notice(format!("could not save provider: {e}")));
                    continue;
                }
                if let Err(e) = apply_connection(&mut agent, &mut cfg, &events, &id).await {
                    let _ = events.send(AgentEvent::Notice(format!("connect failed: {e}")));
                }
            }
            InputCommand::RemoveConnection { id } => {
                if let Err(e) = crate::config::remove_connection(&id) {
                    let _ = events.send(AgentEvent::Notice(format!("could not remove: {e}")));
                    continue;
                }
                match crate::config::load() {
                    Ok(new_cfg) => {
                        cfg = new_cfg;
                        emit_connections(&cfg, &events);
                        let active = cfg.connections.active.clone();
                        if let Err(e) =
                            apply_connection(&mut agent, &mut cfg, &events, &active).await
                        {
                            let _ =
                                events.send(AgentEvent::Notice(format!("reconnect failed: {e}")));
                        }
                    }
                    Err(e) => {
                        let _ = events.send(AgentEvent::Notice(format!("reload failed: {e}")));
                    }
                }
            }
            InputCommand::Clear => {
                agent.reset();
            }
            InputCommand::Compact => {
                if let Err(e) = agent.compact().await {
                    let _ = events.send(AgentEvent::Notice(e));
                }
            }
            InputCommand::SaveUi(ui) => {
                if let Err(e) = crate::config::patch_ui(&ui) {
                    let _ =
                        events.send(AgentEvent::Notice(format!("could not save settings: {e}")));
                }
            }
        }
    }
}

async fn apply_connection(
    agent: &mut Agent,
    cfg: &mut Arc<AppConfig>,
    events: &EventSender,
    id: &str,
) -> anyhow::Result<()> {
    crate::config::activate_connection(id)?;
    let new_cfg = crate::config::load()?;
    *cfg = new_cfg;

    let provider: Arc<dyn LlmProvider> = Arc::new(FireworksProvider::new(
        cfg.provider.base_url.clone(),
        cfg.secrets.provider_api_key.clone(),
    ));
    let model_id = cfg.models.default.id().to_string();
    let model_display = cfg.models.default.display_name().to_string();
    let vision: Arc<dyn VisionDescriber> =
        Arc::new(DescribeVision::new(provider.clone(), model_id.clone()));

    agent.set_provider(provider);
    agent.set_vision(vision);
    agent.set_model(model_id.clone());

    emit_connections(cfg, events);
    let _ = events.send(AgentEvent::ModelChanged {
        id: model_id,
        display: model_display.clone(),
    });
    let label = cfg
        .connections
        .profiles
        .get(id)
        .map(|p| p.label.as_str())
        .unwrap_or(id);
    let _ = events.send(AgentEvent::Notice(format!(
        "Connected: {label} · {model_display}"
    )));
    Ok(())
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

async fn fetch_and_emit_models(cfg: &AppConfig, events: &EventSender) {
    let catalog = fetch_models_dev().await.ok();
    let mut models = Vec::new();
    let mut errors = Vec::new();

    let mut targets: Vec<(String, String, String, String)> = cfg
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
            Some((id.clone(), label, p.base_url.clone(), key))
        })
        .collect();

    // No profiles / no keys on profiles — fall back to active [provider].
    if targets.is_empty() {
        let key = cfg.secrets.provider_api_key.clone();
        if key.is_empty() {
            let _ = events.send(AgentEvent::ModelsListFailed(
                "no provider API key — set one in config or env".into(),
            ));
            return;
        }
        let base = cfg.provider.base_url.clone();
        let label = active_provider_label(cfg, &base);
        let id = if cfg.connections.active.is_empty() {
            "default".into()
        } else {
            cfg.connections.active.clone()
        };
        targets.push((id, label, base, key));
    }

    for (conn_id, label, base, key) in targets {
        match list_provider_models(&base, &key).await {
            Ok(remote) => {
                let hint = models_dev_hint_for_base(&base);
                let cards = enrich_models(&remote, catalog.as_ref(), hint);
                models.extend(catalog_rows(&label, &conn_id, &cards));
            }
            Err(e) => errors.push(format!("{label}: {e}")),
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
    let _ = events.send(AgentEvent::ModelsListed { models });
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

fn catalog_rows(group: &str, connection_id: &str, cards: &[ModelCard]) -> Vec<CatalogModel> {
    cards
        .iter()
        .map(|c| CatalogModel {
            id: c.id.clone(),
            name: c.name.clone(),
            detail: c.badges(),
            group: group.to_string(),
            connection_id: connection_id.to_string(),
        })
        .collect()
}
