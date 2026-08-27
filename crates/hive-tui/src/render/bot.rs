//! Rendering for the hive bot hub: persona rail, chat canvas, create form.
//! Mirrors the approved mockup — the chat canvas sits darker than the side
//! panels, chips replace borders, italics mark metadata.

use comb::{Frame, Line, Modifier, Rect, Span, Style};

use crate::bot::{wrap_text, BotHub, Chat};
use crate::theme::Theme;

/// Pad around chips and fields.
const PAD: u16 = 1;
/// Columns of outer background kept right of the panels.
const RIGHT_MARGIN: u16 = 2;

pub fn draw(f: &mut Frame, hub: &BotHub) {
    let area = f.area();

    let rail_w = (area.width * 24 / 100).clamp(16, 30);
    let chat_w = area
        .width
        .saturating_sub(RIGHT_MARGIN + rail_w)
        .max(10);

    // The rail header carries the brand; panels span the full screen height.
    let rail = Rect::new(0, 0, rail_w, area.height);
    let chat = Rect::new(rail_w, 0, chat_w, area.height);
    {
        let buf = f.buffer();
        draw_panel(buf, rail, hub.theme().strip);
        // The canvas reads darker than the side panels, like the mockup.
        draw_panel(buf, chat, hub.theme().code_bg);
    }
    draw_rail(f, rail, hub);
    draw_chat(f, chat, hub);
}

fn draw_panel(buf: &mut comb::Buffer, rect: Rect, bg: comb::Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    buf.paint(rect, Style::new().bg(bg));
}

/// Filled chip like ` Add ` — returns its width so hit-testing can reuse it.
fn chip(buf: &mut comb::Buffer, x: u16, y: u16, label: &str, theme: &Theme) -> u16 {
    let w = label.chars().count() as u16 + PAD * 2;
    buf.paint(
        Rect::new(x, y, w, 1),
        Style::new().bg(theme.strip_hover).fg(theme.fg),
    );
    buf.set_str(x + PAD, y, label, Style::new().fg(theme.fg));
    w
}

/// `Name, preview` on one line, truncating to fit. The name is plain, the
/// preview is italic metadata.
fn rail_card_label(
    name: &str,
    preview: &str,
    width: usize,
    theme: &Theme,
    selected: bool,
) -> Line {
    let name_style = Style::new()
        .fg(if selected { theme.accent } else { theme.fg })
        .add(if selected { Modifier::BOLD } else { Modifier::NONE });
    let meta_style = Style::new()
        .fg(if selected { theme.dim } else { theme.faint })
        .add(Modifier::ITALIC);

    if preview.is_empty() {
        return Line::from(vec![Span::styled(truncate(name, width), name_style)]);
    }

    // Leave room for `, ` plus at least a few preview chars.
    let name_max = width.saturating_sub(6).max(width / 2).max(1);
    let name_cut = truncate(name, name_max);
    let rest = width.saturating_sub(name_cut.chars().count() + 2);
    let mut p = truncate(preview, rest);
    if p.is_empty() {
        return Line::from(vec![Span::styled(truncate(name, width), name_style)]);
    }
    p.insert(0, ',');
    p.insert(1, ' ');
    Line::from(vec![
        Span::styled(name_cut, name_style),
        Span::styled(p, meta_style),
    ])
}

fn draw_rail(f: &mut Frame, rect: Rect, hub: &BotHub) {
    let theme = hub.theme();
    let buf = f.buffer();

    buf.set_line(
        rect.x + 1,
        rect.y,
        &Line::from(vec![
            Span::styled("Hive Bot", Style::new().fg(theme.fg).add(Modifier::BOLD)),
            Span::styled(" (BETA)", Style::new().fg(theme.dim).add(Modifier::ITALIC)),
        ]),
        rect.width.saturating_sub(10),
    );
    if rect.width > 8 {
        chip(buf, rect.right().saturating_sub(7), rect.y, " Add ", theme);
    }

    if hub.personas().is_empty() {
        buf.set_line(
            rect.x + 1,
            rect.y + 2,
            &Line::from(Span::styled(
                "No chats yet",
                Style::new().fg(theme.faint).add(Modifier::ITALIC),
            )),
            rect.width - 2,
        );
        buf.set_line(
            rect.x + 1,
            rect.y + 3,
            &Line::from(Span::styled(
                "drop a persona into",
                Style::new().fg(theme.faint).add(Modifier::ITALIC),
            )),
            rect.width - 2,
        );
        buf.set_line(
            rect.x + 1,
            rect.y + 4,
            &Line::from(Span::styled(
                ".hive/agents/Name.md",
                Style::new().fg(theme.faint).add(Modifier::ITALIC),
            )),
            rect.width - 2,
        );
        return;
    }

    let mut y = rect.y + 2;
    let card_x = rect.x + 1;
    let card_w = rect.width.saturating_sub(2);
    for (idx, persona) in hub.personas().iter().enumerate() {
        if y + 3 > rect.bottom() {
            break;
        }
        let selected = idx == hub.selected_index();
        let card = Rect::new(card_x, y, card_w, 3);
        let bg = if selected { theme.strip_hover } else { theme.input };
        buf.paint(card, Style::new().bg(bg));

        let chat = hub.chat_for(&persona.name);
        // Messenger-style: last message from either side; a fresh chat falls
        // back to what the persona is about.
        let preview = chat
            .preview()
            .unwrap_or_else(|| first_non_empty(&persona.description, "New chat").to_string());
        // Name and preview share the middle row: `Maya, *Ohhh okay…*`.
        let line = rail_card_label(
            &persona.name,
            &preview,
            card_w.saturating_sub(4) as usize,
            theme,
            selected,
        );
        buf.set_line(card_x + 1, y + 1, &line, card_w - 2);
        if hub.is_streaming(&persona.name) {
            buf.set_str(
                card_x + card_w - 2,
                y + 1,
                "…",
                Style::new().fg(theme.accent),
            );
        }
        y += 4;
    }
}

fn draw_chat(f: &mut Frame, rect: Rect, hub: &BotHub) {
    let theme = hub.theme();
    let buf = f.buffer();
    let Some(name) = hub.selected_name() else {
        buf.set_line(
            rect.x + 2,
            rect.y + rect.height / 2,
            &Line::from(Span::styled(
                "Pick a chat on the left, or add a persona file",
                Style::new().fg(theme.faint).add(Modifier::ITALIC),
            )),
            rect.width - 4,
        );
        return;
    };

    buf.set_str(rect.x + 2, rect.y, name, Style::new().fg(theme.fg));
    if hub.is_streaming(name) {
        buf.set_str(
            rect.right().saturating_sub(4),
            rect.y,
            "…",
            Style::new().fg(theme.dim),
        );
    }

    // The composer strip keeps a margin from the panel edges and sits one
    // blank row above the panel bottom, like the mockup.
    let strip_h = 3;
    let strip_y = rect.bottom().saturating_sub(strip_h + 1);
    let canvas = Rect::new(
        rect.x,
        rect.y + 1,
        rect.width,
        strip_y.saturating_sub(rect.y + 1),
    );
    draw_canvas(buf, canvas, hub, name);
    draw_composer(
        buf,
        Rect::new(rect.x + 2, strip_y, rect.width.saturating_sub(4), strip_h),
        hub,
    );
}

/// One paintable row of the transcript.
#[derive(Clone)]
struct CanvasRow {
    line: Line,
    bg: Option<comb::Color>,
    /// Right-align with this total block width (user bubbles).
    right: Option<u16>,
}

fn draw_canvas(buf: &mut comb::Buffer, rect: Rect, hub: &BotHub, name: &str) {
    if rect.width < 8 || rect.height == 0 {
        return;
    }
    let theme = hub.theme();
    let chat: Chat = hub.chat_for(name);
    let mut rows: Vec<CanvasRow> = Vec::new();
    let text_width = (rect.width - 6).max(8) as usize;

    for msg in &chat.history {
        match msg.role {
            hive_core::message::Role::User => {
                push_user_block(&mut rows, &msg.text(), text_width, theme)
            }
            _ => push_assistant_block(&mut rows, &msg.text(), text_width),
        }
        rows.push(blank_row());
    }
    if !chat.live.is_empty() {
        push_assistant_block(&mut rows, &chat.live, text_width);
    }
    while rows.last().is_some_and(blank_row_is) {
        rows.pop();
    }

    let visible: &[CanvasRow] = if rows.len() > rect.height as usize {
        &rows[rows.len() - rect.height as usize..]
    } else {
        &rows
    };
    let start_y = rect.bottom() - visible.len() as u16;
    for (offset, row) in visible.iter().enumerate() {
        let y = start_y + offset as u16;
        let (x, w) = match row.right {
            Some(block_w) => (rect.right().saturating_sub(1 + block_w), block_w),
            None => (rect.x + 2, rect.width.saturating_sub(4)),
        };
        if let Some(bg) = row.bg {
            buf.paint(Rect::new(x, y, w.min(rect.width), 1), Style::new().bg(bg));
        }
        buf.set_line(x, y, &row.line, rect.width);
    }

    if let Some(error) = &chat.error {
        let y = rect.bottom().saturating_sub(1);
        buf.set_line(
            rect.x + 2,
            y,
            &Line::from(Span::styled(
                truncate(error, (rect.width - 4) as usize),
                Style::new().fg(theme.err).add(Modifier::ITALIC),
            )),
            rect.width - 4,
        );
    }
}

fn push_user_block(rows: &mut Vec<CanvasRow>, text: &str, width: usize, theme: &Theme) {
    let lines = wrap_text(text, width.saturating_sub(2).max(8));
    let block_w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as u16 + 2;
    let bubble = CanvasRow {
        line: Line::from(Span::raw(String::new())),
        bg: Some(theme.user_strip),
        right: Some(block_w),
    };
    rows.push(bubble.clone());
    for l in lines {
        rows.push(CanvasRow {
            line: Line::from(Span::styled(format!(" {l} "), Style::new().fg(theme.fg))),
            bg: Some(theme.user_strip),
            right: Some(block_w),
        });
    }
    rows.push(bubble);
}

fn push_assistant_block(rows: &mut Vec<CanvasRow>, text: &str, width: usize) {
    for l in wrap_text(text, width) {
        rows.push(CanvasRow {
            line: Line::from(Span::raw(l)),
            bg: None,
            right: None,
        });
    }
}

fn blank_row() -> CanvasRow {
    CanvasRow {
        line: Line::from(Span::raw(String::new())),
        bg: None,
        right: None,
    }
}

fn blank_row_is(row: &CanvasRow) -> bool {
    row.bg.is_none() && row.right.is_none()
}

fn draw_composer(buf: &mut comb::Buffer, rect: Rect, hub: &BotHub) {
    let theme = hub.theme();
    buf.paint(rect, Style::new().bg(theme.input));
    let inner_w = (rect.width - 2) as usize;
    let text_y = rect.y + rect.height / 2;

    let streaming_here = hub.selected_name().is_some_and(|n| hub.is_streaming(n));
    if streaming_here {
        buf.set_str(
            rect.right().saturating_sub(2),
            text_y,
            "…",
            Style::new().fg(theme.faint),
        );
    }

    let composer = hub.composer();
    if composer.text.is_empty() {
        buf.set_line(
            rect.x + 1,
            text_y,
            &Line::from(Span::styled(
                "Ask hive anything",
                Style::new().fg(theme.faint).add(Modifier::ITALIC),
            )),
            inner_w as u16,
        );
        return;
    }
    let line = with_caret(&composer.text, composer.caret, true);
    buf.set_line(
        rect.x + 1,
        text_y,
        &Line::from(Span::styled(line, Style::new().fg(theme.fg))),
        inner_w as u16,
    );
}

/// Text with a block caret spliced in while the field is focused.
fn with_caret(text: &str, caret: usize, focused: bool) -> String {
    if !focused {
        return text.to_string();
    }
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i == caret {
            out.push('▌');
        }
        out.push(c);
    }
    if caret >= text.chars().count() {
        out.push('▌');
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn first_non_empty<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.trim().is_empty() {
        b
    } else {
        a
    }
}
