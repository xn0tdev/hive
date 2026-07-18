//! Paint text onto strip / panel backgrounds without black holes.
//!
//! Overlays (About, command palette, …) sit on `theme.strip`. Prefer these
//! helpers over bare [`Buffer::set_line`] / [`Buffer::set_lines`]: those leave
//! space cells and short-line tails with the terminal default background, which
//! shows up as a darker rectangle inside the lighter grey panel (ASCII art gaps,
//! header tails after "Suggested", etc.).
//!
//! Under the hood this is [`Buffer::set_line_on`] / [`Buffer::set_lines_on`] with
//! a strip-bg base style — every glyph and space is patched, and the remainder
//! of each row is cleared to that bg.

use comb::{Buffer, Color, Line, Rect, Style};

fn base(bg: Color) -> Style {
    Style::default().bg(bg)
}

/// Draw a line onto a strip/panel row. Spaces and the cleared remainder keep
/// `strip_bg` — never the terminal default.
pub fn set_line_on_strip(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    line: &Line,
    max_width: u16,
    strip_bg: Color,
) {
    buf.set_line_on(x, y, line, max_width, base(strip_bg));
}

/// Draw multiple lines (ASCII art, lists, …) onto a strip/panel rect.
/// Same bg guarantee as [`set_line_on_strip`] for every cell in the area.
pub fn set_lines_on_strip(
    buf: &mut Buffer,
    area: Rect,
    lines: &[Line],
    scroll: usize,
    strip_bg: Color,
) {
    buf.set_lines_on(area, lines, scroll, base(strip_bg));
}
