//! About-style drawing for the intro wizard.

use comb::{Buffer, Color, Frame, Line, Modifier, Rect, Span, Style};

use super::{
    FetchState, IntroState, ProviderKeyFocus, SearchChoice, SearchFocus, Step, PRESETS,
};
use crate::render::wordmark;
use crate::theme::Theme;

const MIN_W: u16 = 48;
const MAX_W: u16 = 64;
const PAD_X: u16 = 3;
const PAD_Y: u16 = 1;
const CANVAS: Color = Color::Rgb(0x14, 0x14, 0x14);

struct Geom {
    win: Rect,
    content: Rect,
}

fn geom(area: Rect) -> Geom {
    let w = (area.width * 3 / 4).clamp(MIN_W, MAX_W).min(area.width);
    let h = area.height.saturating_sub(2).clamp(14, 22);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let win = Rect::new(x, y, w, h);
    let content = Rect {
        x: win.x + PAD_X,
        y: win.y + PAD_Y,
        width: win.width.saturating_sub(PAD_X * 2),
        height: win.height.saturating_sub(PAD_Y * 2),
    };
    Geom { win, content }
}

/// Paint the intro frame. Returns optional caret position.
pub fn paint(f: &mut Frame<'_>, state: &IntroState) -> Option<(u16, u16)> {
    let area = f.area();
    let buf = f.buffer();
    buf.paint(area, Style::default().bg(CANVAS));
    let theme = &state.theme;
    let panel = theme.strip;
    let g = geom(area);
    dim_outside(buf, area, g.win);
    buf.paint(g.win, Style::default().bg(panel));
    if g.content.width < 20 || g.content.height < 8 {
        return None;
    }
    match state.step {
        Step::Welcome => draw_welcome(buf, g.content, state, theme, panel),
        Step::Modes => draw_modes(buf, g.content, theme, panel),
        Step::Provider => draw_provider(buf, g.content, state, theme, panel),
        Step::ProviderKey => draw_provider_key(buf, g.content, state, theme, panel),
        Step::Models => draw_models(buf, g.content, state, theme, panel),
        Step::Search => draw_search(buf, g.content, state, theme, panel),
        Step::Done => draw_done(buf, g.content, state, theme, panel),
    }
}

fn title(buf: &mut Buffer, area: Rect, label: &str, right: &str, theme: &Theme, bg: Color) {
    let pad = area
        .width
        .saturating_sub(label.chars().count() as u16 + right.chars().count() as u16);
    let line = Line::from(vec![
        Span::styled(
            label.to_string(),
            Style::default().fg(theme.fg).bg(bg).add(Modifier::BOLD),
        ),
        Span::styled(" ".repeat(pad as usize), Style::default().bg(bg)),
        Span::styled(right.to_string(), Style::default().fg(theme.faint).bg(bg)),
    ]);
    crate::render::strip_paint::set_line_on_strip(buf, area.x, area.y, &line, area.width, bg);
}

fn put(buf: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style, bg: Color) {
    let line = Line::from(Span::styled(text.to_string(), style.bg(bg)));
    crate::render::strip_paint::set_line_on_strip(buf, x, y, &line, width, bg);
}

fn center(buf: &mut Buffer, area: Rect, y: u16, text: &str, style: Style, bg: Color) {
    let tw = text.chars().count() as u16;
    let ox = area.x + area.width.saturating_sub(tw) / 2;
    buf.paint(Rect::new(area.x, y, area.width, 1), Style::default().bg(bg));
    put(buf, ox, y, tw.min(area.width), text, style, bg);
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur = word.to_string();
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn dim_outside(buf: &mut Buffer, area: Rect, exclude: Rect) {
    let dim_bg = Style::default().bg(Color::Rgb(0x0c, 0x0c, 0x0c));
    // Top
    if exclude.y > area.y {
        buf.paint(
            Rect::new(area.x, area.y, area.width, exclude.y - area.y),
            dim_bg,
        );
    }
    // Bottom
    let below = exclude.bottom();
    if below < area.bottom() {
        buf.paint(
            Rect::new(area.x, below, area.width, area.bottom() - below),
            dim_bg,
        );
    }
    // Left / right beside panel
    if exclude.x > area.x {
        buf.paint(
            Rect::new(area.x, exclude.y, exclude.x - area.x, exclude.height),
            dim_bg,
        );
    }
    let right = exclude.right();
    if right < area.right() {
        buf.paint(
            Rect::new(right, exclude.y, area.right() - right, exclude.height),
            dim_bg,
        );
    }
}

fn draw_welcome(
    buf: &mut Buffer,
    area: Rect,
    state: &IntroState,
    theme: &Theme,
    bg: Color,
) -> Option<(u16, u16)> {
    title(
        buf,
        area,
        "Welcome",
        &format!("v{}", state.opts.version),
        theme,
        bg,
    );
    let mut y = area.y + 2;
    let mark = wordmark::lines(theme, None);
    let art_x = area.x + area.width.saturating_sub(wordmark::WIDTH) / 2;
    if y + wordmark::HEIGHT <= area.bottom() {
        crate::render::strip_paint::set_lines_on_strip(
            buf,
            Rect::new(art_x, y, wordmark::WIDTH, wordmark::HEIGHT),
            &mark,
            0,
            bg,
        );
    }
    y += wordmark::HEIGHT + 1;
    for line in wrap(
        "Hey — Hive is a coding agent that actually ships work. Tools run without nagging. Let's get you set up.",
        area.width as usize,
    )
    .into_iter()
    .take(4)
    {
        center(
            buf,
            area,
            y,
            &line,
            Style::default().fg(theme.faint),
            bg,
        );
        y += 1;
    }
    // Soft accent tick on the continue hint.
    let pulse = state.tick % 4 < 2;
    let hint_style = if pulse {
        Style::default().fg(theme.accent)
    } else {
        Style::default().fg(theme.faint)
    };
    let hint_y = area.bottom().saturating_sub(1);
    center(
        buf,
        area,
        hint_y,
        "enter / →  continue   ·   esc quit",
        hint_style,
        bg,
    );
    None
}

fn draw_modes(buf: &mut Buffer, area: Rect, theme: &Theme, bg: Color) -> Option<(u16, u16)> {
    title(buf, area, "Modes", "← back", theme, bg);
    let mut y = area.y + 2;
    put(
        buf,
        area.x,
        y,
        area.width,
        "PLAN",
        Style::default().fg(theme.plan).add(Modifier::BOLD),
        bg,
    );
    y += 1;
    for line in wrap(
        "Plan a big feature first so Hive matches what you want before it writes code.",
        area.width as usize,
    ) {
        put(
            buf,
            area.x,
            y,
            area.width,
            &line,
            Style::default().fg(theme.faint),
            bg,
        );
        y += 1;
    }
    y += 1;
    put(
        buf,
        area.x,
        y,
        area.width,
        "BUILD",
        Style::default().fg(theme.build).add(Modifier::BOLD),
        bg,
    );
    y += 1;
    for line in wrap(
        "You code — the agent runs tools and ships. Can spawn one helper when useful.",
        area.width as usize,
    ) {
        put(
            buf,
            area.x,
            y,
            area.width,
            &line,
            Style::default().fg(theme.faint),
            bg,
        );
        y += 1;
    }
    y += 1;
    put(
        buf,
        area.x,
        y,
        area.width,
        "MULTITASK",
        Style::default().fg(theme.multitask).add(Modifier::BOLD),
        bg,
    );
    y += 1;
    for line in wrap(
        "Orchestrator — splits work into parallel subagents in git worktrees, then merges.",
        area.width as usize,
    ) {
        put(
            buf,
            area.x,
            y,
            area.width,
            &line,
            Style::default().fg(theme.faint),
            bg,
        );
        y += 1;
    }
    y += 1;
    put(
        buf,
        area.x,
        y,
        area.width,
        "Toggle anytime with Tab: BUILD → PLAN → MULTITASK.",
        Style::default().fg(theme.dim),
        bg,
    );
    center(
        buf,
        area,
        area.bottom().saturating_sub(1),
        "enter / →  continue",
        Style::default().fg(theme.faint),
        bg,
    );
    None
}

/// Step A — provider names + description (no key field).
fn draw_provider(
    buf: &mut Buffer,
    area: Rect,
    state: &IntroState,
    theme: &Theme,
    bg: Color,
) -> Option<(u16, u16)> {
    let right = if state.can_use_existing_provider() {
        "u use existing  ·  ← back"
    } else {
        "← back"
    };
    title(buf, area, "Provider", right, theme, bg);

    let preset = state.preset();
    let detail_lines = wrap(preset.description, area.width as usize)
        .into_iter()
        .take(3)
        .collect::<Vec<_>>();
    let detail_h = detail_lines.len().max(1) as u16;

    // Bottom: gap · description · status · footer
    let bottom_block = 1 + detail_h + 1 + 1;
    let list_top = area.y + 2;
    let list_bottom = area.bottom().saturating_sub(bottom_block).max(list_top + 3);
    let name_slots = list_bottom.saturating_sub(list_top).max(1) as usize;
    let start = state
        .provider_idx
        .saturating_sub(name_slots / 2)
        .min(PRESETS.len().saturating_sub(name_slots.min(PRESETS.len())));

    for (y, (row, p)) in (list_top..list_bottom).zip(PRESETS.iter().enumerate().skip(start)) {
        let selected = row == state.provider_idx;
        let row_bg = if selected { theme.sel_bg } else { bg };
        let name_fg = if selected { theme.sel_fg } else { theme.fg };
        let detail_fg = if selected { theme.sel_fg } else { theme.faint };
        let host = if p.is_custom {
            ""
        } else {
            crate::render::two_col::host_hint(p.base_url)
        };
        let (left, right, gap) =
            crate::render::two_col::layout_label_host(p.label, host, area.width as usize);
        let line = Line::from(vec![
            Span::styled(left, Style::default().fg(name_fg).bg(row_bg)),
            Span::styled(" ".repeat(gap), Style::default().bg(row_bg)),
            Span::styled(right, Style::default().fg(detail_fg).bg(row_bg)),
        ]);
        crate::render::strip_paint::set_line_on_strip(buf, area.x, y, &line, area.width, row_bg);
    }

    let detail_y = area.bottom().saturating_sub(2 + detail_h).max(list_bottom);
    for (dy, line) in (detail_y..area.bottom().saturating_sub(1)).zip(&detail_lines) {
        put(
            buf,
            area.x,
            dy,
            area.width,
            line,
            Style::default().fg(theme.faint),
            bg,
        );
    }

    if !state.status.is_empty() {
        put(
            buf,
            area.x,
            area.bottom().saturating_sub(2),
            area.width,
            &state.status,
            Style::default().fg(theme.warn),
            bg,
        );
    }
    let footer = if state.can_use_existing_provider() {
        "u use existing  ·  enter / paste key"
    } else {
        "↑↓ choose  ·  enter / →  ·  paste key"
    };
    center(
        buf,
        area,
        area.bottom().saturating_sub(1),
        footer,
        Style::default().fg(theme.faint),
        bg,
    );
    None
}

/// Step B — API key for the selected provider (custom URL/env when needed).
fn draw_provider_key(
    buf: &mut Buffer,
    area: Rect,
    state: &IntroState,
    theme: &Theme,
    bg: Color,
) -> Option<(u16, u16)> {
    title(buf, area, "API key", "← back", theme, bg);

    let preset = state.preset();
    let custom = preset.is_custom;
    let mut y = area.y + 2;

    put(
        buf,
        area.x,
        y,
        area.width,
        &truncate(&format!("› {}", preset.label), area.width as usize),
        Style::default().fg(theme.accent).add(Modifier::BOLD),
        bg,
    );
    y += 1;

    let reminder = if custom {
        "OpenAI-compatible endpoint — URL, env name, then paste the key."
    } else {
        "Paste your API key. Stored in config / the env name below."
    };
    for line in wrap(reminder, area.width as usize).into_iter().take(2) {
        put(
            buf,
            area.x,
            y,
            area.width,
            &line,
            Style::default().fg(theme.faint),
            bg,
        );
        y += 1;
    }
    y += 1;

    if !custom {
        put(
            buf,
            area.x,
            y,
            area.width,
            &truncate(&format!("env  {}", preset.api_key_env), area.width as usize),
            Style::default().fg(theme.dim),
            bg,
        );
        y += 1;
    }

    if custom {
        let url_focus = state.provider_key_focus == ProviderKeyFocus::CustomUrl;
        put(
            buf,
            area.x,
            y,
            area.width,
            &truncate(
                &format!(
                    "{}URL  {}",
                    if url_focus { "›" } else { " " },
                    if state.custom_url.is_empty() {
                        "https://…"
                    } else {
                        &state.custom_url
                    }
                ),
                area.width as usize,
            ),
            Style::default().fg(if url_focus { theme.accent } else { theme.dim }),
            bg,
        );
        y += 1;
        let env_focus = state.provider_key_focus == ProviderKeyFocus::CustomEnv;
        put(
            buf,
            area.x,
            y,
            area.width,
            &truncate(
                &format!(
                    "{}env  {}",
                    if env_focus { "›" } else { " " },
                    state.custom_env
                ),
                area.width as usize,
            ),
            Style::default().fg(if env_focus { theme.accent } else { theme.dim }),
            bg,
        );
        y += 1;
    }

    y += 1;
    let key_focus = state.provider_key_focus == ProviderKeyFocus::Key;
    let masked: String = "•".repeat(state.provider_key.chars().count().min(48));
    let key_label = format!(
        "{}key  {}",
        if key_focus { "›" } else { " " },
        if masked.is_empty() {
            "(paste API key)"
        } else {
            &masked
        }
    );
    put(
        buf,
        area.x,
        y,
        area.width,
        &truncate(&key_label, area.width as usize),
        Style::default().fg(if key_focus { theme.accent } else { theme.dim }),
        bg,
    );

    let caret = if key_focus && state.tick.is_multiple_of(2) {
        let prefix = "›key  ".chars().count() as u16;
        let keyed = state.provider_key.chars().count().min(48) as u16;
        let cx = area.x + prefix + keyed;
        if cx < area.right() {
            Some((cx, y))
        } else {
            None
        }
    } else {
        None
    };

    if !state.status.is_empty() {
        put(
            buf,
            area.x,
            area.bottom().saturating_sub(2),
            area.width,
            &state.status,
            Style::default().fg(theme.warn),
            bg,
        );
    }
    let footer = if custom {
        "tab fields  ·  enter / →  ·  paste"
    } else {
        "paste key  ·  enter / → models"
    };
    center(
        buf,
        area,
        area.bottom().saturating_sub(1),
        footer,
        Style::default().fg(theme.faint),
        bg,
    );
    caret
}

fn draw_models(
    buf: &mut Buffer,
    area: Rect,
    state: &IntroState,
    theme: &Theme,
    bg: Color,
) -> Option<(u16, u16)> {
    let right = if state.can_use_existing_models() {
        "u use existing"
    } else {
        "esc back"
    };
    title(buf, area, "Model", right, theme, bg);

    let footer_y = area.bottom().saturating_sub(1);
    let status_y = if state.status.is_empty() {
        footer_y
    } else {
        footer_y.saturating_sub(1)
    };

    let header_top = area.y + 2;
    let filter_h: u16 = u16::from(!state.model_filter.is_empty());
    let selected_h: u16 = 1;
    let list_top = header_top + selected_h + filter_h;
    let list_bottom = status_y.max(list_top + 1);

    // Selected model line
    let selected_label = if state.model.id.is_empty() {
        "Pick a model (enter)".to_string()
    } else {
        let name = super::short_model_label(&state.model.id, &state.model.name, 40);
        format!("Selected: {name}")
    };
    put(
        buf,
        area.x,
        header_top,
        area.width,
        &truncate(&selected_label, area.width as usize),
        if state.model.id.is_empty() {
            Style::default().fg(theme.dim)
        } else {
            Style::default().fg(theme.accent).add(Modifier::BOLD)
        },
        bg,
    );

    if !state.model_filter.is_empty() {
        put(
            buf,
            area.x,
            header_top + selected_h,
            area.width,
            &truncate(
                &format!("search: {}", state.model_filter),
                area.width as usize,
            ),
            Style::default().fg(theme.dim),
            bg,
        );
    }

    // --- Model list ---
    let mut y = list_top;
    match &state.fetch {
        FetchState::Idle | FetchState::Loading => {
            put(
                buf,
                area.x,
                y,
                area.width,
                "Fetching /models + models.dev…",
                Style::default().fg(theme.accent),
                bg,
            );
        }
        FetchState::Err(e) => {
            for line in wrap(e, area.width as usize).into_iter().take(3) {
                if y >= list_bottom {
                    break;
                }
                put(
                    buf,
                    area.x,
                    y,
                    area.width,
                    &line,
                    Style::default().fg(theme.err),
                    bg,
                );
                y += 1;
            }
        }
        FetchState::Ready(cards) => {
            let indices = state.visible_model_indices();
            let list_h = list_bottom.saturating_sub(list_top).max(1) as usize;
            if indices.is_empty() {
                put(
                    buf,
                    area.x,
                    y,
                    area.width,
                    "no matches",
                    Style::default().fg(theme.dim),
                    bg,
                );
            } else {
                let sel_pos = indices
                    .iter()
                    .position(|&i| i == state.model_idx)
                    .unwrap_or(0);
                let scroll = sel_pos.saturating_sub(list_h / 2);
                let assigned_id = &state.model.id;
                let recommended = state.suggested_id();
                let name_max = (area.width as usize).saturating_sub(6).max(8);
                for (row, &ci) in indices.iter().enumerate().skip(scroll).take(list_h) {
                    let c = &cards[ci];
                    let sel = ci == state.model_idx;
                    let is_assigned = !assigned_id.is_empty() && c.id == *assigned_id;
                    let is_rec = recommended == Some(c.id.as_str());
                    let mark = if is_rec {
                        "★"
                    } else if is_assigned {
                        "•"
                    } else {
                        " "
                    };
                    let name = super::short_model_label(&c.id, &c.name, name_max);
                    let style = if sel {
                        Style::default().fg(theme.sel_fg).bg(theme.sel_bg)
                    } else if is_assigned {
                        Style::default().fg(theme.accent).bg(bg)
                    } else {
                        Style::default().fg(theme.fg).bg(bg)
                    };
                    put(
                        buf,
                        area.x,
                        y + (row - scroll) as u16,
                        area.width,
                        &truncate(
                            &format!("{}{} {}", if sel { "›" } else { " " }, mark, name),
                            area.width as usize,
                        ),
                        style,
                        if sel { theme.sel_bg } else { bg },
                    );
                }
            }
        }
    }

    if !state.status.is_empty() {
        put(
            buf,
            area.x,
            status_y,
            area.width,
            &truncate(&state.status, area.width as usize),
            Style::default().fg(theme.warn),
            bg,
        );
    }

    let footer = if matches!(state.fetch, FetchState::Ready(_)) {
        if state.models_ready() {
            "enter pick · → continue · type to filter"
        } else {
            "enter pick · type to filter"
        }
    } else if state.can_use_existing_models() {
        "u use existing  ·  fetching…"
    } else if matches!(state.fetch, FetchState::Loading | FetchState::Idle) {
        "fetching…"
    } else {
        "esc back"
    };
    center(
        buf,
        area,
        footer_y,
        footer,
        Style::default().fg(theme.faint),
        bg,
    );
    None
}

fn draw_search(
    buf: &mut Buffer,
    area: Rect,
    state: &IntroState,
    theme: &Theme,
    bg: Color,
) -> Option<(u16, u16)> {
    let right = if state.can_use_existing_search() {
        "u use existing"
    } else {
        "fresh info for the agent"
    };
    title(buf, area, "Search", right, theme, bg);
    let mut y = area.y + 2;
    for line in wrap(
        "How should Hive get up-to-date information from the web?",
        area.width as usize,
    ) {
        put(
            buf,
            area.x,
            y,
            area.width,
            &line,
            Style::default().fg(theme.faint),
            bg,
        );
        y += 1;
    }
    y += 1;
    let choices = [
        ("Exa", "neural web search"),
        ("Perplexity", "Sonar answers + citations"),
        ("Skip for now", "configure later"),
    ];
    for (i, (label, note)) in choices.iter().enumerate() {
        let selected = state.search_idx == i;
        let list_focus = state.search_focus == SearchFocus::List;
        let style = if selected && list_focus {
            Style::default().fg(theme.sel_fg).bg(theme.sel_bg)
        } else if selected {
            Style::default().fg(theme.accent).bg(bg)
        } else {
            Style::default().fg(theme.fg).bg(bg)
        };
        put(
            buf,
            area.x,
            y,
            area.width,
            &format!(
                "{}{:<14} {}",
                if selected { "› " } else { "  " },
                label,
                note
            ),
            style,
            if selected && list_focus {
                theme.sel_bg
            } else {
                bg
            },
        );
        y += 1;
    }
    y += 1;
    if state.search_choice() != SearchChoice::Skip {
        let key_focus = state.search_focus == SearchFocus::Key;
        let masked: String = "•".repeat(state.search_key.chars().count().min(40));
        put(
            buf,
            area.x,
            y,
            area.width,
            &format!(
                "{}key  {}",
                if key_focus { "›" } else { " " },
                if masked.is_empty() {
                    "(optional — paste key)"
                } else {
                    &masked
                }
            ),
            Style::default().fg(if key_focus { theme.accent } else { theme.dim }),
            bg,
        );
    }
    let footer = if state.can_use_existing_search() {
        "u use existing  ·  enter configure"
    } else {
        "enter / →  continue"
    };
    center(
        buf,
        area,
        area.bottom().saturating_sub(1),
        footer,
        Style::default().fg(theme.faint),
        bg,
    );
    None
}

fn draw_done(
    buf: &mut Buffer,
    area: Rect,
    state: &IntroState,
    theme: &Theme,
    bg: Color,
) -> Option<(u16, u16)> {
    title(buf, area, "Ready", "← back", theme, bg);

    let preset = state.preset();
    let provider = if preset.is_custom {
        let url = state.resolved_base_url();
        if url.is_empty() {
            "Custom".to_string()
        } else {
            format!("Custom · {}", truncate_url(&url, 36))
        }
    } else {
        preset.label.to_string()
    };

    let mut rows: Vec<(&str, String)> = vec![
        ("provider", provider),
        (
            "model",
            if state.model.name.is_empty() {
                state.model.id.clone()
            } else {
                state.model.name.clone()
            },
        ),
    ];
    match state.search_choice() {
        SearchChoice::Exa => rows.push(("search", "Exa".into())),
        SearchChoice::Perplexity => rows.push(("search", "Perplexity".into())),
        SearchChoice::Skip => {}
    }

    const LABEL_W: usize = 8;
    let max_val = area.width.saturating_sub((LABEL_W as u16) + 2).max(8) as usize;
    let block_w = rows
        .iter()
        .map(|(_, v)| LABEL_W + 2 + truncate(v, max_val).chars().count())
        .max()
        .unwrap_or(LABEL_W)
        .min(area.width as usize) as u16;

    // Tagline + gap + rows — upper-third composition, not floating crumbs.
    let block_h = 1u16 + 1 + rows.len() as u16;
    let footer_y = area.bottom().saturating_sub(1);
    let status_slot = u16::from(!state.status.is_empty());
    let avail_top = area.y + 2;
    let avail_bottom = footer_y.saturating_sub(1 + status_slot).max(avail_top);
    let avail_h = avail_bottom.saturating_sub(avail_top);
    let mut y = if avail_h > block_h + 2 {
        avail_top + (avail_h - block_h) / 4
    } else {
        avail_top + avail_h.saturating_sub(block_h) / 2
    };

    center(
        buf,
        area,
        y,
        "You're set.",
        Style::default().fg(theme.accent).add(Modifier::BOLD),
        bg,
    );
    y += 2;

    let ox = area.x + area.width.saturating_sub(block_w) / 2;
    for (label, value) in &rows {
        if y >= avail_bottom {
            break;
        }
        let val = truncate(value, max_val);
        let pad = " ".repeat(LABEL_W.saturating_sub(label.chars().count()));
        let line = Line::from(vec![
            Span::styled(
                format!("{label}{pad}"),
                Style::default().fg(theme.faint).bg(bg),
            ),
            Span::styled("  ".to_string(), Style::default().bg(bg)),
            Span::styled(val, Style::default().fg(theme.fg).bg(bg)),
        ]);
        crate::render::strip_paint::set_line_on_strip(buf, ox, y, &line, block_w, bg);
        y += 1;
    }

    if !state.status.is_empty() {
        put(
            buf,
            area.x,
            area.bottom().saturating_sub(2),
            area.width,
            &state.status,
            Style::default().fg(theme.warn),
            bg,
        );
    }
    let pulse = state.tick % 4 < 2;
    center(
        buf,
        area,
        footer_y,
        "enter / →  start Hive",
        Style::default().fg(if pulse { theme.accent } else { theme.faint }),
        bg,
    );
    None
}

fn truncate(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let n = s.chars().count();
    if n <= width {
        return s.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// Keep host start + path end so long API URLs stay readable.
fn truncate_url(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let n = s.chars().count();
    if n <= width {
        return s.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    if width < 5 {
        return truncate(s, width);
    }
    let keep = width - 1;
    let head = keep / 2;
    let tail = keep - head;
    let mut out: String = s.chars().take(head).collect();
    out.push('…');
    out.extend(s.chars().skip(n - tail));
    out
}

#[cfg(test)]
mod done_draw_tests {
    use super::{truncate, truncate_url};

    #[test]
    fn truncate_url_keeps_ends() {
        let long = "https://very-long-provider.example.com/openai/v1";
        let t = truncate_url(long, 28);
        assert!(t.chars().count() <= 28);
        assert!(t.starts_with("https://"));
        assert!(t.ends_with("/v1") || t.ends_with('1'));
        assert!(t.contains('…'));
    }

    #[test]
    fn truncate_short_passthrough() {
        assert_eq!(truncate("abc", 8), "abc");
        assert_eq!(truncate_url("https://a.co/v1", 40), "https://a.co/v1");
    }
}
