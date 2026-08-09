use crate::message::Message;
use crate::provider::Usage;

/// The running conversation for one agent: the message history (with the system
/// prompt pinned first) and cumulative token usage.
pub struct Session {
    pub messages: Vec<Message>,
    pub usage: Usage,
}

impl Session {
    pub fn new(system: impl Into<String>) -> Self {
        let system = system.into();
        Session {
            messages: vec![Message::system(system)],
            usage: Usage::default(),
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
        let system = self.system().to_string();
        self.messages = vec![Message::system(system)];
        self.usage = Usage::default();
    }

    /// Current system prompt text.
    pub fn system(&self) -> &str {
        self.messages
            .first()
            .filter(|message| matches!(message.role, crate::message::Role::System))
            .and_then(|message| message.content.first())
            .and_then(|part| match part {
                crate::message::ContentPart::Text(text) => Some(text.as_str()),
                crate::message::ContentPart::Image(_) => None,
            })
            .unwrap_or("")
    }

    /// Replace the entire message list (must keep a system message first).
    pub fn replace_messages(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// Replace the pinned system prompt (e.g. when MAKE/PLAN mode changes).
    pub fn set_system(&mut self, system: impl Into<String>) {
        let system = system.into();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_preserves_the_single_system_source() {
        let mut session = Session::new("rules");
        session.push(Message::user("work"));
        session.reset();

        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.system(), "rules");
    }

    #[test]
    fn replacing_and_updating_messages_updates_the_system_prompt() {
        let mut session = Session::new("old");
        session.replace_messages(vec![Message::system("restored"), Message::user("task")]);
        assert_eq!(session.system(), "restored");

        session.set_system("new");
        assert_eq!(session.system(), "new");
        assert_eq!(session.messages.len(), 2);
    }
}
