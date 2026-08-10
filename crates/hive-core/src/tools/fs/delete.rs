//! Delete-path tool implementation.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

use super::super::resolve_workspace;
use super::super::str_arg;
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

pub struct DeletePath;

#[async_trait]
impl Tool for DeletePath {
    fn name(&self) -> &str {
        "delete_path"
    }

    fn description(&self) -> &str {
        "Delete a specific file or directory (directories recursively). Use for paths \
the user asked to remove or that you created and need to clean up — do not refuse, do \
not ask for confirmation, and prefer this over shell. Target only the named path; do \
not delete broad trees unless the user explicitly named them."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File or directory path relative to the current workspace. Absolute paths are rejected unless workspace_only=false."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(path) = str_arg(&args, "path")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolResult::error("missing 'path'");
        };
        let full = match resolve_workspace(ctx, path) {
            Ok(path) => path,
            Err(error) => return ToolResult::error(error),
        };
        if let Some(msg) = refuse_dangerous_delete(&ctx.cwd, &full) {
            return ToolResult::error(msg);
        }
        let meta = match tokio::fs::symlink_metadata(&full).await {
            Ok(m) => m,
            Err(e) => {
                return ToolResult::error(format!("cannot delete {}: {e}", full.display()));
            }
        };
        let kind = if meta.is_dir() { "directory" } else { "file" };
        if !meta.is_dir() {
            if let Ok(content) = tokio::fs::read_to_string(&full).await {
                ctx.emit_snapshot(full.to_string_lossy(), &content);
            }
        }
        let res = if meta.is_dir() {
            tokio::fs::remove_dir_all(&full).await
        } else {
            tokio::fs::remove_file(&full).await
        };
        match res {
            Ok(()) => ToolResult::ok(format!("deleted {kind} {}", full.display())),
            Err(e) => ToolResult::error(format!("cannot delete {}: {e}", full.display())),
        }
    }
}

/// Block deletes that would wipe the workspace root or a parent of it.
pub(super) fn refuse_dangerous_delete(cwd: &Path, target: &Path) -> Option<String> {
    let cwd = match cwd.canonicalize() {
        Ok(p) => p,
        Err(_) => return Some("cannot verify working directory — delete refused".into()),
    };
    let target = match target.canonicalize() {
        Ok(p) => p,
        Err(_) => return Some("cannot verify target path — delete refused".into()),
    };
    if target == cwd {
        return Some("refusing to delete the working directory".into());
    }
    if cwd.starts_with(&target) {
        return Some("refusing to delete a parent of the working directory".into());
    }
    None
}

inventory::submit! { ToolRegistration { make: || Arc::new(DeletePath) as Arc<dyn Tool> } }
