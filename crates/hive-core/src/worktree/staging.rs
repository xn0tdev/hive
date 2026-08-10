//! Safe staging and commit policy for worker changes.

use std::path::Path;
use std::process::Command;

/// Untracked files with common credential/config names are never swept into a
/// worker commit automatically. They are removed with the disposable
/// worktree, and the integration result names them so the user can recover
/// them from the worker branch if they really intended to keep them.
pub(super) fn looks_sensitive(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    name == ".npmrc"
        || name == ".pypirc"
        || name == "credentials"
        || name == "credentials.json"
        || name == "secret"
        || name == "secrets"
        || name.starts_with(".env")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
}

fn stage_worker_changes(worktree: &Path) -> Result<Vec<String>, String> {
    // The worker may have staged an arbitrary subset itself. Rebuild the index
    // from the branch HEAD so the sensitive-path filter below sees every
    // tracked change and can never leave a previously staged secret behind.
    let reset = Command::new("git")
        .args(["reset", "--quiet"])
        .current_dir(worktree)
        .output()
        .map_err(|e| format!("git reset worker index: {e}"))?;
    if !reset.status.success() {
        return Err(format!(
            "git reset worker index failed: {}",
            String::from_utf8_lossy(&reset.stderr).trim()
        ));
    }

    let tracked = Command::new("git")
        .args(["diff", "--name-only", "-z", "HEAD"])
        .current_dir(worktree)
        .output()
        .map_err(|e| format!("git list tracked changes: {e}"))?;
    if !tracked.status.success() {
        return Err(format!(
            "git list tracked changes failed: {}",
            String::from_utf8_lossy(&tracked.stderr).trim()
        ));
    }

    let mut safe = Vec::new();
    let mut skipped = Vec::new();
    for raw in tracked.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = String::from_utf8_lossy(raw).into_owned();
        if looks_sensitive(&path) {
            skipped.push(path);
        } else {
            safe.push(path);
        }
    }

    let untracked = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .current_dir(worktree)
        .output()
        .map_err(|e| format!("git list untracked files: {e}"))?;
    if !untracked.status.success() {
        return Err(format!(
            "git list untracked files failed: {}",
            String::from_utf8_lossy(&untracked.stderr).trim()
        ));
    }

    for raw in untracked.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = String::from_utf8_lossy(raw).into_owned();
        if looks_sensitive(&path) {
            skipped.push(path);
        } else {
            safe.push(path);
        }
    }

    if !safe.is_empty() {
        let mut add = Command::new("git");
        add.args(["add", "--"]);
        add.args(&safe);
        let added = add
            .current_dir(worktree)
            .output()
            .map_err(|e| format!("git add new files: {e}"))?;
        if !added.status.success() {
            return Err(format!(
                "git add new files failed: {}",
                String::from_utf8_lossy(&added.stderr).trim()
            ));
        }
    }
    Ok(skipped)
}

pub(super) fn commit_worker_changes(worktree: &Path, id: &str) -> Result<Vec<String>, String> {
    let skipped = stage_worker_changes(worktree)?;

    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(worktree)
        .status()
        .map_err(|e| format!("git inspect staged changes: {e}"))?;
    if staged.success() {
        return Ok(skipped);
    }

    let commit = Command::new("git")
        .args(["commit", "-m", &format!("hive multitask: {id}")])
        .current_dir(worktree)
        .output()
        .map_err(|e| format!("git commit worker changes: {e}"))?;
    if !commit.status.success() {
        return Err(format!(
            "git commit worker changes failed: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        ));
    }
    Ok(skipped)
}
