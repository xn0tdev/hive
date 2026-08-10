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
    use super::{delete::refuse_dangerous_delete, read::image_media_type, write::compact_diff};
    use std::fs;
    use std::path::Path;

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
}
