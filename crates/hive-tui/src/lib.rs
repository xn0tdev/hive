//! Hive terminal frontend on the `comb` engine.

mod app;
mod commands;
mod intro;
mod render;
mod run;
mod sound;
mod terminal_input;
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
    /// Catalog says this model accepts image inputs.
    pub vision: bool,
    /// Context window in tokens (0 if unknown).
    pub context: u64,
    /// USD per 1M input tokens (0 if unknown).
    pub cost_input: f64,
    /// USD per 1M output tokens (0 if unknown).
    pub cost_output: f64,
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
    /// Model context window size (`[agent].context_window`).
    pub context_window: u64,
    /// USD per 1M input tokens (0 if unknown).
    pub cost_input: f64,
    /// USD per 1M output tokens (0 if unknown).
    pub cost_output: f64,
}

pub struct PrivateTerminalInput(Vec<u8>);

impl PrivateTerminalInput {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl std::fmt::Debug for PrivateTerminalInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<private terminal input: {} bytes>", self.0.len())
    }
}

/// Messages the TUI sends to the agent driver.
#[derive(Debug)]
pub enum InputCommand {
    User {
        text: String,
        images: Vec<ImageSource>,
        mode: AgentMode,
    },
    TerminalAttach {
        id: String,
    },
    TerminalDetach {
        id: String,
    },
    TerminalInput {
        id: String,
        input: PrivateTerminalInput,
    },
    TerminalResize {
        id: String,
        rows: u16,
        cols: u16,
    },
    TerminalStop {
        id: String,
    },
    /// Switch the active model (`display` is the TUI label).
    SetModel {
        id: String,
        display: String,
        /// When set, activate this `/connect` profile before applying the model.
        connection_id: Option<String>,
        /// Whether the model accepts image inputs (from the live catalog).
        vision: bool,
        /// Context window in tokens (0 if unknown).
        context: u64,
        /// USD per 1M input tokens (0 if unknown).
        cost_input: f64,
        /// USD per 1M output tokens (0 if unknown).
        cost_output: f64,
    },
    /// Update the agent's context window (from catalog, without switching model).
    UpdateContextWindow {
        context: u64,
    },
    /// Set a goal for the autonomous agent loop.
    SetGoal {
        objective: String,
        duration: Option<std::time::Duration>,
    },
    /// Stop the active goal loop.
    StopGoal,
    /// Pause the goal loop (finishes current turn, then waits).
    PauseGoal,
    /// Resume a paused goal loop.
    ResumeGoal,
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
    /// Save the current session to disk.
    SaveSession,
    /// Load a saved session from disk.
    LoadSession { id: String },
    /// List saved sessions (response comes back as AgentEvent).
    ListSessions,
    /// Persist UI prefs into `~/.config/hive/config.toml`.
    SaveUi(UiConfig),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_terminal_input_debug_is_redacted() {
        let input = PrivateTerminalInput::new(b"super-secret".to_vec());
        let rendered = format!("{input:?}");
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("private terminal input"));
    }
}
