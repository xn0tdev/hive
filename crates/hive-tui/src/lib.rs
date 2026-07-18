//! Hive terminal frontend on the `comb` engine.

mod app;
mod commands;
mod render;
mod run;
mod sound;
mod theme;

use hive_core::message::ImageSource;
use hive_core::AgentMode;

pub use run::run;

/// Everything the TUI needs to know at startup.
pub struct TuiInit {
    /// Provider model id (API).
    pub model: String,
    /// Pretty label shown in the footer.
    pub model_display: String,
    pub cwd: String,
    pub theme: String,
    pub version: String,
}

/// Messages the TUI sends to the agent driver.
pub enum InputCommand {
    User {
        text: String,
        images: Vec<ImageSource>,
        mode: AgentMode,
    },
    SetModel(String),
    Clear,
}
