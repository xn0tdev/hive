//! Hive terminal frontend on the `comb` engine.

mod app;
mod commands;
mod intro;
mod render;
mod run;
mod sound;
mod theme;

use hive_core::event::ConnectionInfo;
use hive_core::message::ImageSource;
use hive_core::AgentMode;
use hive_core::UiConfig;

pub use intro::{run_intro, IntroOpts, IntroPrefill, IntroResult, SetupDraft, PRESETS};
pub use run::run;

/// A selectable model for the Switch-model picker.
#[derive(Debug, Clone)]
pub struct ModelChoice {
    /// Value passed to `SetModel` (provider id).
    pub key: String,
    /// Pretty label in the picker.
    pub display: String,
    /// Secondary text (badges / short id).
    pub detail: String,
    /// Section header (provider label).
    pub group: String,
    /// `/connect` profile id — switching model may activate this provider.
    pub connection_id: String,
}

/// A skill exposed in the `/` composer menu (`/skill-name`).
#[derive(Debug, Clone)]
pub struct SkillChoice {
    /// Slash + `read_skill` name.
    pub name: String,
    /// One-line description for the `/` menu.
    pub description: String,
    /// Full `SKILL.md` body injected when the skill is invoked.
    pub content: String,
}

/// Everything the TUI needs to know at startup.
pub struct TuiInit {
    /// Provider model id (API).
    pub model: String,
    /// Pretty label shown in the footer.
    pub model_display: String,
    /// Seed rows for the Switch-model picker (replaced by live catalog).
    pub model_choices: Vec<ModelChoice>,
    /// Skills available via `/name` in the composer.
    pub skills: Vec<SkillChoice>,
    /// Saved `/connect` providers.
    pub connections: Vec<ConnectionInfo>,
    /// Active connection profile id.
    pub active_connection: String,
    pub cwd: String,
    pub theme: String,
    pub version: String,
    /// Chat / sidebar prefs from `[ui]` in config.toml.
    pub ui: UiConfig,
}

/// Messages the TUI sends to the agent driver.
#[derive(Debug)]
pub enum InputCommand {
    User {
        text: String,
        images: Vec<ImageSource>,
        mode: AgentMode,
    },
    /// Switch the active model (`display` is the TUI label).
    SetModel {
        id: String,
        display: String,
        /// When set, activate this `/connect` profile before applying the model.
        connection_id: Option<String>,
    },
    /// Refresh the `/model` picker from `GET /models` + models.dev.
    FetchModels,
    /// Activate a saved `/connect` profile.
    SetConnection {
        id: String,
    },
    /// Add/update a provider profile and activate it.
    UpsertConnection {
        id: String,
        label: String,
        base_url: String,
        api_key_env: String,
        api_key: String,
        model_id: String,
        model_name: String,
    },
    /// Remove a saved provider profile.
    RemoveConnection {
        id: String,
    },
    Clear,
    /// Compress agent conversation history (keeps task continuity).
    Compact,
    /// Persist UI prefs into `~/.config/hive/config.toml`.
    SaveUi(UiConfig),
}
