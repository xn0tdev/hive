//! Main / home composer + follow-up banner + mode chip.

use comb::{Frame, Line, Modifier, Rect, Span, Style};
use hive_core::AgentMode;

use crate::app::input::PROMPT_COLS;
use crate::app::App;

const PROMPT: &str = "→ ";

/// Text columns after the prompt/indent for a strip of the given outer width.
pub fn text_cols(band_width: u16) -> usize {
    band_width
        .saturating_sub(2) // horizontal pad inside the strip
        .saturating_sub(PROMPT_COLS as u16) as usize
}

/// Dark strip above the composer: queued follow-up waiting for the turn.
pub fn draw_follow_up(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &app.theme;
    // Slightly darker than the input strip so it reads as a separate band.
    let bg = theme.code_bg;
    f.buffer().paint(area, Style::default().bg(bg));
    let max = area.width.saturating_sub(4) as usize;
    let Some(preview) = app.follow_up_preview(max.saturating_sub(12)) else {
        return;
    };
    let line = Line::from(vec![
        Span::styled(
            " follow-up  ",
            Style::default().fg(theme.faint).bg(bg).add(Modifier::ITALIC),
        ),
        Span::styled(preview, Style::default().fg(theme.dim).bg(bg)),
    ]);
    f.buffer().set_line(area.x, area.y, &line, area.width);
}

pub fn draw(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let bg = theme.strip;

    // Entire strip is the focus hit-target (pads + text).
    app.input_hit = Some(area);

    // Padding rows above/below the text are part of the design — don't trim them.
    f.buffer().paint(area, Style::default().bg(bg));

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let width = app.input.text_cols.max(1);
    let prompt_style = Style::default().fg(theme.accent).bg(bg);
    let text_style = Style::default().fg(theme.fg).bg(bg);

    // Prompt + text first (caret stays on top); @chips sit under the message.
    let mut lines: Vec<Line> = Vec::new();
    if app.input.is_empty() {
        let placeholder = if app.in_plan_view() && app.plan_composing() {
            "Comment · MARK saves · select more anytime"
        } else if app.has_follow_up() {
            "Enter again → next step · ↑ edit"
        } else if app.running {
            "Add a follow-up"
        } else if app.has_pending_attaches() {
            "Describe what to do with the attachment(s)"
        } else {
            "Ask hive anything"
        };
        lines.push(Line::from(vec![
            Span::styled(PROMPT, prompt_style),
            Span::styled(
                placeholder,
                Style::default()
                    .fg(theme.faint)
                    .bg(bg)
                    .add(Modifier::ITALIC),
            ),
        ]));
    } else {
        for (first, text) in app.input.wrapped_rows(width) {
            let head = if first {
                Span::styled(PROMPT, prompt_style)
            } else {
                Span::styled("  ", text_style)
            };
            lines.push(Line::from(vec![head, Span::styled(text, text_style)]));
        }
    }
    if app.has_pending_attaches() {
        let mut spans = vec![Span::styled("  ", text_style)];
        spans.extend(attach_chip_spans(app, bg));
        lines.push(Line::from(spans));
    }

    let tag_rows = usize::from(app.has_pending_attaches());
    let visible = inner.height as usize;
    let scroll = app
        .input
        .view_scroll(visible.saturating_sub(tag_rows), width);
    f.buffer()
        .set_lines_on(inner, &lines, scroll, Style::default().bg(bg));

    if app.running && scroll == 0 {
        let hint = "esc to stop";
        let free = inner.width as usize;
        let first_len = PROMPT_COLS
            + app
                .input
                .wrapped_rows(width)
                .first()
                .map(|(_, t)| t.chars().count())
                .unwrap_or(0);
        if free > first_len + hint.len() + 4 {
            let hint_w = hint.chars().count() as u16;
            let hx = inner.x + inner.width.saturating_sub(1 + hint_w);
            let hint_line = Line::from(Span::styled(hint, Style::default().fg(theme.faint).bg(bg)));
            f.buffer().set_line(hx, inner.y, &hint_line, hint_w);
        }
    }

    if !app.input_focused || app.palette_open() || app.about_open() {
        return;
    }

    let (vrow, vcol) = app.input.cursor_visual(width);
    let x_off = PROMPT_COLS as u16;
    let max_x = inner.x + inner.width.saturating_sub(1);
    let max_y = inner.y + inner.height.saturating_sub(1);
    let x = (inner.x + x_off + vcol as u16).min(max_x);
    // Chips are below the text — don't push the caret down.
    let y = (inner.y + vrow.saturating_sub(scroll) as u16).min(max_y);
    f.set_cursor(x, y);
}

/// MAKE / PLAN chip with padded label, flush to the right edge of `area`.
pub fn draw_mode_chip(f: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = &app.theme;
    let (label, chip_bg) = match app.agent_mode {
        AgentMode::Plan => (" PLAN ", theme.plan),
        AgentMode::Make => (" MAKE ", theme.make),
        AgentMode::Multitask => (" MULTITASK ", theme.multitask),
    };
    let chip_fg = Style::default()
        .fg(theme.sel_fg)
        .bg(chip_bg)
        .add(Modifier::BOLD);
    let w = label.chars().count() as u16;
    if area.width < w {
        return;
    }
    // Flush right — no extra gap after the chip.
    let x = area.x + area.width.saturating_sub(w);
    let line = Line::from(Span::styled(label, chip_fg));
    f.buffer().set_line(x, area.y, &line, w);
}

/// Soft `@path` chips for pending attachments (accent `@`, dim path).
fn attach_chip_spans(app: &App, bg: comb::Color) -> Vec<Span> {
    let theme = &app.theme;
    let mut spans = Vec::new();
    for (i, a) in app.pending_attaches.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ", Style::default().bg(bg)));
        }
        spans.push(Span::styled(
            "@",
            Style::default().fg(theme.accent).bg(bg).add(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            a.label.clone(),
            Style::default().fg(theme.dim).bg(bg),
        ));
    }
    spans
}

#[cfg(test)]
mod tests {
    use comb::{render_with_cursor, Size};

    use crate::app::App;
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
    fn main_chat_keeps_input_cursor() {
        let mut a = app();
        a.apply(hive_core::event::AgentEvent::TurnStarted);
        a.apply(hive_core::event::AgentEvent::AssistantTextDelta(
            "hi".into(),
        ));
        a.apply(hive_core::event::AgentEvent::TurnFinished);

        let (_buf, cursor) =
            render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert!(cursor.is_some(), "main input should place a caret");
        assert!(a.input_hit.is_some(), "input hit target recorded");
        assert!(a.input_focused);
    }

    #[test]
    fn blurred_input_hides_hardware_cursor() {
        let mut a = app();
        a.blur_input();

        let (_buf, cursor) =
            render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert!(cursor.is_none(), "blurred input must not show a caret");
        assert!(
            a.input_hit.is_some(),
            "hit target still recorded when blurred"
        );
    }

    #[test]
    fn click_outside_blurs_click_inside_focuses() {
        use comb::Rect;

        let mut a = app();
        let (_buf, _) = render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        let hit = a.input_hit.expect("input rect");
        assert!(a.input_focused);

        // Outside the strip → blur.
        let outside = Rect::new(0, 0, 1, 1);
        assert!(!hit.contains(outside.x, outside.y));
        a.blur_input();
        assert!(!a.input_focused);

        // Inside the strip → focus.
        a.focus_input();
        assert!(a.input_contains(hit.x, hit.y));
        assert!(a.input_focused);
    }

    #[test]
    fn live_plan_selection_shows_comment_placeholder() {
        use crate::app::state::PlanDrag;
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "Test".into(),
            body: "# Test\n\nselect this text\n".into(),
        });
        a.open_plan_view();
        // Mouse-down has started a selection, but mouse-up has not made a saved mark yet.
        a.plan_view.drag = Some(PlanDrag {
            anchor: 0,
            current: 6,
        });

        let (buf, _) =
            render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        let text = buf.text();
        assert!(text.contains("Comment"), "{text}");
        assert!(text.contains("MARK"), "{text}");
        assert!(!text.contains("Ask hive anything"), "{text}");
    }

    #[test]
    fn zero_width_plan_drag_keeps_idle_bar() {
        use crate::app::state::PlanDrag;
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "Test".into(),
            body: "# Test\n\nselect this text\n".into(),
        });
        a.open_plan_view();
        // Mouse-down without any movement is only a click, not a selection.
        a.plan_view.drag = Some(PlanDrag {
            anchor: 6,
            current: 6,
        });

        let (buf, _) =
            render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        let text = buf.text();
        assert!(text.contains("MAKE"), "{text}");
        assert!(!text.contains("Comment"), "{text}");
        assert!(!text.contains("  MARK  "), "{text}");
    }

    #[test]
    fn mode_chip_renders_make_plan_and_multitask() {
        use hive_core::AgentMode;

        let mut a = app();
        a.apply(hive_core::event::AgentEvent::TurnStarted);
        a.apply(hive_core::event::AgentEvent::AssistantTextDelta(
            "hi".into(),
        ));
        a.apply(hive_core::event::AgentEvent::TurnFinished);

        let (buf, _) = render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert!(buf.text().contains("MAKE"), "{}", buf.text());

        a.agent_mode = AgentMode::Plan;
        let (buf, _) = render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert!(buf.text().contains("PLAN"), "{}", buf.text());

        a.agent_mode = AgentMode::Multitask;
        let (buf, _) = render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert!(buf.text().contains("MULTITASK"), "{}", buf.text());
    }
}
