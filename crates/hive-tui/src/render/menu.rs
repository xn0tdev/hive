//! Floating composer menus: `/commands` and `@files`.
//! Opaque panel above the input band (same tint), OpenCode-style selection bar.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::files::MAX_MENU_ROWS;
use crate::app::{App, SlashItem};

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

    let window = window_start(selected, items.len(), MAX_MENU_ROWS);
    let visible = &items[window.min(items.len())..items.len().min(window + MAX_MENU_ROWS)];

    let name_w = visible
        .iter()
        .map(|c| 1 + c.name().chars().count())
        .max()
        .unwrap_or(0)
        .min(28);
    // `/name` · description — nothing between them, description takes the rest.
    let left = 2 + name_w + 2;
    let desc_avail = w.saturating_sub(left + 1);

    let mut lines: Vec<Line> = Vec::with_capacity(visible.len() + 1);
    for (i, item) in visible.iter().enumerate() {
        let idx = window + i;
        let is_sel = idx == selected;
        let bg = if is_sel { theme.sel_bg } else { panel_bg };
        // Skills keep the accent on their name — the only thing left telling
        // them apart from built-in commands.
        let name_fg = if is_sel {
            theme.sel_fg
        } else if matches!(item, SlashItem::Skill(_)) {
            theme.accent
        } else {
            theme.fg
        };
        let desc_fg = if is_sel { theme.sel_fg } else { theme.faint };

        let name = ellipsize(&format!("/{}", item.name()), name_w);
        let name_pad = name_w.saturating_sub(name.chars().count());
        let desc = ellipsize(item.desc(), desc_avail);
        let tail = w.saturating_sub(left + desc.chars().count());

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(
                name,
                Style::default().fg(name_fg).bg(bg).add(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(name_pad + 2), Style::default().bg(bg)),
            Span::styled(desc, Style::default().fg(desc_fg).bg(bg)),
            Span::styled(" ".repeat(tail), Style::default().bg(bg)),
        ]));
    }
    if items.len() > MAX_MENU_ROWS {
        let more = if window + MAX_MENU_ROWS < items.len() {
            format!("  ↓ {} more", items.len() - (window + MAX_MENU_ROWS))
        } else if window > 0 {
            format!("  ↑ {} above", window)
        } else {
            "  ↓ more".into()
        };
        lines.push(more_line_text(&more, w, panel_bg, theme.faint));
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
    let visible = &items[window.min(items.len())..items.len().min(window + MAX_MENU_ROWS)];

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
    more_line_text("  ↓ more", w, bg, fg)
}

fn more_line_text(text: &str, w: usize, bg: Color, fg: Color) -> Line {
    let clipped = ellipsize(text, w);
    let pad = w.saturating_sub(clipped.chars().count());
    Line::from(vec![
        Span::styled(clipped, Style::default().fg(fg).bg(bg)),
        Span::styled(" ".repeat(pad), Style::default().bg(bg)),
    ])
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

    /// Type `/resume` far enough to pin the menu to commands that take args.
    fn menu_text(prefix: &str) -> String {
        let mut a = app();
        a.input.value = prefix.to_string();
        a.input.cursor = prefix.chars().count();
        assert!(!a.slash_items().is_empty(), "menu should be open");
        render(Size::new(90, 24), |f| crate::render::draw(f, &mut a))
            .text()
            .to_string()
    }

    #[test]
    fn slash_menu_shows_only_name_and_description() {
        let text = menu_text("/res");
        assert!(text.contains("/resume"), "{text}");
        assert!(text.contains("Browse and resume saved chats"), "{text}");
        assert!(!text.contains("[id]"), "argument hint must be gone: {text}");
    }

    #[test]
    fn no_argument_hints_anywhere_in_the_menu() {
        let text = menu_text("/");
        for hint in ["[id]", "[objective]", "[path]"] {
            assert!(!text.contains(hint), "{hint} still rendered: {text}");
        }
    }
}
