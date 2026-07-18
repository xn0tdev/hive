//! Filesystem tools: read, write, edit, list, glob, grep. Each self-registers.

use async_trait::async_trait;
use globset::GlobBuilder;
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::{bool_arg, resolve, str_arg, u64_arg};

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file. Optionally start at a 1-based line offset and limit the number of lines returned."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path, absolute or relative to the working directory."},
                "offset": {"type": "integer", "description": "1-based line to start reading from."},
                "limit": {"type": "integer", "description": "Maximum number of lines to return."}
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(path) = str_arg(&args, "path") else {
            return ToolResult::error("missing 'path'");
        };
        let full = resolve(&ctx.cwd, path);
        let content = match tokio::fs::read_to_string(&full).await {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("cannot read {}: {e}", full.display())),
        };

        let offset = u64_arg(&args, "offset");
        let limit = u64_arg(&args, "limit");
        let out = if offset.is_some() || limit.is_some() {
            let start = offset.unwrap_or(1).saturating_sub(1) as usize;
            let lines: Vec<&str> = content.lines().collect();
            let end = match limit {
                Some(l) => (start + l as usize).min(lines.len()),
                None => lines.len(),
            };
            lines.get(start..end).unwrap_or(&[]).join("\n")
        } else {
            content
        };

        const CAP: usize = 100_000;
        if out.len() > CAP {
            let mut s: String = out.chars().take(CAP).collect();
            s.push_str("\n… [truncated]");
            ToolResult::ok(s)
        } else {
            ToolResult::ok(out)
        }
    }
}

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
                "path": {"type": "string", "description": "File path, absolute or relative to the working directory."},
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
        let full = resolve(&ctx.cwd, path);

        if let Some(parent) = full.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::error(format!("cannot create {}: {e}", parent.display()));
            }
        }

        let old = tokio::fs::read_to_string(&full).await.unwrap_or_default();

        match tokio::fs::write(&full, content).await {
            Ok(_) => {
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
                "path": {"type": "string", "description": "File path, absolute or relative to the working directory."},
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
        let full = resolve(&ctx.cwd, path);

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
fn compact_diff(old: &str, new: &str) -> String {
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

    let removed = &o[pre..o.len() - suf];
    let added = &n[pre..n.len() - suf];

    const CAP: usize = 14;
    let mut out = String::new();
    let mut push = |sign: char, lines: &[&str]| {
        for (i, l) in lines.iter().enumerate() {
            if i == CAP {
                out.push_str(&format!("… {} more\n", lines.len() - CAP));
                break;
            }
            out.push(sign);
            out.push(' ');
            out.push_str(l);
            out.push('\n');
        }
    };
    push('-', removed);
    push('+', added);
    out
}

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
        let full = resolve(&ctx.cwd, path);

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
        let base = resolve(&ctx.cwd, str_arg(&args, "path").unwrap_or("."));
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
        let base = resolve(&ctx.cwd, str_arg(&args, "path").unwrap_or("."));
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

#[cfg(test)]
mod tests {
    use super::compact_diff;

    #[test]
    fn marks_changed_middle_only() {
        assert_eq!(compact_diff("a\nb\nc\n", "a\nB\nc\n"), "- b\n+ B\n");
    }

    #[test]
    fn new_content_is_all_additions() {
        let d = compact_diff("", "x\ny\n");
        assert!(d.contains("+ x"));
        assert!(d.contains("+ y"));
        assert!(!d.contains("- "));
    }

    #[test]
    fn caps_long_side() {
        let new: String = (0..40).map(|i| format!("l{i}\n")).collect();
        let d = compact_diff("", &new);
        assert!(d.contains("more"));
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(ReadFile) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(WriteFile) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(EditFile) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(ListDir) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(Glob) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(Grep) as Arc<dyn Tool> } }
