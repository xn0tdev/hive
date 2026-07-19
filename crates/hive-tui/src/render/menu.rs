//! Floating composer menus: `/commands` and `@files`.
//! Opaque panel above the input band (same tint), OpenCode-style selection bar.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::files::MAX_MENU_ROWS;
use crate::app::App;

pub fn height(app: &mut App) -> u16 {
    let slash_n = app.slash_items().len();
    if slash_n > 0 {
        return rows_for(slash_n);
    }
    if app.at_mention().is_none() {
        return 0;
    }
    let file_n = app.file_menu_items().len();
    if file_n == 0 {
        return 1; // "no matches"
    }
    rows_for(file_n)
}

fn rows_for(n: usize) -> u16 {
    if n == 0 {
        return 0;
    }
    (n.min(MAX_MENU_ROWS) + usize::from(n > MAX_MENU_ROWS)) as u16
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    if !app.slash_items().is_empty() {
        draw_slash(buf, area, app);
        return;
    }
    if app.at_mention().is_some() {
        draw_files(buf, area, app);
    }
}

fn draw_slash(buf: &mut Buffer, area: Rect, app: &App) {
    let theme = &app.theme;
    let items = app.slash_items();
    if items.is_empty() {
        return;
    }
    let selected = app.menu_index.min(items.len() - 1);
    let panel_bg = theme.strip;
    let w = area.width as usize;
    buf.paint(area, Style::default().bg(panel_bg));

    let name_w = items
        .iter()
        .take(MAX_MENU_ROWS)
        .map(|c| 1 + c.name.chars().count())
        .max()
        .unwrap_or(0);
    let hint_w = items
        .iter()
        .take(MAX_MENU_ROWS)
        .map(|c| c.hint.chars().count())
        .max()
        .unwrap_or(0);

    let left = 2 + name_w + 2 + hint_w + 2;
    let desc_avail = w.saturating_sub(left + 1);

    let mut lines: Vec<Line> = Vec::with_capacity(items.len().min(MAX_MENU_ROWS) + 1);
    for (i, cmd) in items.iter().take(MAX_MENU_ROWS).enumerate() {
        let is_sel = i == selected;
        let bg = if is_sel { theme.sel_bg } else { panel_bg };
        let name_fg = if is_sel { theme.sel_fg } else { theme.fg };
        let hint_fg = if is_sel { theme.sel_fg } else { theme.faint };
        let desc_fg = if is_sel { theme.sel_fg } else { theme.faint };

        let name = format!("/{}", cmd.name);
        let name_pad = name_w.saturating_sub(name.chars().count());
        let hint_pad = hint_w.saturating_sub(cmd.hint.chars().count());
        let desc = ellipsize(cmd.desc, desc_avail);
        let tail = w.saturating_sub(left + desc.chars().count());

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(
                name,
                Style::default().fg(name_fg).bg(bg).add(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(name_pad + 2), Style::default().bg(bg)),
            Span::styled(cmd.hint.to_string(), Style::default().fg(hint_fg).bg(bg)),
            Span::styled(" ".repeat(hint_pad + 2), Style::default().bg(bg)),
            Span::styled(desc, Style::default().fg(desc_fg).bg(bg)),
            Span::styled(" ".repeat(tail), Style::default().bg(bg)),
        ]));
    }
    if items.len() > MAX_MENU_ROWS {
        lines.push(more_line(w, panel_bg, theme.faint));
    }

    buf.set_lines(area, &lines, 0);
}

fn draw_files(buf: &mut Buffer, area: Rect, app: &mut App) {
    let items = app.file_menu_items();
    let selected = app.menu_index;
    let sel_bg = app.theme.sel_bg;
    let sel_fg = app.theme.sel_fg;
    let fg = app.theme.fg;
    let faint = app.theme.faint;
    let panel_bg = app.theme.strip;
    let w = area.width as usize;
    buf.paint(area, Style::default().bg(panel_bg));

    if items.is_empty() {
        let line = Line::from(Span::styled(
            format!("{:<w$}", "  no matches"),
            Style::default().fg(faint).bg(panel_bg),
        ));
        buf.set_lines(area, &[line], 0);
        return;
    }

    let selected = selected.min(items.len() - 1);
    let window = window_start(selected, items.len(), MAX_MENU_ROWS);
    let visible = &items[window..items.len().min(window + MAX_MENU_ROWS)];

    let mut lines: Vec<Line> = Vec::with_capacity(visible.len() + 1);
    for (i, path) in visible.iter().enumerate() {
        let idx = window + i;
        let is_sel = idx == selected;
        let bg = if is_sel { sel_bg } else { panel_bg };
        let row_fg = if is_sel { sel_fg } else { fg };
        let mark = if is_sel { "› " } else { "  " };
        let text = ellipsize(&format!("{mark}{path}"), w);
        let pad = w.saturating_sub(text.chars().count());
        lines.push(Line::from(vec![
            Span::styled(
                text,
                Style::default()
                    .fg(row_fg)
                    .bg(bg)
                    .add_if(is_sel, Modifier::BOLD),
            ),
            Span::styled(" ".repeat(pad), Style::default().bg(bg)),
        ]));
    }
    if items.len() > MAX_MENU_ROWS {
        lines.push(more_line(w, panel_bg, faint));
    }
    buf.set_lines(area, &lines, 0);
}

fn window_start(selected: usize, total: usize, max_rows: usize) -> usize {
    if total <= max_rows || selected < max_rows {
        0
    } else {
        selected + 1 - max_rows
    }
}

fn more_line(w: usize, bg: Color, fg: Color) -> Line {
    Line::from(Span::styled(
        format!("{:<w$}", "  ↓ more"),
        Style::default().fg(fg).bg(bg),
    ))
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

trait AddIf {
    fn add_if(self, cond: bool, m: Modifier) -> Self;
}

impl AddIf for Style {
    fn add_if(self, cond: bool, m: Modifier) -> Self {
        if cond {
            self.add(m)
        } else {
            self
        }
    }
}
