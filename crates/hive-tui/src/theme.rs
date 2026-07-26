//! Colour palette: quiet grayscale. Only ok/err status icons keep a muted
//! tint so state is readable at a glance. All widgets pull colours from here.

use comb::Color;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// Elevation ramp for the grey surfaces. Everything sits on one of these, and
/// hover is always the same step up from whatever it's on — a single shared
/// hover colour makes the lift look twice as strong on a dark surface as on a
/// light one.
const SUNKEN: u8 = 0x1a;
/// App surfaces: cards in the transcript, floating panels.
const SURFACE: u8 = 0x24;
/// Yours: the composer, and the messages you already sent.
const MINE: u8 = 0x2e;
/// How far hover lifts a surface.
const LIFT: u8 = 0x0e;

const fn grey(v: u8) -> Color {
    Color::Rgb(v, v, v)
}

const fn lifted(v: u8) -> Color {
    grey(v.saturating_add(LIFT))
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
    /// The composer — same tone as your sent messages, so "where you type" and
    /// "what you typed" read as one surface.
    pub input: Color,
    /// Your own prompts in the transcript.
    pub user_strip: Color,
    /// Hover on one of your prompts — the same lift the cards get.
    pub user_strip_hover: Color,
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
            strip: grey(SURFACE),
            strip_hover: lifted(SURFACE),
            input: grey(MINE),
            user_strip: grey(MINE),
            user_strip_hover: lifted(MINE),
            assistant_selection_bg: rgb(0x34, 0x34, 0x34),
            mark_pending_bg: rgb(0x4a, 0x4a, 0x4a),
            mark_pending_fg: rgb(0xe0, 0xe0, 0xe0),
            mark_bg: rgb(0x3a, 0x72, 0x98),
            mark_fg: rgb(0xe8, 0xf2, 0xfa),
            code_fg: rgb(0xbd, 0xbd, 0xbd),
            code_bg: grey(SUNKEN),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn level(c: Color) -> u8 {
        match c {
            Color::Rgb(v, g, b) => {
                assert_eq!((v, v), (g, b), "the surfaces are neutral greys");
                v
            }
            Color::Reset => panic!("a surface must be a real colour"),
        }
    }

    #[test]
    fn the_surfaces_form_one_ramp() {
        let t = Theme::gray();

        // Sunken below app surfaces, and your own surfaces above them.
        assert!(level(t.code_bg) < level(t.strip));
        assert!(level(t.strip) < level(t.user_strip));

        // The composer and the messages you sent are the same surface.
        assert_eq!(level(t.input), level(t.user_strip));
    }

    #[test]
    fn hover_lifts_every_surface_by_the_same_step() {
        let t = Theme::gray();
        let card = level(t.strip_hover) - level(t.strip);
        let mine = level(t.user_strip_hover) - level(t.user_strip);
        assert_eq!(
            card, mine,
            "one shared hover colour made the lift depend on what you hovered"
        );
        assert!(card > 0);
    }
}
