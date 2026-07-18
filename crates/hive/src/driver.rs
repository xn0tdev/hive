//! The agent driver: consumes `InputCommand`s from the TUI and runs turns on
//! the agent. Kept separate from the TUI so the loop is UI-agnostic.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedReceiver;

use hive_agent::{Agent, UserInput};
use hive_core::config::{AppConfig, ModelRole};
use hive_core::event::{AgentEvent, EventSender};
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
            InputCommand::User { text, images } => {
                interrupt.store(false, Ordering::Relaxed);
                agent
                    .run_turn(UserInput { text, images }, interrupt.clone())
                    .await;
            }
            InputCommand::SetModel(s) => {
                let model = resolve_model(&cfg, &s);
                agent.set_model(model.clone());
                let _ = events.send(AgentEvent::ModelChanged(model.clone()));
                let _ = events.send(AgentEvent::Notice(format!("Model set to {model}")));
            }
            InputCommand::Clear => {
                agent.reset();
            }
        }
    }
}

/// Accept either a role name (default/smart/fast/vision) or a literal model id.
fn resolve_model(cfg: &AppConfig, s: &str) -> String {
    match ModelRole::parse(s) {
        Some(role) => cfg.model(role).to_string(),
        None => s.to_string(),
    }
}
