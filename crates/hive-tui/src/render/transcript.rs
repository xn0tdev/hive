//! Builds and draws the scrolling conversation transcript. The greeting lives
//! only on the landing screen; the turn working cubes sit by the footer mode chip.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use hive_core::event::{SubagentLine, SubagentStatus};

use crate::app::state::{
    AssistantResponseRow, AssistantRowHit, AssistantRowJoin, Block as UiBlock, ChatView,
    SubagentCard, ToolCard, ToolStatus,
};
use crate::app::{App, MdRows};
use crate::render::tools::{
    compacted_card_lines, format_tool_secs, goal_card_lines, loop_detected_card_lines,
    mode_switch_card_lines, plan_card_lines, subagent_card_lines, terminal_card_lines,
    todo_card_lines, tool_lines, work_summary_card_lines, SPINNER_COL, TITLE_ROW,
};
use crate::render::{markdown, wrap};

struct BuiltAssistantRow {
    line_idx: usize,
    block: usize,
    response_row: usize,
    text: String,
    /// Screen columns from the transcript origin to the selectable text.
    x_off: u16,
    join_before: AssistantRowJoin,
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.is_empty() {
        app.click_hits.clear();
        app.assistant_row_hits.clear();
        app.assistant_rows.clear();
        app.set_transcript_max_scroll(0);
        return;
    }
    // Clear the whole transcript band, including the single top inset, so
    // scroll / shorter lines never leave stale glyphs in vacated cells.
    buf.paint(area, Style::default());

    let width = area.width.max(1) as usize;
    let content_width = width.saturating_sub(2);
    if app.assistant_selection.is_some() && app.assistant_selection_width != content_width {
        app.assistant_selection = None;
    }
    app.assistant_selection_width = content_width;

    if matches!(app.view, ChatView::Main) {
        let animation_epoch = if transcript_has_moving_blocks(app) {
            // Footer animation stays at 10 FPS; transcript elapsed labels and
            // card spinners only require a low-frequency refresh.
            app.spinner / 10
        } else {
            0
        };
        let mut cache = std::mem::take(&mut app.transcript_cache);
        let rebuild = !cache.valid
            || cache.width != width
            || cache.revision != app.transcript_revision
            || cache.animation_epoch != animation_epoch
            || cache.hover_block != app.hover_block
            || cache.show_tool_cards != app.ui.show_tool_cards;
        if rebuild {
            let (lines, heads, rows) = build(app, width);
            cache.width = width;
            cache.revision = app.transcript_revision;
            cache.animation_epoch = animation_epoch;
            cache.hover_block = app.hover_block;
            cache.show_tool_cards = app.ui.show_tool_cards;
            cache.lines = lines;
            cache.heads = heads;
            cache.assistant_rows = rows.into_iter().map(AssistantResponseRow::from).collect();
            cache.builds = cache.builds.saturating_add(1);
            cache.valid = true;
            app.assistant_rows = cache.assistant_rows.clone();
        } else if app.assistant_rows.len() != cache.assistant_rows.len() {
            app.assistant_rows = cache.assistant_rows.clone();
        }
        draw_layout(
            buf,
            area,
            app,
            &cache.lines,
            &cache.heads,
            &cache.assistant_rows,
        );
        app.transcript_cache = cache;
    } else {
        let (lines, heads, rows) = build(app, width);
        let rows: Vec<AssistantResponseRow> =
            rows.into_iter().map(AssistantResponseRow::from).collect();
        app.assistant_rows = rows.clone();
        draw_layout(buf, area, app, &lines, &heads, &rows);
    }
}

impl From<BuiltAssistantRow> for AssistantResponseRow {
    fn from(row: BuiltAssistantRow) -> Self {
        Self {
            line_idx: row.line_idx,
            block: row.block,
            response_row: row.response_row,
            text: row.text,
            x_off: row.x_off,
            join_before: row.join_before,
        }
    }
}

fn transcript_has_moving_blocks(app: &App) -> bool {
    app.blocks.iter().any(|block| match block {
        UiBlock::Tool(card) => card.status == ToolStatus::Running,
        UiBlock::Subagent(card) => card.status == SubagentStatus::Running,
        UiBlock::Plan(card) => card.status == crate::app::state::PlanStatus::Writing,
        UiBlock::Terminal(card) => {
            matches!(card.process, hive_core::TerminalProcessState::Running)
        }
        UiBlock::Reasoning(thought) => thought.elapsed_ms.is_none(),
        UiBlock::Assistant {
            streaming: true, ..
        } => true,
        UiBlock::Compacted(card) => card.before.is_none(),
        UiBlock::Goal(_) => app.goal.as_ref().is_some_and(|goal| !goal.paused),
        _ => false,
    })
}

fn draw_layout(
    buf: &mut Buffer,
    area: Rect,
    app: &mut App,
    lines: &[Line],
    heads: &[(usize, usize)],
    assistant_rows: &[AssistantResponseRow],
) {
    let target = Rect {
        y: area.y.saturating_add(1),
        height: area.height.saturating_sub(1),
        ..area
    };
    let viewport = target.height as usize;
    let max_scroll = lines.len().saturating_sub(viewport);
    app.set_transcript_max_scroll(max_scroll);
    let scroll = max_scroll.saturating_sub(app.scroll_from_bottom);

    app.click_hits.clear();
    for &(line_idx, block_idx) in heads {
        if line_idx >= scroll && line_idx < scroll + target.height as usize {
            app.click_hits
                .push((target.y + (line_idx - scroll) as u16, block_idx));
        }
    }
    app.assistant_row_hits.clear();
    for row in assistant_rows {
        if row.line_idx >= scroll && row.line_idx < scroll + target.height as usize {
            app.assistant_row_hits.push(AssistantRowHit {
                block: row.block,
                response_row: row.response_row,
                screen_row: target.y + (row.line_idx - scroll) as u16,
                x: target.x.saturating_add(row.x_off),
                text: row.text.clone(),
                join_before: row.join_before,
            });
        }
    }

    buf.set_lines(target, lines, scroll);
    paint_live_thought_shimmer(buf, app, heads, scroll, target);
    paint_running_subagent_spinners(buf, app, heads, scroll, target);
    paint_assistant_selection(buf, app);
    app.transcript_hit = Some(target);
}

fn assistant_line_meta(line: &Line) -> (u16, String) {
    let rendered: String = line
        .spans
        .iter()
        .map(|span| span.content.as_str())
        .collect();
    let rest = rendered
        .strip_prefix("  ")
        .or_else(|| rendered.strip_prefix("│ "))
        .unwrap_or(rendered.as_str());
    if markdown::code_line_marker(line).is_some() {
        if let Some(inner) = rest.strip_prefix("│ ") {
            let inner = inner
                .strip_suffix(" │")
                .or_else(|| inner.strip_suffix('│'))
                .unwrap_or(inner);
            return (4, inner.trim_end().to_string());
        }
        return (2, rest.trim_end().to_string());
    }
    (2, rest.trim_end().to_string())
}

fn paint_assistant_selection(buf: &mut Buffer, app: &App) {
    for hit in &app.assistant_row_hits {
        let Some((start, end)) = app.assistant_selected_cols(hit.block, hit.response_row) else {
            continue;
        };
        let start = u16::try_from(start).unwrap_or(u16::MAX);
        let end = u16::try_from(end).unwrap_or(u16::MAX);
        for x in hit.x.saturating_add(start)..hit.x.saturating_add(end) {
            if let Some(cell) = buf.cell_mut(x, hit.screen_row) {
                cell.style = cell.style.bg(app.theme.assistant_selection_bg);
            }
        }
    }
}

/// Overlay the live Thinking header without rebuilding the transcript cache.
/// Three grey steps, coalesced into runs, so the ANSI diff stays a handful of
/// cells instead of a unique SGR per character.
///
/// `heads` also lists the live sentence rows so a click toggles the thought —
/// shimmer belongs only on the header, which is the first heads entry per block.
fn paint_live_thought_shimmer(
    buf: &mut Buffer,
    app: &App,
    heads: &[(usize, usize)],
    scroll: usize,
    target: Rect,
) {
    let view_end = scroll + target.height as usize;
    let mut painted = Vec::new();
    for &(line_idx, block_idx) in heads {
        let Some(UiBlock::Reasoning(th)) = app.blocks.get(block_idx) else {
            continue;
        };
        if !thought_is_live(th, app) {
            continue;
        }
        if painted.contains(&block_idx) {
            continue;
        }
        painted.push(block_idx);
        if line_idx < scroll || line_idx >= view_end {
            continue;
        }
        let label = live_thought_label(th, app);
        let y = target.y + (line_idx - scroll) as u16;
        let x = target.x.saturating_add(2);
        let spans = shimmer_spans(label, app.spinner, &app.theme);
        let w = label.chars().count() as u16;
        buf.set_line(x, y, &Line::from(spans), w);
    }
}

/// Overlay the live braille frame on running subagent cards without rebuilding
/// the transcript cache. Duration labels still refresh on the 1s epoch.
fn paint_running_subagent_spinners(
    buf: &mut Buffer,
    app: &App,
    heads: &[(usize, usize)],
    scroll: usize,
    target: Rect,
) {
    let view_end = scroll + target.height as usize;
    let ch = app.spinner_glyph();
    let style = Style::default().fg(app.theme.accent);
    let mut painted = Vec::new();
    for &(line_idx, block_idx) in heads {
        if painted.contains(&block_idx) {
            continue;
        }
        let Some(UiBlock::Subagent(card)) = app.blocks.get(block_idx) else {
            continue;
        };
        if card.status != SubagentStatus::Running {
            continue;
        }
        painted.push(block_idx);
        let title_idx = line_idx.saturating_add(TITLE_ROW);
        if title_idx < scroll || title_idx >= view_end {
            continue;
        }
        let y = target.y + (title_idx - scroll) as u16;
        let x = target.x.saturating_add(SPINNER_COL);
        buf.set(x, y, ch, style);
    }
}

fn live_thought_label(th: &crate::app::state::Thought, app: &App) -> &'static str {
    match app.activity_label() {
        "Thinking" if th.secs() > 10.0 => "Thinking hard",
        other => other,
    }
}

/// Three luminance steps, merged into runs so adjacent cells share a style.
fn shimmer_spans(text: &str, tick: usize, theme: &crate::theme::Theme) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let n = chars.len() as i32;
    let head = ((tick as i32) / 2) % (n + 4) - 2;
    let color = |level: u8| match level {
        2 => theme.accent,
        1 => theme.fg,
        _ => theme.dim,
    };
    let level_of = |i: i32| {
        let d = (i - head).abs();
        if d == 0 {
            2
        } else if d == 1 {
            1
        } else {
            0
        }
    };
    let mut spans = Vec::new();
    let mut start = 0;
    let mut cur = level_of(0);
    for i in 1..=chars.len() {
        let next = if i == chars.len() {
            cur
        } else {
            level_of(i as i32)
        };
        if i == chars.len() || next != cur {
            let s: String = chars[start..i].iter().collect();
            spans.push(Span::styled(
                s,
                Style::default().fg(color(cur)).add(Modifier::BOLD),
            ));
            start = i;
            cur = next;
        }
    }
    spans
}

/// Build the full, pre-wrapped set of transcript lines (test helper).
#[cfg(test)]
pub fn lines(app: &mut App, width: usize) -> Vec<Line> {
    build(app, width).0
}

/// Like `lines`, but also reports which line index holds each expandable
/// header (thought / subagent) with its block index for mouse hit-testing.
fn build(app: &mut App, width: usize) -> (Vec<Line>, Vec<(usize, usize)>, Vec<BuiltAssistantRow>) {
    if matches!(app.view, ChatView::Subagent(_)) {
        // Clone the card snapshot so we can still use `app` mutably for caches.
        if let Some(card) = app.viewed_subagent().cloned() {
            return (
                subagent_chat_lines(&card, app, width),
                Vec::new(),
                Vec::new(),
            );
        }
        // Stale id (cleared chat) — fall through to main.
    }
    if matches!(app.view, ChatView::Plan) {
        return (plan_preview_lines(app, width), Vec::new(), Vec::new());
    }

    let mut out: Vec<Line> = Vec::new();
    let mut heads: Vec<(usize, usize)> = Vec::new();
    let mut assistant_rows: Vec<BuiltAssistantRow> = Vec::new();
    let n = app.blocks.len();

    // Precompute the last index of each "expandable" block kind in one O(n)
    // reverse pass, so the per-block hint check is O(1) instead of O(n²).
    let mut last_thought = None;
    let mut last_subagent = None;
    let mut last_plan = None;
    let mut last_terminal = None;
    for (i, b) in app.blocks.iter().enumerate().rev() {
        if last_thought.is_none() && matches!(b, UiBlock::Reasoning(_)) {
            last_thought = Some(i);
        }
        if last_subagent.is_none() && matches!(b, UiBlock::Subagent(_)) {
            last_subagent = Some(i);
        }
        if last_plan.is_none() && matches!(b, UiBlock::Plan(_)) {
            last_plan = Some(i);
        }
        if last_terminal.is_none() && matches!(b, UiBlock::Terminal(_)) {
            last_terminal = Some(i);
        }
        if last_thought.is_some()
            && last_subagent.is_some()
            && last_plan.is_some()
            && last_terminal.is_some()
        {
            break;
        }
    }

    let mut previous_visible = None;
    for i in 0..n {
        // Welcome belongs only to the landing screen, and completed tools can
        // be hidden by preference. Neither may leave a separator behind.
        let hidden = matches!(&app.blocks[i], UiBlock::Welcome)
            || matches!(
                &app.blocks[i],
                UiBlock::Tool(card) if !app.ui.show_tool_cards && card.status == ToolStatus::Ok
            );
        if hidden {
            continue;
        }

        // Spacing exists only between blocks that actually rendered. This
        // keeps the first chat row flush with the viewport's top edge.
        let separator_added = if let Some(previous) = previous_visible {
            let prev = &app.blocks[previous];
            let curr = &app.blocks[i];
            let tight = matches!(
                (prev, curr),
                (UiBlock::Tool(_), UiBlock::Tool(_)) | (UiBlock::Notice(_), UiBlock::Notice(_))
            );
            if !tight {
                out.push(Line::from(""));
            }
            !tight
        } else {
            false
        };
        let content_start = out.len();

        let has_later_thought = last_thought.is_some_and(|l| i < l);
        let has_later_subagent = last_subagent.is_some_and(|l| i < l);
        let has_later_plan = last_plan.is_some_and(|l| i < l);
        let has_later_terminal = last_terminal.is_some_and(|l| i < l);

        match &app.blocks[i] {
            // The greeting only lives on the landing screen (the ASCII wordmark);
            // in the active chat we show nothing but the conversation.
            UiBlock::Welcome => {}
            UiBlock::User(text) => {
                let text = text.clone();
                let hovered = app.hover_block == Some(i);
                let start = out.len();
                out.extend(user_lines(&text, app, width, hovered));
                for line_idx in start..out.len() {
                    heads.push((line_idx, i));
                }
            }
            UiBlock::Assistant { text, streaming } => {
                let content_w = width.saturating_sub(2);
                let streaming = *streaming;
                let text = text.clone();
                let theme = app.theme.clone();
                let mut wrapped = if streaming {
                    let body = markdown::plain(&text, &theme);
                    indent(wrap::wrap_lines(body, content_w), width)
                } else {
                    let rows = app.md_cache.rows(&text, content_w, || {
                        let body = markdown::render(&text, &theme, content_w);
                        let wrapped = wrap::wrap_lines_with_joins(body, content_w);
                        let joins = wrapped
                            .iter()
                            .map(|row| match row.join_before {
                                wrap::WrapJoin::Hard => AssistantRowJoin::Hard,
                                wrap::WrapJoin::SoftSpace => AssistantRowJoin::SoftSpace,
                                wrap::WrapJoin::SoftNone => AssistantRowJoin::SoftNone,
                            })
                            .collect();
                        MdRows {
                            lines: indent(
                                wrapped.into_iter().map(|row| row.line).collect(),
                                width,
                            ),
                            joins,
                        }
                    });
                    let start = out.len();
                    for (response_row, line) in rows.lines.iter().enumerate() {
                        let (x_off, text) = assistant_line_meta(line);
                        assistant_rows.push(BuiltAssistantRow {
                            line_idx: start + response_row,
                            block: i,
                            response_row,
                            text,
                            x_off,
                            join_before: rows
                                .joins
                                .get(response_row)
                                .copied()
                                .unwrap_or(AssistantRowJoin::Hard),
                        });
                    }
                    rows.lines
                };
                if streaming {
                    push_caret(&mut wrapped, theme.accent);
                }
                out.extend(wrapped);
            }
            UiBlock::Reasoning(th) => {
                // Hint only on older thoughts; the latest one stays clean.
                let show_hint = has_later_thought;
                let open = th.open;
                let body = if open && !th.text.trim().is_empty() {
                    Some(th.text.clone())
                } else {
                    None
                };
                // Snapshot fields used by the header so we don't hold a blocks borrow.
                let th_snap = crate::app::state::Thought {
                    text: th.text.clone(),
                    started: th.started,
                    elapsed_ms: th.elapsed_ms,
                    open: th.open,
                };
                heads.push((out.len(), i));
                out.push(thought_header(&th_snap, app, show_hint));
                // Live and collapsed: a short window on the train of thought,
                // newest brightest, older ones fading out behind it. A fixed
                // number of one-row sentences — the window slides, it never
                // grows, and the transcript below it doesn't shift.
                if body.is_none() && thought_is_live(&th_snap, app) {
                    let window = recent_sentences(
                        &th_snap.text,
                        THOUGHT_WINDOW,
                        width.saturating_sub(QUOTE_W),
                    );
                    let n = window.len();
                    for (k, sentence) in window.into_iter().enumerate() {
                        heads.push((out.len(), i));
                        out.push(Line::from(vec![
                            quote_pad(),
                            Span::styled(
                                sentence,
                                Style::default()
                                    .fg(fade(&app.theme, k, n))
                                    .add(Modifier::ITALIC),
                            ),
                        ]));
                    }
                }
                if let Some(text) = body {
                    let style = Style::default().fg(app.theme.faint).add(Modifier::ITALIC);
                    let raw: Vec<Line> = text
                        .split('\n')
                        .map(|l| Line::from(Span::styled(l.to_string(), style)))
                        .collect();
                    for mut l in wrap::wrap_lines(raw, width.saturating_sub(QUOTE_W)) {
                        l.spans.insert(0, quote_pad());
                        // Click anywhere on the thought body also toggles it.
                        heads.push((out.len(), i));
                        out.push(l);
                    }
                }
            }
            UiBlock::Subagent(card) => {
                let show_hint = !has_later_subagent;
                let card = card.clone();
                let hovered = app.hover_block == Some(i);
                // Every row of the soft strip is hover/click — not just the title.
                let start = out.len();
                out.extend(subagent_card_lines(&card, app, width, show_hint, hovered));
                for line_idx in start..out.len() {
                    heads.push((line_idx, i));
                }
            }
            UiBlock::Plan(card) => {
                let show_hint = !has_later_plan;
                let card = card.clone();
                let hovered = app.hover_block == Some(i);
                let start = out.len();
                out.extend(plan_card_lines(&card, app, width, show_hint, hovered));
                for line_idx in start..out.len() {
                    heads.push((line_idx, i));
                }
            }
            UiBlock::Terminal(card) => {
                let show_hint = !has_later_terminal;
                let hovered = app.hover_block == Some(i);
                let start = out.len();
                out.extend(terminal_card_lines(card, app, width, show_hint, hovered));
                for line_idx in start..out.len() {
                    heads.push((line_idx, i));
                }
            }
            UiBlock::Tool(card) => {
                // tool_lines only needs a few fields; clone the small card.
                let card = ToolCard {
                    id: card.id.clone(),
                    name: card.name.clone(),
                    args: card.args.clone(),
                    output: card.output.clone(),
                    status: card.status,
                    started: card.started,
                    elapsed_ms: card.elapsed_ms,
                    details_open: card.details_open,
                    snapshot: None,
                };
                let hovered = app.hover_block == Some(i);
                // Every row is hover/click, like the other cards — that's what
                // reaches the Copy / Revert menu.
                let start = out.len();
                out.extend(tool_lines(&card, app, width, hovered));
                for line_idx in start..out.len() {
                    heads.push((line_idx, i));
                }
            }
            UiBlock::ModeSwitch(card) => {
                let card = card.clone();
                out.extend(mode_switch_card_lines(&card, app, width));
            }
            UiBlock::LoopDetected(_) => {
                out.extend(loop_detected_card_lines(app, width));
            }
            UiBlock::Compacted(card) => {
                let card = card.clone();
                out.extend(compacted_card_lines(&card, app, width));
            }
            UiBlock::WorkSummary(card) => {
                let card = card.clone();
                let hovered = app.hover_block == Some(i);
                let start = out.len();
                out.extend(work_summary_card_lines(&card, app, width, hovered));
                for line_idx in start..out.len() {
                    heads.push((line_idx, i));
                }
            }
            UiBlock::Goal(card) => {
                let card = card.clone();
                out.extend(goal_card_lines(&card, app, width));
            }
            UiBlock::GoalCircle(n) => {
                let line_color = comb::Color::Rgb(0x40, 0x40, 0x40);
                let text = format!(" Circle {} ", n);
                let text_w = text.chars().count();
                let fill = width.saturating_sub(text_w);
                let left = fill / 2;
                let right = fill.saturating_sub(left);
                out.push(Line::from(vec![
                    Span::styled("─".repeat(left), Style::default().fg(line_color)),
                    Span::styled(text, Style::default().fg(app.theme.dim)),
                    Span::styled("─".repeat(right), Style::default().fg(line_color)),
                ]));
            }
            UiBlock::Todos(items) => {
                let items = items.clone();
                out.extend(todo_card_lines(&items, app, width));
            }
            UiBlock::Notice(s) => {
                let s = s.clone();
                out.extend(wrap::wrap_lines(
                    vec![Line::from(vec![
                        Span::styled("  · ", Style::default().fg(app.theme.faint)),
                        Span::styled(s, Style::default().fg(app.theme.dim)),
                    ])],
                    width,
                ));
            }
            UiBlock::Error(s) => {
                let s = s.clone();
                out.extend(wrap::wrap_lines(
                    vec![Line::from(vec![
                        Span::styled("  Error  ", Style::default().fg(app.theme.faint)),
                        Span::styled(s, Style::default().fg(app.theme.dim)),
                    ])],
                    width,
                ));
            }
        }

        if out.len() == content_start {
            if separator_added {
                out.pop();
            }
        } else {
            previous_visible = Some(i);
        }
    }

    (out, heads, assistant_rows)
}

/// Markdown preview of Plan.md with section cursor / selection highlights.
fn plan_preview_lines(app: &mut App, width: usize) -> Vec<Line> {
    use crate::render::tools::{layout_plan_body, paint_highlights, MarkTone};

    let theme = app.theme.clone();
    let body = app.plan_card().map(|c| c.body.clone()).unwrap_or_default();

    let mut out = Vec::new();
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "Plan.md".to_string(),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        ),
        Span::styled(
            "  drag · comment · MARK · click to edit · SEND".to_string(),
            Style::default().fg(theme.faint),
        ),
    ]));
    out.push(Line::from(""));

    let body_line0 = out.len();
    let (mut rendered, spans) = layout_plan_body(&body, width, &theme);

    let mut ranges: Vec<(usize, usize, MarkTone)> = app
        .plan_view
        .corrections
        .iter()
        .map(|c| {
            let tone = if c.note.trim().is_empty() {
                MarkTone::Pending
            } else {
                MarkTone::Noted
            };
            (c.start, c.end, tone)
        })
        .collect();
    if let Some((a, b)) = app.plan_view.drag_range() {
        ranges.push((a, b, MarkTone::Pending));
    }
    if !ranges.is_empty() {
        paint_highlights(&mut rendered, &spans, &body, &ranges, &theme);
    }

    app.plan_view.body_line0 = body_line0;
    app.plan_view.row_spans = spans.iter().map(|s| (s.start, s.end)).collect();

    out.append(&mut rendered);
    out
}

/// Dedicated read-only chat for a subagent: task as a user strip, then its
/// thoughts / tools / assistant messages — same visual language as the main chat.
fn subagent_chat_lines(card: &SubagentCard, app: &mut App, width: usize) -> Vec<Line> {
    let theme = app.theme.clone();
    let mut out: Vec<Line> = Vec::new();

    let title = if card.label.trim().is_empty() {
        "Checking project"
    } else {
        card.label.as_str()
    };
    let (icon, icon_fg) = match card.status {
        SubagentStatus::Running => (app.spinner_char().to_string(), theme.accent),
        SubagentStatus::Done => ("✓".to_string(), theme.ok),
        SubagentStatus::Failed => ("✗".to_string(), theme.err),
    };
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{icon} "), Style::default().fg(icon_fg)),
        Span::styled(
            title.to_string(),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · {}", format_tool_secs(card.secs())),
            Style::default().fg(theme.dim),
        ),
    ]));
    out.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(
            card.status_text().to_string(),
            Style::default().fg(theme.dim),
        ),
    ]));
    out.push(Line::from(""));

    if !card.prompt.trim().is_empty() {
        out.extend(user_lines(&card.prompt, app, width, false));
        out.push(Line::from(""));
    }

    for line in &card.lines {
        match line {
            SubagentLine::Thinking(t) if !t.trim().is_empty() => {
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        "Thinking",
                        Style::default().fg(theme.fg).add(Modifier::BOLD),
                    ),
                ]));
                let style = Style::default().fg(theme.faint).add(Modifier::ITALIC);
                let raw: Vec<Line> = t
                    .lines()
                    .map(|l| Line::from(Span::styled(l.to_string(), style)))
                    .collect();
                for mut l in wrap::wrap_lines(raw, width.saturating_sub(QUOTE_W)) {
                    l.spans.insert(0, quote_pad());
                    out.push(l);
                }
                out.push(Line::from(""));
            }
            SubagentLine::Assistant(t) if !t.trim().is_empty() => {
                let content_w = width.saturating_sub(2);
                let t = t.clone();
                let theme = theme.clone();
                let wrapped = app.md_cache.lines(&t, content_w, || {
                    let body = markdown::render(&t, &theme, content_w);
                    indent(wrap::wrap_lines(body, content_w), width)
                });
                out.extend(wrapped);
                out.push(Line::from(""));
            }
            SubagentLine::Tool {
                name,
                args,
                summary,
                ok,
            } => {
                let status = match ok {
                    None => ToolStatus::Running,
                    Some(true) => ToolStatus::Ok,
                    Some(false) => ToolStatus::Err,
                };
                let args_line = if !args.is_empty() {
                    args.clone()
                } else {
                    summary.clone()
                };
                let tool = ToolCard {
                    id: String::new(),
                    name: name.clone(),
                    args: args_line,
                    output: summary.clone(),
                    status,
                    started: std::time::Instant::now(),
                    elapsed_ms: Some(0),
                    details_open: false,
                    snapshot: None,
                };
                // Read-only subagent thread: nothing here is clickable.
                out.extend(tool_lines(&tool, app, width, false));
                out.push(Line::from(""));
            }
            SubagentLine::Notice(t) => {
                out.push(Line::from(vec![
                    Span::styled("  · ", Style::default().fg(theme.faint)),
                    Span::styled(t.clone(), Style::default().fg(theme.dim)),
                ]));
                out.push(Line::from(""));
            }
            _ => {}
        }
    }

    out
}

/// A thought the model is still writing: the header shimmers and carries a
/// live tail. Once it closes, the same block becomes a "Thought for Ns" summary.
fn thought_is_live(th: &crate::app::state::Thought, app: &App) -> bool {
    th.elapsed_ms.is_none() && app.running
}

/// Quoted reasoning sits flush under its own header, with no rule and no extra
/// indent. The fade already tells it apart from the answer below, so a rule
/// only put a column of chrome between the label and the text under it.
const QUOTE: &str = "  ";
const QUOTE_W: usize = 2;

fn quote_pad() -> Span {
    Span::raw(QUOTE)
}

/// `4.2s` · `47s` · `1m 04s`. Live headers round to whole seconds so the
/// decimal doesn't flicker while the counter ticks.
fn think_secs(secs: f64, live: bool) -> String {
    if secs >= 60.0 {
        let t = secs.round() as u64;
        return format!("{}m {:02}s", t / 60, t % 60);
    }
    // Keep the decimal only where it reads as precision: a live counter would
    // just flicker it, and a long or instant think has nothing to say with it.
    if !live && (0.05..10.0).contains(&secs) {
        format!("{secs:.1}s")
    } else {
        format!("{secs:.0}s")
    }
}

/// How many sentences of a live thought stay on screen at once.
const THOUGHT_WINDOW: usize = 3;

/// Fade across the thought window: the oldest sentence sits at `faint`, the
/// newest at `dim`, so the train of thought reads newest-first and none of it
/// competes with the answer below.
fn fade(theme: &crate::theme::Theme, i: usize, n: usize) -> Color {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (theme.faint, theme.dim) else {
        return theme.dim;
    };
    if n <= 1 {
        return theme.dim;
    }
    let t = i as f32 / (n - 1) as f32;
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color::Rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1))
}

/// Split reasoning into sentences. A newline ends one too, and a `.` only
/// counts when whitespace follows — otherwise every `3.5` splits in two.
fn split_sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        let breaks = match c {
            '\n' => true,
            '.' | '!' | '?' => text[end..].chars().next().is_none_or(char::is_whitespace),
            _ => false,
        };
        if !breaks {
            continue;
        }
        let piece = text[start..end].trim();
        if !piece.is_empty() {
            out.push(piece);
        }
        start = end;
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// The last `want` sentences of a live thought, oldest first, one row each.
///
/// The window slides as the model writes, so the block keeps its height and
/// the transcript under it never shifts.
fn recent_sentences(text: &str, want: usize, width: usize) -> Vec<String> {
    if want == 0 || width == 0 {
        return Vec::new();
    }
    let all = split_sentences(text);
    let last = all.len().saturating_sub(1);
    all.iter()
        .enumerate()
        .skip(all.len().saturating_sub(want))
        .map(|(i, s)| {
            let one_row = s.split_whitespace().collect::<Vec<_>>().join(" ");
            fit(&one_row, width, i == last)
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Squeeze a sentence onto one row, marking where it was cut. The newest
/// sentence is still being written, so it follows the writing head; the ones
/// behind it have settled and read from the start.
fn fit(s: &str, width: usize, from_end: bool) -> String {
    let n = s.chars().count();
    if n <= width {
        return s.to_string();
    }
    if from_end {
        std::iter::once('…')
            .chain(s.chars().skip(n - (width - 1)))
            .collect()
    } else {
        s.chars()
            .take(width - 1)
            .chain(std::iter::once('…'))
            .collect()
    }
}

/// Thought header: bold shimmering "Thinking" while active, then a clear
/// "Thought for Ns" summary. Click (or ctrl+t) to expand/collapse the body.
fn thought_header(th: &crate::app::state::Thought, app: &App, show_hint: bool) -> Line {
    let theme = &app.theme;
    let active = thought_is_live(th, app);

    let mut spans: Vec<Span> = vec![Span::raw("  ")];
    if active {
        // Static header in the cached line; draw_layout overlays the shimmer
        // on the visible row so a 10 FPS tick does not rebuild history.
        let label = live_thought_label(th, app);
        spans.push(Span::styled(
            label.to_string(),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("  {}", think_secs(th.secs(), true)),
            Style::default().fg(theme.dim),
        ));
    } else {
        spans.push(Span::styled(
            format!("Thought for {}", think_secs(th.secs(), false)),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        ));
        if th.approx_tokens() > 0 {
            spans.push(Span::styled(
                format!(" · ~{} tokens", th.approx_tokens()),
                Style::default().fg(theme.dim),
            ));
        }
        if show_hint {
            spans.push(Span::styled(
                if th.open {
                    "  click to hide"
                } else {
                    "  click to show"
                },
                Style::default().fg(theme.faint),
            ));
        }
    }
    Line::from(spans)
}

/// User message: a full-width gray strip like the input bar — one tinted
/// padding row above and below, text rows in the middle, lightly inset.
/// `@path` chips keep the accent `@` so attachments read like the composer.
fn user_lines(text: &str, app: &App, width: usize, hovered: bool) -> Vec<Line> {
    let theme = &app.theme;
    let bg = if hovered {
        theme.user_strip_hover
    } else {
        theme.user_strip
    };
    let body = Style::default().fg(theme.fg).bg(bg);
    let pad_row = || Line::from(Span::styled(" ".repeat(width), body));

    let raw: Vec<Line> = text
        .split('\n')
        .map(|l| Line::from(style_user_line(l, theme, bg)))
        .collect();
    let wrapped = wrap::wrap_lines(raw, width.saturating_sub(4));

    let mut out = vec![pad_row()];
    out.extend(wrapped.into_iter().map(|line| {
        let mut used = 2usize;
        let mut spans = vec![Span::styled("  ", body)];
        // Re-tint the content spans onto the strip background.
        for s in line.spans {
            used += s.width();
            spans.push(Span::styled(s.content, s.style.bg(bg)));
        }
        if width > used {
            spans.push(Span::styled(" ".repeat(width - used), body));
        }
        Line::from(spans)
    }));
    out.push(pad_row());
    out
}

fn style_user_line(text: &str, theme: &crate::theme::Theme, bg: Color) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some(at) = rest.find('@') else {
            spans.push(Span::styled(
                rest.to_string(),
                Style::default().fg(theme.fg).bg(bg),
            ));
            break;
        };
        if at > 0 {
            spans.push(Span::styled(
                rest[..at].to_string(),
                Style::default().fg(theme.fg).bg(bg),
            ));
        }
        let after = &rest[at + 1..];
        let end = after
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(after.len());
        let label = &after[..end];
        if label.is_empty() {
            spans.push(Span::styled("@", Style::default().fg(theme.fg).bg(bg)));
            rest = after;
            continue;
        }
        spans.push(Span::styled(
            "@",
            Style::default().fg(theme.accent).bg(bg).add(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            label.to_string(),
            Style::default().fg(theme.dim).bg(bg),
        ));
        rest = &after[end..];
    }
    if spans.is_empty() {
        spans.push(Span::styled(
            String::new(),
            Style::default().fg(theme.fg).bg(bg),
        ));
    }
    spans
}

fn indent(lines: Vec<Line>, _width: usize) -> Vec<Line> {
    lines
        .into_iter()
        .map(|mut l| {
            if markdown::code_line_marker(&l).is_some() {
                l.spans.insert(1, Span::raw("  "));
                return l;
            }
            let pad = match indent_fill_bg(&l) {
                Some(bg) => Span::styled("  ", Style::default().bg(bg)),
                None => Span::raw("  "),
            };
            l.spans.insert(0, pad);
            l
        })
        .collect()
}

/// Background for the 2-col assistant gutter, if any.
///
/// Table chrome may carry its surface into the assistant gutter. Inline `code`
/// chips must not — especially after a wrap, when the next row starts with one.
fn indent_fill_bg(line: &Line) -> Option<Color> {
    let plain: String = line.spans.iter().map(|s| s.content.as_str()).collect();
    let table_chrome = plain.starts_with('┌')
        || plain.starts_with('└')
        || plain.starts_with('├')
        || plain.starts_with('│');
    if !table_chrome {
        return None;
    }
    line.spans
        .iter()
        .find(|s| !s.content.is_empty())
        .and_then(|s| s.style.bg)
}

fn push_caret(lines: &mut Vec<Line>, color: Color) {
    // Thin bar — the fat `▊` slab reads as a gray patch after the text.
    match lines.last_mut() {
        Some(last) => last
            .spans
            .push(Span::styled("▏", Style::default().fg(color))),
        None => lines.push(Line::from(Span::styled("  ▏", Style::default().fg(color)))),
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::render::tools::tool_card::diff_lines;
    use crate::TuiInit;

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

    #[test]
    fn stable_long_history_is_built_once_across_animation_frames() {
        use comb::{render, Size};

        let mut a = app();
        for i in 0..400 {
            a.notice(format!("completed history row {i}"));
        }
        let _ = render(Size::new(100, 30), |frame| {
            crate::render::draw(frame, &mut a)
        });
        let builds = a.transcript_cache.builds;
        assert_eq!(builds, 1);

        for _ in 0..20 {
            a.spinner += 1;
            let _ = render(Size::new(100, 30), |frame| {
                crate::render::draw(frame, &mut a)
            });
        }
        assert_eq!(
            a.transcript_cache.builds, builds,
            "footer animation must not rebuild stable transcript history"
        );
    }

    #[test]
    fn live_thought_shimmer_does_not_rebuild_the_transcript() {
        use comb::{render, Size};
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta("Considering the lexer.".into()));
        let _ = render(Size::new(80, 24), |frame| {
            crate::render::draw(frame, &mut a)
        });
        let builds = a.transcript_cache.builds;
        assert!(builds >= 1);

        for _ in 0..8 {
            a.spinner += 1;
            let _ = render(Size::new(80, 24), |frame| {
                crate::render::draw(frame, &mut a)
            });
        }
        assert_eq!(
            a.transcript_cache.builds, builds,
            "Thinking shimmer must overlay the cached header, not rebuild history"
        );
        let text = render(Size::new(80, 24), |frame| {
            crate::render::draw(frame, &mut a)
        })
        .text();
        assert!(text.contains("Thinking"), "{text}");
        assert!(
            text.contains("Considering the lexer."),
            "shimmer must not overwrite the live sentences: {text}"
        );
        let thinking_hits = text.matches("Thinking").count();
        assert_eq!(
            thinking_hits, 1,
            "Thinking belongs on the header only: {text}"
        );
    }

    #[test]
    fn running_subagent_spinner_overlays_without_rebuild() {
        use comb::{render, Size};
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking project".into(),
            prompt: "go".into(),
        });
        a.spinner = 0;
        let _ = render(Size::new(80, 24), |frame| {
            crate::render::draw(frame, &mut a)
        });
        let builds = a.transcript_cache.builds;
        assert!(builds >= 1);

        a.spinner = 3;
        let buf = render(Size::new(80, 24), |frame| {
            crate::render::draw(frame, &mut a)
        });
        assert_eq!(
            a.transcript_cache.builds, builds,
            "card spinner must overlay the cached title, not rebuild history"
        );

        let ch = crate::render::spinner::glyph(3);
        let text = buf.text();
        assert!(text.contains(ch), "live spinner frame missing: {text}");
        assert!(text.contains("Checking project"), "{text}");

        let mut found = None;
        for y in 0..buf.height {
            for x in 0..buf.width {
                if buf.get(x, y).is_some_and(|cell| cell.ch == ch) {
                    found = Some((x, y));
                }
            }
        }
        let (x, y) = found.expect("spinner cell");
        assert_eq!(
            buf.get(x, y).unwrap().style.bg,
            Some(a.theme.strip),
            "overlay must keep the card strip background"
        );
        assert_eq!(
            buf.get(x, y).unwrap().style.fg,
            Some(a.theme.accent),
            "overlay keeps the running accent"
        );
    }

    #[test]
    fn live_thought_shimmer_stays_on_the_header() {
        use comb::{render, Size};
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta(
            "First I read the lexer. Then I check the parser. Finally I write the fix.".into(),
        ));
        let last = a.blocks.len() - 1;
        if let crate::app::state::Block::Reasoning(th) = &mut a.blocks[last] {
            th.started = std::time::Instant::now() - std::time::Duration::from_secs(20);
        }

        let text = render(Size::new(80, 24), |frame| {
            crate::render::draw(frame, &mut a)
        })
        .text();
        assert!(text.contains("Thinking hard"), "{text}");
        assert!(text.contains("First I read the lexer."), "{text}");
        assert!(text.contains("Then I check the parser."), "{text}");
        assert!(text.contains("Finally I write the fix."), "{text}");
        assert_eq!(
            text.matches("Thinking hard").count(),
            1,
            "label must not stamp every sentence: {text}"
        );
    }

    #[test]
    fn active_goal_deadline_refreshes_cached_transcript_once_per_second() {
        use comb::{render, Size};
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::GoalSet {
            objective: "keep working".into(),
            deadline: Some(std::time::Instant::now() + std::time::Duration::from_secs(60)),
        });
        a.running = false;
        let _ = render(Size::new(100, 30), |frame| {
            crate::render::draw(frame, &mut a)
        });
        let builds = a.transcript_cache.builds;
        a.spinner += 10;
        let _ = render(Size::new(100, 30), |frame| {
            crate::render::draw(frame, &mut a)
        });
        assert_eq!(a.transcript_cache.builds, builds + 1);
    }

    #[test]
    fn the_live_line_says_what_the_agent_is_doing() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta("planning the edit".into()));
        assert_eq!(a.activity_label(), "Thinking");

        a.apply(AgentEvent::ToolStarted {
            id: "t1".into(),
            name: "run_shell".into(),
            args_preview: "cargo build".into(),
        });
        assert_eq!(
            a.activity_label(),
            "Working",
            "a running tool isn't thinking"
        );

        // A terminal keeps running in the background while the model reasons,
        // so that's the case with a live header to label.
        a.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "sudo dnf upgrade".into(),
            description: String::new(),
            rows: 20,
            cols: 80,
        });
        a.apply(AgentEvent::ReasoningDelta("waiting on the install".into()));
        assert_eq!(a.activity_label(), "Working in terminal");

        let t = tool_text(&mut a, 80);
        assert!(t.contains("Working in terminal"), "{t}");
        assert!(!t.contains("Thinking"), "not while a terminal is up: {t}");
    }

    #[test]
    fn an_empty_thought_between_tools_leaves_no_gap() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.blocks.clear();
        a.apply(AgentEvent::TurnStarted);
        // Reasoning that never produced text, then a tool: the placeholder
        // thought used to close as "Thought for 0.0s" and stack up.
        a.apply(AgentEvent::ReasoningDelta("   ".into()));
        a.apply(AgentEvent::ToolStarted {
            id: "t1".into(),
            name: "run_shell".into(),
            args_preview: "ls".into(),
        });

        assert!(
            !a.blocks
                .iter()
                .any(|b| matches!(b, crate::app::state::Block::Reasoning(_))),
            "an empty thought is dropped, not stamped"
        );
        let t = tool_text(&mut a, 80);
        assert!(!t.contains("Thought for"), "{t}");
    }

    #[test]
    fn active_chat_has_exactly_one_top_inset() {
        use comb::{render, Size};

        let mut a = app();
        // Keep the initial invisible Welcome block: only the intentional
        // viewport inset may reserve a row above the active chat.
        a.push_user("first message".into());

        let buf = render(Size::new(70, 14), |f| crate::render::draw(f, &mut a));
        let row = buf
            .text()
            .lines()
            .position(|l| l.contains("first message"))
            .expect("user message");
        // Row 0 is the one layout inset; row 1 already belongs to the user
        // strip. An invisible Welcome must not add another row between them.
        assert_eq!(row, 2, "exactly one row above the message band");
        // The chat column is centred, so sample inside it, not in the margin.
        let x = buf
            .text()
            .lines()
            .nth(2)
            .and_then(|l| l.find("first message"))
            .expect("column") as u16;
        assert_eq!(
            buf.get(x, 0).and_then(|c| c.style.bg),
            None,
            "the top row is the single blank layout inset"
        );
        assert_eq!(
            buf.get(x, 1).and_then(|c| c.style.bg),
            Some(a.theme.user_strip),
            "the next row already belongs to the message band"
        );
    }

    #[test]
    fn thoughts_collapse_and_expand() {
        use hive_core::event::AgentEvent;
        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta(
            "First I read the lexer. Now let me ponder this.".into(),
        ));

        // Active: shimmering header plus one row echoing the newest sentence —
        // enough to see what it's on, never the whole body.
        let text = |lines: &[comb::Line]| {
            lines
                .iter()
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let t = text(&super::lines(&mut a, 80));
        assert!(t.contains("Thinking"), "{t}");
        assert!(t.contains("let me ponder this"), "live window: {t}");

        // Model moves on → thought closes with a duration; still collapsed.
        // Latest thought omits the click hint (older ones keep it).
        a.apply(AgentEvent::AssistantTextDelta("answer".into()));
        let t = text(&super::lines(&mut a, 80));
        assert!(t.contains("Thought for"));
        assert!(!t.contains('∴'), "thought rows should be plain text labels");
        assert!(t.contains("tokens"));
        assert!(!t.contains("ponder"));
        assert!(!t.contains("click to show"), "{t}");
        assert!(!t.contains("click to hide"), "{t}");

        // ctrl+t reveals the full text.
        a.toggle_thoughts();
        let t = text(&super::lines(&mut a, 80));
        assert!(t.contains("ponder"));
        assert!(!t.contains("click to hide"), "{t}");
    }

    #[test]
    fn the_thought_window_slides_instead_of_growing() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.blocks.clear();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta(
            "Reading the lexer. Quotes break it. Switch to a scanner.".into(),
        ));
        let full = super::lines(&mut a, 80).len();
        assert_eq!(full, 1 + super::THOUGHT_WINDOW, "header + a full window");

        // Two more sentences land: the window slides, the block keeps its height.
        a.apply(AgentEvent::ReasoningDelta(
            " Keep a quote state. Then wire it through.".into(),
        ));
        let t = tool_text(&mut a, 80);
        assert_eq!(super::lines(&mut a, 80).len(), full, "same height: {t}");
        assert!(t.contains("Then wire it through"), "newest: {t}");
        assert!(t.contains("Switch to a scanner"), "still in frame: {t}");
        assert!(!t.contains("Reading the lexer"), "slid off the top: {t}");
    }

    #[test]
    fn the_window_fades_from_oldest_to_newest() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.blocks.clear();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta("One. Two. Three.".into()));

        let lines = super::lines(&mut a, 80);
        let level = |l: &comb::Line| match l.spans.last().and_then(|s| s.style.fg) {
            Some(comb::Color::Rgb(v, _, _)) => v,
            other => panic!("thought row needs a colour, got {other:?}"),
        };
        let rows: Vec<u8> = lines[1..=super::THOUGHT_WINDOW].iter().map(level).collect();
        assert!(
            rows.windows(2).all(|w| w[0] < w[1]),
            "each row brighter than the one above it: {rows:?}"
        );
        let theme = crate::theme::Theme::gray();
        assert_eq!(super::fade(&theme, 0, 3), theme.faint, "oldest at faint");
        assert_eq!(super::fade(&theme, 2, 3), theme.dim, "newest at dim");
        assert_eq!(super::fade(&theme, 0, 1), theme.dim, "a lone row is newest");
    }

    #[test]
    fn a_long_sentence_is_cut_where_it_matters() {
        // The newest sentence is still being written, so it follows the head.
        let live = super::fit("rewrite the whole scanner from scratch today", 20, true);
        assert_eq!(live.chars().count(), 20, "fits the row exactly");
        assert!(live.starts_with('…'), "the cut is marked: {live}");
        assert!(live.ends_with("scratch today"), "newest words: {live}");

        // A settled one has stopped moving — it reads from the start.
        let settled = super::fit("rewrite the whole scanner from scratch today", 20, false);
        assert_eq!(settled.chars().count(), 20);
        assert!(settled.starts_with("rewrite the whole"), "{settled}");
        assert!(settled.ends_with('…'), "{settled}");

        // Short enough to fit → untouched, no ellipsis.
        assert_eq!(super::fit("all good", 20, true), "all good");
    }

    #[test]
    fn quoted_thought_rows_line_up_under_the_header() {
        use hive_core::event::AgentEvent;

        assert_eq!(
            super::QUOTE.chars().count(),
            super::QUOTE_W,
            "wrap math is measured off QUOTE_W — it must match the real prefix"
        );

        let mut a = app();
        a.blocks.clear();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta("One thought.".into()));

        let lines = super::lines(&mut a, 80);
        // Columns, not bytes — the prefix may hold multi-byte glyphs.
        let col = |l: &comb::Line, needle: &str| {
            let text: String = l.spans.iter().map(|s| s.content.as_str()).collect();
            let at = text.find(needle).expect("row text");
            text[..at].chars().count()
        };
        assert_eq!(
            col(&lines[1], "One thought."),
            col(&lines[0], "Thinking"),
            "no rule, no hanging indent — quoted reasoning is flush with its header"
        );
    }

    #[test]
    fn sentences_split_on_prose_not_on_decimals() {
        let s = super::split_sentences("Took 3.5s to run. Now retry!\nNext line");
        assert_eq!(s, vec!["Took 3.5s to run.", "Now retry!", "Next line"]);
        assert!(super::split_sentences("   \n  ").is_empty());
        assert!(super::recent_sentences("anything", 3, 0).is_empty());
        assert!(super::recent_sentences("anything", 0, 40).is_empty());
    }

    #[test]
    fn long_thoughts_read_in_minutes() {
        assert_eq!(super::think_secs(4.24, false), "4.2s");
        assert_eq!(
            super::think_secs(4.24, true),
            "4s",
            "no live decimal jitter"
        );
        assert_eq!(super::think_secs(47.0, false), "47s");
        assert_eq!(super::think_secs(64.0, true), "1m 04s");
        assert_eq!(super::think_secs(600.0, false), "10m 00s");
    }

    #[test]
    fn the_page_margin_beside_a_card_is_not_the_card() {
        use comb::{render, Size};

        let mut a = app();
        a.blocks.clear();
        a.push_user("my message".into());
        let _ = render(Size::new(100, 20), |f| crate::render::draw(f, &mut a));

        let band = a.transcript_hit.expect("chat column");
        let (row, block) = *a.click_hits.first().expect("message row");
        assert_eq!(a.expandable_at(band.x, row), Some(block), "on the card");
        assert_eq!(
            a.expandable_at(band.right() - 1, row),
            Some(block),
            "the far edge is still the card"
        );

        // Left/right page padding and the sidebar share the row but are not it.
        assert!(band.x > 0, "the chat column is inset");
        assert_eq!(a.expandable_at(band.x - 1, row), None, "left margin");
        assert_eq!(a.expandable_at(band.right(), row), None, "right margin");
        assert_eq!(a.expandable_at(99, row), None, "screen edge");
    }

    #[test]
    fn older_thought_keeps_click_hint() {
        use hive_core::event::AgentEvent;
        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta("first thought".into()));
        a.apply(AgentEvent::AssistantTextDelta("mid".into()));
        a.apply(AgentEvent::ReasoningDelta("second thought".into()));
        a.apply(AgentEvent::AssistantTextDelta("done".into()));
        a.apply(AgentEvent::TurnFinished);

        let t = super::lines(&mut a, 80)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Exactly one hint — on the older thought, not the latest.
        assert_eq!(
            t.matches("click to show").count(),
            1,
            "older thought keeps hint: {t}"
        );
    }

    #[test]
    fn click_toggles_one_thought() {
        use comb::{render, Size};
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta("secret plan".into()));
        a.apply(AgentEvent::AssistantTextDelta("done".into()));
        a.apply(AgentEvent::TurnFinished);

        let before = render(Size::new(90, 24), |f| crate::render::draw(f, &mut a));

        // Collapsed: the header row is registered for hit-testing, text hidden.
        let (row, _) = *a.click_hits.first().expect("header row recorded");
        assert!(!before.text().contains("secret plan"));

        // A click on that row opens exactly that thought.
        let col = a.transcript_hit.expect("chat column").x;
        let idx = a.expandable_at(col, row).expect("click hits the header");
        a.activate_expandable_at(idx);
        let after = render(Size::new(90, 24), |f| crate::render::draw(f, &mut a));
        assert!(after.text().contains("secret plan"));
    }

    #[test]
    fn subagent_renders_inline_without_border() {
        use hive_core::event::{AgentEvent, SubagentLine, SubagentStatus};

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking project".into(),
            prompt: "Run cargo check and report.".into(),
        });
        a.spinner = 0;

        let t = tool_text(&mut a, 72);
        assert!(t.contains("Checking project"), "{t}");
        assert!(t.contains("working"), "{t}");
        assert!(
            !t.contains('╭') && !t.contains('╯'),
            "no border chrome: {t}"
        );

        a.apply(AgentEvent::SubagentStatus {
            id: "v1".into(),
            status: SubagentStatus::Done,
            detail: "done".into(),
        });
        a.apply(AgentEvent::SubagentTranscript {
            id: "v1".into(),
            line: SubagentLine::Assistant("## Verification Report\nAll good.".into()),
        });

        let collapsed = tool_text(&mut a, 72);
        assert!(collapsed.contains('✓'), "{collapsed}");
        assert!(collapsed.contains("done"), "{collapsed}");
        assert!(
            !collapsed.contains("Verification Report"),
            "report stays in dedicated view: {collapsed}"
        );
        assert!(collapsed.contains("click to open"), "{collapsed}");

        // Click switches into the read-only subagent chat view.
        let idx = a
            .blocks
            .iter()
            .position(|b| matches!(b, crate::app::state::Block::Subagent(_)))
            .expect("subagent block");
        a.activate_expandable_at(idx);
        assert!(a.in_subagent_view());
        let open = tool_text(&mut a, 72);
        assert!(open.contains("Run cargo check"), "{open}");
        assert!(open.contains("Verification Report"), "{open}");

        a.leave_subagent_view();
        assert!(!a.in_subagent_view());
        let back = tool_text(&mut a, 72);
        assert!(
            !back.contains("Verification Report"),
            "main transcript again: {back}"
        );
    }

    #[test]
    fn tools_have_no_strip_background() {
        use hive_core::event::{AgentEvent, SubagentLine, SubagentStatus};

        // Main-agent tools: no strip bg.
        let mut a = app();
        a.apply(AgentEvent::ToolStarted {
            id: "r1".into(),
            name: "read_file".into(),
            args_preview: "src/main.rs".into(),
        });
        a.apply(AgentEvent::ToolFinished {
            id: "r1".into(),
            name: "read_file".into(),
            ok: true,
            summary: "ok".into(),
        });
        let lines = super::lines(&mut a, 72);
        let header = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("Reading")))
            .expect("Reading header");
        assert!(
            header.spans.iter().all(|s| s.style.bg.is_none()),
            "regular tools must not have strip bg"
        );

        // Tools inside a subagent thread: also no strip bg.
        let mut a = app();
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking project".into(),
            prompt: "check".into(),
        });
        a.apply(AgentEvent::SubagentTranscript {
            id: "v1".into(),
            line: SubagentLine::Tool {
                name: "run_shell".into(),
                args: "cargo check".into(),
                summary: "ok".into(),
                ok: Some(true),
            },
        });
        a.apply(AgentEvent::SubagentStatus {
            id: "v1".into(),
            status: SubagentStatus::Done,
            detail: "done".into(),
        });
        a.open_subagent_view("v1".into());

        let lines = super::lines(&mut a, 72);
        let tool = lines
            .iter()
            .find(|l| {
                l.spans
                    .iter()
                    .any(|s| s.content.contains("cargo check") || s.content.contains('$'))
            })
            .expect("tool line");
        assert!(
            tool.spans.iter().all(|s| s.style.bg.is_none()),
            "tools in subagent view must not have strip bg"
        );
    }

    #[test]
    fn subagent_card_has_soft_background_and_pads() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking project".into(),
            prompt: "check".into(),
        });
        let lines = super::lines(&mut a, 72);
        let title_i = lines
            .iter()
            .position(|l| {
                l.spans
                    .iter()
                    .any(|s| s.content.contains("Checking project"))
            })
            .expect("subagent title");
        assert!(
            lines[title_i]
                .spans
                .iter()
                .any(|s| s.style.bg == Some(a.theme.strip)),
            "subagent card keeps soft strip bg"
        );
        assert!(
            title_i > 0
                && lines[title_i - 1]
                    .spans
                    .iter()
                    .any(|s| s.style.bg == Some(a.theme.strip)),
            "top pad row shares strip bg"
        );
        assert!(
            lines
                .get(title_i + 2)
                .is_some_and(|l| l.spans.iter().any(|s| s.style.bg == Some(a.theme.strip))),
            "bottom pad row shares strip bg"
        );
    }

    #[test]
    fn verify_project_tool_card_suppressed() {
        use crate::app::state::Block;
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::ToolStarted {
            id: "t1".into(),
            name: "spawn_subagent".into(),
            args_preview: "project check".into(),
        });
        assert!(
            !a.blocks
                .iter()
                .any(|b| matches!(b, Block::Tool(c) if c.name == "spawn_subagent")),
            "spawn_subagent must not appear as a transcript tool card"
        );
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking project".into(),
            prompt: "check".into(),
        });
        assert!(
            a.blocks.iter().any(|b| matches!(b, Block::Subagent(_))),
            "subagent should appear inline in the transcript"
        );
    }

    #[test]
    fn indent_skips_bg_for_wrapped_inline_code() {
        use crate::render::markdown;

        let theme = crate::theme::Theme::gray();
        let md = "Want me to auto-fix the trivial ones (`useless_format`, `manual_contains`, `unnecessary_unwrap`)?";
        let body = markdown::render(md, &theme, 58);
        let wrapped = crate::render::wrap::wrap_lines(body, 58);
        let indented = super::indent(wrapped, 60);
        // Continuation rows that start with a chip must keep a plain gutter —
        // inheriting code_bg from the first span was the crooked gray pad.
        for line in &indented {
            let text: String = line.spans.iter().map(|s| s.content.as_str()).collect();
            if text.contains("manual_contains") || text.contains("unnecessary_unwrap") {
                assert!(
                    line.spans.first().is_some_and(|s| s.style.bg.is_none()),
                    "gutter must stay clear on chip continuation: {text:?}"
                );
            }
        }
    }

    #[test]
    fn assistant_code_fence_is_a_box_and_copies_without_chrome() {
        use crate::app::state::Block;
        use comb::{render, Size};

        let mut a = app();
        a.blocks.clear();
        a.blocks.push(Block::Assistant {
            text: "```javascript\nfunction nav(user, amount) {}\n```".into(),
            streaming: false,
        });

        let buf = render(Size::new(80, 16), |f| crate::render::draw(f, &mut a));
        let hit = a
            .assistant_row_hits
            .iter()
            .find(|hit| hit.text.contains("function nav"))
            .cloned()
            .expect("code row");

        assert_eq!(hit.text, "function nav(user, amount) {}");
        let rail = buf.get(hit.x - 2, hit.screen_row).expect("box rail");
        assert_eq!(rail.ch, '│');
        assert_ne!(rail.style.bg, Some(a.theme.code_bg));
        let keyword = comb::HighlightTheme::dark().keyword.fg;
        assert_eq!(
            buf.get(hit.x, hit.screen_row)
                .and_then(|cell| cell.style.fg),
            keyword
        );
        assert_ne!(keyword, Some(a.theme.accent));
        assert!(
            (hit.x..hit.x + hit.text.len() as u16).all(|x| {
                buf.get(x, hit.screen_row)
                    .is_some_and(|cell| cell.style.bg != Some(a.theme.code_bg))
            }),
            "inside the box stays on the terminal surface"
        );
        assert!(
            a.assistant_row_hits
                .iter()
                .any(|hit| hit.text.contains("JavaScript")),
            "language caption on the frame"
        );

        assert!(a.start_assistant_selection(hit.x, hit.screen_row));
        assert!(a.update_assistant_selection(hit.x + hit.text.len() as u16 - 1, hit.screen_row,));
        assert_eq!(
            a.finish_assistant_selection(hit.x + hit.text.len() as u16 - 1, hit.screen_row,)
                .as_deref(),
            Some("function nav(user, amount) {}")
        );
    }

    #[test]
    fn diff_rows_are_coloured_and_blocked() {
        let a = app();
        let lines = diff_lines("- gone\n+ added new\n… 3 more\n", &a, 60);
        assert_eq!(lines.len(), 3);
        // Every row is [indent, styled block].
        for l in &lines {
            assert_eq!(l.spans.len(), 2);
        }
        assert_eq!(lines[0].spans[1].style.bg, Some(a.theme.del_bg));
        assert_eq!(lines[1].spans[1].style.bg, Some(a.theme.add_bg));
        // All blocks share one width (a neat rectangle), sized to content.
        let w = lines[0].spans[1].content.chars().count();
        assert!(lines
            .iter()
            .all(|l| l.spans[1].content.chars().count() == w));
        assert!(w < 60); // not the full terminal width
    }

    fn tool_text(a: &mut App, width: usize) -> String {
        super::lines(a, width)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn tool_duration_stays_on_shell_header() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ToolStarted {
            id: "t1".into(),
            name: "run_shell".into(),
            args_preview: "cargo check --workspace 2>&1".into(),
        });
        a.apply(AgentEvent::ToolOutput {
            id: "t1".into(),
            chunk: "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.91s\n"
                .into(),
        });
        a.apply(AgentEvent::ToolFinished {
            id: "t1".into(),
            name: "run_shell".into(),
            ok: true,
            summary: "exit 0".into(),
        });

        let t = tool_text(&mut a, 72);
        // Duration belongs on the `$ …` header, not as a column-0 orphan.
        assert!(t.contains("$ cargo check"), "shell header: {t}");
        assert!(t.contains("·"), "duration separator: {t}");
        let header = t
            .lines()
            .find(|l| l.contains("$ cargo check"))
            .unwrap_or("");
        assert!(header.contains('s'), "duration on shell header line: {t}");
        // Successful tools collapse the body — cargo's own "in 1.91s" stays out.
        assert!(
            !t.contains("Finished"),
            "ok shell should not dump output body: {t}"
        );
    }

    #[test]
    fn hiding_completed_tools_keeps_progress_and_errors_visible() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.ui.show_tool_cards = false;
        a.apply(AgentEvent::ToolStarted {
            id: "tool".into(),
            name: "run_shell".into(),
            args_preview: "cargo test".into(),
        });
        assert!(tool_text(&mut a, 72).contains("cargo test"));

        a.apply(AgentEvent::ToolFinished {
            id: "tool".into(),
            name: "run_shell".into(),
            ok: true,
            summary: "done".into(),
        });
        assert!(!tool_text(&mut a, 72).contains("cargo test"));

        a.apply(AgentEvent::ToolStarted {
            id: "failed".into(),
            name: "run_shell".into(),
            args_preview: "cargo broken".into(),
        });
        a.apply(AgentEvent::ToolFinished {
            id: "failed".into(),
            name: "run_shell".into(),
            ok: false,
            summary: "failed".into(),
        });
        assert!(tool_text(&mut a, 72).contains("cargo broken"));
    }

    #[test]
    fn file_tools_use_reading_verb() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ToolStarted {
            id: "r1".into(),
            name: "read_file".into(),
            args_preview: "crates/hive-tui/src/render/transcript.rs".into(),
        });
        a.apply(AgentEvent::ToolFinished {
            id: "r1".into(),
            name: "read_file".into(),
            ok: true,
            summary: "ok".into(),
        });

        let t = tool_text(&mut a, 80);
        assert!(t.contains("Reading crates/hive-tui"), "verb header: {t}");
        assert!(!t.contains("read_file"), "no raw tool name: {t}");
        assert!(!t.contains("✓"), "no status icon on tool rows: {t}");
    }

    #[test]
    fn finished_assistant_registers_selectable_rows_without_changing_text() {
        use crate::app::state::Block;
        use comb::{render, Size};

        let mut a = app();
        a.blocks.clear();
        a.blocks.push(Block::Assistant {
            text: "selectable answer".into(),
            streaming: false,
        });

        let first = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert!(first.text().contains("selectable answer"));
        assert!(!a.assistant_row_hits.is_empty());
        assert!(a
            .assistant_row_hits
            .iter()
            .all(|hit| hit.text.contains("selectable answer")));

        let second = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert_eq!(first.text(), second.text());
    }

    #[test]
    fn assistant_selection_uses_graphite_bg_and_preserves_markdown_fg() {
        use crate::app::state::Block;
        use comb::{render, Color, Size};

        let mut a = app();
        a.blocks.clear();
        a.blocks.push(Block::Assistant {
            text: "`hello` world".into(),
            streaming: false,
        });

        let before = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        let hit = a
            .assistant_row_hits
            .first()
            .cloned()
            .expect("assistant row hit");
        assert!(a.start_assistant_selection(hit.x, hit.screen_row));
        assert!(a.update_assistant_selection(hit.x + 4, hit.screen_row));
        let graphite = Color::Rgb(0x34, 0x34, 0x34);
        assert_eq!(a.theme.assistant_selection_bg, graphite);

        let after = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert_eq!(
            before.text(),
            after.text(),
            "selection must not change glyphs"
        );
        for x in hit.x..=hit.x + 4 {
            let before_cell = before.get(x, hit.screen_row).expect("original cell");
            let selected_cell = after.get(x, hit.screen_row).expect("selected cell");
            assert_eq!(
                selected_cell.style.bg,
                Some(graphite),
                "selected bg at x={x}"
            );
            assert_eq!(
                selected_cell.style.fg, before_cell.style.fg,
                "selection must preserve markdown foreground at x={x}"
            );
            assert_eq!(
                selected_cell.style.mods, before_cell.style.mods,
                "selection must preserve markdown modifiers at x={x}"
            );
        }
        assert_ne!(
            after
                .get(hit.x + 6, hit.screen_row)
                .expect("unselected cell")
                .style
                .bg,
            Some(graphite),
            "selection must not tint neighboring cells"
        );
    }

    #[test]
    fn streaming_assistant_is_not_selectable() {
        use crate::app::state::Block;
        use comb::{render, Size};

        let mut a = app();
        a.blocks.clear();
        a.blocks.push(Block::Assistant {
            text: "still streaming".into(),
            streaming: true,
        });

        let _ = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert!(a.assistant_row_hits.is_empty());
    }

    #[test]
    fn scrolling_extends_active_selection_across_offscreen_rows() {
        use crate::app::state::Block;
        use comb::{render, Size};

        let mut a = app();
        a.blocks.clear();
        a.blocks.push(Block::Assistant {
            text: (0..20)
                .map(|i| format!("line {i:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
            streaming: false,
        });

        let _ = render(Size::new(50, 12), |f| crate::render::draw(f, &mut a));
        let first = a.assistant_row_hits.first().cloned().expect("first row");
        let last = a.assistant_row_hits.last().cloned().expect("last row");
        assert!(a.start_assistant_selection(last.x + last.text.len() as u16 - 1, last.screen_row));
        assert!(a.update_assistant_selection(first.x, first.screen_row));
        assert!(a.assistant_selection_active());

        // One bottom viewport row is the transcript's trailing spacer, so move
        // by two to push the selected last response row out of view.
        a.scroll_up(2);
        let _ = render(Size::new(50, 12), |f| crate::render::draw(f, &mut a));
        assert!(a.assistant_selection_active());

        let new_first = a
            .assistant_row_hits
            .first()
            .cloned()
            .expect("new first row");
        assert!(new_first.response_row < first.response_row);
        assert!(a.update_assistant_selection(new_first.x, new_first.screen_row));

        let selected = a
            .finish_assistant_selection(new_first.x, new_first.screen_row)
            .expect("selection across scrolled rows");
        let expected = (new_first.response_row..=last.response_row)
            .map(|i| format!("line {i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(selected, expected);
    }

    #[test]
    fn scrolling_past_response_clamps_selection_to_its_start() {
        use crate::app::state::Block;
        use comb::{render, Size};

        let mut a = app();
        a.blocks.clear();
        a.blocks.push(Block::Assistant {
            text: (0..30)
                .map(|i| format!("old {i:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
            streaming: false,
        });
        a.blocks.push(Block::Assistant {
            text: (0..20)
                .map(|i| format!("new {i:02}"))
                .collect::<Vec<_>>()
                .join("\n"),
            streaming: false,
        });

        let _ = render(Size::new(50, 12), |f| crate::render::draw(f, &mut a));
        let selected_block = 1;
        let visible: Vec<_> = a
            .assistant_row_hits
            .iter()
            .filter(|hit| hit.block == selected_block)
            .cloned()
            .collect();
        let first = visible.first().expect("first selected row");
        let last = visible.last().expect("last selected row");
        assert!(a.start_assistant_selection(last.x + last.text.len() as u16 - 1, last.screen_row));
        assert!(a.update_assistant_selection(first.x, first.screen_row));

        a.scroll_up(usize::MAX);
        let _ = render(Size::new(50, 12), |f| crate::render::draw(f, &mut a));
        assert!(a
            .assistant_row_hits
            .iter()
            .all(|hit| hit.block != selected_block));
        let release = a.assistant_row_hits.first().cloned().expect("release row");
        let selected = a
            .finish_assistant_selection(release.x, release.screen_row)
            .expect("selection clamped to response start");

        let expected = (0..20)
            .map(|i| format!("new {i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(selected, expected);
    }

    #[test]
    fn plan_card_opens_preview() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "Ship it".into(),
            body: "# Plan\n\n## Ship it\n\nDo the thing.\n".into(),
        });
        let t = tool_text(&mut a, 72);
        assert!(t.contains("Plan.md"), "{t}");
        assert!(t.contains("Ship it"), "{t}");

        let idx = a
            .blocks
            .iter()
            .position(|b| matches!(b, crate::app::state::Block::Plan(_)))
            .expect("plan block");
        a.activate_expandable_at(idx);
        assert!(a.in_plan_view());
        let preview = tool_text(&mut a, 72);
        assert!(preview.contains("drag"), "{preview}");
        assert!(preview.contains("MARK"), "{preview}");
        assert!(preview.contains("Ship it"), "{preview}");
    }

    /// A finished tool card with a file snapshot, ready to hit-test.
    fn app_with_tool() -> App {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.blocks.clear();
        a.apply(AgentEvent::ToolStarted {
            id: "call_1".into(),
            name: "write_file".into(),
            args_preview: "src/main.rs".into(),
        });
        a.apply(AgentEvent::FileSnapshot {
            id: "call_1".into(),
            path: "src/main.rs".into(),
            content: "before".into(),
        });
        a.apply(AgentEvent::ToolFinished {
            id: "call_1".into(),
            name: "write_file".into(),
            ok: true,
            summary: "+ after".into(),
        });
        a
    }

    fn tool_block_idx(a: &App) -> usize {
        a.blocks
            .iter()
            .position(|b| matches!(b, crate::app::state::Block::Tool(_)))
            .expect("tool block")
    }

    #[test]
    fn tool_rows_are_clickable_and_reach_the_menu() {
        use comb::{render, Size};

        let mut a = app_with_tool();
        let _ = render(Size::new(90, 24), |f| crate::render::draw(f, &mut a));

        let idx = tool_block_idx(&a);
        let rows: Vec<u16> = a
            .click_hits
            .iter()
            .filter(|(_, b)| *b == idx)
            .map(|(row, _)| *row)
            .collect();
        assert!(!rows.is_empty(), "tool card rows must be hit-testable");

        // Clicking one opens the tool menu — Copy / Revert live nowhere else.
        let col = a.transcript_hit.expect("chat column").x;
        let hit = a.expandable_at(col, rows[0]).expect("row maps to the card");
        assert_eq!(hit, idx);
        a.activate_expandable_at(hit);
        assert!(a.context_menu_open(), "tool context menu");
    }

    #[test]
    fn hovering_a_tool_card_tints_its_rows() {
        use comb::{render, Size};

        let mut a = app_with_tool();
        let idx = tool_block_idx(&a);

        let band = a.theme.strip_hover;
        let banded = |buf: &comb::Buffer, row: u16| {
            (0..90).any(|x| buf.get(x, row).and_then(|c| c.style.bg) == Some(band))
        };

        let plain = render(Size::new(90, 24), |f| crate::render::draw(f, &mut a));
        let row = a
            .click_hits
            .iter()
            .find(|(_, b)| *b == idx)
            .map(|(row, _)| *row)
            .expect("tool row");
        assert!(!banded(&plain, row), "idle card stays unpainted");

        assert!(a.set_hover_block(Some(idx)));
        let hovered = render(Size::new(90, 24), |f| crate::render::draw(f, &mut a));
        assert!(banded(&hovered, row), "hover must be visible on the card");
    }
}
