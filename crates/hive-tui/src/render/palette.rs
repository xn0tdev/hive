//! Centered command palette / model picker overlay (Ctrl+P).

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::palette::{PaletteMode, PaletteState};
use crate::app::App;
use crate::commands::PaletteRow;

const MAX_LIST: u16 = 14;
const MIN_W: u16 = 36;
const MAX_W: u16 = 48;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;
/// Title + search + gap before the list.
const CHROME_ROWS: u16 = 3;

struct PaletteGeom {
    win: Rect,
    content: Rect,
    search_y: u16,
    list: Rect,
}

fn geom(area: Rect, list_rows: u16) -> PaletteGeom {
    let w = (area.width / 2).clamp(MIN_W, MAX_W).min(area.width);
    let list_h = list_rows.clamp(1, MAX_LIST);
    let h = (PAD_Y * 2 + CHROME_ROWS + list_h)
        .min(area.height.saturating_sub(2))
        .max(PAD_Y * 2 + CHROME_ROWS + 1);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let win = Rect::new(x, y, w, h);
    let content = Rect {
        x: win.x + PAD_X,
        y: win.y + PAD_Y,
        width: win.width.saturating_sub(PAD_X * 2),
        height: win.height.saturating_sub(PAD_Y * 2),
    };
    let search_y = content.y + 1;
    let list_top = search_y + 2; // blank row under search
    let list_h = content
        .height
        .saturating_sub(CHROME_ROWS)
        .min(win.y + win.height - PAD_Y - list_top);
    let list = Rect::new(content.x, list_top, content.width, list_h);
    PaletteGeom {
        win,
        content,
        search_y,
        list,
    }
}

fn list_row_count(pal: &PaletteState, app: &App) -> u16 {
    match pal.mode {
        PaletteMode::Commands => pal.command_rows().len() as u16,
        PaletteMode::Models => {
            // Category header + model rows.
            1 + pal.model_rows(&app.model_choices).len() as u16
        }
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    let Some(pal) = app.palette.as_ref() else {
        return;
    };
    let theme = &app.theme;
    let panel = theme.strip;
    let panel_style = Style::default().bg(panel);

    let g = geom(area, list_row_count(pal, app));
    // Soft scrim behind the panel so it reads with a bit of depth.
    dim_outside(buf, area, g.win);
    buf.paint(g.win, panel_style);

    if g.content.width < 8 || g.content.height < 2 {
        return;
    }

    let title = match pal.mode {
        PaletteMode::Commands => "Commands",
        PaletteMode::Models => "Switch model",
    };
    draw_title(buf, g.content, title, theme.fg, theme.faint, panel);
    draw_search(
        buf,
        Rect::new(g.content.x, g.search_y, g.content.width, 1),
        pal,
        theme,
        panel,
    );
    // Breathing room under search (blank panel row).
    buf.paint(
        Rect::new(g.content.x, g.search_y + 1, g.content.width, 1),
        panel_style,
    );

    match pal.mode {
        PaletteMode::Commands => draw_command_list(buf, g.list, pal, theme, panel),
        PaletteMode::Models => draw_model_list(buf, g.list, pal, app, theme, panel),
    }
}

fn draw_title(buf: &mut Buffer, area: Rect, title: &str, fg: Color, faint: Color, bg: Color) {
    let esc = "esc";
    let base = Style::default().bg(bg);
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
            base,
        ),
        Span::styled(esc, Style::default().fg(faint).bg(bg)),
    ]);
    crate::render::strip_paint::set_line_on_strip(buf, area.x, area.y, &title_line, area.width, bg);
}

fn draw_search(
    buf: &mut Buffer,
    area: Rect,
    pal: &PaletteState,
    theme: &crate::theme::Theme,
    bg: Color,
) {
    let empty = pal.query.is_empty();
    let text = if empty { "Search" } else { pal.query.as_str() };
    let fg = if empty { theme.faint } else { theme.fg };
    let mut style_text = Style::default().fg(fg).bg(bg);
    if empty {
        style_text = style_text.add(Modifier::ITALIC);
    }
    let line = Line::from(vec![Span::styled(text.to_string(), style_text)]);
    crate::render::strip_paint::set_line_on_strip(buf, area.x, area.y, &line, area.width, bg);
    let _ = pal.cursor;
}

/// Darken cells outside `exclude` slightly (scrim / depth cue).
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

/// ~62% luminance — noticeable but not a heavy modal veil.
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

/// Screen position for the search caret when search is focused.
pub fn search_cursor(area: Rect, app: &App) -> Option<(u16, u16)> {
    let pal = app.palette.as_ref()?;
    if !pal.search_focused {
        return None;
    }
    let g = geom(area, list_row_count(pal, app));
    let caret_x = g.content.x + pal.cursor as u16;
    let caret_y = g.search_y;
    Some((
        caret_x.min(g.content.x + g.content.width.saturating_sub(1)),
        caret_y,
    ))
}

/// List viewport height in rows (for keeping keyboard selection painted).
pub fn list_visible(area: Rect, app: &App) -> u16 {
    let Some(pal) = app.palette.as_ref() else {
        return 0;
    };
    geom(area, list_row_count(pal, app)).list.height
}

fn draw_command_list(
    buf: &mut Buffer,
    area: Rect,
    pal: &PaletteState,
    theme: &crate::theme::Theme,
    panel: Color,
) {
    if area.height == 0 {
        return;
    }
    let panel_style = Style::default().bg(panel);
    let rows = pal.command_rows();
    if rows.is_empty() {
        let line = Line::from(Span::styled(
            "No matching commands",
            Style::default().fg(theme.faint).bg(panel),
        ));
        crate::render::strip_paint::set_line_on_strip(
            buf, area.x, area.y, &line, area.width, panel,
        );
        return;
    }

    // Scroll via list_offset, keeping the selection in view.
    let sel = pal.selected.min(rows.len() - 1);
    let visible = area.height as usize;
    let offset = pal.visible_offset(&[], visible);

    for row in 0..visible {
        let idx = offset + row;
        let y = area.y + row as u16;
        let Some(item) = rows.get(idx) else {
            buf.paint(Rect::new(area.x, y, area.width, 1), panel_style);
            continue;
        };
        match item {
            PaletteRow::Spacer => {
                buf.paint(Rect::new(area.x, y, area.width, 1), panel_style);
            }
            PaletteRow::Header(cat) => {
                let line = Line::from(Span::styled(
                    cat.label().to_string(),
                    Style::default()
                        .fg(theme.build)
                        .bg(panel)
                        .add(Modifier::BOLD),
                ));
                crate::render::strip_paint::set_line_on_strip(
                    buf, area.x, y, &line, area.width, panel,
                );
            }
            PaletteRow::Command(cmd) => {
                let is_sel = idx == sel;
                let bg = if is_sel { theme.sel_bg } else { panel };
                let row_base = Style::default().bg(bg);
                let name_fg = if is_sel { theme.sel_fg } else { theme.fg };
                let desc_fg = if is_sel { theme.sel_fg } else { theme.faint };
                let short_fg = if is_sel { theme.sel_fg } else { theme.dim };

                let shortcut = cmd.shortcut.unwrap_or("");
                let label = cmd.label;
                let desc = cmd.desc;
                let left = label.to_string();
                let left_w = left.chars().count();
                let short_w = shortcut.chars().count();
                let avail = area.width as usize;
                let mut spans = vec![Span::styled(
                    left,
                    Style::default().fg(name_fg).bg(bg),
                )];
                let mid_budget = avail.saturating_sub(left_w + short_w + 2);
                if mid_budget > 4 && !desc.is_empty() {
                    let d = ellipsize(desc, mid_budget.saturating_sub(1));
                    spans.push(Span::styled(
                        format!(" {d}"),
                        Style::default().fg(desc_fg).bg(bg),
                    ));
                }
                let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
                let gap = avail.saturating_sub(used + short_w);
                spans.push(Span::styled(" ".repeat(gap), row_base));
                if !shortcut.is_empty() {
                    spans.push(Span::styled(
                        shortcut.to_string(),
                        Style::default().fg(short_fg).bg(bg),
                    ));
                }
                crate::render::strip_paint::set_line_on_strip(
                    buf,
                    area.x,
                    y,
                    &Line::from(spans),
                    area.width,
                    bg,
                );
            }
        }
    }
}

fn draw_model_list(
    buf: &mut Buffer,
    area: Rect,
    pal: &PaletteState,
    app: &App,
    theme: &crate::theme::Theme,
    panel: Color,
) {
    if area.height == 0 {
        return;
    }
    let panel_style = Style::default().bg(panel);
    let rows = pal.model_rows(&app.model_choices);
    if rows.is_empty() {
        let line = Line::from(Span::styled(
            "No matching models",
            Style::default().fg(theme.faint).bg(panel),
        ));
        crate::render::strip_paint::set_line_on_strip(
            buf, area.x, area.y, &line, area.width, panel,
        );
        return;
    }

    let sel = pal.selected.min(rows.len() - 1);
    let visible = area.height as usize;
    let offset = pal.visible_offset(&app.model_choices, visible);

    let header_y = area.y;
    let header = Line::from(Span::styled(
        "Models",
        Style::default()
            .fg(theme.build)
            .bg(panel)
            .add(Modifier::BOLD),
    ));
    crate::render::strip_paint::set_line_on_strip(
        buf, area.x, header_y, &header, area.width, panel,
    );

    let list_top = area.y + 1;
    let list_h = area.height.saturating_sub(1) as usize;
    for row in 0..list_h {
        let idx = offset + row;
        let y = list_top + row as u16;
        let Some(choice) = rows.get(idx) else {
            buf.paint(Rect::new(area.x, y, area.width, 1), panel_style);
            continue;
        };
        let is_sel = idx == sel;
        let bg = if is_sel { theme.sel_bg } else { panel };
        let base = Style::default().bg(bg);
        let name_fg = if is_sel { theme.sel_fg } else { theme.fg };
        let detail_fg = if is_sel { theme.sel_fg } else { theme.faint };
        let current = choice.key == app.model
            || choice.display == app.model_display
            || (choice.key == "default" && app.model_display == choice.display);

        let mark = if current { "●" } else { " " };
        let left = format!("{mark} {}", choice.display);
        let right = if choice.detail.is_empty() {
            choice.key.clone()
        } else {
            choice.detail.clone()
        };
        let left_w = left.chars().count();
        let right_w = right.chars().count();
        let gap = (area.width as usize).saturating_sub(left_w + right_w);
        let line = Line::from(vec![
            Span::styled(left, Style::default().fg(name_fg).bg(bg)),
            Span::styled(" ".repeat(gap), base),
            Span::styled(right, Style::default().fg(detail_fg).bg(bg)),
        ]);
        crate::render::strip_paint::set_line_on_strip(buf, area.x, y, &line, area.width, bg);
    }
}

fn ellipsize(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TuiInit;
    use comb::{render, Size};

    fn app() -> App {
        let mut a = App::new(TuiInit {
            model: "m".into(),
            model_display: "Kimi 2.6".into(),
            model_choices: Vec::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
        });
        a.open_palette();
        a
    }

    fn panel_bg() -> Color {
        Color::Rgb(0x26, 0x26, 0x26)
    }

    #[test]
    fn palette_is_centered_and_compact() {
        let a = app();
        let size = Size::new(80, 24);
        let area = Rect::new(0, 0, size.width, size.height);
        let list_rows = list_row_count(a.palette.as_ref().unwrap(), &a);
        let g = geom(area, list_rows);

        assert!(g.win.width <= MAX_W, "width={}", g.win.width);
        assert!(g.win.width >= MIN_W.min(size.width));
        let expected_x = (size.width - g.win.width) / 2;
        let expected_y = (size.height - g.win.height) / 2;
        assert_eq!(g.win.x, expected_x);
        assert_eq!(g.win.y, expected_y);
    }

    #[test]
    fn palette_has_no_border_and_fills_header_bg() {
        let mut a = app();
        let buf = render(Size::new(80, 24), |f| {
            crate::render::draw(f, &mut a);
        });
        let text = buf.text();
        assert!(text.contains("Commands"), "{text}");
        assert!(text.contains("Suggested"), "{text}");
        assert!(
            !text.contains('╭') && !text.contains('╮') && !text.contains('╰') && !text.contains('╯'),
            "rounded border drawn: {text}"
        );

        // Find the Suggested header row and assert trailing cells keep panel bg.
        let mut header_y = None;
        for y in 0..buf.height {
            for x in 0..buf.width {
                if buf.get(x, y).map(|c| c.ch) == Some('S') {
                    // Look for "Suggested" starting here.
                    let slice: String = (0..9)
                        .filter_map(|i| buf.get(x + i, y).map(|c| c.ch))
                        .collect();
                    if slice.starts_with("Suggested") {
                        header_y = Some((x, y));
                        break;
                    }
                }
            }
            if header_y.is_some() {
                break;
            }
        }
        let (hx, hy) = header_y.expect("Suggested header");
        // Cell after the label should still be panel strip, not default/reset.
        let after = buf.get(hx + "Suggested".len() as u16 + 2, hy).unwrap();
        assert_eq!(after.style.bg, Some(panel_bg()), "header trailing bg");
        assert_eq!(after.ch, ' ');
    }

    #[test]
    fn search_has_no_cursor_until_focused() {
        let mut a = app();
        let area = Rect::new(0, 0, 80, 24);
        assert!(
            !a.palette.as_ref().unwrap().search_focused,
            "opens without search focus"
        );
        assert!(search_cursor(area, &a).is_none());

        a.palette.as_mut().unwrap().focus_search();
        let (cx, cy) = search_cursor(area, &a).expect("cursor when focused");
        let g = geom(area, list_row_count(a.palette.as_ref().unwrap(), &a));
        assert_eq!(cy, g.search_y);
        assert_eq!(cx, g.content.x);
    }

    #[test]
    fn typing_focuses_search() {
        let mut a = app();
        let choices = a.model_choices.clone();
        assert!(!a.palette.as_ref().unwrap().search_focused);
        a.palette.as_mut().unwrap().insert('f', &choices);
        assert!(a.palette.as_ref().unwrap().search_focused);
        assert_eq!(a.palette.as_ref().unwrap().query, "f");
    }

    #[test]
    fn scrim_dims_cells_outside_panel() {
        let a = app();
        let size = Size::new(80, 24);
        let area = Rect::new(0, 0, size.width, size.height);
        let list_rows = list_row_count(a.palette.as_ref().unwrap(), &a);
        let g = geom(area, list_rows);
        assert!(!g.win.contains(0, 0), "probe cell must be outside panel");

        let mut buf = Buffer::blank(size);
        let bright = Style::default()
            .fg(Color::Rgb(0xff, 0xff, 0xff))
            .bg(Color::Rgb(0x80, 0x80, 0x80));
        buf.set(0, 0, 'X', bright);
        draw(&mut buf, area, &a);

        let cell = buf.get(0, 0).unwrap();
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.style.fg, Some(Color::Rgb(0xa0, 0xa0, 0xa0))); // 255*160/255
        assert_eq!(cell.style.bg, Some(Color::Rgb(0x50, 0x50, 0x50))); // 128*160/255
        assert!(cell.style.mods.contains(Modifier::DIM));
    }

    #[test]
    fn list_visible_matches_geom() {
        let a = app();
        let area = Rect::new(0, 0, 80, 24);
        let expected = geom(area, list_row_count(a.palette.as_ref().unwrap(), &a))
            .list
            .height;
        assert_eq!(list_visible(area, &a), expected);
        assert!(expected > 0);
    }
}
