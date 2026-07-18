//! Builds and draws the scrolling conversation transcript and the "Working"
//! indicator. The greeting lives only on the landing screen, not here.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::state::{Block as UiBlock, ToolCard, ToolStatus};
use crate::app::App;
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

    // Remember which screen rows hold thought headers so a mouse click can be
    // mapped back to its block.
    app.thought_hits.clear();
    for (line_idx, block_idx) in heads {
        if line_idx >= scroll && line_idx < scroll + target.height as usize {
            app.thought_hits
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

/// Per-character grayscale "shimmer": a bright highlight sweeps across `text`,
/// driven by the animation tick, so the word glows while a turn runs.
fn shimmer(text: &str, tick: usize) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as i32;
    // Highlight head sweeps from just before the word to just after, then repeats.
    let head = (tick as i32 % (n + 6)) - 3;
    let (lo, hi) = (0x70i32, 0xf2i32); // base gray → near-white peak
    chars
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let dist = (i as i32 - head).abs() as f32;
            let t = (1.0 - dist / 3.0).max(0.0); // falloff over ~3 cells
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

/// Like `lines`, but also reports which line index holds each thought header
/// (with its block index) so mouse clicks can be hit-tested.
fn build(app: &App, width: usize) -> (Vec<Line>, Vec<(usize, usize)>) {
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
                let body = if *streaming {
                    markdown::plain(text, theme)
                } else {
                    markdown::render(text, theme)
                };
                let mut wrapped = indent(wrap::wrap_lines(body, width.saturating_sub(2)));
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

/// The one-line summary of a thought: shimmering "thinking" with a live timer
/// while the model reasons, then a quiet "thought for Ns · ~N tokens" line.
/// Click the line (or ctrl+t) to reveal or hide the full text.
fn thought_header(
    th: &crate::app::state::Thought,
    app: &App,
    show_hint: bool,
) -> Line {
    let theme = &app.theme;
    let active = th.elapsed_ms.is_none() && app.running;

    let mut spans: Vec<Span> = vec![Span::raw("  ")];
    if active {
        spans.extend(shimmer("thinking", app.spinner));
        spans.push(Span::styled(
            format!(" {:.0}s", th.secs()),
            Style::default().fg(theme.faint),
        ));
    } else {
        spans.push(Span::styled("∴ ", Style::default().fg(theme.faint)));
        spans.push(Span::styled(
            format!("thought for {:.1}s", th.secs()),
            Style::default().fg(theme.dim).add(Modifier::ITALIC),
        ));
        if th.approx_tokens() > 0 {
            spans.push(Span::styled(
                format!(" · ~{} tokens", th.approx_tokens()),
                Style::default().fg(theme.faint),
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
            l.spans.insert(0, Span::raw("  "));
            l
        })
        .collect()
}

fn push_caret(lines: &mut Vec<Line>, color: Color) {
    match lines.last_mut() {
        Some(last) => last
            .spans
            .push(Span::styled("▊", Style::default().fg(color))),
        None => lines.push(Line::from(Span::styled("  ▊", Style::default().fg(color)))),
    }
}

/// One compact line per tool; live output tail only while running or on error.
fn tool_lines(card: &ToolCard, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let (icon, color) = match card.status {
        ToolStatus::Running => (app.spinner_char().to_string(), theme.accent),
        ToolStatus::Ok => ("✔".to_string(), theme.ok),
        ToolStatus::Err => ("✘".to_string(), theme.err),
    };

    let mut header = vec![
        Span::styled(format!("  {icon} "), Style::default().fg(color)),
        Span::styled(
            card.name.clone(),
            Style::default().fg(theme.dim).add(Modifier::BOLD),
        ),
    ];
    if !card.args.is_empty() {
        header.push(Span::styled(
            format!("  {}", card.args),
            Style::default().fg(theme.faint),
        ));
    }
    let mut out = wrap::wrap_lines(vec![Line::from(header)], width);

    // Code edits render a compact green/red diff block, whatever the status.
    let is_edit = matches!(card.name.as_str(), "edit_file" | "write_file");
    if is_edit && !card.output.trim().is_empty() {
        out.extend(diff_lines(&card.output, app, width));
        return out;
    }

    // Other tools: a short tail of live output while running or on error.
    let show_tail = match card.status {
        ToolStatus::Running => 6,
        ToolStatus::Err => 3,
        ToolStatus::Ok => 0,
    };
    if show_tail > 0 && !card.output.trim().is_empty() {
        let tail: Vec<&str> = {
            let mut v: Vec<&str> = card.output.lines().collect();
            if v.len() > show_tail {
                v = v.split_off(v.len() - show_tail);
            }
            v
        };
        let raw: Vec<Line> = tail
            .into_iter()
            .map(|l| {
                Line::from(Span::styled(
                    format!("    {l}"),
                    Style::default().fg(theme.faint),
                ))
            })
            .collect();
        out.extend(wrap::wrap_lines(raw, width));
    }

    out
}

/// Render a diff (lines prefixed `+`/`-`) as an indented block where each row's
/// background hugs the content width — a green add / red delete band, never the
/// full terminal width.
fn diff_lines(diff: &str, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let indent = 4usize;
    let avail = width.saturating_sub(indent + 1).max(8);
    let rows: Vec<&str> = diff.lines().filter(|l| !l.trim().is_empty()).collect();

    // Block width tracks the longest row, but is capped to the space we have.
    let block_w = rows
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(1, avail);

    rows.into_iter()
        .map(|row| {
            let (fg, bg) = if row.starts_with("+ ") {
                (theme.add_fg, theme.add_bg)
            } else if row.starts_with("- ") {
                (theme.del_fg, theme.del_bg)
            } else {
                (theme.faint, theme.code_bg)
            };
            let mut text: String = row.chars().take(block_w).collect();
            let pad = block_w.saturating_sub(text.chars().count());
            if pad > 0 {
                text.push_str(&" ".repeat(pad));
            }
            Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(text, Style::default().fg(fg).bg(bg)),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::diff_lines;
    use crate::app::App;
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

        // Active: shimmering "thinking" header, no body text.
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
        assert!(t.contains("thinking"));
        assert!(!t.contains("ponder"));

        // Model moves on → thought closes with a duration; still collapsed.
        a.apply(AgentEvent::AssistantTextDelta("answer".into()));
        let t = text(&super::lines(&a, 80));
        assert!(t.contains("thought for"));
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
        let (row, _) = *a.thought_hits.first().expect("header row recorded");
        assert!(!before.text().contains("secret plan"));

        // A click on that row opens exactly that thought.
        let idx = a.thought_at_row(row).expect("click hits the header");
        a.toggle_thought_at(idx);
        let after = render(Size::new(90, 24), |f| crate::render::draw(f, &mut a));
        assert!(after.text().contains("secret plan"));
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
}
