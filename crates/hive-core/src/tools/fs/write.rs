//! Write and edit file tool implementations.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::bool_arg;
use super::super::resolve_workspace;
use super::super::str_arg;
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

pub struct WriteFile;

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file with the given content. Parent directories are created automatically."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the current workspace. Absolute paths are rejected unless workspace_only=false."},
                "content": {"type": "string", "description": "Full file contents to write."}
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(path) = str_arg(&args, "path") else {
            return ToolResult::error("missing 'path'");
        };
        let Some(content) = str_arg(&args, "content") else {
            return ToolResult::error("missing 'content'");
        };
        let full = match resolve_workspace(ctx, path) {
            Ok(path) => path,
            Err(error) => return ToolResult::error(error),
        };

        if let Some(parent) = full.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::error(format!("cannot create {}: {e}", parent.display()));
            }
        }

        let old = tokio::fs::read_to_string(&full).await.unwrap_or_default();

        match tokio::fs::write(&full, content).await {
            Ok(_) => {
                ctx.emit_snapshot(full.to_string_lossy(), &old);
                ctx.emit_output(compact_diff(&old, content));
                ToolResult::ok(format!(
                    "wrote {} bytes to {}",
                    content.len(),
                    full.display()
                ))
            }
            Err(e) => ToolResult::error(format!("cannot write {}: {e}", full.display())),
        }
    }
}

pub struct EditFile;

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Replace an exact substring in a file. By default the old string must be unique; set replace_all to replace every occurrence."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the current workspace. Absolute paths are rejected unless workspace_only=false."},
                "old_string": {"type": "string", "description": "Exact text to find (include enough context to be unique)."},
                "new_string": {"type": "string", "description": "Replacement text."},
                "replace_all": {"type": "boolean", "description": "Replace all occurrences instead of requiring uniqueness."}
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(path) = str_arg(&args, "path") else {
            return ToolResult::error("missing 'path'");
        };
        let Some(old) = str_arg(&args, "old_string") else {
            return ToolResult::error("missing 'old_string'");
        };
        let Some(new) = str_arg(&args, "new_string") else {
            return ToolResult::error("missing 'new_string'");
        };
        let replace_all = bool_arg(&args, "replace_all");
        let full = match resolve_workspace(ctx, path) {
            Ok(path) => path,
            Err(error) => return ToolResult::error(error),
        };

        let content = match tokio::fs::read_to_string(&full).await {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("cannot read {}: {e}", full.display())),
        };

        let count = content.matches(old).count();
        if count == 0 {
            return ToolResult::error(format!("old_string not found in {}", full.display()));
        }
        if count > 1 && !replace_all {
            return ToolResult::error(format!(
                "old_string is not unique in {} ({count} matches); pass replace_all or add more context",
                full.display()
            ));
        }

        let updated = if replace_all {
            content.replace(old, new)
        } else {
            content.replacen(old, new, 1)
        };

        match tokio::fs::write(&full, &updated).await {
            Ok(_) => {
                ctx.emit_snapshot(full.to_string_lossy(), &content);
                ctx.emit_output(compact_diff(old, new));
                let n = if replace_all { count } else { 1 };
                ToolResult::ok(format!(
                    "edited {} ({n} replacement{})",
                    full.display(),
                    if n == 1 { "" } else { "s" }
                ))
            }
            Err(e) => ToolResult::error(format!("cannot write {}: {e}", full.display())),
        }
    }
}

/// A compact unified-ish diff of `old` → `new`: common leading/trailing lines
/// are dropped, the changed middle is shown as `-`/`+` rows (capped). Used only
/// for the TUI display of edits, not for the model's tool result.
pub(super) fn compact_diff(old: &str, new: &str) -> String {
    let o: Vec<&str> = old.split('\n').collect();
    let n: Vec<&str> = new.split('\n').collect();

    let mut pre = 0;
    while pre < o.len() && pre < n.len() && o[pre] == n[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < o.len() - pre && suf < n.len() - pre && o[o.len() - 1 - suf] == n[n.len() - 1 - suf]
    {
        suf += 1;
    }

    let removed = &o[pre..o.len().saturating_sub(suf).max(pre)];
    let added = &n[pre..n.len().saturating_sub(suf).max(pre)];

    const CAP: usize = 14;
    let mut out = String::new();
    // `{sign}{line}\t{text}`: the line number is what tells you *where* an edit
    // landed, and `pre` already knows it. The tab keeps a source line's own
    // leading whitespace intact.
    let mut push = |sign: char, lines: &[&str]| {
        for (i, l) in lines.iter().enumerate() {
            if i == CAP {
                out.push_str(&format!("… {} more\n", lines.len() - CAP));
                break;
            }
            out.push_str(&format!("{sign}{}\t{l}\n", pre + i + 1));
        }
    };
    push('-', removed);
    push('+', added);
    out
}

inventory::submit! { ToolRegistration { make: || Arc::new(WriteFile) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(EditFile) as Arc<dyn Tool> } }
