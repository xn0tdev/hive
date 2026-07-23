//! `hive-core` — contracts plus the agent brain.
//!
//! Layout:
//! - root modules — shared types and traits (`config`, `tool`, `provider`, …)
//! - [`agent`] — the YOLO loop, session, prompt
//! - [`tools`] — filesystem, shell, web, skill, delegate
//! - [`swarm`] — concurrent subagents
//! - [`skills`] — disk-backed `SKILL.md` loader

pub mod config;
pub mod error;
pub mod event;
pub mod message;
pub mod provider;
pub mod skill;
pub mod spawner;
pub mod terminal;
pub mod tool;

pub mod agent;
pub mod skills;
pub mod swarm;
pub mod tools;
pub mod worktree;

pub use config::{
    AppConfig, ModelRef, ModelRole, SearchBackend, Secrets, SidebarMode, UiConfig,
    DEFAULT_CONTEXT_WINDOW,
};
pub use error::{CoreError, Result};
pub use event::{
    AgentEvent, CatalogModel, ConnectionInfo, EventReceiver, EventSender, Renderer, SubagentLine,
    SubagentStatus,
};
pub use message::{ContentPart, ImageSource, Message, Role, ToolCall};
pub use provider::{ChatOutcome, ChatRequest, Delta, LlmProvider, ToolSpec, Usage};
pub use skill::{no_skills, NoSkills, SkillMeta, SkillSource};
pub use spawner::{noop_spawner, NoopSpawner, SubagentOutcome, SubagentSpawner, SubagentTask};
pub use terminal::{
    TerminalController, TerminalError, TerminalHandle, TerminalKey, TerminalManager,
    TerminalOutputFrame, TerminalProcessState, TerminalReadResult, TerminalSnapshot,
    TerminalWriteRequest, DEFAULT_COLS, DEFAULT_ROWS,
};
pub use tool::{Tool, ToolContext, ToolRegistration, ToolResult};

pub use agent::{
    discover_context_files, is_plan_path, multitask_mode_check, multitask_mode_tool_allowed,
    plan_mode_check, plan_mode_tool_allowed, plan_path, plan_summary, Agent, AgentBuilder,
    AgentMode, ContextFile, FollowUpSlot, Session, UserInput, PLAN_REL_PATH,
};
pub use skills::DiskSkills;
pub use swarm::new_spawner;
pub use tools::all_tools;
