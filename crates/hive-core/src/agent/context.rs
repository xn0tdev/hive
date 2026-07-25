//! Project instruction files (AGENTS.md, CLAUDE.md, …) loaded into the system prompt.

use std::fs;
use std::path::Path;

/// Soft cap per file (characters). Larger files are truncated with a note.
pub const MAX_CHARS_PER_FILE: usize = 16_000;
/// Soft cap across all instruction files combined.
pub const MAX_CHARS_TOTAL: usize = 48_000;

/// One project instruction file that exists under the working directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextFile {
    /// Canonical display name (e.g. `AGENTS.md`).
    pub name: String,
    /// Path relative to `cwd`.
    pub rel_path: String,
}

/// Candidate names for each logical instruction file (first existing wins).
const GROUPS: &[(&str, &[&str])] = &[
    ("AGENTS.md", &["AGENTS.md", "agents.md"]),
    ("CLAUDE.md", &["CLAUDE.md", "claude.md"]),
    (
        "HIVE.md",
        &["HIVE.md", "hive.md", ".hive/HIVE.md", ".hive/hive.md"],
    ),
];

/// List instruction files present under `cwd` (no content load).
pub fn discover_context_files(cwd: &Path) -> Vec<ContextFile> {
    let mut out = Vec::new();
    for (name, candidates) in GROUPS {
        if let Some(rel) = first_existing(cwd, candidates) {
            out.push(ContextFile {
                name: (*name).to_string(),
                rel_path: rel,
            });
        }
    }
    out
}

/// Load present instruction files and format them for the system prompt.
///
/// Returns empty string when none exist. Truncates oversized content and notes it.
pub fn format_project_instructions(cwd: &Path) -> String {
    let files = discover_context_files(cwd);
    if files.is_empty() {
        return String::new();
    }

    let mut p = String::new();
    p.push_str("## Project instructions\n");
    p.push_str(
        "The following project files are authoritative guidance for this repository. \
Follow them. When they conflict with general defaults, prefer these files \
(unless the user explicitly overrides them for this turn).\n\n",
    );

    let mut remaining = MAX_CHARS_TOTAL;
    for file in &files {
        if remaining == 0 {
            p.push_str(&format!(
                "### {}\n(skipped: remaining project-instruction budget exhausted)\n\n",
                file.name
            ));
            continue;
        }
        let abs = cwd.join(&file.rel_path);
        let Ok(raw) = fs::read_to_string(&abs) else {
            continue;
        };
        let budget = remaining.min(MAX_CHARS_PER_FILE);
        let (body, truncated) = truncate_chars(&raw, budget);
        remaining = remaining.saturating_sub(body.chars().count());

        p.push_str(&format!("### {}\n", file.name));
        if file.rel_path != file.name {
            p.push_str(&format!("(from `{}`)\n", file.rel_path));
        }
        p.push_str(body.trim_end());
        p.push('\n');
        if truncated {
            p.push_str("\n…(truncated; open the file for the full text)\n");
        }
        p.push('\n');
    }
    p
}

fn first_existing(cwd: &Path, candidates: &[&str]) -> Option<String> {
    for rel in candidates {
        let path = cwd.join(rel);
        if path.is_file() {
            return Some((*rel).to_string());
        }
    }
    None
}

fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    let n = s.chars().count();
    if n <= max {
        return (s.to_string(), false);
    }
    (s.chars().take(max).collect(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("hive-ctx-{n}"));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn discovers_agents_and_claude() {
        let dir = tmp_dir();
        fs::write(dir.join("AGENTS.md"), "# agents\nrule a").unwrap();
        fs::write(dir.join("CLAUDE.md"), "# claude\nrule b").unwrap();
        let found = discover_context_files(&dir);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "AGENTS.md");
        assert_eq!(found[1].name, "CLAUDE.md");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn prefers_canonical_over_lowercase() {
        let dir = tmp_dir();
        fs::write(dir.join("AGENTS.md"), "canonical").unwrap();
        fs::write(dir.join("agents.md"), "lower").unwrap();
        let found = discover_context_files(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rel_path, "AGENTS.md");
        let text = format_project_instructions(&dir);
        assert!(text.contains("canonical"));
        assert!(!text.contains("lower"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncates_with_note() {
        let dir = tmp_dir();
        let big = "x".repeat(MAX_CHARS_PER_FILE + 50);
        fs::write(dir.join("AGENTS.md"), &big).unwrap();
        let text = format_project_instructions(&dir);
        assert!(text.contains("truncated"));
        assert!(text.contains("### AGENTS.md"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovers_hive_md_under_dot_hive() {
        let dir = tmp_dir();
        fs::create_dir_all(dir.join(".hive")).unwrap();
        fs::write(dir.join(".hive/HIVE.md"), "hive rules").unwrap();
        let found = discover_context_files(&dir);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "HIVE.md");
        assert_eq!(found[0].rel_path, ".hive/HIVE.md");
        let _ = fs::remove_dir_all(&dir);
    }
}
