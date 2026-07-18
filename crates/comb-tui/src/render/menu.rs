//! The slash command menu: an opaque panel that floats directly above the
//! input band (same tinted background, so they read as one card) when the
//! user's input is just a `/command` prefix — never when a slash appears
//! mid-text. The selected row is a full-width light bar, OpenCode-style.

use comb::{Buffer, Line, Modifier, Rect, Span, Style};

use crate::app::App;

const MAX_ROWS: usize = 8;

pub fn height(app: &App) -> u16 {
    let n = app.menu_items().len();
    if n == 0 {
        return 0;
    }
    (n.min(MAX_ROWS) + usize::from(n > MAX_ROWS)) as u16
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    let theme = &app.theme;
    let items = app.menu_items();
    if items.is_empty() || area.width < 8 || area.height == 0 {
        return;
    }
    let selected = app.menu_index.min(items.len() - 1);
    let panel_bg = theme.strip;
    let w = area.width as usize;

    // Opaque card, merged with the input band below it.
    buf.paint(area, Style::default().bg(panel_bg));

    // Column widths from the visible items, so the hint and description columns
    // line up in a clean grid.
    let name_w = items
        .iter()
        .take(MAX_ROWS)
        .map(|c| 1 + c.name.chars().count())
        .max()
        .unwrap_or(0);
    let hint_w = items
        .iter()
        .take(MAX_ROWS)
        .map(|c| c.hint.chars().count())
        .max()
        .unwrap_or(0);

    // inset(2) + name + gap(2) + hint + gap(2) + desc + inset(1)
    let left = 2 + name_w + 2 + hint_w + 2;
    let desc_avail = w.saturating_sub(left + 1);

    let mut lines: Vec<Line> = Vec::with_capacity(items.len().min(MAX_ROWS) + 1);
    for (i, cmd) in items.iter().take(MAX_ROWS).enumerate() {
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
            // gap + hint column, nudged right and greyed so it reads apart.
            Span::styled(" ".repeat(name_pad + 2), Style::default().bg(bg)),
            Span::styled(cmd.hint.to_string(), Style::default().fg(hint_fg).bg(bg)),
            Span::styled(" ".repeat(hint_pad + 2), Style::default().bg(bg)),
            Span::styled(desc, Style::default().fg(desc_fg).bg(bg)),
            Span::styled(" ".repeat(tail), Style::default().bg(bg)),
        ]));
    }
    if items.len() > MAX_ROWS {
        lines.push(Line::from(Span::styled(
            format!("{:<w$}", "  ↓ more"),
            Style::default().fg(theme.faint).bg(panel_bg),
        )));
    }

    buf.set_lines(area, &lines, 0);
}

/// Cut a description to `max` display cells, ending with `…` instead of a
/// mid-word clip at the pane edge.
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
