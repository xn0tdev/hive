//! Centered context menu overlay for transcript block actions
//! (Copy, Recall, Revert). Styled like the Ctrl+P command palette.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::App;

const MIN_W: u16 = 36;
const MAX_W: u16 = 52;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;

struct MenuGeom {
    win: Rect,
    content: Rect,
    list: Rect,
}

fn geom(area: Rect, n_items: u16) -> MenuGeom {
    let prefer_w = area.width / 2;
    let w = prefer_w.clamp(MIN_W, MAX_W).min(area.width);
    let list_h = n_items.max(1);
    // pad + title + gap + items
    let h = (PAD_Y * 2 + 2 + list_h).min(area.height.saturating_sub(2));
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let win = Rect::new(x, y, w, h);
    let content = Rect {
        x: win.x + PAD_X,
        y: win.y + PAD_Y,
        width: win.width.saturating_sub(PAD_X * 2),
        height: win.height.saturating_sub(PAD_Y * 2),
    };
    // title row, then a blank gap, then items
    let list = Rect::new(content.x, content.y + 2, content.width, list_h);
    MenuGeom { win, content, list }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    app.context_menu_win = None;
    app.context_menu_hits.clear();
    let Some(menu) = app.context_menu.as_ref() else {
        return;
    };
    let theme = &app.theme;
    let panel = theme.strip;
    let panel_style = Style::default().bg(panel);

    let g = geom(area, menu.items.len() as u16);
    let win = g.win;
    let hits: Vec<(Rect, usize)> = (0..menu.items.len())
        .map(|i| (Rect::new(g.list.x, g.list.y + i as u16, g.list.width, 1), i))
        .filter(|(rect, _)| rect.y < g.list.bottom())
        .collect();
    dim_outside(buf, area, g.win);
    buf.paint(g.win, panel_style);

    if g.content.width < 8 {
        return;
    }

    // Title row with "esc" on the right (like the palette).
    let title = "Actions";
    let esc = "esc";
    let base = Style::default().bg(panel);
    let title_line = Line::from(vec![
        Span::styled(
            title.to_string(),
            Style::default().fg(theme.fg).bg(panel).add(Modifier::BOLD),
        ),
        Span::styled(
            " ".repeat(
                g.content
                    .width
                    .saturating_sub(title.chars().count() as u16 + esc.len() as u16)
                    as usize,
            ),
            base,
        ),
        Span::styled(esc, Style::default().fg(theme.faint).bg(panel)),
    ]);
    crate::render::strip_paint::set_line_on_strip(
        buf,
        g.content.x,
        g.content.y,
        &title_line,
        g.content.width,
        panel,
    );

    // Blank gap row between title and items.
    let gap_line = Line::from(Span::styled("", base));
    crate::render::strip_paint::set_line_on_strip(
        buf,
        g.content.x,
        g.content.y + 1,
        &gap_line,
        g.content.width,
        panel,
    );

    // Item rows.
    for (i, item) in menu.items.iter().enumerate() {
        let row_y = g.list.y + i as u16;
        if row_y >= g.list.bottom() {
            break;
        }
        let is_sel = i == menu.selected;
        let bg = if is_sel { theme.sel_bg } else { panel };
        let name_fg = if is_sel { theme.sel_fg } else { theme.fg };
        let style = Style::default().fg(name_fg).bg(bg);
        let line = Line::from(vec![Span::styled(&item.label, style)]);
        crate::render::strip_paint::set_line_on_strip(
            buf,
            g.list.x,
            row_y,
            &line,
            g.list.width,
            bg,
        );
    }

    app.context_menu_win = Some(win);
    app.context_menu_hits = hits;
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
