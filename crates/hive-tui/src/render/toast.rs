//! Short-lived status notifications — a filled chip in the bottom-right.

use comb::{Color, Frame, Line, Rect, Span, Style};

use crate::app::App;
use crate::render::strip_paint;

const INSET: u16 = 1;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;
/// Long toasts wrap/ellipsis instead of stretching across the chat.
const MAX_CHIP_W: u16 = 36;

/// Draw active Error / Info notifications in the bottom-right of `area`.
/// Error stacks above Info. Strip fill with a pad row above and below the text.
pub fn draw(f: &mut Frame, area: Rect, app: &App) -> bool {
    if area.width < 8 || area.height == 0 {
        return false;
    }
    let theme = &app.theme;
    let bg = theme.strip;
    let max_w = area
        .width
        .saturating_sub(INSET.saturating_mul(2))
        .min(MAX_CHIP_W);
    let mut cards: Vec<(&str, Color)> = Vec::new();
    if let Some(msg) = app.error_toast() {
        cards.push((msg, theme.err));
    }
    if let Some(msg) = app.flash_text() {
        cards.push((msg, theme.fg));
    }
    if cards.is_empty() {
        return false;
    }

    // Bottom-up: Info sits on the floor, Error stacks above it.
    let mut y = area.bottom().saturating_sub(INSET);
    for (msg, fg) in cards.into_iter().rev() {
        if y <= area.y {
            break;
        }
        let h = chip_h(
            area.bottom()
                .saturating_sub(area.y)
                .min(y.saturating_sub(area.y)),
        );
        if h == 0 {
            break;
        }
        y = y.saturating_sub(h);
        if let Some(chip) = layout_chip(area, y, max_w, h, msg, fg) {
            paint_card(f, &chip, bg);
        }
    }
    true
}

fn chip_h(max_h: u16) -> u16 {
    (1 + PAD_Y * 2).min(max_h)
}

/// One toast card with its on-screen placement.
struct Chip {
    rect: Rect,
    msg: String,
    fg: Color,
}

fn layout_chip(area: Rect, y: u16, max_w: u16, h: u16, msg: &str, fg: Color) -> Option<Chip> {
    if h == 0 || max_w == 0 {
        return None;
    }
    let inner = max_w.saturating_sub(PAD_X * 2).max(1) as usize;
    let body = if msg.chars().count() > inner && inner > 1 {
        let mut s: String = msg.chars().take(inner - 1).collect();
        s.push('…');
        s
    } else {
        msg.to_string()
    };
    let w = (body.chars().count() as u16)
        .saturating_add(PAD_X * 2)
        .clamp(1, max_w);
    let x = area
        .right()
        .saturating_sub(INSET)
        .saturating_sub(w)
        .max(area.x);
    Some(Chip {
        rect: Rect::new(x, y, w, h),
        msg: body,
        fg,
    })
}

fn paint_card(f: &mut Frame, chip: &Chip, bg: Color) {
    let pad = " ".repeat(PAD_X as usize);
    let text = format!("{pad}{}", chip.msg);
    let text_row = if chip.rect.height > PAD_Y { PAD_Y } else { 0 };
    let mut lines = Vec::new();
    for row in 0..chip.rect.height {
        if row == text_row {
            lines.push(Line::from(Span::styled(
                text.clone(),
                Style::default().fg(chip.fg).bg(bg),
            )));
        } else {
            lines.push(Line::from(""));
        }
    }
    strip_paint::set_lines_on_strip(f.buffer(), chip.rect, &lines, 0, bg);
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::TuiInit;
    use comb::{render, Size};

    fn app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: Vec::new(),
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
            ui: Default::default(),
            context_window: 128_000,
            cost_input: 0.0,
            cost_output: 0.0,
        })
    }

    fn find_copied(buf: &comb::Buffer, size: Size) -> Option<(u16, u16)> {
        for y in 0..size.height {
            for x in 0..size.width {
                if buf.get(x, y).is_some_and(|c| c.ch == 'C') {
                    let slice: String = (0..6)
                        .filter_map(|i| buf.get(x + i, y).map(|c| c.ch))
                        .collect();
                    if slice.starts_with("Copied") {
                        return Some((x, y));
                    }
                }
            }
        }
        None
    }

    #[test]
    fn notification_sits_bottom_right_with_pad_rows() {
        let mut a = app();
        a.push_user("hi".into());
        a.flash("Copied");
        let size = Size::new(80, 24);
        let buf = render(size, |f| crate::render::draw(f, &mut a));
        let text = buf.text();
        assert!(text.contains("Copied"), "{text}");
        assert!(!text.contains("Info"), "no kind label: {text}");

        let (x, y) = find_copied(&buf, size).expect("Copied glyph");
        assert!(y >= size.height / 2, "lower half, not the top: y={y}");
        assert!(x >= size.width / 2, "right half, not the left edge: x={x}");
        let cell = buf.get(x, y).unwrap();
        assert_eq!(cell.style.bg, Some(a.theme.strip), "filled chip");
        // Pad rows above and below the text share the strip fill.
        if y > 0 {
            assert_eq!(
                buf.get(x, y - 1).and_then(|c| c.style.bg),
                Some(a.theme.strip),
                "pad row above"
            );
        }
        if y + 1 < size.height {
            assert_eq!(
                buf.get(x, y + 1).and_then(|c| c.style.bg),
                Some(a.theme.strip),
                "pad row below"
            );
        }

        let mut top = String::new();
        for col in 0..size.width {
            top.push(buf.get(col, 0).map(|c| c.ch).unwrap_or(' '));
        }
        assert!(
            !top.contains("Copied"),
            "must not sit on the first row: {top:?}"
        );
    }
}
