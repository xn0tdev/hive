mod buffer;
mod manager;
mod session;
mod types;

pub use manager::TerminalManager;
pub use session::TerminalOutputFrame;
pub use types::{
    awaiting_user_input, TerminalController, TerminalError, TerminalKey, TerminalProcessState,
    TerminalReadResult, TerminalSnapshot, TerminalWriteRequest,
};
pub type TerminalHandle = std::sync::Arc<TerminalManager>;

pub const DEFAULT_ROWS: u16 = 30;
pub const DEFAULT_COLS: u16 = 120;
pub const SCROLLBACK_ROWS: usize = 10_000;
pub const OUTPUT_JOURNAL_CAP: usize = 1024 * 1024;
pub const MAX_READ_WAIT_MS: u64 = 5_000;

pub fn is_terminal_tool(name: &str) -> bool {
    matches!(
        name,
        "terminal_start" | "terminal_read" | "terminal_write" | "terminal_stop"
    )
}
