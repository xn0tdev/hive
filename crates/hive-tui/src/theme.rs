//! Colour palette: quiet grayscale. Only ok/err status icons keep a muted
//! tint so state is readable at a glance. All widgets pull colours from here.

use comb::Color;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

#[derive(Clone)]
pub struct Theme {
    pub fg: Color,
    pub dim: Color,
    pub faint: Color,
    /// Bright gray for the prompt arrow, spinner, and selected menu row.
    pub accent: Color,
    pub tool: Color,
    pub ok: Color,
    pub err: Color,
    pub warn: Color,
    /// PLAN mode chip background (warm / orange).
    pub plan: Color,
    /// BUILD mode chip background (cool / blue).
    pub build: Color,
    /// MULTITASK mode chip background (muted violet).
    pub multitask: Color,
    /// Subtle background strip for the input bar and user messages.
    pub strip: Color,
    pub code_fg: Color,
    pub code_bg: Color,
    pub heading: Color,
    /// Diff line colours (fg + row background) for code edits.
    pub add_fg: Color,
    pub add_bg: Color,
    pub del_fg: Color,
    pub del_bg: Color,
    /// Menu selection bar: light band with dark text.
    pub sel_fg: Color,
    pub sel_bg: Color,
}

impl Theme {
    pub fn gray() -> Self {
        Theme {
            fg: rgb(0xd4, 0xd4, 0xd4),
            dim: rgb(0x9c, 0x9c, 0x9c),
            faint: rgb(0x63, 0x63, 0x63),
            accent: rgb(0xe6, 0xe6, 0xe6),
            tool: rgb(0xb0, 0xb0, 0xb0),
            ok: rgb(0x98, 0xc3, 0x79),  // muted green, status only
            err: rgb(0xd1, 0x7b, 0x88), // muted red, status only
            warn: rgb(0xc9, 0xb4, 0x7f),
            plan: rgb(0xc4, 0x8a, 0x3a),
            build: rgb(0x5a, 0x8f, 0xb0),
            multitask: rgb(0x8a, 0x7a, 0xb0),
            strip: rgb(0x26, 0x26, 0x26),
            code_fg: rgb(0xbd, 0xbd, 0xbd),
            code_bg: rgb(0x1c, 0x1c, 0x1c),
            heading: rgb(0xe6, 0xe6, 0xe6),
            add_fg: rgb(0xc0, 0xe8, 0xc4),
            add_bg: rgb(0x1d, 0x3a, 0x24),
            del_fg: rgb(0xf0, 0xc2, 0xc8),
            del_bg: rgb(0x40, 0x22, 0x28),
            sel_fg: rgb(0x1a, 0x1a, 0x1a),
            sel_bg: rgb(0xd8, 0xd8, 0xd8),
        }
    }

    pub fn from_name(_name: &str) -> Self {
        // Only one theme for now; more can be added here.
        Theme::gray()
    }
}
