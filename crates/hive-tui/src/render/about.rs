//! Centered About overlay — HIVE wordmark, version, short tagline.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::App;
use crate::render::wordmark;

const MIN_W: u16 = 40;
const MAX_W: u16 = 52;
const PAD_X: u16 = 3;
const PAD_Y: u16 = 1;

/// Short description of Hive (wrapped to panel width at draw time).
pub const TAGLINE: &str =
    "A YOLO coding agent for your terminal — tools, swarm, and skills, no confirmations.";

const HINT: &str = "esc close  ·  Ctrl+P commands";

struct AboutGeom {
    win: Rect,
    content: Rect,
}

fn geom(area: Rect) -> AboutGeom {
    let w = (area.width * 2 / 3).clamp(MIN_W, MAX_W).min(area.width);
    // title + gap + wordmark + gap + version + gap + up to 3 tagline + gap + hint
    let h = (PAD_Y * 2 + 1 + 1 + wordmark::HEIGHT + 1 + 1 + 1 + 3 + 1 + 1)
        .min(area.height.saturating_sub(2))
        .max(PAD_Y * 2 + wordmark::HEIGHT + 6);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let win = Rect::new(x, y, w, h);
    let content = Rect {
        x: win.x + PAD_X,
        y: win.y + PAD_Y,
        width: win.width.saturating_sub(PAD_X * 2),
        height: win.height.saturating_sub(PAD_Y * 2),
    };
    AboutGeom { win, content }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    if !app.about_open() {
        return;
    }
    let theme = &app.theme;
    let panel = theme.strip;
    let panel_style = Style::default().bg(panel);
    let g = geom(area);

    dim_outside(buf, area, g.win);
    buf.paint(g.win, panel_style);

    if g.content.width < 12 || g.content.height < 8 {
        return;
    }

    let mut y = g.content.y;
    draw_title(buf, g.content, theme.fg, theme.faint, panel);
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

fn draw_title(buf: &mut Buffer, area: Rect, fg: Color, faint: Color, bg: Color) {
    let title = "About";
    let esc = "esc";
    let pad = Style::default().bg(bg);
    let title_line = Line::from(vec![
        Span::styled(
            title.to_string(),
            Style::default().fg(fg).bg(bg).add(Modifier::BOLD),
        ),
        Span::styled(
            " ".repeat(
                area.width
                    .saturating_sub(title.chars().count() as u16 + esc.len() as u16)
                    as usize,
            ),
            pad,
        ),
        Span::styled(esc, Style::default().fg(faint).bg(bg)),
    ]);
    crate::render::strip_paint::set_line_on_strip(buf, area.x, area.y, &title_line, area.width, bg);
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

fn dim_outside(buf: &mut Buffer, area: Rect, exclude: Rect) {
    let area = area.intersection(buf.area());
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if exclude.contains(x, y) {
                continue;
            }
            if let Some(cell) = buf.cell_mut(x, y) {
                if let Some(fg) = cell.style.fg {
                    cell.style.fg = Some(darken_color(fg));
                }
                cell.style.bg = Some(match cell.style.bg {
                    Some(bg) => darken_color(bg),
                    None => Color::Rgb(0x0a, 0x0a, 0x0a),
                });
                cell.style = cell.style.add(Modifier::DIM);
            }
        }
    }
}

fn darken_color(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            ((r as u16 * 160) / 255) as u8,
            ((g as u16 * 160) / 255) as u8,
            ((b as u16 * 160) / 255) as u8,
        ),
        Color::Reset => Color::Rgb(0x0a, 0x0a, 0x0a),
    }
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
        assert!(g.win.width <= MAX_W);
        assert_eq!(g.win.x, (size.width - g.win.width) / 2);
        assert_eq!(g.win.y, (size.height - g.win.height) / 2);
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
