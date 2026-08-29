//! Filesystem tools grouped by concern: read, write, delete, and search.

#[path = "delete.rs"]
mod delete;
#[path = "read.rs"]
mod read;
#[path = "search.rs"]
mod search;
#[path = "write.rs"]
mod write;

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::{
        delete::refuse_dangerous_delete, read::image_media_type, write::EditFile,
        write::compact_diff,
    };
    use crate::tool::{Tool, ToolContext};
    use crate::{config::AppConfig, skill::no_skills, spawner::noop_spawner};
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    #[test]
    fn detects_image_extensions() {
        assert_eq!(
            image_media_type(Path::new("a/b/pic.png")),
            Some("image/png")
        );
        assert_eq!(image_media_type(Path::new("shot.JPG")), Some("image/jpeg"));
        assert_eq!(image_media_type(Path::new("anim.gif")), Some("image/gif"));
        assert_eq!(image_media_type(Path::new("notes.txt")), None);
        assert_eq!(image_media_type(Path::new("README")), None);
    }

    #[test]
    fn marks_changed_middle_only() {
        assert_eq!(compact_diff("a\nb\nc\n", "a\nB\nc\n"), "-2\tb\n+2\tB\n");
    }

    #[test]
    fn new_content_is_all_additions() {
        let d = compact_diff("", "x\ny\n");
        assert_eq!(d, "+1\tx\n+2\ty\n");
        assert!(!d.contains('-'));
    }

    #[test]
    fn line_numbers_start_at_the_first_change() {
        // Three unchanged lines, then an edit on line 4.
        let d = compact_diff("a\nb\nc\nd\ne\n", "a\nb\nc\nD\ne\n");
        assert_eq!(d, "-4\td\n+4\tD\n");
    }

    #[test]
    fn a_tab_indented_line_keeps_its_indent() {
        let d = compact_diff("x\n", "\t\tdeep\n");
        assert_eq!(d, "-1\tx\n+1\t\t\tdeep\n");
    }

    #[test]
    fn caps_long_side() {
        let new: String = (0..40).map(|i| format!("l{i}\n")).collect();
        let d = compact_diff("", &new);
        assert!(d.contains("more"));
    }

    #[test]
    fn refuses_deleting_cwd() {
        let dir = std::env::temp_dir().join(format!(
            "hive-del-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let msg = refuse_dangerous_delete(&dir, &dir).expect("refuse cwd");
        assert!(msg.contains("working directory"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn allows_deleting_child() {
        let dir = std::env::temp_dir().join(format!(
            "hive-del-ok-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let child = dir.join("web");
        fs::create_dir_all(&child).unwrap();
        assert!(refuse_dangerous_delete(&dir, &child).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_file_rejects_empty_old_string() {
        let dir = std::env::temp_dir().join(format!(
            "hive-edit-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("a.txt"), "hello").await.unwrap();

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ToolContext {
            cwd: dir.clone(),
            events: tx,
            spawner: noop_spawner(),
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
            terminal: None,
            vision: false,
            depth: 0,
            call_id: "call-1".to_string(),
            isolate_worktrees: false,
            interrupt: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let result = EditFile
            .execute(
                serde_json::json!({"path": "a.txt", "old_string": "", "new_string": "x"}),
                &ctx,
            )
            .await;

        assert!(result.is_error);
        assert!(
            result.content.contains("must not be empty"),
            "{}",
            result.content
        );
        // The file must be untouched.
        assert_eq!(tokio::fs::read_to_string(dir.join("a.txt")).await.unwrap(), "hello");
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
