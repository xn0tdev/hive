//! Session persistence: save/load conversations as JSON files.
//!
//! Sessions are stored in `~/.config/hive/sessions/` (or `.hive/sessions/` as
//! fallback). Each session is one `.json` file containing the message history,
//! usage, the model *and the connection it belongs to*, and a heuristic title
//! derived from the first user message.
//!
//! Every function has an `_in` variant that takes the directory explicitly —
//! the plain ones just delegate to [`sessions_dir`].

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::message::{Message, Role};
use crate::provider::Usage;

/// A serializable snapshot of a session — everything needed to resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: String,
    pub title: String,
    pub model: String,
    /// Connection profile the session ran on. A model id only means something
    /// on the provider it came from, so resume has to restore this first.
    /// Empty for sessions saved before this field existed.
    #[serde(default)]
    pub connection_id: String,
    /// Context window of the model at save time (0 = unknown).
    #[serde(default)]
    pub context_window: u64,
    /// Whether the model accepted image parts.
    #[serde(default)]
    pub vision: bool,
    /// Cached so the picker can list sessions without materializing messages.
    #[serde(default)]
    pub message_count: usize,
    pub messages: Vec<Message>,
    pub usage: Usage,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Session context that lives outside the message list.
#[derive(Debug, Clone, Default)]
pub struct SessionEnv {
    pub connection_id: String,
    pub context_window: u64,
    pub vision: bool,
    /// Creation time of an already-saved session. `None` stamps "now" —
    /// pass the existing value when re-saving or `created_at` walks forward
    /// with every autosave.
    pub created_at: Option<u64>,
}

/// Metadata for the sessions list (no messages — lightweight).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub model: String,
    pub message_count: usize,
    pub created_at: u64,
    pub updated_at: u64,
}

impl From<&SessionSnapshot> for SessionMeta {
    fn from(s: &SessionSnapshot) -> Self {
        SessionMeta {
            id: s.id.clone(),
            title: s.title.clone(),
            model: s.model.clone(),
            message_count: s.message_count,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// The header of a session file, parsed without materializing `messages`.
/// Unknown fields (including the whole transcript) are skipped by serde
/// without allocating, which keeps the picker cheap on large sessions.
#[derive(Deserialize)]
struct SessionHeader {
    id: String,
    title: String,
    model: String,
    #[serde(default)]
    message_count: usize,
    created_at: u64,
    updated_at: u64,
}

/// Where session files live: `~/.config/hive/sessions/`.
pub fn sessions_dir() -> PathBuf {
    let base = directories::BaseDirs::new()
        .map(|b| b.config_dir().join("hive"))
        .unwrap_or_else(|| PathBuf::from(".hive"));
    base.join("sessions")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Session ids reach us from user input (`/resume <id>`), so they have to stay
/// a flat filename — otherwise a crafted id escapes the sessions directory.
fn id_is_safe(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn session_path(dir: &Path, id: &str) -> std::io::Result<PathBuf> {
    if !id_is_safe(id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid session id: {id}"),
        ));
    }
    Ok(dir.join(format!("{id}.json")))
}

/// Generate a short title from the first user message in the list.
/// Falls back to "New chat" if there are no user messages.
pub fn generate_title(messages: &[Message]) -> String {
    let first_user = messages
        .iter()
        .find(|m| matches!(m.role, Role::User))
        .map(|m| m.text());

    match first_user {
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return "New chat".to_string();
            }
            let first_line = trimmed.lines().next().unwrap_or(trimmed);
            let chars: Vec<char> = first_line.chars().collect();
            let title: String = chars.iter().take(60).collect();
            if chars.len() > 60 {
                format!("{title}…")
            } else {
                title
            }
        }
        None => "New chat".to_string(),
    }
}

/// Save a session snapshot to disk. Returns the file path.
pub fn save(snapshot: &SessionSnapshot) -> std::io::Result<PathBuf> {
    save_in(&sessions_dir(), snapshot)
}

/// Save into an explicit directory.
pub fn save_in(dir: &Path, snapshot: &SessionSnapshot) -> std::io::Result<PathBuf> {
    let path = session_path(dir, &snapshot.id)?;
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(snapshot).map_err(std::io::Error::other)?;
    write_atomic(&path, json.as_bytes())?;
    Ok(path)
}

/// Write through a temp file so an interrupted save can't leave a half-written
/// session behind — `list` silently drops files it can't parse, so a torn
/// write would make the session vanish from the picker.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Load a session by id.
pub fn load(id: &str) -> std::io::Result<SessionSnapshot> {
    load_in(&sessions_dir(), id)
}

/// Load from an explicit directory.
pub fn load_in(dir: &Path, id: &str) -> std::io::Result<SessionSnapshot> {
    let path = session_path(dir, id)?;
    let data = std::fs::read_to_string(&path)?;
    let mut snap: SessionSnapshot = serde_json::from_str(&data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if snap.message_count == 0 {
        snap.message_count = count_visible(&snap.messages);
    }
    Ok(snap)
}

/// List all saved sessions, newest first.
pub fn list() -> Vec<SessionMeta> {
    list_in(&sessions_dir())
}

/// List sessions in an explicit directory.
pub fn list_in(dir: &Path) -> Vec<SessionMeta> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut metas = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(header) = serde_json::from_str::<SessionHeader>(&data) else {
            continue;
        };
        let message_count = if header.message_count > 0 {
            header.message_count
        } else {
            // Pre-`message_count` file: fall back to a full parse.
            serde_json::from_str::<SessionSnapshot>(&data)
                .map(|s| count_visible(&s.messages))
                .unwrap_or(0)
        };
        metas.push(SessionMeta {
            id: header.id,
            title: header.title,
            model: header.model,
            message_count,
            created_at: header.created_at,
            updated_at: header.updated_at,
        });
    }
    metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at));
    metas
}

/// Delete a session by id.
pub fn delete(id: &str) -> std::io::Result<()> {
    delete_in(&sessions_dir(), id)
}

/// Delete from an explicit directory.
pub fn delete_in(dir: &Path, id: &str) -> std::io::Result<()> {
    let path = session_path(dir, id)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Messages a human would count as part of the conversation.
fn count_visible(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|m| matches!(m.role, Role::User | Role::Assistant))
        .count()
}

/// Create a new snapshot from the current session state.
pub fn snapshot(
    id: &str,
    model: &str,
    messages: Vec<Message>,
    usage: Usage,
    env: SessionEnv,
) -> SessionSnapshot {
    let now = now_secs();
    let title = generate_title(&messages);
    let message_count = count_visible(&messages);
    SessionSnapshot {
        id: id.to_string(),
        title,
        model: model.to_string(),
        connection_id: env.connection_id,
        context_window: env.context_window,
        vision: env.vision,
        message_count,
        messages,
        usage,
        created_at: env.created_at.unwrap_or(now),
        updated_at: now,
    }
}

/// Generate a unique session id. The random half matters: two sessions can
/// start in the same second (two hive instances, or `/clear` then a prompt)
/// and a timestamp-only id would let one silently overwrite the other.
pub fn new_id() -> String {
    let now = now_secs();
    let rand = uuid::Uuid::new_v4().simple().to_string();
    format!("s_{now}_{}", &rand[..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    /// A fresh temp dir per test — `sessions_dir()` points at the real config
    /// directory, which tests must never touch.
    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hive-sessions-test-{}", new_id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn snap_with(id: &str, messages: Vec<Message>, created_at: Option<u64>) -> SessionSnapshot {
        snapshot(
            id,
            "acc/models/test",
            messages,
            Usage::default(),
            SessionEnv {
                connection_id: "work".into(),
                context_window: 128_000,
                vision: true,
                created_at,
            },
        )
    }

    #[test]
    fn title_from_first_user_message() {
        let msgs = vec![
            Message::system("system"),
            Message::user("Fix the login bug in auth.rs"),
        ];
        assert_eq!(generate_title(&msgs), "Fix the login bug in auth.rs");
    }

    #[test]
    fn title_truncates_long_messages() {
        let long = "x".repeat(100);
        let msgs = vec![Message::user(&long)];
        let title = generate_title(&msgs);
        assert!(title.ends_with('…'));
        assert_eq!(title.chars().count(), 61); // 60 chars + ellipsis
    }

    #[test]
    fn title_uses_first_line_only() {
        let msgs = vec![Message::user("First line\nSecond line\nThird")];
        assert_eq!(generate_title(&msgs), "First line");
    }

    #[test]
    fn title_no_user_messages() {
        let msgs = vec![Message::system("system")];
        assert_eq!(generate_title(&msgs), "New chat");
    }

    #[test]
    fn title_empty_user_message() {
        let msgs = vec![Message::system("system"), Message::user("")];
        assert_eq!(generate_title(&msgs), "New chat");
    }

    #[test]
    fn roundtrip_keeps_connection_and_model() {
        let dir = temp_dir();
        let snap = snap_with("s_1_aaaa", vec![Message::user("hi")], None);
        save_in(&dir, &snap).expect("save");
        let back = load_in(&dir, "s_1_aaaa").expect("load");
        assert_eq!(back.connection_id, "work");
        assert_eq!(back.model, "acc/models/test");
        assert_eq!(back.context_window, 128_000);
        assert!(back.vision);
        assert_eq!(back.messages.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resave_preserves_created_at() {
        let dir = temp_dir();
        let first = snap_with("s_2_bbbb", vec![Message::user("hi")], None);
        save_in(&dir, &first).expect("save");

        // Re-save with the original creation time, as the driver does.
        let again = snap_with(
            "s_2_bbbb",
            vec![Message::user("hi"), Message::assistant("yo")],
            Some(first.created_at),
        );
        save_in(&dir, &again).expect("resave");

        let back = load_in(&dir, "s_2_bbbb").expect("load");
        assert_eq!(back.created_at, first.created_at);
        assert!(back.updated_at >= back.created_at);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_reports_counts_and_newest_first() {
        let dir = temp_dir();
        let mut old = snap_with("s_3_cccc", vec![Message::user("a")], None);
        old.updated_at = 100;
        let mut new = snap_with(
            "s_4_dddd",
            vec![
                Message::system("sys"),
                Message::user("b"),
                Message::assistant("c"),
            ],
            None,
        );
        new.updated_at = 200;
        save_in(&dir, &old).expect("save old");
        save_in(&dir, &new).expect("save new");

        let metas = list_in(&dir);
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].id, "s_4_dddd");
        assert_eq!(metas[0].message_count, 2); // system doesn't count
        assert_eq!(metas[1].message_count, 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_skips_partial_writes() {
        let dir = temp_dir();
        let snap = snap_with("s_5_eeee", vec![Message::user("hi")], None);
        save_in(&dir, &snap).expect("save");
        std::fs::write(dir.join("torn.json"), "{\"id\": \"torn\", ").expect("torn file");

        let metas = list_in(&dir);
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, "s_5_eeee");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn traversal_ids_are_rejected() {
        let dir = temp_dir();
        for bad in ["../escape", "a/b", "", "with space", "dot.dot"] {
            assert!(load_in(&dir, bad).is_err(), "load accepted {bad:?}");
            assert!(delete_in(&dir, bad).is_err(), "delete accepted {bad:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_removes_the_file() {
        let dir = temp_dir();
        let snap = snap_with("s_6_ffff", vec![Message::user("hi")], None);
        save_in(&dir, &snap).expect("save");
        delete_in(&dir, "s_6_ffff").expect("delete");
        assert!(list_in(&dir).is_empty());
        // Deleting a missing session is a no-op, not an error.
        delete_in(&dir, "s_6_ffff").expect("delete again");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Sessions written before `connection_id` / `vision` / `message_count`
    /// existed still have to load and list.
    #[test]
    fn loads_files_without_the_newer_fields() {
        let dir = temp_dir();
        let legacy = r#"{
          "id": "s_1785055436_c8cc",
          "title": "ку бро",
          "model": "accounts/fireworks/models/kimi",
          "messages": [
            {"role": "system", "content": [{"Text": "sys"}], "tool_calls": [],
             "tool_call_id": null, "name": null},
            {"role": "user", "content": [{"Text": "ку бро"}], "tool_calls": [],
             "tool_call_id": null, "name": null},
            {"role": "assistant", "content": [{"Text": "здарова"}], "tool_calls": [],
             "tool_call_id": null, "name": null}
          ],
          "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
          "created_at": 1785055436,
          "updated_at": 1785055436
        }"#;
        std::fs::write(dir.join("s_1785055436_c8cc.json"), legacy).expect("legacy file");

        let snap = load_in(&dir, "s_1785055436_c8cc").expect("load legacy");
        assert_eq!(snap.connection_id, "", "unknown connection stays empty");
        assert_eq!(snap.context_window, 0, "unknown window stays 0");
        assert!(!snap.vision);
        assert_eq!(snap.message_count, 2, "counted from messages");
        assert_eq!(snap.usage.total_tokens, 15);

        let metas = list_in(&dir);
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].title, "ку бро");
        assert_eq!(metas[0].message_count, 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ids_are_unique_within_a_second() {
        let ids: std::collections::HashSet<String> = (0..64).map(|_| new_id()).collect();
        assert_eq!(ids.len(), 64);
        assert!(ids.iter().all(|id| id_is_safe(id)));
    }
}
