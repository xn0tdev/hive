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
    /// MULTITASK mode chip background (soft lilac).
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
            fg: rgb(0xdc, 0xdc, 0xdc),
            dim: rgb(0xaa, 0xaa, 0xaa),
            faint: rgb(0x78, 0x78, 0x78),
            accent: rgb(0xee, 0xee, 0xee),
            tool: rgb(0xbc, 0xbc, 0xbc),
            ok: rgb(0xa6, 0xcc, 0x8a),  // muted green, status only
            err: rgb(0xdc, 0x8e, 0x9a), // muted red, status only
            warn: rgb(0xd4, 0xc0, 0x8e),
            plan: rgb(0xd4, 0xa0, 0x4e),
            build: rgb(0x7a, 0xad, 0xc8),
            multitask: rgb(0xbe, 0xb0, 0xdc),
            strip: rgb(0x32, 0x32, 0x32),
            code_fg: rgb(0xc8, 0xc8, 0xc8),
            code_bg: rgb(0x26, 0x26, 0x26),
            heading: rgb(0xee, 0xee, 0xee),
            add_fg: rgb(0xcc, 0xee, 0xd0),
            add_bg: rgb(0x28, 0x48, 0x30),
            del_fg: rgb(0xf4, 0xce, 0xd4),
            del_bg: rgb(0x4c, 0x2c, 0x32),
            sel_fg: rgb(0x1a, 0x1a, 0x1a),
            sel_bg: rgb(0xe0, 0xe0, 0xe0),
        }
    }

    pub fn from_name(_name: &str) -> Self {
        // Only one theme for now; more can be added here.
        Theme::gray()
    }
}
