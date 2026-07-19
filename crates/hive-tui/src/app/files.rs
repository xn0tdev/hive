//! `@file` mentions in the composer — fuzzy path picker over the project tree.

use std::path::Path;

/// Max files kept in the in-memory index.
const MAX_INDEX: usize = 4_000;
/// Rows shown in the floating menu (same feel as slash).
pub const MAX_MENU_ROWS: usize = 8;

/// Active `@query` at the cursor, if the file menu should open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtQuery {
    /// Char index of `@` in the input.
    pub at: usize,
    /// Text after `@` up to the cursor (no whitespace).
    pub query: String,
}

/// Parse `@mention` immediately before `cursor` (char index).
///
/// Opens when `@` starts a token (start of input or after whitespace) and the
/// query has no spaces yet — same idea as the `/command` menu.
pub fn at_query(value: &str, cursor: usize) -> Option<AtQuery> {
    let chars: Vec<char> = value.chars().collect();
    if cursor > chars.len() {
        return None;
    }
    let before = &chars[..cursor];
    let at = before.iter().rposition(|&c| c == '@')?;
    if at > 0 && !before[at - 1].is_whitespace() {
        return None;
    }
    let query: String = before[at + 1..].iter().collect();
    if query.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    Some(AtQuery { at, query })
}

/// Walk `cwd` (respecting `.gitignore`) and collect relative file paths.
pub fn index_files(cwd: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(cwd)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_depth(Some(12))
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if should_skip(path) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(cwd) else {
            continue;
        };
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.is_empty() {
            continue;
        }
        out.push(s);
        if out.len() >= MAX_INDEX {
            break;
        }
    }
    out.sort();
    out
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        matches!(
            s.as_ref(),
            "target" | "node_modules" | ".git" | "dist" | "build" | ".next" | "venv" | ".venv"
        )
    })
}

/// Filter indexed paths by a case-insensitive substring / path prefix query.
pub fn filter_files<'a>(index: &'a [String], query: &str) -> Vec<&'a str> {
    let q = query.trim();
    if q.is_empty() {
        return index.iter().map(|s| s.as_str()).take(200).collect();
    }
    let q_lower = q.to_lowercase();
    let mut scored: Vec<(i32, usize, &str)> = Vec::new();
    for path in index {
        let lower = path.to_lowercase();
        if !lower.contains(&q_lower) {
            continue;
        }
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        let name_l = name.to_lowercase();
        let score = if name_l == q_lower {
            0
        } else if name_l.starts_with(&q_lower) {
            1
        } else if lower.starts_with(&q_lower) {
            2
        } else if name_l.contains(&q_lower) {
            3
        } else {
            4
        };
        // Prefer shorter paths when scores tie (closer to project root).
        scored.push((score, path.len(), path.as_str()));
    }
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(b.2))
    });
    scored.into_iter().map(|(_, _, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn at_query_basic() {
        assert_eq!(
            at_query("@foo", 4),
            Some(AtQuery {
                at: 0,
                query: "foo".into()
            })
        );
        assert_eq!(
            at_query("see @src/a", 10),
            Some(AtQuery {
                at: 4,
                query: "src/a".into()
            })
        );
        assert_eq!(at_query("email@x", 7), None);
        assert_eq!(at_query("@a b", 4), None);
        assert_eq!(
            at_query("hi @", 4),
            Some(AtQuery {
                at: 3,
                query: String::new()
            })
        );
    }

    #[test]
    fn filter_prefers_basename_prefix() {
        let idx = vec![
            "crates/hive-tui/src/run.rs".into(),
            "README.md".into(),
            "src/run.rs".into(),
        ];
        let hits = filter_files(&idx, "run");
        assert_eq!(hits[0], "src/run.rs");
    }

    #[test]
    fn index_finds_file() {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hive-at-{n}"));
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        let files = index_files(&dir);
        assert!(files.iter().any(|f| f == "src/main.rs"), "{files:?}");
        let _ = fs::remove_dir_all(&dir);
    }
}
