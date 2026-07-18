//! `hive-agent`: the brain. It owns the YOLO agent loop *and* everything the
//! loop needs to do its work — the tool set, the swarm that runs subagents, the
//! skills loader, and the vision fallback. It stays UI-agnostic: it only speaks
//! `hive_core` traits and emits `AgentEvent`s.
//!
//! Layout:
//! - `agent` / `session` / `prompt` — the loop and its conversation state
//! - `tools`  — the concrete tools the agent can call (grouped by area)
//! - `swarm`  — runs subagents concurrently (implements `SubagentSpawner`)
//! - `skills` — loads SKILL.md files (implements `SkillSource`)
//! - `vision` — describe-and-inject image fallback (implements `VisionDescriber`)

mod agent;
mod prompt;
mod session;
mod skills;
mod swarm;
pub mod tools;
mod vision;

pub use agent::{Agent, AgentBuilder, UserInput};
pub use session::Session;
pub use skills::DiskSkills;
pub use swarm::new_spawner;
pub use tools::all_tools;
pub use vision::DescribeVision;
