//! BUILD vs PLAN agent modes.

use std::path::{Path, PathBuf};

/// Relative path of the workspace plan file.
pub const PLAN_REL_PATH: &str = ".hive/Plan.md";

/// How the top-level agent should behave for the current turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentMode {
    /// Full coding agent (default).
    #[default]
    Build,
    /// Research + write/update `.hive/Plan.md` only — no project edits or shell.
    Plan,
}

impl AgentMode {
    pub fn label(self) -> &'static str {
        match self {
            AgentMode::Build => "BUILD",
            AgentMode::Plan => "PLAN",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            AgentMode::Build => AgentMode::Plan,
            AgentMode::Plan => AgentMode::Build,
        }
    }
}

/// Absolute path to `.hive/Plan.md` under `cwd`.
pub fn plan_path(cwd: &Path) -> PathBuf {
    cwd.join(PLAN_REL_PATH)
}

/// True when `path` (relative or absolute) resolves to the plan file.
pub fn is_plan_path(cwd: &Path, path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed == PLAN_REL_PATH {
        return true;
    }
    let path = Path::new(trimmed);
    path.is_absolute() && path == plan_path(cwd)
}

/// Tools the model may call while in PLAN mode (writes still path-gated).
pub fn plan_mode_tool_allowed(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_dir"
            | "glob"
            | "grep"
            | "web_search"
            | "web_get_contents"
            | "read_skill"
            | "write_file"
            | "edit_file"
    )
}

/// Gate a tool call in PLAN mode. `path` is required for write/edit.
pub fn plan_mode_check(name: &str, path: Option<&str>, cwd: &Path) -> Result<(), String> {
    if !plan_mode_tool_allowed(name) {
        return Err(format!(
            "`{name}` is not available in PLAN mode — switch to BUILD to run it"
        ));
    }
    if matches!(name, "write_file" | "edit_file") {
        let Some(path) = path else {
            return Err("missing 'path'".into());
        };
        if !is_plan_path(cwd, path) {
            return Err(format!(
                "in PLAN mode you may only write `{PLAN_REL_PATH}` (got `{path}`)"
            ));
        }
    }
    Ok(())
}

/// Pull a short summary from plan markdown (first `##` heading, else first line).
pub fn plan_summary(body: &str) -> String {
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("## ") {
            let s = rest.trim();
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .or_else(|| body.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("Plan.md")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn plan_mode_blocks_shell() {
        let cwd = Path::new("/tmp/proj");
        assert!(plan_mode_check("run_shell", None, cwd).is_err());
        assert!(plan_mode_check("verify_project", None, cwd).is_err());
    }

    #[test]
    fn plan_mode_allows_plan_write() {
        let cwd = Path::new("/tmp/proj");
        assert!(plan_mode_check("write_file", Some(PLAN_REL_PATH), cwd).is_ok());
        assert!(plan_mode_check("write_file", Some("/tmp/proj/.hive/Plan.md"), cwd).is_ok());
        assert!(plan_mode_check("write_file", Some("src/main.rs"), cwd).is_err());
        assert!(plan_mode_check("read_file", Some("src/main.rs"), cwd).is_ok());
    }

    #[test]
    fn plan_mode_rejects_plan_path_outside_workspace() {
        let cwd = Path::new("/tmp/proj");
        assert!(plan_mode_check("write_file", Some("/tmp/other/.hive/Plan.md"), cwd).is_err());
        assert!(plan_mode_check("write_file", Some("other/.hive/Plan.md"), cwd).is_err());
    }

    #[test]
    fn summary_prefers_h2() {
        let body = "# Title\n\n## First step\n\nDo things\n";
        assert_eq!(plan_summary(body), "First step");
    }
}
