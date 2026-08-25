//! Recap overlay: Generating shimmer, then a short retelling of one turn.

use comb::{Buffer, Line, Modifier, Rect, Span, Style};

use crate::app::state::RecapBody;
use crate::app::App;
use crate::render::panel::Panel;
use crate::render::wrap;

const MIN_W: u16 = 44;
const MAX_W: u16 = 60;
const PAD_X: u16 = 3;
const PAD_Y: u16 = 1;
const PANEL_H: u16 = 18;
const GENERATING: &str = "Generating....";

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    let Some(st) = app.recap_overlay.as_ref() else {
        return;
    };
    let recap_id = st.recap_id;
    let reveal = st.reveal_stream;
    let scroll = st.scroll;
    let Some(card) = app.recap_card(recap_id).cloned() else {
        return;
    };
    let theme = app.theme.clone();
    let panel = theme.strip;
    let spinner = app.spinner;
    let g = overlay().render(buf, area, &theme);
    app.recap_generating_hit = None;

    if g.content.width < 12 || g.content.height < 4 {
        return;
    }

    let show_generating = match &card.recap {
        RecapBody::Generating { text } => !reveal || text.is_empty(),
        RecapBody::Idle => true,
        _ => false,
    };

    if show_generating {
        draw_generating(buf, g.content, spinner, &theme, panel);
        let w = GENERATING.chars().count() as u16;
        let x = g.content.x + g.content.width.saturating_sub(w) / 2;
        let y = g.content.y + g.content.height / 2;
        app.recap_generating_hit = Some(Rect::new(x, y, w, 1));
        if let Some(st) = app.recap_overlay.as_mut() {
            st.visible = 0;
        }
        return;
    }

    let (title, body, failed) = match &card.recap {
        RecapBody::Ready { text } => ("Recap", text.as_str(), false),
        RecapBody::Generating { text } => ("Recap", text.as_str(), false),
        RecapBody::Failed { error } => ("Recap", error.as_str(), true),
        RecapBody::Idle => ("Recap", "", false),
    };

    let title_w = title.chars().count() as u16;
    let pad = g.content.width.saturating_sub(title_w) / 2;
    crate::render::strip_paint::set_line_on_strip(
        buf,
        g.content.x,
        g.content.y,
        &Line::from(vec![
            Span::styled(" ".repeat(pad as usize), Style::default().bg(panel)),
            Span::styled(
                title.to_string(),
                Style::default().fg(theme.fg).bg(panel).add(Modifier::BOLD),
            ),
        ]),
        g.content.width,
        panel,
    );

    let body_area = Rect::new(
        g.content.x,
        g.content.y.saturating_add(2),
        g.content.width,
        g.content.height.saturating_sub(2),
    );
    let fg = if failed { theme.dim } else { theme.fg };
    let mut wrapped = wrap_body(body, body_area.width as usize, fg, panel);
    if matches!(card.recap, RecapBody::Generating { .. }) && !body.is_empty() {
        match wrapped.last_mut() {
            Some(last) => last.spans.push(Span::styled(
                "▏",
                Style::default().fg(theme.accent).bg(panel),
            )),
            None => wrapped.push(Line::from(Span::styled(
                "▏",
                Style::default().fg(theme.accent).bg(panel),
            ))),
        }
    }
    let vis = body_area.height as usize;
    let max_off = wrapped.len().saturating_sub(vis);
    let off = scroll.min(max_off);
    if let Some(st) = app.recap_overlay.as_mut() {
        st.visible = vis;
        st.scroll = off;
    }

    for row in 0..vis {
        let y = body_area.y + row as u16;
        match wrapped.get(off + row) {
            Some(line) => {
                crate::render::strip_paint::set_line_on_strip(
                    buf,
                    body_area.x,
                    y,
                    line,
                    body_area.width,
                    panel,
                );
            }
            None => {
                buf.paint(
                    Rect::new(body_area.x, y, body_area.width, 1),
                    Style::default().bg(panel),
                );
            }
        }
    }
}

fn wrap_body(text: &str, width: usize, fg: comb::Color, bg: comb::Color) -> Vec<Line> {
    let style = Style::default().fg(fg).bg(bg);
    let lines: Vec<Line> = if text.is_empty() {
        vec![Line::from("")]
    } else {
        text.lines()
            .map(|l| Line::from(Span::styled(l.to_string(), style)))
            .collect()
    };
    wrap::wrap_lines(lines, width.max(1))
}

fn draw_generating(
    buf: &mut Buffer,
    area: Rect,
    spinner: usize,
    theme: &crate::theme::Theme,
    panel: comb::Color,
) {
    let spans = shimmer_spans(GENERATING, spinner, theme);
    let w = GENERATING.chars().count() as u16;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height / 2;
    buf.set_line(x, y, &Line::from(spans), w);
    let _ = panel;
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

fn overlay() -> Panel<'static> {
    Panel::new("", "", PANEL_H)
        .width_ratio(1, 2)
        .width_bounds(MIN_W, MAX_W)
        .padding(PAD_X, PAD_Y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::recap::RecapOverlay;
    use crate::app::state::{Block, RecapBody, WorkSummaryCard};
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

    fn with_card(recap: RecapBody) -> App {
        let mut a = app();
        a.blocks.push(Block::WorkSummary(WorkSummaryCard {
            secs: 2,
            recap_id: 7,
            recap,
        }));
        a.recap_overlay = Some(RecapOverlay {
            recap_id: 7,
            reveal_stream: false,
            scroll: 0,
            visible: 0,
        });
        a
    }

    #[test]
    fn generating_is_centered_without_a_header() {
        let mut a = with_card(RecapBody::Generating {
            text: String::new(),
        });
        let shown = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a))
            .text()
            .to_string();
        assert!(shown.contains("Generating"), "{shown}");
        assert!(!shown.contains("Settings"), "{shown}");
        let recap_count = shown.matches("Recap").count();
        assert_eq!(recap_count, 0, "no Recap title while generating: {shown}");
    }

    #[test]
    fn ready_shows_recap_title_and_body() {
        let mut a = with_card(RecapBody::Ready {
            text: "Fixed the hover on Worked for.".into(),
        });
        let shown = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a))
            .text()
            .to_string();
        assert!(shown.contains("Recap"), "{shown}");
        assert!(shown.contains("Fixed the hover"), "{shown}");
        assert!(!shown.contains("Generating"), "{shown}");
    }
}
