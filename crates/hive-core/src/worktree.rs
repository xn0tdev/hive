//! Git worktree helpers for MULTITASK isolation.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Branch name for a Multitask worker: `hive/<id>`.
pub fn branch_name(id: &str) -> String {
    format!("hive/{id}")
}

/// Path under the main repo: `.hive/worktrees/<id>`.
pub fn worktree_path(repo: &Path, id: &str) -> PathBuf {
    repo.join(".hive").join("worktrees").join(id)
}

/// True when `repo` is inside a git working tree.
pub fn is_git_repo(repo: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create an isolated worktree on a new branch from HEAD.
pub fn create(repo: &Path, id: &str) -> Result<(PathBuf, String), String> {
    if !is_git_repo(repo) {
        return Err(
            "MULTITASK needs a git repository — initialize git or switch to MAKE".into(),
        );
    }
    let path = worktree_path(repo, id);
    let branch = branch_name(id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir worktrees: {e}"))?;
    }
    if path.exists() {
        let _ = remove(repo, id);
    }
    let out = Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            &branch,
            path.to_str().unwrap_or(""),
            "HEAD",
        ])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git worktree add: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok((path, branch))
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
            let capped = if s.len() > 6_000 {
                format!("{}…\n(diff truncated)", &s[..6_000])
            } else {
                s.into_owned()
            };
            parts.push(format!("### git diff\n{capped}"));
        }
    }
    if parts.is_empty() {
        "(no local changes in worktree)".into()
    } else {
        parts.join("\n\n")
    }
}

/// Merge `branch` into the main repo checkout, then remove the worktree.
pub fn integrate(repo: &Path, id: &str) -> Result<String, String> {
    if !is_git_repo(repo) {
        return Err("not a git repository".into());
    }
    let branch = branch_name(id);
    let path = worktree_path(repo, id);

    // Commit any leftover changes in the worktree so merge has commits.
    if path.is_dir() {
        let _ = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&path)
            .output();
        let _ = Command::new("git")
            .args([
                "commit",
                "-m",
                &format!("hive multitask: {id}"),
                "--allow-empty-message",
            ])
            .current_dir(&path)
            .output();
    }

    let merge = Command::new("git")
        .args(["merge", "--no-edit", &branch])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git merge: {e}"))?;
    let stdout = String::from_utf8_lossy(&merge.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&merge.stderr).trim().to_string();
    if !merge.status.success() {
        let _ = remove(repo, id);
        return Err(format!(
            "merge conflict or failure for `{branch}`:\n{stdout}\n{stderr}\n\
Resolve conflicts in the main checkout, then retry or abort the merge."
        ));
    }

    let _ = remove(repo, id);
    Ok(format!(
        "Integrated `{branch}` into the main checkout.\n{stdout}"
    ))
}

/// Remove worktree directory and prune; delete local branch if possible.
pub fn remove(repo: &Path, id: &str) -> Result<(), String> {
    let path = worktree_path(repo, id);
    let branch = branch_name(id);
    if path.exists() {
        let out = Command::new("git")
            .args([
                "worktree",
                "remove",
                "--force",
                path.to_str().unwrap_or(""),
            ])
            .current_dir(repo)
            .output()
            .map_err(|e| format!("git worktree remove: {e}"))?;
        if !out.status.success() {
            // Fall back to manual delete + prune.
            let _ = std::fs::remove_dir_all(&path);
            let _ = Command::new("git")
                .args(["worktree", "prune"])
                .current_dir(repo)
                .output();
        }
    }
    let _ = Command::new("git")
        .args(["branch", "-D", &branch])
        .current_dir(repo)
        .output();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_repo() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hive-wt-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["config", "user.email", "hive@test"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["config", "user.name", "hive"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success());
        std::fs::write(dir.join("README"), "hi").unwrap();
        assert!(Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success());
        dir
    }

    #[test]
    fn create_worktree_has_distinct_cwd() {
        let repo = temp_repo();
        let (path, branch) = create(&repo, "abc123").unwrap();
        assert!(path.is_dir());
        assert_ne!(path, repo);
        assert_eq!(branch, "hive/abc123");
        assert!(path.join("README").is_file());
        let _ = remove(&repo, "abc123");
        let _ = std::fs::remove_dir_all(&repo);
    }
}
