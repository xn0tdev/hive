//! The agent's tools, grouped by area. Every tool implements `crate::Tool`
//! and self-registers via `inventory`, so `all_tools()` returns them with zero
//! central wiring — dropping a new tool into any of these files is enough.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::tool::{Tool, ToolRegistration};

mod delegate;
mod files;
mod mode;
mod plan;
mod shell;
mod skill;
mod terminal;
mod web;

/// Build fresh instances of every registered tool, sorted by name.
/// Mode gating (MAKE hides `spawn_swarm` / `integrate_worktree`) happens in the agent.
pub fn all_tools() -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = inventory::iter::<ToolRegistration>()
        .map(|r| (r.make)())
        .collect();
    tools.sort_by(|a, b| a.name().cmp(b.name()));
    tools
}

// --- small shared helpers used across the tool files ---

/// A process-wide reqwest client shared by the networked tools (Exa, etc.).
pub(crate) fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Resolve a possibly-relative path against the agent's working directory.
pub(crate) fn resolve(cwd: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

pub(crate) fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

pub(crate) fn u64_arg(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

pub(crate) fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tools_offers_delegate_suite() {
        let tools = all_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"verify_project"));
        assert!(names.contains(&"spawn_subagent"));
        assert!(names.contains(&"spawn_swarm"));
        assert!(names.contains(&"integrate_worktree"));
        assert!(names.contains(&"write_plan"));
        assert!(names.contains(&"delete_path"));
        assert!(names.contains(&"switch_mode"));
    }
}
