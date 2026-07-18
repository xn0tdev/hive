//! `hive-tui`: the terminal frontend, built on the native `comb` engine.
//! Consumes `AgentEvent`s to render a live, streaming conversation and sends the
//! user's `InputCommand`s back to the agent driver. Knows nothing about
//! providers, tools, or the swarm.

mod app;
mod commands;
mod render;
mod run;
mod theme;

use hive_core::message::ImageSource;

pub use run::run;

/// Everything the TUI needs to know at startup.
pub struct TuiInit {
    pub model: String,
    pub cwd: String,
    pub theme: String,
    pub version: String,
}

/// Messages the TUI sends to the agent driver.
pub enum InputCommand {
    User {
        text: String,
        images: Vec<ImageSource>,
    },
    SetModel(String),
    Clear,
}
