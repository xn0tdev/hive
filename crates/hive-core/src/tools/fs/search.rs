//! Directory listing, glob, and grep tool implementations.

use async_trait::async_trait;
use globset::GlobBuilder;
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::resolve_workspace;
use super::super::str_arg;
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

pub struct ListDir;

#[async_trait]
impl Tool for ListDir {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List the entries of a directory (defaults to the working directory), marking directories with a trailing slash."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory path (default: working directory)."}
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let path = str_arg(&args, "path").unwrap_or(".");
        let full = match resolve_workspace(ctx, path) {
            Ok(path) => path,
            Err(error) => return ToolResult::error(error),
        };

        let mut rd = match tokio::fs::read_dir(&full).await {
            Ok(rd) => rd,
            Err(e) => return ToolResult::error(format!("cannot list {}: {e}", full.display())),
        };

        let mut dirs = Vec::new();
        let mut files = Vec::new();
        loop {
            match rd.next_entry().await {
                Ok(Some(entry)) => {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                    if is_dir {
                        dirs.push(format!("{name}/"));
                    } else {
                        files.push(name);
                    }
                }
                Ok(None) => break,
                Err(e) => return ToolResult::error(format!("error reading dir: {e}")),
            }
        }

        dirs.sort();
        files.sort();
        let mut out = String::new();
        for d in dirs {
            out.push_str(&d);
            out.push('\n');
        }
        for f in files {
            out.push_str(&f);
            out.push('\n');
        }
        if out.is_empty() {
            out.push_str("(empty)");
        }
        ToolResult::ok(out)
    }
}

pub struct Glob;

#[async_trait]
impl Tool for Glob {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern (e.g. `**/*.rs`, `src/**/mod.rs`). Respects .gitignore. Returns paths relative to the search base."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern. Use ** to cross directories."},
                "path": {"type": "string", "description": "Base directory to search (default: working directory)."}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(pattern) = str_arg(&args, "pattern") else {
            return ToolResult::error("missing 'pattern'");
        };
        let base = match resolve_workspace(ctx, str_arg(&args, "path").unwrap_or(".")) {
            Ok(path) => path,
            Err(error) => return ToolResult::error(error),
        };
        let pattern = pattern.to_string();

        let matcher = match GlobBuilder::new(&pattern).literal_separator(true).build() {
            Ok(g) => g.compile_matcher(),
            Err(e) => return ToolResult::error(format!("invalid pattern: {e}")),
        };

        let results = tokio::task::spawn_blocking(move || {
            let mut out: Vec<String> = Vec::new();
            for dent in WalkBuilder::new(&base).hidden(false).build().flatten() {
                let p = dent.path();
                if let Ok(rel) = p.strip_prefix(&base) {
                    if matcher.is_match(rel) {
                        out.push(rel.display().to_string());
                    }
                }
                if out.len() >= 1000 {
                    break;
                }
            }
            out.sort();
            out
        })
        .await
        .unwrap_or_default();

        if results.is_empty() {
            ToolResult::ok("(no matches)")
        } else {
            ToolResult::ok(results.join("\n"))
        }
    }
}

pub struct Grep;

#[async_trait]
impl Tool for Grep {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents with a regular expression. Respects .gitignore. Returns matching lines as `path:line: text` (capped at 200 matches)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regular expression to search for."},
                "path": {"type": "string", "description": "Base directory or file to search (default: working directory)."}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(pattern) = str_arg(&args, "pattern") else {
            return ToolResult::error("missing 'pattern'");
        };
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("invalid regex: {e}")),
        };
        let base = match resolve_workspace(ctx, str_arg(&args, "path").unwrap_or(".")) {
            Ok(path) => path,
            Err(error) => return ToolResult::error(error),
        };
        let cwd = ctx.cwd.clone();

        let results = tokio::task::spawn_blocking(move || {
            let mut out: Vec<String> = Vec::new();
            'walk: for dent in WalkBuilder::new(&base).hidden(false).build().flatten() {
                if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                let p = dent.path();
                let content = match std::fs::read_to_string(p) {
                    Ok(c) => c,
                    Err(_) => continue, // binary or unreadable
                };
                let shown = p.strip_prefix(&cwd).unwrap_or(p).display().to_string();
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        let text = line.trim_end();
                        let text: String = text.chars().take(240).collect();
                        out.push(format!("{shown}:{}: {text}", i + 1));
                        if out.len() >= 200 {
                            break 'walk;
                        }
                    }
                }
            }
            out
        })
        .await
        .unwrap_or_default();

        if results.is_empty() {
            ToolResult::ok("(no matches)")
        } else {
            ToolResult::ok(results.join("\n"))
        }
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(ListDir) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(Glob) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(Grep) as Arc<dyn Tool> } }
