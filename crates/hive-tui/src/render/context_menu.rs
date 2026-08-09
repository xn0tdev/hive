//! Centered context menu overlay for transcript block actions
//! (Copy, Recall, Revert). Styled like the Ctrl+P command palette.

use comb::{Buffer, Line, ModalLayout, Rect, Span, Style};

use crate::app::App;
use crate::render::panel::Panel;

const MIN_W: u16 = 36;
const MAX_W: u16 = 52;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;

struct MenuGeom {
    panel: Rect,
    content: Rect,
    list: Rect,
}

fn overlay(n_items: u16) -> Panel<'static> {
    // pad + title + gap + items
    let height = PAD_Y * 2 + 2 + n_items.max(1);
    Panel::new("Actions", "esc", height)
        .width_ratio(1, 2)
        .width_bounds(MIN_W, MAX_W)
        .padding(PAD_X, PAD_Y)
}

fn from_layout(layout: ModalLayout, n_items: u16) -> MenuGeom {
    let content = layout.content;
    // title row, then a blank gap, then items
    let list_y = (content.y + 2).min(content.bottom());
    let list_h = n_items.max(1).min(content.bottom().saturating_sub(list_y));
    let list = Rect::new(content.x, list_y, content.width, list_h);
    MenuGeom {
        panel: layout.panel,
        content,
        list,
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    app.context_menu_win = None;
    app.context_menu_hits.clear();
    let Some(menu) = app.context_menu.as_ref() else {
        return;
    };
    let theme = &app.theme;
    let panel = theme.strip;
    let item_count = menu.items.len() as u16;
    let layout = overlay(item_count).render(buf, area, theme);
    let g = from_layout(layout, item_count);
    let win = g.panel;
    let hits: Vec<(Rect, usize)> = (0..menu.items.len())
        .map(|i| (Rect::new(g.list.x, g.list.y + i as u16, g.list.width, 1), i))
        .filter(|(rect, _)| rect.y < g.list.bottom())
        .collect();
    if g.content.width < 8 {
        return;
    }

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
