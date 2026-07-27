//! Centered command palette / model picker overlay (Ctrl+P).

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::palette::{ConnectRow, ModelRow, PaletteMode, PaletteState};
use crate::app::{App, ModelsCatalogState};
use crate::commands::PaletteRow;
use crate::intro::PRESETS;

const MAX_LIST: u16 = 14;
const MIN_W: u16 = 36;
/// Compact width for commands / providers.
const MAX_W: u16 = 48;
/// Wider panel for `/model` so long ids and badges fit.
const MAX_W_MODELS: u16 = 72;
/// `/connect` carries shortcut hints in its header — give them room.
const MAX_W_CONNECT: u16 = 56;
const MIN_W_CONNECT: u16 = 52;
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

fn max_width_for(mode: PaletteMode) -> u16 {
    match mode {
        PaletteMode::Models => MAX_W_MODELS,
        PaletteMode::Connect => MAX_W_CONNECT,
        _ => MAX_W,
    }
}

fn geom(area: Rect, list_rows: u16, mode: PaletteMode) -> PaletteGeom {
    let max_w = max_width_for(mode);
    // Prefer ~2/3 of the terminal for models; ~1/2 for compact palettes.
    let prefer = match mode {
        PaletteMode::Models => area.width.saturating_mul(2) / 3,
        PaletteMode::Connect => (area.width / 2).max(MIN_W_CONNECT),
        _ => area.width / 2,
    };
    let w = prefer.clamp(MIN_W, max_w).min(area.width);
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
        PaletteMode::Models => match &app.models_catalog {
            ModelsCatalogState::Loading => 1,
            ModelsCatalogState::Failed(_) => 1,
            ModelsCatalogState::Ready | ModelsCatalogState::Idle => {
                pal.model_rows(&app.model_choices).len().max(1) as u16
            }
        },
        PaletteMode::Connect => pal.connect_rows(&app.connections).len().max(1) as u16,
        PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => 1,
        PaletteMode::Sessions => {
            if app.saved_sessions.is_empty() {
                1
            } else {
                pal.session_rows(&app.saved_sessions).len().max(1) as u16
            }
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

    let g = geom(area, list_row_count(pal, app), pal.mode);
    // Soft scrim behind the panel so it reads with a bit of depth.
    dim_outside(buf, area, g.win);
    buf.paint(g.win, panel_style);

    if g.content.width < 8 || g.content.height < 2 {
        return;
    }

    let title = match pal.mode {
        PaletteMode::Commands => "Commands",
        PaletteMode::Models => "Switch model",
        PaletteMode::Connect => "Providers",
        PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => "API key",
        PaletteMode::Sessions => "Resume session",
    };
    let title_hint = if matches!(pal.mode, PaletteMode::Connect) {
        "ctrl+e key · ctrl+r remove · esc"
    } else {
        "esc"
    };
    draw_title(
        buf,
        g.content,
        title,
        title_hint,
        theme.fg,
        theme.faint,
        panel,
    );
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
        PaletteMode::Connect => draw_connect_list(buf, g.list, pal, app, theme, panel),
        PaletteMode::Sessions => draw_sessions_list(buf, g.list, pal, app, theme, panel),
        PaletteMode::ConnectKey { preset_idx } => {
            let label = PRESETS
                .get(preset_idx)
                .map(|p| p.label)
                .unwrap_or("Provider");
            draw_connect_key_hint(buf, g.list, label, theme, panel);
        }
        PaletteMode::EditConnectionKey => {
            let label = pal
                .edit_connection
                .as_deref()
                .and_then(|id| app.connections.iter().find(|c| c.id == id))
                .map(|c| c.label.as_str())
                .unwrap_or("Provider");
            draw_connect_key_hint(buf, g.list, label, theme, panel);
        }
    }
}

fn draw_title(
    buf: &mut Buffer,
    area: Rect,
    title: &str,
    hint: &str,
    fg: Color,
    faint: Color,
    bg: Color,
) {
    let base = Style::default().bg(bg);
    let width = area.width as usize;
    let title_w = title.chars().count();
    // A hint that won't fit falls back to plain "esc", then to nothing —
    // never onto the title.
    const MIN_GAP: usize = 2;
    let hint = if title_w + MIN_GAP + hint.chars().count() <= width {
        hint
    } else if title_w + MIN_GAP + 3 <= width {
        "esc"
    } else {
        ""
    };
    let gap = width.saturating_sub(title_w + hint.chars().count());
    let title_line = Line::from(vec![
        Span::styled(
            title.to_string(),
            Style::default().fg(fg).bg(bg).add(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(gap), base),
        Span::styled(hint, Style::default().fg(faint).bg(bg)),
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
    let masked = matches!(
        pal.mode,
        PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey
    );
    let placeholder = if masked { "Paste API key…" } else { "Search" };
    let shown = if empty {
        placeholder.to_string()
    } else if masked {
        "•".repeat(pal.query.chars().count().min(area.width as usize))
    } else {
        pal.query.clone()
    };
    let fg = if empty { theme.faint } else { theme.fg };
    let mut style_text = Style::default().fg(fg).bg(bg);
    if empty {
        style_text = style_text.add(Modifier::ITALIC);
    }
    let line = Line::from(vec![Span::styled(shown, style_text)]);
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
    let g = geom(area, list_row_count(pal, app), pal.mode);
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
    geom(area, list_row_count(pal, app), pal.mode).list.height
}

/// The panel rect, for telling a click inside the overlay from one that means
/// "dismiss this".
pub fn window_rect(area: Rect, app: &App) -> Option<Rect> {
    let pal = app.palette.as_ref()?;
    Some(geom(area, list_row_count(pal, app), pal.mode).win)
}

/// Row index under the pointer, or `None` when it is not over the list.
///
/// The geometry is a pure function of the area and the palette state, so this
/// re-derives it rather than having the renderer stash rects per row.
pub fn row_at(area: Rect, app: &App, col: u16, row: u16) -> Option<usize> {
    let pal = app.palette.as_ref()?;
    let g = geom(area, list_row_count(pal, app), pal.mode);
    if !g.list.contains(col, row) {
        return None;
    }
    let offset = pal.visible_offset(
        &app.model_choices,
        &app.connections,
        &app.saved_sessions,
        g.list.height as usize,
    );
    Some(offset + usize::from(row - g.list.y))
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
    let offset = pal.visible_offset(&[], &[], &[], visible);

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
                    Style::default().fg(theme.fg).bg(panel).add(Modifier::BOLD),
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
                let mut spans = vec![Span::styled(left, Style::default().fg(name_fg).bg(bg))];
                let mid_budget = avail.saturating_sub(left_w + short_w + 2);
                if mid_budget > 4 && !desc.is_empty() {
                    let d = crate::render::two_col::ellipsize(desc, mid_budget.saturating_sub(1));
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

    match &app.models_catalog {
        ModelsCatalogState::Loading | ModelsCatalogState::Idle => {
            let line = Line::from(Span::styled(
                "Loading models…",
                Style::default().fg(theme.faint).bg(panel),
            ));
            crate::render::strip_paint::set_line_on_strip(
                buf, area.x, area.y, &line, area.width, panel,
            );
            return;
        }
        ModelsCatalogState::Failed(err) => {
            let msg = crate::render::two_col::ellipsize(err, area.width as usize);
            let line = Line::from(Span::styled(msg, Style::default().fg(theme.err).bg(panel)));
            crate::render::strip_paint::set_line_on_strip(
                buf, area.x, area.y, &line, area.width, panel,
            );
            return;
        }
        ModelsCatalogState::Ready => {}
    }

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
    let offset = pal.visible_offset(
        &app.model_choices,
        &app.connections,
        &app.saved_sessions,
        visible,
    );

    for row in 0..visible {
        let idx = offset + row;
        let y = area.y + row as u16;
        let Some(item) = rows.get(idx) else {
            buf.paint(Rect::new(area.x, y, area.width, 1), panel_style);
            continue;
        };
        match item {
            ModelRow::Header(label) => {
                let line = Line::from(Span::styled(
                    (*label).to_string(),
                    Style::default().fg(theme.fg).bg(panel).add(Modifier::BOLD),
                ));
                crate::render::strip_paint::set_line_on_strip(
                    buf, area.x, y, &line, area.width, panel,
                );
            }
            ModelRow::Model(choice) => {
                let is_sel = idx == sel;
                let bg = if is_sel { theme.sel_bg } else { panel };
                let base = Style::default().bg(bg);
                let name_fg = if is_sel { theme.sel_fg } else { theme.fg };
                let detail_fg = if is_sel { theme.sel_fg } else { theme.faint };
                let current = choice.key == app.model || choice.display == app.model_display;

                let mark = if current { "›" } else { " " };
                let left = format!("{mark} {}", choice.display);
                let right = if choice.detail.is_empty() {
                    String::new()
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
                crate::render::strip_paint::set_line_on_strip(
                    buf, area.x, y, &line, area.width, bg,
                );
            }
        }
    }
}

fn draw_connect_list(
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
    let rows = pal.connect_rows(&app.connections);
    if rows.is_empty() {
        return;
    }
    let sel = pal.selected.min(rows.len() - 1);
    let visible = area.height as usize;
    let offset = pal.visible_offset(&[], &app.connections, &[], visible);
    for row in 0..visible {
        let idx = offset + row;
        let y = area.y + row as u16;
        let Some(item) = rows.get(idx) else {
            buf.paint(Rect::new(area.x, y, area.width, 1), panel_style);
            continue;
        };
        let is_sel = idx == sel;
        let bg = if is_sel { theme.sel_bg } else { panel };
        // Providers you haven't set up read one step back from the rest.
        let configured = !matches!(item, ConnectRow::Preset(_));
        let name_fg = if is_sel {
            theme.sel_fg
        } else if configured {
            theme.fg
        } else {
            theme.dim
        };
        let detail_fg = if is_sel { theme.sel_fg } else { theme.faint };
        let (raw_left, raw_right) = match item {
            ConnectRow::Profile(c) => {
                // Every configured provider is authorized; the active runtime
                // connection is intentionally not distinguished in this list.
                (format!("✓ {}", c.label), c.detail.clone())
            }
            ConnectRow::Preset(i) => match crate::PRESETS.get(*i) {
                Some(p) => (format!("  {}", p.label), short_host(p.base_url)),
                None => continue,
            },
        };
        let (left, right, gap) =
            crate::render::two_col::layout_label_host(&raw_left, &raw_right, area.width as usize);
        let line = Line::from(vec![
            Span::styled(left, Style::default().fg(name_fg).bg(bg)),
            Span::styled(" ".repeat(gap), Style::default().bg(bg)),
            Span::styled(right, Style::default().fg(detail_fg).bg(bg)),
        ]);
        crate::render::strip_paint::set_line_on_strip(buf, area.x, y, &line, area.width, bg);
    }
}

fn draw_sessions_list(
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

    if app.saved_sessions.is_empty() {
        let line = Line::from(Span::styled(
            "No saved sessions yet.",
            Style::default().fg(theme.faint).bg(panel),
        ));
        crate::render::strip_paint::set_line_on_strip(
            buf, area.x, area.y, &line, area.width, panel,
        );
        return;
    }

    let rows = pal.session_rows(&app.saved_sessions);
    if rows.is_empty() {
        let line = Line::from(Span::styled(
            "No matching sessions",
            Style::default().fg(theme.faint).bg(panel),
        ));
        crate::render::strip_paint::set_line_on_strip(
            buf, area.x, area.y, &line, area.width, panel,
        );
        return;
    }

    let sel = pal.selected.min(rows.len() - 1);
    let visible = area.height as usize;
    let offset = pal.visible_offset(&[], &[], &app.saved_sessions, visible);

    for row in 0..visible {
        let idx = offset + row;
        let y = area.y + row as u16;
        let Some(item) = rows.get(idx) else {
            buf.paint(Rect::new(area.x, y, area.width, 1), panel_style);
            continue;
        };
        let is_sel = idx == sel;
        let bg = if is_sel { theme.sel_bg } else { panel };
        let name_fg = if is_sel { theme.sel_fg } else { theme.fg };
        let detail_fg = if is_sel { theme.sel_fg } else { theme.faint };

        let raw_left = format!(" {}", item.title);
        let raw_right = format!("{} msgs · {}", item.message_count, short_model(&item.model));
        // Long titles and long model names both overflow narrow palettes;
        // layout_label_host squeezes them instead of underflowing the gap.
        let (left, right, gap) =
            crate::render::two_col::layout_label_host(&raw_left, &raw_right, area.width as usize);

        let line = Line::from(vec![
            Span::styled(left, Style::default().fg(name_fg).bg(bg)),
            Span::styled(" ".repeat(gap), Style::default().bg(bg)),
            Span::styled(right, Style::default().fg(detail_fg).bg(bg)),
        ]);
        crate::render::strip_paint::set_line_on_strip(buf, area.x, y, &line, area.width, bg);
    }
}

/// Host of a preset base URL — the same secondary line saved profiles show.
fn short_host(base_url: &str) -> String {
    base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(base_url)
        .to_string()
}

fn short_model(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

fn draw_connect_key_hint(
    buf: &mut Buffer,
    area: Rect,
    label: &str,
    theme: &crate::theme::Theme,
    panel: Color,
) {
    if area.height == 0 {
        return;
    }
    let line = Line::from(Span::styled(
        format!("Enter key for {label} · enter to save"),
        Style::default().fg(theme.faint).bg(panel),
    ));
    crate::render::strip_paint::set_line_on_strip(buf, area.x, area.y, &line, area.width, panel);
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
        a.open_palette();
        a
    }

    /// The panel tone, taken from the theme rather than pinned to a literal —
    /// this test is about the fill reaching the edge, not about which grey.
    fn panel_bg() -> Color {
        crate::theme::Theme::from_name("gray").strip
    }

    #[test]
    fn palette_is_centered_and_compact() {
        let a = app();
        let size = Size::new(80, 24);
        let area = Rect::new(0, 0, size.width, size.height);
        let pal = a.palette.as_ref().unwrap();
        let g = geom(area, list_row_count(pal, &a), pal.mode);

        assert!(g.win.width <= MAX_W, "width={}", g.win.width);
        assert!(g.win.width >= MIN_W.min(size.width));
        let expected_x = (size.width - g.win.width) / 2;
        let expected_y = (size.height - g.win.height) / 2;
        assert_eq!(g.win.x, expected_x);
        assert_eq!(g.win.y, expected_y);
    }

    #[test]
    fn model_palette_is_wider() {
        let mut a = app();
        a.palette = Some(PaletteState::models());
        let size = Size::new(100, 24);
        let area = Rect::new(0, 0, size.width, size.height);
        let pal = a.palette.as_ref().unwrap();
        let g = geom(area, list_row_count(pal, &a), pal.mode);
        assert!(g.win.width > MAX_W, "width={}", g.win.width);
        assert!(g.win.width <= MAX_W_MODELS, "width={}", g.win.width);
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
            !text.contains('╭')
                && !text.contains('╮')
                && !text.contains('╰')
                && !text.contains('╯'),
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
        let pal = a.palette.as_ref().unwrap();
        let g = geom(area, list_row_count(pal, &a), pal.mode);
        assert_eq!(cy, g.search_y);
        assert_eq!(cx, g.content.x);
    }

    #[test]
    fn typing_focuses_search() {
        let mut a = app();
        let choices = a.model_choices.clone();
        assert!(!a.palette.as_ref().unwrap().search_focused);
        a.palette.as_mut().unwrap().insert('f', &choices, &[], &[]);
        assert!(a.palette.as_ref().unwrap().search_focused);
        assert_eq!(a.palette.as_ref().unwrap().query, "f");
    }

    #[test]
    fn scrim_dims_cells_outside_panel() {
        let a = app();
        let size = Size::new(80, 24);
        let area = Rect::new(0, 0, size.width, size.height);
        let pal = a.palette.as_ref().unwrap();
        let g = geom(area, list_row_count(pal, &a), pal.mode);
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
    fn sessions_list_survives_long_titles_and_models() {
        let mut a = app();
        a.palette = Some(PaletteState::sessions());
        a.saved_sessions = vec![
            hive_core::SessionMeta {
                id: "s_1_aaaaaaaa".into(),
                title: "A very long session title that will not fit anywhere".into(),
                model: "accounts/fireworks/models/some-extremely-long-model-name".into(),
                message_count: 128,
                created_at: 1,
                updated_at: 2,
            },
            hive_core::SessionMeta {
                id: "s_2_bbbbbbbb".into(),
                title: "короткий".into(),
                model: "acc/models/m".into(),
                message_count: 3,
                created_at: 1,
                updated_at: 1,
            },
        ];

        // Narrow terminals used to underflow the padding width and panic.
        for width in [20u16, 32, 48, 80, 120] {
            let buf = render(Size::new(width, 24), |f| {
                crate::render::draw(f, &mut a);
            });
            assert!(buf.text().contains("Resume session"), "width={width}");
        }
    }

    #[test]
    fn providers_list_marks_configured_and_offers_the_rest() {
        let mut a = app();
        a.connections = vec![
            hive_core::event::ConnectionInfo {
                id: "fireworks".into(),
                label: "Fireworks".into(),
                detail: "api.fireworks.ai".into(),
            },
            hive_core::event::ConnectionInfo {
                id: "groq".into(),
                label: "Groq".into(),
                detail: "api.groq.com".into(),
            },
        ];
        a.active_connection = "groq".into();
        a.palette = Some(PaletteState::connect());

        let text = render(Size::new(90, 30), |f| crate::render::draw(f, &mut a))
            .text()
            .to_string();

        assert!(text.contains("Providers"), "{text}");
        assert!(
            text.contains("\u{2713} Groq"),
            "active provider is shown as authorized: {text}"
        );
        assert!(
            text.contains("\u{2713} Fireworks"),
            "configured marked: {text}"
        );
        assert!(
            !text.contains("\u{203a} Groq"),
            "no active-provider arrow: {text}"
        );
        assert!(text.contains("ctrl+e key"), "key shortcut shown: {text}");
        // Providers you haven't set up are right there in the same list.
        assert!(text.contains("OpenRouter"), "{text}");
        assert!(text.contains("openrouter.ai"), "host shown: {text}");
        assert!(!text.contains("Add provider"), "no add step left: {text}");
    }

    #[test]
    fn the_header_hint_never_lands_on_the_title() {
        let mut a = app();
        a.connections = vec![hive_core::event::ConnectionInfo {
            id: "openai".into(),
            label: "OpenAI".into(),
            detail: "api.openai.com".into(),
        }];
        a.palette = Some(PaletteState::connect());

        for width in [30u16, 40, 60, 90, 140] {
            let text = render(Size::new(width, 26), |f| crate::render::draw(f, &mut a))
                .text()
                .to_string();
            assert!(
                !text.contains("Providersctrl") && !text.contains("Providersesc"),
                "hint collided with the title at width {width}: {text}"
            );
        }
    }

    #[test]
    fn list_visible_matches_geom() {
        let a = app();
        let area = Rect::new(0, 0, 80, 24);
        let pal = a.palette.as_ref().unwrap();
        let expected = geom(area, list_row_count(pal, &a), pal.mode).list.height;
        assert_eq!(list_visible(area, &a), expected);
        assert!(expected > 0);
    }
}
