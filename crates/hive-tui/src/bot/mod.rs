//! Hive bot hub — a second TUI for talking to persistent agent personas.
//!
//! Layout follows docs/HIVE_BOT.md and the approved mockup: a chat rail on
//! the left and one long chat per persona in the middle. Personas are plain
//! markdown files under the scope's `agents/` dir; unlike the main hive TUI
//! this is a standalone loop with a direct provider connection and no
//! sessions, tools, or worktrees.

pub mod engine;

use std::collections::HashMap;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use comb::{Event, Key, KeyCode, Terminal};

use hive_core::message::{Message, Role};
use hive_core::provider::LlmProvider;

use crate::theme::Theme;

use engine::{send, EngineMsg};

/// How often the hub wakes to poll the engine channel.
const IDLE_TICK: Duration = Duration::from_millis(120);
/// Cap on engine messages drained per frame.
const MAX_ENGINE_MSGS: usize = 256;

/// Everything the bot hub needs at startup.
pub struct BotInit {
    /// Provider model id used for every persona chat.
    pub model: String,
    pub provider: Arc<dyn LlmProvider>,
    /// Persona directories in precedence order (project first, global last).
    pub roots: Vec<PathBuf>,
}

/// Entry point: owns the terminal until the hub quits.
pub fn run(init: BotInit) -> io::Result<()> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::other("hive bot needs an interactive terminal"));
    }
    let mut terminal = Terminal::new()?;
    let mut hub = BotHub::new(init);
    // Paint once up front — a quiet hub would otherwise show a blank screen
    // until the first key or engine message.
    let mut dirty = true;
    loop {
        dirty |= hub.drain_engine();
        if dirty {
            let snapshot = &hub;
            terminal.draw(|f| crate::render::bot::draw(f, snapshot))?;
            dirty = false;
        }
        if let Some(ev) = terminal.read_event(IDLE_TICK)? {
            match ev {
                Event::Key(key) => {
                    if hub.handle_key(key) {
                        return Ok(());
                    }
                    dirty = true;
                }
                Event::Paste(text) => {
                    hub.handle_paste(&text);
                    dirty = true;
                }
                Event::Resize(_, _) => dirty = true,
                Event::Mouse(_) => {}
            }
        }
    }
}

/// One long-running conversation with a persona.
#[derive(Clone, Default)]
pub(crate) struct Chat {
    /// Committed turns (user + assistant), replayed on every request.
    pub(crate) history: Vec<Message>,
    /// Assistant reply currently streaming in.
    pub(crate) live: String,
    /// Last engine error, shown as a dim line above the composer.
    pub(crate) error: Option<String>,
}

impl Chat {
    /// Last message from either side, messenger-style. `None` when the chat
    /// has no turns yet; the rail falls back to the persona description.
    pub(crate) fn preview(&self) -> Option<String> {
        if !self.live.is_empty() {
            return Some(first_line(&self.live));
        }
        self.history.iter().rev().find_map(|m| {
            let line = first_line(&m.text());
            if line.is_empty() {
                return None;
            }
            Some(match m.role {
                Role::User => format!("You: {line}"),
                _ => line,
            })
        })
    }
}

/// A single-line editable text field.
#[derive(Default)]
pub(crate) struct LineEdit {
    pub(crate) text: String,
    /// Cursor position in chars (may equal text.len()).
    pub(crate) caret: usize,
}

impl LineEdit {
    #[cfg(test)]
    fn new(text: &str) -> Self {
        let caret = text.chars().count();
        LineEdit {
            text: text.to_string(),
            caret,
        }
    }
    fn insert(&mut self, c: char) {
        let byte = char_to_byte(&self.text, self.caret);
        self.text.insert(byte, c);
        self.caret += 1;
    }

    fn backspace(&mut self) {
        if self.caret == 0 {
            return;
        }
        let start = char_to_byte(&self.text, self.caret - 1);
        let byte = char_to_byte(&self.text, self.caret);
        self.text.replace_range(start..byte, "");
        self.caret -= 1;
    }

    fn delete(&mut self) {
        if self.caret >= self.text.chars().count() {
            return;
        }
        let start = char_to_byte(&self.text, self.caret);
        let end = char_to_byte(&self.text, self.caret + 1);
        self.text.replace_range(start..end, "");
    }

    fn left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }

    fn right(&mut self) {
        self.caret = (self.caret + 1).min(self.text.chars().count());
    }

    fn home(&mut self) {
        self.caret = 0;
    }

    fn end(&mut self) {
        self.caret = self.text.chars().count();
    }

    fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            if c == '\n' || c == '\r' {
                continue;
            }
            self.insert(c);
        }
    }
}

fn char_to_byte(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

pub struct BotHub {
    theme: Theme,
    model: String,
    provider: Arc<dyn LlmProvider>,
    roots: Vec<PathBuf>,
    book: hive_core::persona::PersonaBook,
    personas: Vec<hive_core::persona::PersonaMeta>,
    selected: usize,
    chats: HashMap<String, Chat>,
    composer: LineEdit,
    streaming: Option<String>,
    engine_tx: tokio::sync::mpsc::UnboundedSender<EngineMsg>,
    engine_rx: tokio::sync::mpsc::UnboundedReceiver<EngineMsg>,
}

impl BotHub {
    pub fn new(init: BotInit) -> Self {
        let (engine_tx, engine_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut hub = BotHub {
            theme: Theme::gray(),
            model: init.model,
            provider: init.provider,
            roots: init.roots,
            book: hive_core::persona::PersonaBook::load(Vec::new()),
            personas: Vec::new(),
            selected: 0,
            chats: HashMap::new(),
            composer: LineEdit::default(),
            streaming: None,
            engine_tx,
            engine_rx,
        };
        hub.reload_book();
        hub
    }

    /// Re-scan persona directories, keeping chats by name intact.
    pub fn reload_book(&mut self) {
        self.book = hive_core::persona::PersonaBook::load(self.roots.clone());
        self.personas = self.book.list();
        if self.selected >= self.personas.len() {
            self.selected = self.personas.len().saturating_sub(1);
        }
    }

    pub fn personas(&self) -> &[hive_core::persona::PersonaMeta] {
        &self.personas
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.personas().get(self.selected).map(|p| p.name.as_str())
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    fn chat(&self, name: &str) -> Chat {
        self.chats.get(name).cloned().unwrap_or_default()
    }

    /// Move the persona selection, creating the chat lazily.
    pub fn select(&mut self, delta: isize) {
        let count = self.personas().len();
        if count == 0 {
            return;
        }
        let next = (self.selected as isize + delta).rem_euclid(count as isize);
        self.selected = next as usize;
    }

    pub fn handle_paste(&mut self, text: &str) {
        self.composer.insert_str(text);
    }

    /// Handle one key. Returns true when the hub should quit.
    pub fn handle_key(&mut self, key: Key) -> bool {
        let ctrl = key.mods.ctrl;
        if key.code == KeyCode::Char('q') && ctrl || key.code == KeyCode::Char('c') && ctrl {
            return true;
        }

        if key.mods.alt {
            match key.code {
                KeyCode::Up => self.select(-1),
                KeyCode::Down => self.select(1),
                _ => {}
            }
            return false;
        }

        match key.code {
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => self.composer.backspace(),
            KeyCode::Delete => self.composer.delete(),
            KeyCode::Left => self.composer.left(),
            KeyCode::Right => self.composer.right(),
            KeyCode::Home => self.composer.home(),
            KeyCode::End => self.composer.end(),
            KeyCode::Char('u') if ctrl => {
                self.composer = LineEdit::default();
            }
            KeyCode::Char(c) if !ctrl => self.composer.insert(c),
            _ => {}
        }
        false
    }

    /// Route one engine message; returns true when something changed.
    pub fn drain_engine(&mut self) -> bool {
        let mut dirty = false;
        for _ in 0..MAX_ENGINE_MSGS {
            match self.engine_rx.try_recv() {
                Ok(msg) => dirty |= self.apply_engine(msg),
                Err(_) => break,
            }
        }
        dirty
    }

    fn apply_engine(&mut self, msg: EngineMsg) -> bool {
        let name = match &msg {
            EngineMsg::Delta { persona, .. }
            | EngineMsg::Done { persona }
            | EngineMsg::Failed { persona, .. } => persona.clone(),
        };
        let chat = self.chats.entry(name.clone()).or_default();
        match msg {
            EngineMsg::Delta { text, .. } => {
                chat.live.push_str(&text);
                true
            }
            EngineMsg::Done { .. } => {
                let text = std::mem::take(&mut chat.live);
                if !text.trim().is_empty() {
                    chat.history.push(Message::assistant(text));
                }
                if self.streaming.as_deref() == Some(name.as_str()) {
                    self.streaming = None;
                }
                true
            }
            EngineMsg::Failed { error, .. } => {
                chat.live.clear();
                chat.error = Some(error);
                if self.streaming.as_deref() == Some(name.as_str()) {
                    self.streaming = None;
                }
                true
            }
        }
    }

    /// Send the composer text to the selected persona.
    pub fn submit(&mut self) {
        let name = match self.selected_name() {
            Some(n) => n.to_string(),
            None => return,
        };
        let text = std::mem::take(&mut self.composer.text).trim().to_string();
        self.composer.caret = 0;
        if text.is_empty() || self.streaming.is_some() {
            return;
        }
        let persona = match self.book.read(&name) {
            Some(p) => p.clone(),
            None => return,
        };
        let meta = &persona.meta;
        let system = if persona.prompt.trim().is_empty() {
            generated_prompt(meta.name.trim(), &meta.description)
        } else {
            format!("{}\n\n{}", persona.prompt, ROLE_HINT)
        };
        let name = meta.name.clone();
        let chat = self.chats.entry(name.clone()).or_default();
        chat.error = None;
        let history = chat.history.clone();
        chat.history.push(Message::user(text.clone()));
        self.streaming = Some(name.clone());
        send(
            self.provider.clone(),
            self.model.clone(),
            system,
            name,
            history,
            text,
            self.engine_tx.clone(),
        );
    }

    // ---- read-only accessors used by the renderer ----

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub(crate) fn composer(&self) -> &LineEdit {
        &self.composer
    }

    pub fn is_streaming(&self, name: &str) -> bool {
        self.streaming.as_deref() == Some(name)
    }

    pub(crate) fn chat_for(&self, name: &str) -> Chat {
        self.chat(name)
    }
}

const ROLE_HINT: &str = "\
Talk to the user directly, one readable message at a time. \
Plain text, no markdown fences. Stay concise and useful.";

/// Body prompt stored with a newly created persona.
fn generated_prompt(name: &str, description: &str) -> String {
    if description.is_empty() {
        format!("You are {name}. {ROLE_HINT}")
    } else {
        format!("You are {name}. {description}\n\n{ROLE_HINT}")
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Greedy word wrap; long words are hard-broken. Width 0 yields one word per line.
pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split(' ') {
            if word.chars().count() > width && width > 0 {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                let mut chunk = String::new();
                for c in word.chars() {
                    if chunk.chars().count() == width {
                        out.push(std::mem::take(&mut chunk));
                    }
                    chunk.push(c);
                }
                line = chunk;
            } else if line.is_empty() {
                line = word.to_string();
            } else if line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::engine::EngineMsg;
    use async_trait::async_trait;
    use hive_core::error::CoreError;
    use hive_core::provider::{ChatOutcome, ChatRequest, Delta};

    struct StubProvider;

    #[async_trait]
    impl LlmProvider for StubProvider {
        async fn chat_stream(
            &self,
            _req: ChatRequest<'_>,
            _on_delta: &mut (dyn FnMut(Delta) + Send),
        ) -> hive_core::error::Result<ChatOutcome> {
            Err(CoreError::Cancelled)
        }
    }

    fn hub() -> BotHub {
        hub_with_roots(Vec::new())
    }

    fn hub_with_roots(roots: Vec<std::path::PathBuf>) -> BotHub {
        BotHub::new(BotInit {
            model: "m".into(),
            provider: Arc::new(StubProvider),
            roots,
        })
    }

    #[test]
    fn line_edit_inserts_and_deletes_at_caret() {
        let mut e = LineEdit::new("abc");
        e.left();
        e.insert('X');
        assert_eq!(e.text, "abXc");
        assert_eq!(e.caret, 3);
        e.backspace();
        assert_eq!(e.text, "abc");
        assert_eq!(e.caret, 2);
        e.end();
        e.delete();
        assert_eq!(e.text, "abc");
    }

    #[test]
    fn wrap_breaks_words_and_long_tokens() {
        assert_eq!(
            wrap_text("hello brave world", 5),
            ["hello", "brave", "world"]
        );
        assert_eq!(wrap_text("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap_text("a\n\nb", 5), ["a", "", "b"]);
    }

    #[test]
    fn preview_prefers_live_then_last_turn_any_role() {
        let mut c = Chat::default();
        assert!(c.preview().is_none());

        c.history.push(Message::user("check my gmail"));
        assert_eq!(c.preview().as_deref(), Some("You: check my gmail"));

        c.history.push(Message::assistant("on it.\nsecond line"));
        assert_eq!(c.preview().as_deref(), Some("on it."));

        c.live.push_str("still typing");
        assert_eq!(c.preview().as_deref(), Some("still typing"));
    }

    #[test]
    fn engine_messages_commit_and_fail() {
        let mut h = hub();
        h.chats.entry("maya".into()).or_default();
        assert!(h.apply_engine(EngineMsg::Delta {
            persona: "maya".into(),
            text: "hi ".into(),
        }));
        assert!(h.apply_engine(EngineMsg::Delta {
            persona: "maya".into(),
            text: "there".into(),
        }));
        assert!(h.apply_engine(EngineMsg::Done {
            persona: "maya".into(),
        }));
        let chat = h.chat("maya");
        assert_eq!(chat.history.len(), 1);
        assert!(matches!(chat.history[0].role, Role::Assistant));
        assert_eq!(chat.history[0].text(), "hi there");
        assert!(chat.live.is_empty());

        assert!(h.apply_engine(EngineMsg::Failed {
            persona: "maya".into(),
            error: "boom".into(),
        }));
        assert_eq!(h.chat("maya").error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn submit_stores_user_turn_and_locks_while_streaming() {
        let scope = std::env::temp_dir().join(format!("hive-bot-submit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scope);
        hive_core::persona::PersonaBook::save(
            &scope,
            &hive_core::persona::Persona::new("maya", "", ""),
        )
        .unwrap();
        let mut h = hub_with_roots(vec![scope.clone()]);
        h.selected = 0;
        h.composer.insert_str("check gmail");
        h.submit();
        assert_eq!(h.chat("maya").history.len(), 1);
        assert!(matches!(h.chat("maya").history[0].role, Role::User));
        assert_eq!(h.streaming.as_deref(), Some("maya"));

        h.composer.insert_str("second");
        h.submit();
        assert_eq!(h.chat("maya").history.len(), 1, "streaming blocks sends");
        let _ = std::fs::remove_dir_all(&scope);
    }
}
