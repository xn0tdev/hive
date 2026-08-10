//! Compact worktree status and diff summaries.

use std::path::Path;
use std::process::Command;

/// Characters of patch text handed to the orchestrator.
pub(super) const DIFF_CAP: usize = 6_000;

/// Cap a patch, counting characters rather than bytes. Diffs carry whatever the
/// code does — Cyrillic strings, box drawing, emoji — and a byte-index cut
/// lands inside a character and panics, taking the whole turn with it.
pub(super) fn cap_patch(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((end, _)) => format!("{}…\n(diff truncated)", &s[..end]),
        None => s.to_string(),
    }
}

/// Short status + diff from a worktree (for the orchestrator).
pub fn summarize(worktree: &Path) -> String {
    let mut parts = Vec::new();
    if let Ok(o) = Command::new("git")
        .args(["status", "--short"])
        .current_dir(worktree)
        .output()
    {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !s.is_empty() {
            parts.push(format!("### git status\n{s}"));
        }
    }
    if let Ok(o) = Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(worktree)
        .output()
    {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !s.is_empty() {
            parts.push(format!("### git diff --stat\n{s}"));
        }
    }
    // Include unstaged + staged summary vs merge-base with main checkout's HEAD
    // by showing full patch capped.
    if let Ok(o) = Command::new("git")
        .args(["diff", "HEAD"])
        .current_dir(worktree)
        .output()
    {
        let s = String::from_utf8_lossy(&o.stdout);
        if !s.trim().is_empty() {
            parts.push(format!("### git diff\n{}", cap_patch(&s, DIFF_CAP)));
        }
    }
    if parts.is_empty() {
        "(no local changes in worktree)".into()
    } else {
        parts.join("\n\n")
    }
}
