//! The braille spinner used across the UI for anything in flight.

pub const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Stable glyph baked into cached transcript lines. Live frames are painted
/// over this cell so a spinner tick does not rebuild the conversation.
pub const PLACEHOLDER: &str = FRAMES[0];

pub fn frame(tick: usize) -> &'static str {
    FRAMES[tick % FRAMES.len()]
}

pub fn glyph(tick: usize) -> char {
    frame(tick).chars().next().unwrap_or('⠋')
}
