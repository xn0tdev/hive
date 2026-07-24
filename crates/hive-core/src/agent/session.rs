use crate::message::Message;
use crate::provider::Usage;

/// The running conversation for one agent: the message history (with the system
/// prompt pinned first) and cumulative token usage.
pub struct Session {
    pub messages: Vec<Message>,
    pub usage: Usage,
    system: String,
}

impl Session {
    pub fn new(system: impl Into<String>) -> Self {
        let system = system.into();
        Session {
            messages: vec![Message::system(system.clone())],
            usage: Usage::default(),
            system,
        }
    }

    pub fn push(&mut self, m: Message) {
        self.messages.push(m);
    }

    pub fn add_usage(&mut self, u: Usage) {
        self.usage += u;
    }

    /// Clear the conversation but keep the system prompt.
    pub fn reset(&mut self) {
        self.messages = vec![Message::system(self.system.clone())];
        self.usage = Usage::default();
    }

    /// Current system prompt text.
    pub fn system(&self) -> &str {
        &self.system
    }

    /// Replace the entire message list (must keep a system message first).
    pub fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
        if let Some(first) = self.messages.first() {
            if matches!(first.role, crate::message::Role::System) {
                self.system = first.text();
            }
        }
    }

    /// Replace the pinned system prompt (e.g. when MAKE/PLAN mode changes).
    pub fn set_system(&mut self, system: impl Into<String>) {
        let system = system.into();
        self.system = system.clone();
        if let Some(first) = self.messages.first_mut() {
            *first = Message::system(system);
        } else {
            self.messages.insert(0, Message::system(system));
        }
    }

    /// Text of the most recent assistant message, if any.
    pub fn last_assistant_text(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, crate::message::Role::Assistant))
            .map(|m| m.text())
    }
}
