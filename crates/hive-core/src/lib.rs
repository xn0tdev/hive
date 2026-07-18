//! `hive-core` — contracts plus the agent brain.
//!
//! Layout:
//! - root modules — shared types and traits (`config`, `tool`, `provider`, …)
//! - [`agent`] — the YOLO loop, session, prompt
//! - [`tools`] — filesystem, shell, web, skill, delegate
//! - [`swarm`] — concurrent subagents
//! - [`skills`] — disk-backed `SKILL.md` loader
//! - [`vision_impl`] — describe-and-inject vision fallback

pub mod config;
pub mod error;
pub mod event;
pub mod message;
pub mod provider;
pub mod skill;
pub mod spawner;
pub mod tool;
pub mod vision;

pub mod agent;
pub mod skills;
pub mod swarm;
pub mod tools;
mod vision_impl;

pub use config::{AppConfig, ModelRole, Secrets};
pub use error::{CoreError, Result};
pub use event::{
    AgentEvent, EventReceiver, EventSender, Renderer, SubagentLine, SubagentStatus,
};
pub use message::{ContentPart, ImageSource, Message, Role, ToolCall};
pub use provider::{ChatOutcome, ChatRequest, Delta, LlmProvider, ToolSpec, Usage};
pub use skill::{no_skills, NoSkills, SkillMeta, SkillSource};
pub use spawner::{noop_spawner, NoopSpawner, SubagentOutcome, SubagentSpawner, SubagentTask};
pub use tool::{Tool, ToolContext, ToolRegistration, ToolResult};
pub use vision::{no_vision, NoVision, VisionDescriber};

pub use agent::{Agent, AgentBuilder, Session, UserInput};
pub use skills::DiskSkills;
pub use swarm::new_spawner;
pub use tools::all_tools;
pub use vision_impl::DescribeVision;
