//! `hive-core` is the contract layer: traits and plain domain types shared by
//! every other crate. It has no heavy dependencies and knows nothing about
//! concrete providers, tools, or the TUI. Everything else depends on this and
//! only this, which keeps the workspace decoupled and easy to extend.

pub mod config;
pub mod error;
pub mod event;
pub mod message;
pub mod provider;
pub mod skill;
pub mod spawner;
pub mod tool;
pub mod vision;

pub use config::{AppConfig, ModelRole, Secrets};
pub use error::{CoreError, Result};
pub use event::{AgentEvent, EventReceiver, EventSender, Renderer, SubagentStatus};
pub use message::{ContentPart, ImageSource, Message, Role, ToolCall};
pub use provider::{ChatOutcome, ChatRequest, Delta, LlmProvider, ToolSpec, Usage};
pub use skill::{no_skills, NoSkills, SkillMeta, SkillSource};
pub use spawner::{noop_spawner, NoopSpawner, SubagentOutcome, SubagentSpawner, SubagentTask};
pub use tool::{Tool, ToolContext, ToolRegistration, ToolResult};
pub use vision::{no_vision, NoVision, VisionDescriber};
