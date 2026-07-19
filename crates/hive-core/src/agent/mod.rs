//! The agent loop: session state, system prompt, and turn execution.

mod context;
mod mode;
mod prompt;
mod run;
mod session;

pub use context::{discover_context_files, ContextFile};
pub use mode::{
    is_plan_path, plan_mode_check, plan_mode_tool_allowed, plan_path, plan_summary, AgentMode,
    PLAN_REL_PATH,
};
pub use run::{Agent, AgentBuilder, UserInput};
pub use session::Session;
