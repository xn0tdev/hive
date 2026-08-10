//! Create, integrate, and remove worker worktrees.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::git::{branch_name, is_git_repo, worktree_path};
use super::staging::commit_worker_changes;

/// Create an isolated worktree on a new branch from HEAD.
pub fn create(repo: &Path, id: &str) -> Result<(PathBuf, String), String> {
    if !is_git_repo(repo) {
        return Err("MULTITASK needs a git repository — initialize git or switch to MAKE".into());
    }
    let path = worktree_path(repo, id);
    let branch = branch_name(id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir worktrees: {e}"))?;
        // Worker checkouts live inside the repo, so without this the user's
        // next `git add -A` sweeps them in as embedded repositories.
        let _ = std::fs::write(parent.join(".gitignore"), "*\n");
    }
    if path.exists() {
        let _ = remove(repo, id);
    }
    let out = Command::new("git")
        .args(["worktree", "add", "-b", &branch])
        .arg(&path)
        .arg("HEAD")
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

/// Merge `branch` into the main repo checkout, then remove the worktree.
pub fn integrate(repo: &Path, id: &str) -> Result<String, String> {
    if !is_git_repo(repo) {
        return Err("not a git repository".into());
    }
    let branch = branch_name(id);
    let path = worktree_path(repo, id);

    // Commit leftover worker changes, but do not blindly sweep credentials or
    // ignored/untracked configuration into the branch. Tracked changes and
    // ordinary new source files are included; sensitive untracked paths are
    // reported and left out of the merge.
    let skipped = if path.is_dir() {
        commit_worker_changes(&path, id)?
    } else {
        Vec::new()
    };

    let merge = Command::new("git")
        .args(["merge", "--no-edit", &branch])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git merge: {e}"))?;
    let stdout = String::from_utf8_lossy(&merge.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&merge.stderr).trim().to_string();
    if !merge.status.success() {
        // Put the main checkout back the way it was — a half-merged tree is
        // worse than no merge, and MULTITASK has no shell to dig out of it.
        // The branch and worktree stay: they hold the only copy of the work,
        // and "retry after resolving" is meaningless once they're deleted.
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(repo)
            .output();
        return Err(format!(
            "merge conflict or failure for `{branch}`:\n{stdout}\n{stderr}\n\
The merge was rolled back and the main checkout is clean. The work is kept on \
branch `{branch}` (worker id `{id}`) — resolve it there or in MAKE mode, then \
retry `integrate_worktree`."
        ));
    }

    let _ = remove(repo, id);
    let warning = if skipped.is_empty() {
        String::new()
    } else {
        format!(
            "\nSkipped sensitive untracked paths: {}. They were not merged.",
            skipped.join(", ")
        )
    };
    Ok(format!(
        "Integrated `{branch}` into the main checkout.\n{stdout}{warning}"
    ))
}

/// Remove worktree directory and prune; delete local branch if possible.
pub fn remove(repo: &Path, id: &str) -> Result<(), String> {
    let path = worktree_path(repo, id);
    let branch = branch_name(id);
    if path.exists() {
        let out = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&path)
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
