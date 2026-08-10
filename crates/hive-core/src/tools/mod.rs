//! The agent's tools, grouped by area. Every tool implements `crate::Tool`
//! and self-registers via `inventory`, so `all_tools()` returns them with zero
//! central wiring — dropping a new tool into any of these files is enough.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::tool::{Tool, ToolContext, ToolRegistration};

mod delegate;
mod files;
mod mode;
mod plan;
mod shell;
mod skill;
mod terminal;
mod todo;
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
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Resolve a possibly-relative path against the agent's working directory.
pub(crate) fn resolve(cwd: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    lexical_normalize(&joined)
}

/// Resolve a path for structured filesystem tools.
///
/// The default policy is workspace-only. The check is both lexical and
/// filesystem-aware: `..` cannot escape the root, and symlinks cannot redirect
/// a read/write/delete operation outside it. Nonexistent paths are checked
/// through their nearest existing parent so new files remain supported.
pub(crate) fn resolve_workspace(ctx: &ToolContext, p: &str) -> Result<PathBuf, String> {
    let candidate = resolve(&ctx.cwd, p);
    if !ctx.config.agent.workspace_only {
        return Ok(candidate);
    }

    let root = ctx
        .cwd
        .canonicalize()
        .map_err(|e| format!("cannot verify workspace {}: {e}", ctx.cwd.display()))?;
    let checked = canonicalize_for_access(&candidate).map_err(|e| {
        format!(
            "cannot verify path {} inside workspace: {e}",
            candidate.display()
        )
    })?;
    if !checked.starts_with(&root) {
        return Err(format!(
            "path {} is outside the workspace; use a relative path inside {}",
            candidate.display(),
            root.display()
        ));
    }
    Ok(candidate)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

fn canonicalize_for_access(path: &Path) -> std::io::Result<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut suffix: Vec<OsString> = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name().map(OsString::from) else {
            break;
        };
        suffix.push(name);
        if !existing.pop() {
            break;
        }
    }

    let mut canonical = std::fs::canonicalize(&existing)?;
    for part in suffix.iter().rev() {
        canonical.push(part);
    }
    Ok(canonical)
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
    use crate::config::AppConfig;
    use std::sync::Arc;

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

    #[test]
    fn workspace_paths_allow_children_and_reject_parent_escape() {
        let root = std::env::temp_dir().join(format!("hive-path-policy-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let ctx = ToolContext {
            cwd: root.clone(),
            events: tokio::sync::mpsc::unbounded_channel().0,
            spawner: crate::noop_spawner(),
            skills: crate::no_skills(),
            config: Arc::new(AppConfig::default()),
            terminal: None,
            vision: false,
            depth: 0,
            call_id: "path-test".into(),
            isolate_worktrees: false,
            interrupt: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        assert!(resolve_workspace(&ctx, "src/main.rs").is_ok());
        assert!(resolve_workspace(&ctx, "../outside.txt").is_err());

        let _ = std::fs::remove_dir_all(root);
    }
}
