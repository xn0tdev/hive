#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalController {
    Agent,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalProcessState {
    Running,
    Exited { code: i32 },
    Failed { message: String },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TerminalSnapshot {
    pub id: String,
    pub command: String,
    pub controller: TerminalController,
    pub process: TerminalProcessState,
    pub revision: u64,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TerminalReadResult {
    pub session: TerminalSnapshot,
    pub screen: Option<String>,
    pub output: Option<String>,
    pub output_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKey {
    Enter,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Backspace,
    CtrlC,
    CtrlD,
}

impl TerminalKey {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "enter" => Some(Self::Enter),
            "tab" => Some(Self::Tab),
            "escape" => Some(Self::Escape),
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "backspace" => Some(Self::Backspace),
            "ctrl_c" => Some(Self::CtrlC),
            "ctrl_d" => Some(Self::CtrlD),
            _ => None,
        }
    }

    pub fn bytes(self, application_cursor: bool) -> &'static [u8] {
        match self {
            Self::Enter => b"\r",
            Self::Tab => b"\t",
            Self::Escape => b"\x1b",
            Self::Up if application_cursor => b"\x1bOA",
            Self::Down if application_cursor => b"\x1bOB",
            Self::Right if application_cursor => b"\x1bOC",
            Self::Left if application_cursor => b"\x1bOD",
            Self::Up => b"\x1b[A",
            Self::Down => b"\x1b[B",
            Self::Right => b"\x1b[C",
            Self::Left => b"\x1b[D",
            Self::Backspace => b"\x7f",
            Self::CtrlC => b"\x03",
            Self::CtrlD => b"\x04",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalWriteRequest {
    pub text: Option<String>,
    pub key: Option<TerminalKey>,
    pub submit: bool,
}

impl TerminalWriteRequest {
    pub fn bytes(&self, application_cursor: bool) -> Result<Vec<u8>, TerminalError> {
        let mut bytes = Vec::new();
        if let Some(text) = &self.text {
            bytes.extend_from_slice(text.as_bytes());
        }
        if let Some(key) = self.key {
            bytes.extend_from_slice(key.bytes(application_cursor));
        }
        if self.submit {
            bytes.push(b'\r');
        }
        if bytes.is_empty() {
            return Err(TerminalError::EmptyInput);
        }
        Ok(bytes)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("interactive terminal is unavailable in this agent")]
    Unavailable,
    #[error("terminal session {id} is already running")]
    AlreadyRunning { id: String },
    #[error("terminal session not found: {id}")]
    NotFound { id: String },
    #[error("terminal session is controlled by the user")]
    UserControlled,
    #[error("terminal session is not running")]
    NotRunning,
    #[error("terminal input is empty")]
    EmptyInput,
    #[error("terminal I/O failed: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_key_parses_and_encodes_cursor_modes() {
        assert_eq!(TerminalKey::parse("enter"), Some(TerminalKey::Enter));
        assert_eq!(TerminalKey::parse("ctrl_c"), Some(TerminalKey::CtrlC));
        assert_eq!(TerminalKey::parse("unknown"), None);
        assert_eq!(TerminalKey::Enter.bytes(false), b"\r");
        assert_eq!(TerminalKey::CtrlC.bytes(false), b"\x03");
        assert_eq!(TerminalKey::Up.bytes(false), b"\x1b[A");
        assert_eq!(TerminalKey::Up.bytes(true), b"\x1bOA");
    }

    #[test]
    fn terminal_write_request_orders_text_key_and_submit() {
        let request = TerminalWriteRequest {
            text: Some("yes".into()),
            key: Some(TerminalKey::Tab),
            submit: true,
        };
        assert_eq!(request.bytes(false).unwrap(), b"yes\t\r");
    }

    #[test]
    fn terminal_write_request_rejects_empty_input() {
        let request = TerminalWriteRequest {
            text: None,
            key: None,
            submit: false,
        };
        assert!(matches!(
            request.bytes(false),
            Err(TerminalError::EmptyInput)
        ));
    }
}
