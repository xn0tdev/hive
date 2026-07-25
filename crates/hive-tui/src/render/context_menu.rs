//! Centered context menu overlay for transcript block actions
//! (Copy, Recall, Revert). Shown when clicking a user prompt or tool card.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::App;

const MIN_W: u16 = 28;
const MAX_W: u16 = 52;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    let Some(menu) = &app.context_menu else {
        return;
    };

    let n = menu.items.len() as u16;
    let prefer_w = menu
        .items
        .iter()
        .map(|i| i.label.chars().count() as u16 + 6)
        .max()
        .unwrap_or(MIN_W)
        .clamp(MIN_W, MAX_W);
    let h = n + 3; // top border + title + items + hint
    let w = prefer_w;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let win = Rect::new(x, y, w, h);

    dim_outside(buf, area, win);

    let theme = &app.theme;
    let bg = theme.strip;

    // Title row.
    let title_style = Style::default().fg(theme.fg).bg(bg).add(Modifier::BOLD);
    let title_line = Line::from(Span::styled("  Actions", title_style));
    crate::render::strip_paint::set_line_on_strip(buf, win.x, win.y, &title_line, win.width, bg);

    // Item rows.
    for (i, item) in menu.items.iter().enumerate() {
        let row_y = win.y + 1 + i as u16;
        let selected = i == menu.selected;
        let row_bg = if selected {
            theme.strip_hover
        } else {
            bg
        };
        let fg = if selected { theme.fg } else { theme.dim };
        let style = Style::default().fg(fg).bg(row_bg);
        let prefix = if selected { " › " } else { "   " };
        let label_line = Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(&item.label, style),
        ]);
        crate::render::strip_paint::set_line_on_strip(
            buf, win.x, row_y, &label_line, win.width, row_bg,
        );
    }

    // Bottom hint row.
    let hint_y = win.y + 1 + n;
    let hint_style = Style::default().fg(theme.faint).bg(bg);
    let hint_line = Line::from(Span::styled("  ↑↓ select · Enter · Esc", hint_style));
    crate::render::strip_paint::set_line_on_strip(buf, win.x, hint_y, &hint_line, win.width, bg);
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
