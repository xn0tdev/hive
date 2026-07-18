use hive_core::message::Message;
use hive_core::provider::Usage;

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

    /// Text of the most recent assistant message, if any.
    pub fn last_assistant_text(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, hive_core::message::Role::Assistant))
            .map(|m| m.text())
    }
}
