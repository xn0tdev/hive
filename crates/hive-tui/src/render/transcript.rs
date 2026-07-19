//! Builds and draws the scrolling conversation transcript. The greeting lives
//! only on the landing screen; the turn working cubes sit by the footer mode chip.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use hive_core::event::{SubagentLine, SubagentStatus};

use crate::app::state::{Block as UiBlock, ChatView, SubagentCard, ToolCard, ToolStatus};
use crate::app::App;
use crate::render::tools::{format_tool_secs, plan_card_lines, subagent_card_lines, tool_lines};
use crate::render::{markdown, wrap};

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.is_empty() {
        app.click_hits.clear();
        app.set_transcript_max_scroll(0);
        return;
    }
    // Clear the whole transcript band (including the top spacer row) so
    // scroll / shorter lines never leave stale glyphs in vacated cells.
    buf.paint(area, Style::default());

    let width = area.width.max(1) as usize;
    let (all, heads) = build(app, width);
    let total = all.len();
    // Top-down with one blank row of breathing room under the top edge.
    let target = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    let viewport = target.height as usize;
    let max_scroll = total.saturating_sub(viewport);
    app.set_transcript_max_scroll(max_scroll);
    let scroll = max_scroll.saturating_sub(app.scroll_from_bottom);

    // Remember which screen rows hold expandable headers (thoughts / subagents)
    // so a mouse click can be mapped back to its block.
    app.click_hits.clear();
    for (line_idx, block_idx) in heads {
        if line_idx >= scroll && line_idx < scroll + target.height as usize {
            app.click_hits
                .push((target.y + (line_idx - scroll) as u16, block_idx));
        }
    }

    buf.set_lines(target, &all, scroll);
}

/// Brighter shimmer for the live Thinking header — readable at a glance.
fn shimmer_bright(text: &str, tick: usize) -> Vec<Span> {
    shimmer_range(text, tick, 0xb8, 0xff)
}

fn shimmer_range(text: &str, tick: usize, lo: i32, hi: i32) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as i32;
    let head = (tick as i32 % (n + 6)) - 3;
    chars
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let dist = (i as i32 - head).abs() as f32;
            let t = (1.0 - dist / 3.0).max(0.0);
            let v = (lo as f32 + (hi - lo) as f32 * t) as u8;
            Span::styled(
                c.to_string(),
                Style::default().fg(Color::Rgb(v, v, v)).add(Modifier::BOLD),
            )
        })
        .collect()
}

/// Build the full, pre-wrapped set of transcript lines (test helper).
#[cfg(test)]
pub fn lines(app: &mut App, width: usize) -> Vec<Line> {
    build(app, width).0
}

/// Like `lines`, but also reports which line index holds each expandable
/// header (thought / subagent) with its block index for mouse hit-testing.
fn build(app: &mut App, width: usize) -> (Vec<Line>, Vec<(usize, usize)>) {
    if matches!(app.view, ChatView::Subagent(_)) {
        // Clone the card snapshot so we can still use `app` mutably for caches.
        if let Some(card) = app.viewed_subagent().cloned() {
            return (subagent_chat_lines(&card, app, width), Vec::new());
        }
        // Stale id (cleared chat) — fall through to main.
    }
    if matches!(app.view, ChatView::Plan) {
        return (plan_preview_lines(app, width), Vec::new());
    }

    let mut out: Vec<Line> = Vec::new();
    let mut heads: Vec<(usize, usize)> = Vec::new();
    let n = app.blocks.len();

    for i in 0..n {
        let next_is_tool = matches!(app.blocks.get(i + 1), Some(UiBlock::Tool(_)));
        let next_is_notice = matches!(app.blocks.get(i + 1), Some(UiBlock::Notice(_)));
        let has_later_thought = app
            .blocks
            .iter()
            .skip(i + 1)
            .any(|b| matches!(b, UiBlock::Reasoning(_)));
        let has_later_subagent = app
            .blocks
            .iter()
            .skip(i + 1)
            .any(|b| matches!(b, UiBlock::Subagent(_)));
        let has_later_plan = app
            .blocks
            .iter()
            .skip(i + 1)
            .any(|b| matches!(b, UiBlock::Plan(_)));

        match &app.blocks[i] {
            // The greeting only lives on the landing screen (the ASCII wordmark);
            // in the active chat we show nothing but the conversation.
            UiBlock::Welcome => {}
            UiBlock::User(text) => {
                let text = text.clone();
                out.extend(user_lines(&text, app, width));
                out.push(Line::from(""));
            }
            UiBlock::Assistant { text, streaming } => {
                let content_w = width.saturating_sub(2);
                let streaming = *streaming;
                let text = text.clone();
                let theme = app.theme.clone();
                let mut wrapped = if streaming {
                    let body = markdown::plain(&text, &theme);
                    indent(wrap::wrap_lines(body, content_w))
                } else {
                    app.md_cache.lines(&text, content_w, || {
                        let body = markdown::render(&text, &theme, content_w);
                        indent(wrap::wrap_lines(body, content_w))
                    })
                };
                if streaming {
                    push_caret(&mut wrapped, theme.accent);
                }
                out.extend(wrapped);
                out.push(Line::from(""));
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
                if let Some(text) = body {
                    let style = Style::default().fg(app.theme.faint).add(Modifier::ITALIC);
                    let raw: Vec<Line> = text
                        .split('\n')
                        .map(|l| Line::from(Span::styled(l.to_string(), style)))
                        .collect();
                    for mut l in wrap::wrap_lines(raw, width.saturating_sub(4)) {
                        l.spans.insert(0, Span::raw("    "));
                        out.push(l);
                    }
                }
                out.push(Line::from(""));
            }
            UiBlock::Subagent(card) => {
                let show_hint = !has_later_subagent;
                let card = card.clone();
                // Title sits after the top pad row of the soft strip.
                heads.push((out.len() + 1, i));
                out.extend(subagent_card_lines(&card, app, width, show_hint));
                out.push(Line::from(""));
            }
            UiBlock::Plan(card) => {
                let show_hint = !has_later_plan;
                let card = card.clone();
                heads.push((out.len() + 1, i));
                out.extend(plan_card_lines(&card, app, width, show_hint));
                out.push(Line::from(""));
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
                };
                out.extend(tool_lines(&card, app, width));
                // Keep consecutive tools tight; add air after the last one.
                if !next_is_tool {
                    out.push(Line::from(""));
                }
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
                if !next_is_notice {
                    out.push(Line::from(""));
                }
            }
            UiBlock::Error(s) => {
                let s = s.clone();
                out.extend(wrap::wrap_lines(
                    vec![Line::from(vec![
                        Span::styled("  ✗ ", Style::default().fg(app.theme.err)),
                        Span::styled(s, Style::default().fg(app.theme.err)),
                    ])],
                    width,
                ));
                out.push(Line::from(""));
            }
        }
    }

    (out, heads)
}

/// Markdown preview of Plan.md with section cursor / selection highlights.
fn plan_preview_lines(app: &mut App, width: usize) -> Vec<Line> {
    let theme = app.theme.clone();
    let body = app
        .plan_card()
        .map(|c| c.body.clone())
        .unwrap_or_else(|| "_No plan yet._".into());
    let content_w = width.saturating_sub(2);
    let mut rendered = app.md_cache.lines(&body, content_w, || {
        let body_md = markdown::render(&body, &theme, content_w);
        indent(wrap::wrap_lines(body_md, content_w))
    });

    // Highlight selected / cursor sections by painting matching source lines.
    // Map is approximate: paint status header + hint under the title.
    let mut out = Vec::new();
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "Plan.md".to_string(),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        ),
        Span::styled(
            "  space select · ↑↓ · b build".to_string(),
            Style::default().fg(theme.faint),
        ),
    ]));
    out.push(Line::from(""));

    // Section list for clear selection UI (source of truth for cursor).
    if !app.plan_view.sections.is_empty() {
        for (i, sec) in app.plan_view.sections.iter().enumerate() {
            let on_cursor = i == app.plan_view.cursor;
            let selected = app.plan_view.selected.contains(&i);
            let mark = if selected { "●" } else { "○" };
            let mut style = Style::default().fg(if selected { theme.plan } else { theme.dim });
            if on_cursor {
                style = style.bg(theme.strip).fg(theme.fg).add(Modifier::BOLD);
            }
            out.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{mark} "), style),
                Span::styled(sec.title.clone(), style),
            ]));
        }
        out.push(Line::from(""));
    }

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
        out.extend(user_lines(&card.prompt, app, width));
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
                for mut l in wrap::wrap_lines(raw, width.saturating_sub(4)) {
                    l.spans.insert(0, Span::raw("    "));
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
                    indent(wrap::wrap_lines(body, content_w))
                });
                out.extend(wrapped);
                out.push(Line::from(""));
            }
            SubagentLine::Tool { name, detail, ok } => {
                let status = match ok {
                    None => ToolStatus::Running,
                    Some(true) => ToolStatus::Ok,
                    Some(false) => ToolStatus::Err,
                };
                let tool = ToolCard {
                    id: String::new(),
                    name: name.clone(),
                    args: detail.clone(),
                    output: String::new(),
                    status,
                    started: std::time::Instant::now(),
                    elapsed_ms: Some(0),
                };
                out.extend(tool_lines(&tool, app, width));
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

/// Thought header: bold shimmering "Thinking" while active, then a clear
/// "Thought for Ns" summary. Click (or ctrl+t) to expand/collapse the body.
fn thought_header(th: &crate::app::state::Thought, app: &App, show_hint: bool) -> Line {
    let theme = &app.theme;
    let active = th.elapsed_ms.is_none() && app.running;

    let mut spans: Vec<Span> = vec![Span::raw("  ")];
    if active {
        // Brighter + bold — not a tiny faint "thinking".
        spans.extend(shimmer_bright("Thinking", app.spinner));
        spans.push(Span::styled(
            format!("  {:.0}s", th.secs()),
            Style::default().fg(theme.dim),
        ));
    } else {
        spans.push(Span::styled(
            format!("Thought for {:.1}s", th.secs()),
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
fn user_lines(text: &str, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let bg = theme.strip;
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
            spans.push(Span::styled(
                "@",
                Style::default().fg(theme.fg).bg(bg),
            ));
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

fn indent(lines: Vec<Line>) -> Vec<Line> {
    lines
        .into_iter()
        .map(|mut l| {
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
/// Full-line code fences / table chrome keep a continuous band into the gutter.
/// Inline `code` chips must not — especially after a wrap, when the next row
/// *starts* with a chip (first-span inheritance painted a gray gutter blob).
fn indent_fill_bg(line: &Line) -> Option<Color> {
    let plain: String = line.spans.iter().map(|s| s.content.as_str()).collect();
    let table_chrome = plain.starts_with('┌')
        || plain.starts_with('└')
        || plain.starts_with('├')
        || plain.starts_with('│');
    // Fence body: left-padded `  {tokens…}` with `code_bg` on every span
    // (syntax highlight → many spans). Inline chips use a single pad space.
    let fence_body = is_code_fence_body(line, &plain);
    if !table_chrome && !fence_body {
        return None;
    }
    line.spans
        .iter()
        .find(|s| !s.content.is_empty())
        .and_then(|s| s.style.bg)
}

fn is_code_fence_body(line: &Line, plain: &str) -> bool {
    if !plain.starts_with("  ") {
        return false;
    }
    let mut bg: Option<Color> = None;
    for s in &line.spans {
        if s.content.is_empty() {
            continue;
        }
        match (bg, s.style.bg) {
            (None, Some(c)) => bg = Some(c),
            (Some(expected), Some(c)) if c == expected => {}
            _ => return false,
        }
    }
    bg.is_some()
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
        })
    }

    #[test]
    fn thoughts_collapse_and_expand() {
        use hive_core::event::AgentEvent;
        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::ReasoningDelta("let me ponder this".into()));

        // Active: shimmering "Thinking" header, no body text.
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
        assert!(t.contains("Thinking"));
        assert!(!t.contains("ponder"));

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
        let idx = a.expandable_at_row(row).expect("click hits the header");
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
        assert!(t.contains("cargo check · review"), "{t}");
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
                detail: "cargo check".into(),
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
            name: "verify_project".into(),
            args_preview: "project check".into(),
        });
        assert!(
            !a.blocks
                .iter()
                .any(|b| matches!(b, Block::Tool(c) if c.name == "verify_project")),
            "verify_project must not appear as a transcript tool card"
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
        let indented = super::indent(wrapped);
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
        assert!(!a.plan_view.sections.is_empty());
        let preview = tool_text(&mut a, 72);
        assert!(preview.contains("space select"), "{preview}");
        assert!(preview.contains("Ship it"), "{preview}");
    }
}
