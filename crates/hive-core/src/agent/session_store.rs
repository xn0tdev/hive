//! Session persistence: save/load conversations as JSON files.
//!
//! Sessions are stored in `~/.config/hive/sessions/` (or `.hive/sessions/` as
//! fallback). Each session is one `.json` file containing the message history,
//! usage, model, and a heuristic title derived from the first user message.

use std::path::PathBuf;
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
    pub messages: Vec<Message>,
    pub usage: Usage,
    pub created_at: u64,
    pub updated_at: u64,
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
        let message_count = s
            .messages
            .iter()
            .filter(|m| matches!(m.role, Role::User | Role::Assistant))
            .count();
        SessionMeta {
            id: s.id.clone(),
            title: s.title.clone(),
            model: s.model.clone(),
            message_count,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
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
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", snapshot.id));
    let json = serde_json::to_string_pretty(snapshot)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&path, json)?;
    Ok(path)
}

/// Load a session by id.
pub fn load(id: &str) -> std::io::Result<SessionSnapshot> {
    let path = sessions_dir().join(format!("{id}.json"));
    let data = std::fs::read_to_string(&path)?;
    serde_json::from_str(&data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// List all saved sessions, newest first.
pub fn list() -> Vec<SessionMeta> {
    let dir = sessions_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut metas = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(snap) = serde_json::from_str::<SessionSnapshot>(&data) {
                metas.push(SessionMeta::from(&snap));
            }
        }
    }
    metas.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    metas
}

/// Delete a session by id.
pub fn delete(id: &str) -> std::io::Result<()> {
    let path = sessions_dir().join(format!("{id}.json"));
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Create a new snapshot from the current session state.
pub fn snapshot(
    id: &str,
    model: &str,
    messages: Vec<Message>,
    usage: Usage,
) -> SessionSnapshot {
    let now = now_secs();
    let title = generate_title(&messages);
    SessionSnapshot {
        id: id.to_string(),
        title,
        model: model.to_string(),
        messages,
        usage,
        created_at: now,
        updated_at: now,
    }
}

/// Generate a unique session id from a timestamp.
pub fn new_id() -> String {
    let now = now_secs();
    format!("s_{now}_{:04x}", now as u16 & 0xffff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

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
}
