//! Hive bot hub — a second TUI for talking to persistent agent personas.
//!
//! Layout follows docs/HIVE_BOT.md and the approved mockup: a persona rail on
//! the left, one long chat per persona in the middle, and a create form on the
//! right. Unlike the main hive TUI this is a standalone loop with a direct
//! provider connection and no sessions, tools, or worktrees.

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
    /// Where personas created in this session are written.
    pub save_root: PathBuf,
    /// Short scope tag shown in the UI ("project" / "global").
    pub scope_label: String,
}

/// Entry point: owns the terminal until the hub quits.
pub fn run(init: BotInit) -> io::Result<()> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::other("hive bot needs an interactive terminal"));
    }
    let mut terminal = Terminal::new()?;
    let mut hub = BotHub::new(init);
    loop {
        let dirty = hub.drain_engine();
        if dirty {
            let snapshot = &hub;
            terminal.draw(|f| crate::render::bot::draw(f, snapshot))?;
        }
        if let Some(ev) = terminal.read_event(IDLE_TICK)? {
            let mut quit = false;
            let mut dirty_now = true;
            match ev {
                Event::Key(key) => quit = hub.handle_key(key),
                Event::Paste(text) => hub.handle_paste(&text),
                Event::Resize(_, _) => {}
                Event::Mouse(_) => dirty_now = false,
            }
            if quit {
                return Ok(());
            }
            if dirty_now {
                let snapshot = &hub;
                terminal.draw(|f| crate::render::bot::draw(f, snapshot))?;
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
    pub(crate) fn preview(&self) -> String {
        let source = if self.live.is_empty() {
            self.history
                .iter()
                .rev()
                .find(|m| matches!(m.role, Role::Assistant))
                .map(|m| m.text())
                .unwrap_or_default()
        } else {
            self.live.clone()
        };
        first_line(&source)
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Composer,
    FieldName,
    FieldDesc,
}

/// State of the right-hand create-persona panel.
pub(crate) struct Form {
    pub(crate) name: LineEdit,
    pub(crate) desc: LineEdit,
    pub(crate) focus: Focus,
}

impl Form {
    fn new() -> Self {
        Form {
            name: LineEdit::default(),
            desc: LineEdit::default(),
            focus: Focus::FieldName,
        }
    }

    fn focus_next(&mut self) {
        self.focus = match self.focus {
            Focus::FieldName => Focus::FieldDesc,
            _ => Focus::FieldName,
        };
    }
}

pub struct BotHub {
    theme: Theme,
    model: String,
    provider: Arc<dyn LlmProvider>,
    roots: Vec<PathBuf>,
    save_root: PathBuf,
    scope_label: String,
    book: hive_core::persona::PersonaBook,
    personas: Vec<hive_core::persona::PersonaMeta>,
    selected: usize,
    chats: HashMap<String, Chat>,
    composer: LineEdit,
    form: Option<Form>,
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
            save_root: init.save_root,
            scope_label: init.scope_label,
            book: hive_core::persona::PersonaBook::load(Vec::new()),
            personas: Vec::new(),
            selected: 0,
            chats: HashMap::new(),
            composer: LineEdit::default(),
            form: None,
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

    fn focus(&self) -> Focus {
        self.form
            .as_ref()
            .map(|f| f.focus)
            .unwrap_or(Focus::Composer)
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

    /// Save the create form into the session's save root.
    pub fn save_form(&mut self) {
        let Some(form) = &self.form else {
            return;
        };
        let name = form.name.text.trim().to_string();
        let desc = form.desc.text.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.form.as_mut().unwrap().focus = Focus::FieldName;
        let persona = hive_core::persona::Persona::new(
            name.clone(),
            desc.clone(),
            generated_prompt(&name, &desc),
        );
        if hive_core::persona::PersonaBook::save(&self.save_root, &persona).is_err() {
            return;
        }
        self.form = None;
        self.reload_book();
        if let Some(idx) = self
            .personas()
            .iter()
            .position(|p| p.name == persona.meta.name)
        {
            self.selected = idx;
        }
    }

    pub fn open_form(&mut self) {
        if self.form.is_none() {
            self.form = Some(Form::new());
        }
    }

    pub fn close_form(&mut self) {
        self.form = None;
    }

    pub fn handle_paste(&mut self, text: &str) {
        match self.focus() {
            Focus::Composer => self.composer.insert_str(text),
            Focus::FieldName => {
                if let Some(f) = &mut self.form {
                    f.name.insert_str(&text.replace(['\n', '\r'], " "));
                }
            }
            Focus::FieldDesc => {
                if let Some(f) = &mut self.form {
                    f.desc.insert_str(&text.replace(['\n', '\r'], " "));
                }
            }
        }
    }

    /// Handle one key. Returns true when the hub should quit.
    pub fn handle_key(&mut self, key: Key) -> bool {
        let ctrl = key.mods.ctrl;
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('c') if ctrl => return true,
            KeyCode::Char('n') if ctrl => {
                if self.form.is_none() {
                    self.open_form();
                }
                return false;
            }
            _ => {}
        }

        if key.mods.alt {
            match key.code {
                KeyCode::Up => self.select(-1),
                KeyCode::Down => self.select(1),
                _ => {}
            }
            return false;
        }

        if self.form.is_some() {
            self.handle_form_key(key);
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

    fn handle_form_key(&mut self, key: Key) {
        match key.code {
            KeyCode::Esc => self.close_form(),
            KeyCode::Char('s') if key.mods.ctrl => self.save_form(),
            KeyCode::Tab => {
                if let Some(f) = &mut self.form {
                    f.focus_next();
                }
            }
            KeyCode::Enter => {
                match self.focus() {
                    Focus::FieldName => {
                        if let Some(f) = &mut self.form {
                            f.focus = Focus::FieldDesc;
                        }
                    }
                    Focus::FieldDesc => self.save_form(),
                    Focus::Composer => {}
                }
            }
            KeyCode::Backspace => match self.focus() {
                Focus::FieldName => {
                    if let Some(f) = &mut self.form {
                        f.name.backspace();
                    }
                }
                _ => {
                    if let Some(f) = &mut self.form {
                        f.desc.backspace();
                    }
                }
            },
            KeyCode::Delete => match self.focus() {
                Focus::FieldName => {
                    if let Some(f) = &mut self.form {
                        f.name.delete();
                    }
                }
                _ => {
                    if let Some(f) = &mut self.form {
                        f.desc.delete();
                    }
                }
            },
            KeyCode::Left => match self.focus() {
                Focus::FieldName => {
                    if let Some(f) = &mut self.form {
                        f.name.left();
                    }
                }
                _ => {
                    if let Some(f) = &mut self.form {
                        f.desc.left();
                    }
                }
            },
            KeyCode::Right => match self.focus() {
                Focus::FieldName => {
                    if let Some(f) = &mut self.form {
                        f.name.right();
                    }
                }
                _ => {
                    if let Some(f) = &mut self.form {
                        f.desc.right();
                    }
                }
            },
            KeyCode::Char(c) if !key.mods.ctrl => match self.focus() {
                Focus::FieldName => {
                    if let Some(f) = &mut self.form {
                        f.name.insert(c);
                    }
                }
                _ => {
                    if let Some(f) = &mut self.form {
                        f.desc.insert(c);
                    }
                }
            },
            _ => {}
        }
    }

    // ---- read-only accessors used by the renderer ----

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn scope_label(&self) -> &str {
        &self.scope_label
    }

    pub(crate) fn composer(&self) -> &LineEdit {
        &self.composer
    }

    pub(crate) fn form(&self) -> Option<&Form> {
        self.form.as_ref()
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
        let mut h = BotHub::new(BotInit {
            model: "m".into(),
            provider: Arc::new(StubProvider),
            roots,
            save_root: std::env::temp_dir().join("hive-bot-hub-test"),
            scope_label: "project".into(),
        });
        h.save_root = std::env::temp_dir().join("hive-bot-hub-test");
        h
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
        assert_eq!(wrap_text("hello brave world", 5), ["hello", "brave", "world"]);
        assert_eq!(wrap_text("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap_text("a\n\nb", 5), ["a", "", "b"]);
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

    #[test]
    fn form_save_writes_persona_and_selects_it() {
        let scope = std::env::temp_dir().join(format!("hive-bot-form-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scope);
        let mut h = hub_with_roots(vec![scope.clone()]);
        h.save_root = scope.clone();
        h.open_form();
        if let Some(f) = &mut h.form {
            f.name.insert_str("Maya");
            f.focus = Focus::FieldDesc;
            f.desc.insert_str("Personal assistant");
        }
        h.save_form();
        assert!(h.form.is_none());
        assert_eq!(h.selected_name(), Some("Maya"));
        let saved = std::fs::read_to_string(scope.join("Maya.md")).unwrap();
        assert!(saved.contains("name: Maya"));
        assert!(saved.contains("Personal assistant"));
        let _ = std::fs::remove_dir_all(&scope);
    }

    #[test]
    fn empty_name_blocks_save() {
        let mut h = hub();
        h.open_form();
        h.save_form();
        assert!(h.form.is_some(), "no name, no save");
    }
}
