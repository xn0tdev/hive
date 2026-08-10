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

/// Characters of patch text handed to the orchestrator.
const DIFF_CAP: usize = 6_000;

/// Untracked files with common credential/config names are never swept into a
/// worker commit automatically. They are removed with the disposable
/// worktree, and the integration result names them so the user can recover
/// them from the worker branch if they really intended to keep them.
fn looks_sensitive(path: &str) -> bool {
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

fn commit_worker_changes(worktree: &Path, id: &str) -> Result<Vec<String>, String> {
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

/// Cap a patch, counting characters rather than bytes. Diffs carry whatever the
/// code does — Cyrillic strings, box drawing, emoji — and a byte-index cut
/// lands inside a character and panics, taking the whole turn with it.
fn cap_patch(s: &str, max: usize) -> String {
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

    fn write_commit(dir: &Path, name: &str, body: &str, msg: &str) {
        std::fs::write(dir.join(name), body).unwrap();
        for args in [vec!["add", "-A"], vec!["commit", "-m", msg]] {
            assert!(Command::new("git")
                .args(&args)
                .current_dir(dir)
                .status()
                .unwrap()
                .success());
        }
    }

    #[test]
    fn capping_a_patch_never_splits_a_character() {
        // One ASCII byte then two-byte characters, so the byte at the cap is
        // guaranteed to sit inside a character.
        let text = format!("a{}", "я".repeat(DIFF_CAP));
        let capped = cap_patch(&text, DIFF_CAP);

        assert!(capped.ends_with("(diff truncated)"), "{capped}");
        assert_eq!(
            capped.chars().take_while(|c| *c != '…').count(),
            DIFF_CAP,
            "keeps exactly the cap in characters"
        );

        // Shorter than the cap: handed through untouched.
        assert_eq!(cap_patch("привет", DIFF_CAP), "привет");
    }

    #[test]
    fn sensitive_untracked_names_are_not_auto_staged() {
        for path in [
            ".env",
            ".env.local",
            "config/api.key",
            "id_rsa",
            "src/main.rs",
        ] {
            assert_eq!(looks_sensitive(path), path != "src/main.rs", "{path}");
        }
    }

    #[test]
    fn a_non_ascii_diff_is_summarised_without_panicking() {
        let repo = temp_repo();
        let (path, _) = create(&repo, "cyr1").unwrap();
        // Well past the 6 KB cap, and multi-byte throughout, so a byte-index
        // cut lands inside a character.
        std::fs::write(
            path.join("notes.txt"),
            "Кэш сброшен и RAM очищена. ".repeat(600),
        )
        .unwrap();
        assert!(Command::new("git")
            .args(["add", "-A"])
            .current_dir(&path)
            .status()
            .unwrap()
            .success());

        let summary = summarize(&path);
        assert!(summary.contains("(diff truncated)"), "{summary}");
        assert!(summary.contains("Кэш"), "{summary}");

        let _ = remove(&repo, "cyr1");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn a_conflicting_merge_keeps_the_work_and_leaves_a_clean_tree() {
        let repo = temp_repo();
        write_commit(&repo, "shared.txt", "base\n", "base");
        let (path, branch) = create(&repo, "conf1").unwrap();

        // Both sides touch the same line.
        write_commit(&path, "shared.txt", "from the worker\n", "worker");
        write_commit(&repo, "shared.txt", "from main\n", "main");

        let err = integrate(&repo, "conf1").expect_err("should conflict");
        assert!(err.contains(&branch), "names the branch to recover: {err}");

        // The branch still exists — the work isn't thrown away.
        let branches = Command::new("git")
            .args(["branch", "--list", &branch])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&branches.stdout).contains(&branch),
            "the worker's branch must survive a conflict"
        );

        // And the main checkout isn't left mid-merge.
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&status.stdout);
        assert!(
            !status.contains("UU") && !status.contains("AA"),
            "merge left conflicts behind: {status}"
        );

        let _ = remove(&repo, "conf1");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn integration_stages_source_but_skips_sensitive_untracked_files() {
        let repo = temp_repo();
        let (path, _) = create(&repo, "safe1").unwrap();
        std::fs::create_dir_all(path.join("src")).unwrap();
        std::fs::write(path.join("src/new.rs"), "pub fn new_file() {}\n").unwrap();
        std::fs::write(path.join(".env"), "TOKEN=do-not-merge\n").unwrap();

        let result = integrate(&repo, "safe1").expect("integration should succeed");
        assert!(
            result.contains("Skipped sensitive untracked paths"),
            "{result}"
        );
        assert!(repo.join("src/new.rs").is_file());
        assert!(!repo.join(".env").exists());

        let _ = std::fs::remove_dir_all(&repo);
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
