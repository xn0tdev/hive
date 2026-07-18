//! The braille spinner used across the UI for anything in flight.

pub const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn frame(tick: usize) -> &'static str {
    FRAMES[tick % FRAMES.len()]
}
