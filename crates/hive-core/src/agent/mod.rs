//! The agent loop: session state, system prompt, and turn execution.

mod compact;
mod context;
mod mode;
mod prompt;
mod run;
mod session;
pub mod session_store;

pub use compact::{should_compact, COMPACT_RATIO};
pub use context::{discover_context_files, ContextFile};
pub use mode::{
    is_orchestrator_tool, is_plan_path, multitask_mode_check, multitask_mode_tool_allowed,
    plan_mode_check, plan_mode_tool_allowed, plan_path, plan_summary, AgentMode, PLAN_REL_PATH,
};
pub use run::{tool_args_preview, Agent, AgentBuilder, FollowUpSlot, UserInput, MAX_ROUNDS};
pub use session::Session;
pub use session_store::{SessionMeta, SessionSnapshot};
