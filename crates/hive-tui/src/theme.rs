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
    /// MAKE mode chip background (cool / blue).
    pub make: Color,
    /// Slightly lighter MAKE for hover on the plan-view action button.
    pub make_hover: Color,
    /// MULTITASK mode chip background (clear violet).
    pub multitask: Color,
    /// Subtle background strip for the input bar and user messages.
    pub strip: Color,
    /// Lighter strip for hover on clickable cards / back control.
    pub strip_hover: Color,
    /// Assistant-response drag selection. Foreground styling is preserved.
    pub assistant_selection_bg: Color,
    /// Pending plan selection (drag / mark without a note yet) — gray wash.
    pub mark_pending_bg: Color,
    pub mark_pending_fg: Color,
    /// Noted plan mark (after MARK) — light blue wash.
    pub mark_bg: Color,
    pub mark_fg: Color,
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
    /// GOAL mode chip background (warm green).
    pub goal: Color,
}

impl Theme {
    pub fn gray() -> Self {
        Theme {
            fg: rgb(0xd4, 0xd4, 0xd4),
            dim: rgb(0x9c, 0x9c, 0x9c),
            faint: rgb(0x63, 0x63, 0x63),
            accent: rgb(0xe6, 0xe6, 0xe6),
            tool: rgb(0xb0, 0xb0, 0xb0),
            ok: rgb(0x8f, 0xc4, 0x78),  // clear green, status only
            err: rgb(0xd4, 0x74, 0x82), // clear rose, status only
            warn: rgb(0xc9, 0xaa, 0x5a),
            plan: rgb(0xc8, 0x8e, 0x2e),
            make: rgb(0x4a, 0x96, 0xc4),
            make_hover: rgb(0x5e, 0xa8, 0xd0),
            multitask: rgb(0x8e, 0x78, 0xd0),
            strip: rgb(0x26, 0x26, 0x26),
            strip_hover: rgb(0x3a, 0x3a, 0x3a),
            assistant_selection_bg: rgb(0x34, 0x34, 0x34),
            mark_pending_bg: rgb(0x4a, 0x4a, 0x4a),
            mark_pending_fg: rgb(0xe0, 0xe0, 0xe0),
            mark_bg: rgb(0x3a, 0x72, 0x98),
            mark_fg: rgb(0xe8, 0xf2, 0xfa),
            code_fg: rgb(0xbd, 0xbd, 0xbd),
            code_bg: rgb(0x1c, 0x1c, 0x1c),
            heading: rgb(0xe6, 0xe6, 0xe6),
            add_fg: rgb(0xb8, 0xe4, 0xbc),
            add_bg: rgb(0x1d, 0x3a, 0x24),
            del_fg: rgb(0xec, 0xb8, 0xc0),
            del_bg: rgb(0x40, 0x22, 0x28),
            sel_fg: rgb(0x1a, 0x1a, 0x1a),
            sel_bg: rgb(0xd8, 0xd8, 0xd8),
            goal: rgb(0x5a, 0x8a, 0x4e),
        }
    }

    pub fn from_name(_name: &str) -> Self {
        // Only one theme for now; more can be added here.
        Theme::gray()
    }
}
