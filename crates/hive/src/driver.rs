//! Agent driver: consumes `InputCommand`s from the TUI and runs turns.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedReceiver;

use hive_core::config::{AppConfig, ModelRole};
use hive_core::event::{AgentEvent, CatalogModel, EventSender};
use hive_core::provider::LlmProvider;
use hive_core::vision::VisionDescriber;
use hive_core::{Agent, DescribeVision, UserInput};
use hive_llm::catalog::{
    enrich_models, fetch_models_dev, group_by_id_prefix, list_provider_models,
    models_dev_hint_for_base, provider_label_for_base, ModelCard,
};
use hive_llm::FireworksProvider;
use hive_tui::InputCommand;

pub async fn run(
    mut agent: Agent,
    mut input_rx: UnboundedReceiver<InputCommand>,
    events: EventSender,
    interrupt: Arc<AtomicBool>,
    mut cfg: Arc<AppConfig>,
) {
    while let Some(cmd) = input_rx.recv().await {
        match cmd {
            InputCommand::User { text, images, mode } => {
                interrupt.store(false, Ordering::Relaxed);
                agent
                    .run_turn(UserInput { text, images, mode }, interrupt.clone())
                    .await;
            }
            InputCommand::SetModel { id, display } => {
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
    let base = cfg.provider.base_url.as_str();
    let key = cfg.secrets.provider_api_key.as_str();
    if key.is_empty() {
        let _ = events.send(AgentEvent::ModelsListFailed(
            "no provider API key — set one in config or env".into(),
        ));
        return;
    }

    let remote = match list_provider_models(base, key).await {
        Ok(m) => m,
        Err(e) => {
            let _ = events.send(AgentEvent::ModelsListFailed(e.to_string()));
            return;
        }
    };

    let catalog = fetch_models_dev().await.ok();
    let hint = models_dev_hint_for_base(base);
    let cards = enrich_models(&remote, catalog.as_ref(), hint);
    let mut models = catalog_rows(base, &cards);
    models.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    let _ = events.send(AgentEvent::ModelsListed { models });
}

fn catalog_rows(base_url: &str, cards: &[ModelCard]) -> Vec<CatalogModel> {
    let provider = provider_label_for_base(base_url);
    let split = group_by_id_prefix(base_url);
    cards
        .iter()
        .map(|c| {
            let group = if split {
                c.id.split('/').next().unwrap_or(provider).to_string()
            } else {
                provider.to_string()
            };
            CatalogModel {
                id: c.id.clone(),
                name: c.name.clone(),
                detail: c.badges(),
                group,
            }
        })
        .collect()
}
