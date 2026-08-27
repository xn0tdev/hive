//! Rendering for the hive bot hub: persona rail, chat canvas, create form.
//! Mirrors the approved mockup — flat dark panels, chips instead of borders,
//! italic for metadata.

use comb::{Frame, Line, Modifier, Rect, Span, Style};
use hive_core::message::Role;

use crate::bot::{wrap_text, BotHub, Chat, Focus};
use crate::theme::Theme;

/// Pad around chips and fields.
const PAD: u16 = 1;

pub fn draw(f: &mut Frame, hub: &BotHub) {
    let area = f.area();
    let top = 1; // row 0 stays a slim outer label

    let rail_w = (area.width * 24 / 100).clamp(16, 30);
    let form_w = if hub.form().is_some() {
        (area.width * 28 / 100).clamp(20, 36)
    } else {
        0
    };
    let chat_w = area.width.saturating_sub(rail_w + form_w);

    // Outer label row.
    let buf = f.buffer();
    buf.set_str(
        1,
        0,
        "Hive Bot",
        Style::new().fg(hub.theme().dim).add(Modifier::ITALIC),
    );

    let rail = Rect::new(0, top, rail_w, area.height.saturating_sub(top));
    let chat = Rect::new(rail_w, top, chat_w, area.height.saturating_sub(top));
    draw_panel(f.buffer(), rail, hub.theme());
    draw_panel(f.buffer(), chat, hub.theme());
    draw_rail(f, rail, hub);
    draw_chat(f, chat, hub);
    if hub.form().is_some() {
        let form = Rect::new(rail_w + chat_w, top, form_w, area.height.saturating_sub(top));
        draw_panel(f.buffer(), form, hub.theme());
        draw_form(f, form, hub);
    }
}

fn draw_panel(buf: &mut comb::Buffer, rect: Rect, theme: &Theme) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    buf.paint(rect, Style::new().bg(theme.strip));
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

fn draw_rail(f: &mut Frame, rect: Rect, hub: &BotHub) {
    let theme = hub.theme();
    let buf = f.buffer();

    buf.set_line(
        rect.x + 1,
        rect.y,
        &Line::from(vec![
            Span::styled("Hive Bot", Style::new().fg(theme.fg).add(Modifier::BOLD)),
            Span::styled(
                " (BETA)",
                Style::new().fg(theme.dim).add(Modifier::ITALIC),
            ),
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
                "No personas yet",
                Style::new().fg(theme.faint).add(Modifier::ITALIC),
            )),
            rect.width - 2,
        );
        buf.set_line(
            rect.x + 1,
            rect.y + 3,
            &Line::from(Span::styled(
                "Ctrl+N to add one",
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
        let preview = chat.preview();
        buf.set_line(
            card_x + 1,
            y,
            &Line::from(Span::styled(
                &persona.name,
                Style::new()
                    .fg(if selected { theme.accent } else { theme.fg })
                    .add(if selected { Modifier::BOLD } else { Modifier::NONE }),
            )),
            card_w - 2,
        );
        buf.set_line(
            card_x + 1,
            y + 1,
            &Line::from(Span::styled(
                preview,
                Style::new().fg(if selected { theme.dim } else { theme.faint }),
            )),
            card_w - 2,
        );
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
                "Select a persona, or Ctrl+N to create one",
                Style::new().fg(theme.faint).add(Modifier::ITALIC),
            )),
            rect.width - 4,
        );
        return;
    };

    buf.set_str(rect.x + 1, rect.y, name, Style::new().fg(theme.fg));
    if hub.is_streaming(name) {
        buf.set_str(
            rect.right().saturating_sub(3),
            rect.y,
            "…",
            Style::new().fg(theme.dim),
        );
    }

    let composer_y = rect.bottom().saturating_sub(2);
    let canvas = Rect::new(
        rect.x,
        rect.y + 1,
        rect.width,
        composer_y.saturating_sub(rect.y + 1),
    );
    draw_canvas(buf, canvas, hub, name);
    draw_composer(
        buf,
        Rect::new(rect.x + 1, composer_y, rect.width - 2, 1),
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
            Role::User => push_user_block(&mut rows, &msg.text(), text_width, theme),
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

    let streaming_here = hub
        .selected_name()
        .is_some_and(|n| hub.is_streaming(n));
    if streaming_here {
        buf.set_str(
            rect.right().saturating_sub(2),
            rect.y,
            "…",
            Style::new().fg(theme.faint),
        );
    }

    let composer = hub.composer();
    if composer.text.is_empty() {
        buf.set_line(
            rect.x + 1,
            rect.y,
            &Line::from(Span::styled(
                "Ask hive anything",
                Style::new().fg(theme.faint).add(Modifier::ITALIC),
            )),
            inner_w as u16,
        );
        return;
    }
    let line = with_caret(&composer.text, composer.caret, hub.form().is_none());
    buf.set_line(
        rect.x + 1,
        rect.y,
        &Line::from(Span::styled(line, Style::new().fg(theme.fg))),
        inner_w as u16,
    );
}

fn draw_form(f: &mut Frame, rect: Rect, hub: &BotHub) {
    let Some(form) = hub.form() else {
        return;
    };
    let theme = hub.theme();
    let buf = f.buffer();
    let inner_w = rect.width.saturating_sub(2);

    chip(
        buf,
        rect.right().saturating_sub(9),
        rect.y,
        " Close ",
        theme,
    );

    let mut y = rect.y + 2;
    buf.set_str(
        rect.x + 1,
        y,
        "Name",
        Style::new()
            .fg(if form.focus == Focus::FieldName {
                theme.fg
            } else {
                theme.dim
            })
            .add(Modifier::ITALIC),
    );
    let name_field = Rect::new(rect.x + 1, y + 1, inner_w, 1);
    buf.paint(name_field, Style::new().bg(theme.input));
    let line = with_caret(&form.name.text, form.name.caret, form.focus == Focus::FieldName);
    buf.set_line(
        name_field.x + 1,
        name_field.y,
        &Line::from(Span::styled(line, Style::new().fg(theme.fg))),
        inner_w.saturating_sub(2),
    );

    y += 3;
    buf.set_str(
        rect.x + 1,
        y,
        "Description",
        Style::new()
            .fg(if form.focus == Focus::FieldDesc {
                theme.fg
            } else {
                theme.dim
            })
            .add(Modifier::ITALIC),
    );
    let desc_field = Rect::new(rect.x + 1, y + 1, inner_w, 5.min(rect.height.saturating_sub(y + 3)));
    buf.paint(desc_field, Style::new().bg(theme.input));
    let line = with_caret(&form.desc.text, form.desc.caret, form.focus == Focus::FieldDesc);
    buf.set_line(
        desc_field.x + 1,
        desc_field.y,
        &Line::from(Span::styled(line, Style::new().fg(theme.fg))),
        inner_w.saturating_sub(2),
    );

    if desc_field.bottom() + 2 < rect.bottom() {
        chip(buf, rect.x + 1, desc_field.bottom() + 2, " Save ", theme);
    }
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
