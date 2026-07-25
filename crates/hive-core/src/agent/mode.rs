//! MAKE / PLAN / MULTITASK agent modes.

use std::path::{Path, PathBuf};

/// Relative path of the workspace plan file.
pub const PLAN_REL_PATH: &str = ".hive/Plan.md";

/// How the top-level agent should behave for the current turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentMode {
    /// Full coding agent (default).
    #[default]
    Make,
    /// Research + write/update `.hive/Plan.md` only — no project edits or shell.
    Plan,
    /// Orchestrator: split work into parallel subagents; almost no coding itself.
    Multitask,
}

impl AgentMode {
    pub fn label(self) -> &'static str {
        match self {
            AgentMode::Make => "MAKE",
            AgentMode::Plan => "PLAN",
            AgentMode::Multitask => "MULTITASK",
        }
    }

    /// Title-cased name for prose / cards ("Switched to Plan Mode").
    pub fn title(self) -> &'static str {
        match self {
            AgentMode::Make => "Make",
            AgentMode::Plan => "Plan",
            AgentMode::Multitask => "Multitask",
        }
    }

    /// Parse a mode name (case-insensitive) as used by the `switch_mode` tool.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "make" => Some(AgentMode::Make),
            "plan" => Some(AgentMode::Plan),
            "multitask" => Some(AgentMode::Multitask),
            _ => None,
        }
    }

    /// Tab cycle: MAKE → PLAN → MULTITASK → MAKE.
    pub fn toggle(self) -> Self {
        match self {
            AgentMode::Make => AgentMode::Plan,
            AgentMode::Plan => AgentMode::Multitask,
            AgentMode::Multitask => AgentMode::Make,
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
            | "write_plan"
            | "write_file"
            | "edit_file"
    )
}

/// Gate a tool call in PLAN mode. `path` is required for write/edit.
pub fn plan_mode_check(name: &str, path: Option<&str>, cwd: &Path) -> Result<(), String> {
    if !plan_mode_tool_allowed(name) {
        return Err(format!(
            "`{name}` is not available in PLAN mode — switch to MAKE to run it"
        ));
    }
    if name == "write_plan" {
        return Ok(());
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

/// Tools the orchestrator may call in MULTITASK mode (no project edits/shell).
pub fn multitask_mode_tool_allowed(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "list_dir"
            | "glob"
            | "grep"
            | "web_search"
            | "web_get_contents"
            | "read_skill"
            | "spawn_subagent"
            | "spawn_swarm"
            | "integrate_worktree"
            | "switch_mode"
    )
}

/// Gate a tool call in MULTITASK mode for the main agent.
pub fn multitask_mode_check(name: &str) -> Result<(), String> {
    if multitask_mode_tool_allowed(name) {
        Ok(())
    } else {
        Err(format!(
            "`{name}` is not available in MULTITASK — spawn subagents to implement, or switch to MAKE"
        ))
    }
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
        let cwd = if cfg!(windows) {
            Path::new("C:\\proj")
        } else {
            Path::new("/tmp/proj")
        };
        assert!(plan_mode_check("write_plan", None, cwd).is_ok());
        assert!(plan_mode_check("write_file", Some(PLAN_REL_PATH), cwd).is_ok());
        let abs_plan = plan_path(cwd).to_string_lossy().to_string();
        assert!(plan_mode_check("write_file", Some(&abs_plan), cwd).is_ok());
        assert!(plan_mode_check("write_file", Some("src/main.rs"), cwd).is_err());
        assert!(plan_mode_check("read_file", Some("src/main.rs"), cwd).is_ok());
    }

    #[test]
    fn plan_mode_rejects_plan_path_outside_workspace() {
        let cwd = if cfg!(windows) {
            Path::new("C:\\proj")
        } else {
            Path::new("/tmp/proj")
        };
        let other = if cfg!(windows) {
            "C:\\other\\.hive\\Plan.md"
        } else {
            "/tmp/other/.hive/Plan.md"
        };
        assert!(plan_mode_check("write_file", Some(other), cwd).is_err());
        assert!(plan_mode_check("write_file", Some("other/.hive/Plan.md"), cwd).is_err());
    }

    #[test]
    fn summary_prefers_h2() {
        let body = "# Title\n\n## First step\n\nDo things\n";
        assert_eq!(plan_summary(body), "First step");
    }

    #[test]
    fn mode_cycles_three_ways() {
        assert_eq!(AgentMode::Make.toggle(), AgentMode::Plan);
        assert_eq!(AgentMode::Plan.toggle(), AgentMode::Multitask);
        assert_eq!(AgentMode::Multitask.toggle(), AgentMode::Make);
        assert_eq!(AgentMode::Multitask.label(), "MULTITASK");
    }

    #[test]
    fn mode_parses_case_insensitively() {
        assert_eq!(AgentMode::parse("plan"), Some(AgentMode::Plan));
        assert_eq!(AgentMode::parse("  PLAN "), Some(AgentMode::Plan));
        assert_eq!(AgentMode::parse("Make"), Some(AgentMode::Make));
        assert_eq!(AgentMode::parse("MultiTask"), Some(AgentMode::Multitask));
        assert_eq!(AgentMode::parse("nope"), None);
        assert_eq!(AgentMode::Plan.title(), "Plan");
    }

    #[test]
    fn make_is_the_full_coding_mode() {
        let mode = AgentMode::parse("  MaKe ").expect("MAKE mode");

        assert_eq!(mode.label(), "MAKE");
        assert_eq!(mode.title(), "Make");
        assert_eq!(AgentMode::Multitask.toggle(), mode);
    }

    #[test]
    fn switch_mode_gated_out_of_plan_but_allowed_elsewhere() {
        let cwd = Path::new("/tmp/proj");
        // Agent can enter PLAN from MAKE/MULTITASK, but not switch modes while
        // already in PLAN — the user stays in control of leaving PLAN.
        assert!(plan_mode_check("switch_mode", None, cwd).is_err());
        assert!(multitask_mode_tool_allowed("switch_mode"));
    }

    #[test]
    fn multitask_blocks_coding_tools() {
        assert!(multitask_mode_check("run_shell").is_err());
        assert!(multitask_mode_check("write_file").is_err());
        assert!(multitask_mode_check("edit_file").is_err());
        assert!(multitask_mode_check("verify_project").is_err());
        assert!(multitask_mode_check("spawn_swarm").is_ok());
        assert!(multitask_mode_check("spawn_subagent").is_ok());
        assert!(multitask_mode_check("integrate_worktree").is_ok());
        assert!(multitask_mode_check("read_file").is_ok());
    }

    #[test]
    fn terminal_tools_are_rejected_outside_make() {
        let cwd = Path::new("/tmp/proj");
        for name in [
            "terminal_start",
            "terminal_read",
            "terminal_write",
            "terminal_stop",
        ] {
            assert!(plan_mode_check(name, None, cwd).is_err(), "{name}");
            assert!(multitask_mode_check(name).is_err(), "{name}");
        }
    }
}
