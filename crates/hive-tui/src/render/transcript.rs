//! Builds and draws the scrolling conversation transcript and the "Working"
//! indicator. The greeting lives only on the landing screen, not here.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use hive_core::event::{SubagentLine, SubagentStatus};

use crate::app::state::{Block as UiBlock, ChatView, SubagentCard, ToolCard, ToolStatus};
use crate::app::App;
use crate::render::tools::{format_tool_secs, subagent_card_lines, tool_lines};
use crate::render::{markdown, wrap};

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    let width = area.width.max(1) as usize;
    let (all, heads) = build(app, width);
    let total = all.len();
    let viewport = area.height as usize;
    let max_scroll = total.saturating_sub(viewport);
    let scroll = max_scroll.saturating_sub(app.scroll_from_bottom);

    // Bottom-anchor: a short conversation hugs the input instead of floating
    // at the top with a screenful of emptiness in between.
    let target = if total < viewport {
        Rect {
            y: area.y + (viewport - total) as u16,
            height: total as u16,
            ..area
        }
    } else {
        area
    };

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

/// The spinner block shown above the input while a turn is running.
pub fn draw_working(buf: &mut Buffer, area: Rect, app: &App) {
    let theme = &app.theme;
    let mut spans = vec![Span::styled(
        format!("{} ", app.spinner_char()),
        Style::default().fg(theme.accent),
    )];
    spans.extend(shimmer("Working", app.spinner));
    // A blank line above and below so the indicator breathes.
    let lines = vec![Line::default(), Line::from(spans), Line::default()];
    buf.set_lines(area, &lines, 0);
}

/// Per-character grayscale shimmer for the "Working" indicator.
fn shimmer(text: &str, tick: usize) -> Vec<Span> {
    shimmer_range(text, tick, 0x70, 0xf2)
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
                Style::default()
                    .fg(Color::Rgb(v, v, v))
                    .add(Modifier::BOLD),
            )
        })
        .collect()
}

/// Build the full, pre-wrapped set of transcript lines (test helper).
#[cfg(test)]
pub fn lines(app: &App, width: usize) -> Vec<Line> {
    build(app, width).0
}

/// Like `lines`, but also reports which line index holds each expandable
/// header (thought / subagent) with its block index for mouse hit-testing.
fn build(app: &App, width: usize) -> (Vec<Line>, Vec<(usize, usize)>) {
    if matches!(app.view, ChatView::Subagent(_)) {
        if let Some(card) = app.viewed_subagent() {
            return (subagent_chat_lines(card, app, width), Vec::new());
        }
        // Stale id (cleared chat) — fall through to main.
    }

    let theme = &app.theme;
    let mut out: Vec<Line> = Vec::new();
    let mut heads: Vec<(usize, usize)> = Vec::new();

    for (i, block) in app.blocks.iter().enumerate() {
        let next = app.blocks.get(i + 1);
        match block {
            // The greeting only lives on the landing screen (the ASCII wordmark);
            // in the active chat we show nothing but the conversation.
            UiBlock::Welcome => {}
            UiBlock::User(text) => {
                out.extend(user_lines(text, app, width));
                out.push(Line::from(""));
            }
            UiBlock::Assistant { text, streaming } => {
                let content_w = width.saturating_sub(2);
                let body = if *streaming {
                    markdown::plain(text, theme)
                } else {
                    markdown::render(text, theme, content_w)
                };
                let mut wrapped = indent(wrap::wrap_lines(body, content_w));
                if *streaming {
                    push_caret(&mut wrapped, theme.accent);
                }
                out.extend(wrapped);
                out.push(Line::from(""));
            }
            UiBlock::Reasoning(th) => {
                let is_last_thought = !app
                    .blocks
                    .iter()
                    .skip(i + 1)
                    .any(|b| matches!(b, UiBlock::Reasoning(_)));
                heads.push((out.len(), i));
                out.push(thought_header(th, app, is_last_thought));
                if th.open && !th.text.trim().is_empty() {
                    let style = Style::default()
                        .fg(theme.faint)
                        .add(Modifier::ITALIC);
                    let raw: Vec<Line> = th
                        .text
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
                let show_hint = !app
                    .blocks
                    .iter()
                    .skip(i + 1)
                    .any(|b| matches!(b, UiBlock::Subagent(_)));
                // Title sits after the top pad row of the soft strip.
                heads.push((out.len() + 1, i));
                out.extend(subagent_card_lines(card, app, width, show_hint));
                out.push(Line::from(""));
            }
            UiBlock::Tool(card) => {
                out.extend(tool_lines(card, app, width));
                // Keep consecutive tools tight; add air after the last one.
                if !matches!(next, Some(UiBlock::Tool(_))) {
                    out.push(Line::from(""));
                }
            }
            UiBlock::Notice(s) => {
                out.extend(wrap::wrap_lines(
                    vec![Line::from(vec![
                        Span::styled("  · ", Style::default().fg(theme.faint)),
                        Span::styled(s.clone(), Style::default().fg(theme.dim)),
                    ])],
                    width,
                ));
                if !matches!(next, Some(UiBlock::Notice(_))) {
                    out.push(Line::from(""));
                }
            }
            UiBlock::Error(s) => {
                out.extend(wrap::wrap_lines(
                    vec![Line::from(vec![
                        Span::styled("  ✗ ", Style::default().fg(theme.err)),
                        Span::styled(s.clone(), Style::default().fg(theme.err)),
                    ])],
                    width,
                ));
                out.push(Line::from(""));
            }
        }
    }

    (out, heads)
}

/// Dedicated read-only chat for a subagent: task as a user strip, then its
/// thoughts / tools / assistant messages — same visual language as the main chat.
fn subagent_chat_lines(card: &SubagentCard, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
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
                        "∴ Thinking",
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
                let body = markdown::render(t, theme, content_w);
                out.extend(indent(wrap::wrap_lines(body, content_w)));
                out.push(Line::from(""));
            }
            SubagentLine::Tool { name, detail, ok } => {
                let status = match ok {
                    None => ToolStatus::Running,
                    Some(true) => ToolStatus::Ok,
                    Some(false) => ToolStatus::Err,
                };
                let card = ToolCard {
                    id: String::new(),
                    name: name.clone(),
                    args: detail.clone(),
                    output: String::new(),
                    status,
                    started: std::time::Instant::now(),
                    elapsed_ms: Some(0),
                };
                out.extend(tool_lines(&card, app, width));
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
fn thought_header(
    th: &crate::app::state::Thought,
    app: &App,
    show_hint: bool,
) -> Line {
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
            "∴ ",
            Style::default().fg(theme.accent).add(Modifier::BOLD),
        ));
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
fn user_lines(text: &str, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let bg = theme.strip;
    let body = Style::default().fg(theme.fg).bg(bg);
    let pad_row = || Line::from(Span::styled(" ".repeat(width), body));

    let raw: Vec<Line> = text
        .split('\n')
        .map(|l| Line::from(Span::styled(l.to_string(), body)))
        .collect();
    let wrapped = wrap::wrap_lines(raw, width.saturating_sub(4));

    let mut out = vec![pad_row()];
    out.extend(wrapped.into_iter().map(|line| {
        let mut used = 2usize;
        let mut spans = vec![Span::styled("  ", body)];
        // Re-tint the content spans onto the strip background.
        for s in line.spans {
            used += s.content.chars().count();
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
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
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
        let t = text(&super::lines(&a, 80));
        assert!(t.contains("Thinking"));
        assert!(!t.contains("ponder"));

        // Model moves on → thought closes with a duration; still collapsed.
        a.apply(AgentEvent::AssistantTextDelta("answer".into()));
        let t = text(&super::lines(&a, 80));
        assert!(t.contains("Thought for"));
        assert!(t.contains("tokens"));
        assert!(!t.contains("ponder"));

        // ctrl+t reveals the full text.
        a.toggle_thoughts();
        let t = text(&super::lines(&a, 80));
        assert!(t.contains("ponder"));
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

        let t = tool_text(&a, 72);
        assert!(t.contains("Checking project"), "{t}");
        assert!(t.contains("cargo check · review"), "{t}");
        assert!(!t.contains('╭') && !t.contains('╯'), "no border chrome: {t}");

        a.apply(AgentEvent::SubagentStatus {
            id: "v1".into(),
            status: SubagentStatus::Done,
            detail: "done".into(),
        });
        a.apply(AgentEvent::SubagentTranscript {
            id: "v1".into(),
            line: SubagentLine::Assistant("## Verification Report\nAll good.".into()),
        });

        let collapsed = tool_text(&a, 72);
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
        let open = tool_text(&a, 72);
        assert!(open.contains("Run cargo check"), "{open}");
        assert!(open.contains("Verification Report"), "{open}");

        a.leave_subagent_view();
        assert!(!a.in_subagent_view());
        let back = tool_text(&a, 72);
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
        let lines = super::lines(&a, 72);
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

        let lines = super::lines(&a, 72);
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
        let lines = super::lines(&a, 72);
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
            a.blocks
                .iter()
                .any(|b| matches!(b, Block::Subagent(_))),
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
        assert!(lines.iter().all(|l| l.spans[1].content.chars().count() == w));
        assert!(w < 60); // not the full terminal width
    }

    fn tool_text(a: &App, width: usize) -> String {
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

        let t = tool_text(&a, 72);
        // Duration belongs on the `$ …` header, not as a column-0 orphan.
        assert!(t.contains("$ cargo check"), "shell header: {t}");
        assert!(t.contains("·"), "duration separator: {t}");
        let header = t
            .lines()
            .find(|l| l.contains("$ cargo check"))
            .unwrap_or("");
        assert!(
            header.contains('s'),
            "duration on shell header line: {t}"
        );
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

        let t = tool_text(&a, 80);
        assert!(t.contains("Reading crates/hive-tui"), "verb header: {t}");
        assert!(!t.contains("read_file"), "no raw tool name: {t}");
        assert!(!t.contains("✓"), "no status icon on tool rows: {t}");
    }
}
