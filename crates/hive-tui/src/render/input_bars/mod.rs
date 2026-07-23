//! Input bars: main chat composer vs special-view controls (back / MAKE).

mod chat;
mod special;
mod terminal;

pub use chat::{draw, draw_follow_up, draw_mode_chip, text_cols};
pub use special::{draw_back, draw_plan_bar, PLAN_ACTION_COLS};
pub use terminal::draw_terminal_bar;

pub(crate) const BACK: &str = "← back";
