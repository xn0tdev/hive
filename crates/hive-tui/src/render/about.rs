//! Centered About overlay — HIVE wordmark, version, short tagline.

use comb::{Buffer, Color, Line, ModalLayout, Rect, Span, Style};

use crate::app::App;
use crate::render::panel::Panel;
use crate::render::wordmark;

const MIN_W: u16 = 40;
const MAX_W: u16 = 52;
const PAD_X: u16 = 3;
const PAD_Y: u16 = 1;

/// Short description of Hive (wrapped to panel width at draw time).
pub const TAGLINE: &str =
    "A YOLO coding agent for your terminal — tools, swarm, and skills, no confirmations.";

const HINT: &str = "esc close  ·  Ctrl+P commands";

fn overlay() -> Panel<'static> {
    // title + gap + wordmark + gap + version + gap + up to 3 tagline + gap + hint
    let h = (PAD_Y * 2 + 1 + 1 + wordmark::HEIGHT + 1 + 1 + 1 + 3 + 1 + 1)
        .max(PAD_Y * 2 + wordmark::HEIGHT + 6);
    Panel::new("About", "esc", h)
        .width_bounds(MIN_W, MAX_W)
        .padding(PAD_X, PAD_Y)
}

fn geom(area: Rect) -> ModalLayout {
    overlay().layout(area)
}

/// The card rect, so a click off it can dismiss the overlay.
pub fn window_rect(area: Rect, app: &App) -> Option<Rect> {
    app.about_open().then(|| geom(area).panel)
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    if !app.about_open() {
        return;
    }
    let theme = &app.theme;
    let panel = theme.strip;
    let panel_style = Style::default().bg(panel);
    let g = overlay().render(buf, area, theme);

    if g.content.width < 12 || g.content.height < 8 {
        return;
    }

    let mut y = g.content.y;
    y += 2; // title + breathing room

    // Block-letter HIVE wordmark, centered. Use strip paint so letter gaps
    // (spaces in the ASCII art) keep the panel bg, not terminal default black.
    let mark = wordmark::lines(theme, None);
    let art_x = g.content.x + g.content.width.saturating_sub(wordmark::WIDTH) / 2;
    if y + wordmark::HEIGHT <= g.content.bottom() {
        crate::render::strip_paint::set_lines_on_strip(
            buf,
            Rect::new(art_x, y, wordmark::WIDTH, wordmark::HEIGHT),
            &mark,
            0,
            panel,
        );
    }
    y += wordmark::HEIGHT + 1;

    // Version.
    if y < g.content.bottom() {
        let ver = format!("v{}", app.version);
        center_line(
            buf,
            g.content.x,
            y,
            g.content.width,
            &ver,
            Style::default().fg(theme.dim).bg(panel),
            panel_style,
        );
        y += 2;
    }

    // Tagline — wrapped, centered lines.
    let wrap_w = g.content.width as usize;
    let lines = wrap_words(TAGLINE, wrap_w);
    for line in lines.iter().take(3) {
        if y >= g.content.bottom() {
            break;
        }
        center_line(
            buf,
            g.content.x,
            y,
            g.content.width,
            line,
            Style::default().fg(theme.faint).bg(panel),
            panel_style,
        );
        y += 1;
    }

    // Key hints at the bottom of the panel.
    let hint_y = g.content.bottom().saturating_sub(1);
    if hint_y > y {
        // Clear middle rows so the panel stays solid.
        for yy in y..hint_y {
            buf.paint(Rect::new(g.content.x, yy, g.content.width, 1), panel_style);
        }
    }
    if hint_y >= g.content.y {
        center_line(
            buf,
            g.content.x,
            hint_y,
            g.content.width,
            HINT,
            Style::default().fg(theme.faint).bg(panel),
            panel_style,
        );
    }
}

fn center_line(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    text: &str,
    style: Style,
    fill: Style,
) {
    let tw = text.chars().count() as u16;
    let ox = x + width.saturating_sub(tw) / 2;
    buf.paint(Rect::new(x, y, width, 1), fill);
    let line = Line::from(Span::styled(text.to_string(), style));
    let bg = fill.bg.unwrap_or(Color::Reset);
    crate::render::strip_paint::set_line_on_strip(buf, ox, y, &line, tw.min(width), bg);
}

fn wrap_words(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur = word.to_string();
            continue;
        }
        if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(cur);
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TuiInit;
    use comb::{render, Size};

    fn app_with_about() -> App {
        let mut a = App::new(TuiInit {
            model: "m".into(),
            model_display: "Kimi 2.6".into(),
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
        });
        a.open_about();
        a
    }

    #[test]
    fn about_is_centered_overlay() {
        let _a = app_with_about();
        let size = Size::new(80, 24);
        let area = Rect::new(0, 0, size.width, size.height);
        let g = geom(area);
        assert!(g.panel.width <= MAX_W);
        assert_eq!(g.panel.x, (size.width - g.panel.width) / 2);
        assert_eq!(g.panel.y, (size.height - g.panel.height) / 2);
    }

    #[test]
    fn about_renders_wordmark_version_and_tagline() {
        let mut a = app_with_about();
        let buf = render(Size::new(80, 24), |f| {
            crate::render::draw(f, &mut a);
        });
        let text = buf.text();
        assert!(text.contains("About"), "{text}");
        assert!(text.contains("v0.1.0"), "{text}");
        // Block-letter HIVE wordmark (█ glyphs), not the bee art.
        assert!(text.contains('█'), "expected HIVE wordmark blocks: {text}");
        assert!(text.contains("YOLO"), "{text}");
        assert!(
            !text.contains("(o.o)") && !text.contains("~hive~"),
            "bee art should not appear on About: {text}"
        );
        assert!(
            !text.contains('╭') && !text.contains('╮'),
            "no hard border: {text}"
        );
    }

    #[test]
    fn wrap_words_keeps_short_lines() {
        let lines = wrap_words("one two three four five", 10);
        assert!(lines.iter().all(|l| l.chars().count() <= 10));
        assert!(lines.len() >= 2);
    }

    #[test]
    fn about_wordmark_spaces_keep_panel_bg() {
        let mut a = app_with_about();
        let panel = a.theme.strip;
        let buf = render(Size::new(80, 24), |f| {
            crate::render::draw(f, &mut a);
        });

        // Locate a solid block of the HIVE wordmark, then a space cell in that
        // same art row (letter gap). Space must share the panel strip bg.
        let mut block: Option<(u16, u16)> = None;
        for y in 0..buf.height {
            for x in 0..buf.width {
                if buf.get(x, y).map(|c| c.ch) == Some('█') {
                    block = Some((x, y));
                    break;
                }
            }
            if block.is_some() {
                break;
            }
        }
        let (bx, by) = block.expect("wordmark block glyph");

        let mut gap: Option<(u16, u16)> = None;
        for x in bx..buf.width.min(bx + wordmark::WIDTH) {
            if buf.get(x, by).map(|c| c.ch) == Some(' ') {
                gap = Some((x, by));
                break;
            }
        }
        let (gx, gy) = gap.expect("space inside wordmark row");
        let cell = buf.get(gx, gy).unwrap();
        assert_eq!(
            cell.style.bg,
            Some(panel),
            "wordmark gap at ({gx},{gy}) must use panel strip bg, got {:?}",
            cell.style.bg
        );
    }
}
