#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalController {
    Agent,
    User,
}

/// What kind of input an interactive program is waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalInputKind {
    /// Password, passphrase, PIN, OTP, or another value the model must not see.
    Private,
    /// A visible yes/no-style decision the agent may answer when it is safe.
    Confirmation,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TerminalInputRequest {
    pub kind: TerminalInputKind,
    pub prompt: String,
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
    /// Structured prompt state so callers do not need to infer it from screen text.
    pub input_request: Option<TerminalInputRequest>,
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
    #[error("terminal is waiting for private input; ask the user to open the terminal card and enter it there")]
    PrivateInputRequired,
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

/// Detect a prompt that needs either private user input or an explicit
/// confirmation. Only the final non-empty screen line is considered.
pub fn terminal_input_request(screen: &str) -> Option<TerminalInputRequest> {
    let line = screen.lines().rev().find(|l| !l.trim().is_empty())?.trim();
    let lower = line.to_ascii_lowercase();

    let prompt_end = lower.ends_with(':') || lower.ends_with('?');
    let private = (lower.contains("password")
        || lower.contains("passphrase")
        || lower.contains("verification code")
        || lower.contains("authentication code")
        || lower.contains("one-time")
        || lower.contains("2fa")
        || lower.contains("otp")
        || lower.contains(" pin")
        || lower.starts_with("pin"))
        && prompt_end;
    let confirm = lower.ends_with("(yes/no)?")
        || lower.ends_with("[y/n]")
        || lower.ends_with("[y/n]:")
        || lower.contains("(yes/no/[fingerprint])");

    let kind = if private {
        TerminalInputKind::Private
    } else if confirm {
        TerminalInputKind::Confirmation
    } else {
        return None;
    };
    Some(TerminalInputRequest {
        kind,
        prompt: line.to_string(),
    })
}

/// Backwards-compatible prompt text helper used by the TUI status label.
pub fn awaiting_user_input(screen: &str) -> Option<String> {
    terminal_input_request(screen).map(|request| request.prompt)
}

impl TerminalInputRequest {
    pub fn is_private(&self) -> bool {
        self.kind == TerminalInputKind::Private
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::{awaiting_user_input, terminal_input_request, TerminalInputKind};

    #[test]
    fn spots_the_prompts_a_human_has_to_answer() {
        for screen in [
            "building…\n[sudo] password for gotlib:",
            "Password:",
            "Enter passphrase for key '/home/u/.ssh/id_ed25519':",
            "Are you sure you want to continue connecting (yes/no)?",
            "Remove 3 packages? [y/N]",
            "Enter verification code:",
        ] {
            assert!(
                awaiting_user_input(screen).is_some(),
                "missed a prompt: {screen:?}"
            );
        }
    }

    #[test]
    fn ordinary_output_is_not_a_prompt() {
        for screen in [
            "",
            "Compiling hive-core v0.2.1",
            "note: password rotation is documented in SECURITY.md",
            "$ ",
            "Finished in 3.2s",
        ] {
            assert_eq!(
                awaiting_user_input(screen),
                None,
                "false positive on {screen:?}"
            );
        }
    }

    #[test]
    fn reports_the_prompt_line_itself() {
        let got = awaiting_user_input("x\n[sudo] password for gotlib:").unwrap();
        assert_eq!(got, "[sudo] password for gotlib:");
    }

    #[test]
    fn separates_private_input_from_safe_confirmation() {
        let private = terminal_input_request("Enter verification code:").unwrap();
        assert_eq!(private.kind, TerminalInputKind::Private);
        assert!(private.is_private());

        let confirm = terminal_input_request("Install packages? [y/N]").unwrap();
        assert_eq!(confirm.kind, TerminalInputKind::Confirmation);
        assert!(!confirm.is_private());
    }
}
