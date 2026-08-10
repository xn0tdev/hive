//! Git worktree helpers for MULTITASK isolation.

mod git;
mod lifecycle;
mod staging;
mod summary;

pub use git::{branch_name, is_git_repo, worktree_path};
pub use lifecycle::{create, integrate, remove};
pub use summary::summarize;

#[cfg(test)]
mod tests {
    use super::staging::looks_sensitive;
    use super::summary::{cap_patch, DIFF_CAP};
    use super::*;
    use std::path::{Path, PathBuf};
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
