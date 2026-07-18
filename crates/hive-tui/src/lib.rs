//! Hive terminal frontend on the `comb` engine.

mod app;
mod commands;
mod render;
mod run;
mod sound;
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
