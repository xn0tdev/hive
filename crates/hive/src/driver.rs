//! Agent driver: consumes `InputCommand`s from the TUI and runs turns.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedReceiver;

use hive_core::config::{AppConfig, ModelRole};
use hive_core::event::{AgentEvent, EventSender};
use hive_core::{Agent, UserInput};
use hive_tui::InputCommand;

pub async fn run(
    mut agent: Agent,
    mut input_rx: UnboundedReceiver<InputCommand>,
    events: EventSender,
    interrupt: Arc<AtomicBool>,
    cfg: Arc<AppConfig>,
) {
    while let Some(cmd) = input_rx.recv().await {
        match cmd {
            InputCommand::User { text, images, mode } => {
                interrupt.store(false, Ordering::Relaxed);
                agent
                    .run_turn(UserInput { text, images, mode }, interrupt.clone())
                    .await;
            }
            InputCommand::SetModel(s) => {
                let (id, display) = resolve_model(&cfg, &s);
                agent.set_model(id.clone());
                let _ = events.send(AgentEvent::ModelChanged {
                    id: id.clone(),
                    display: display.clone(),
                });
                let _ = events.send(AgentEvent::Notice(format!("Model set to {display}")));
            }
            InputCommand::Clear => {
                agent.reset();
            }
        }
    }
}

fn resolve_model(cfg: &AppConfig, s: &str) -> (String, String) {
    match ModelRole::parse(s) {
        Some(role) => (
            cfg.model(role).to_string(),
            cfg.model_display(role).to_string(),
        ),
        None => {
            let id = s.to_string();
            let display = cfg.display_for_model_id(&id);
            (id, display)
        }
    }
}
