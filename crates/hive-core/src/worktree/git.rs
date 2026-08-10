//! Git naming and repository primitives for worker worktrees.

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
