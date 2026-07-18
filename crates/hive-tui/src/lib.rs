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

/// A selectable model role / id for the Switch-model picker.
#[derive(Debug, Clone)]
pub struct ModelChoice {
    /// Value passed to `SetModel` (role name or provider id).
    pub key: String,
    /// Pretty label in the picker.
    pub display: String,
    /// Secondary text (e.g. role name).
    pub detail: String,
}

/// Everything the TUI needs to know at startup.
pub struct TuiInit {
    /// Provider model id (API).
    pub model: String,
    /// Pretty label shown in the footer.
    pub model_display: String,
    /// Roles / models offered in the Switch-model picker.
    pub model_choices: Vec<ModelChoice>,
    pub cwd: String,
    pub theme: String,
    pub version: String,
}

/// Messages the TUI sends to the agent driver.
#[derive(Debug)]
pub enum InputCommand {
    User {
        text: String,
        images: Vec<ImageSource>,
        mode: AgentMode,
    },
    SetModel(String),
    Clear,
}
