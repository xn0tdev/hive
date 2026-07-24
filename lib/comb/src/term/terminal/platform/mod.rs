//! OS-specific raw console enter/leave, size, and stdin wait/read.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
pub use windows::*;

/// Result of waiting for stdin to become readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    Ready,
    Timeout,
}
