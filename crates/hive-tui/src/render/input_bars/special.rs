//! Special-view bars: `← back` (subagent) and plan bar (back/composer + MAKE/MARK/SEND).

use comb::{Frame, Line, Modifier, Rect, Span, Style};
use hive_core::AgentMode;

use crate::app::App;

use super::chat::{draw, text_cols};
use super::BACK;

/// One roomy chip size for MAKE / MARK / SEND (`  word  ` = 8 cells).
pub const PLAN_ACTION_COLS: u16 = 8;

/// Clickable back control shown while browsing a subagent thread.
///
/// Not an input: no hardware cursor, no IME target — just a button hit-target.
/// Esc / ← / click all leave via the app event handlers.
pub fn draw_back(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let bg = if app.hover_back {
        theme.strip_hover
    } else {
        theme.strip
    };
    f.buffer().paint(area, Style::default().bg(bg));

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    app.input_hit = None;
    if inner.width == 0 || inner.height == 0 {
        app.back_hit = None;
        return;
    }

    let back_line = Line::from(Span::styled(
        BACK,
        Style::default().fg(theme.accent).bg(bg).add(Modifier::BOLD),
    ));
    f.buffer()
        .set_lines_on(inner, &[back_line], 0, Style::default().bg(bg));

    let hint = "esc";
    let hint_w = hint.chars().count() as u16;
    if inner.width > hint_w + BACK.chars().count() as u16 + 2 {
        let hx = inner.x + inner.width.saturating_sub(hint_w);
        let hint_fg = if app.hover_back {
            theme.dim
        } else {
            theme.faint
        };
        let hint_line = Line::from(Span::styled(hint, Style::default().fg(hint_fg).bg(bg)));
        f.buffer().set_line(hx, inner.y, &hint_line, hint_w);
    }

    app.back_hit = Some(area);
    app.make_hit = None;
}

/// Plan bar:
/// - idle, no comments → `← back` + MAKE
/// - composing a comment → composer + MARK
/// - idle with comments → `← back` + SEND (same place as MAKE)
pub fn draw_plan_bar(f: &mut Frame, area: Rect, app: &mut App) {
    let composing = app.plan_composing();
    let word = app.plan_action_word();
    let aw = PLAN_ACTION_COLS;

    let can_action = area.width > aw + BACK.chars().count() as u16 + 6;
    let action = if can_action {
        Some(Rect {
            x: area.x + area.width.saturating_sub(aw),
            y: area.y,
            width: aw,
            height: area.height,
        })
    } else {
        None
    };

    let left_w = action
        .map(|r| r.x.saturating_sub(area.x))
        .unwrap_or(area.width);
    let left = Rect {
        x: area.x,
        y: area.y,
        width: left_w.max(1),
        height: area.height,
    };

    // Wrap width for the composer only (action column reserved).
    app.input.text_cols = text_cols(left.width);

    if composing {
        app.agent_mode = AgentMode::Plan;
        draw(f, left, app);
        app.back_hit = None;
    } else {
        draw_back_in(f, left, app);
    }

    if let Some(br) = action {
        let theme = &app.theme;
        let bg = if app.hover_make {
            theme.make_hover
        } else {
            theme.make
        };
        // Same blue rect for every action; comb centers the word inside.
        f.buffer().put_label(
            br,
            word,
            Style::default().fg(theme.sel_fg).bg(bg).add(Modifier::BOLD),
        );
        app.make_hit = Some(br);
    } else {
        app.make_hit = None;
    }
}

fn draw_back_in(f: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.theme;
    let bg = if app.hover_back {
        theme.strip_hover
    } else {
        theme.strip
    };
    f.buffer().paint(area, Style::default().bg(bg));

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    app.input_hit = None;
    if inner.width == 0 || inner.height == 0 {
        app.back_hit = None;
        return;
    }

    let back_line = Line::from(Span::styled(
        BACK,
        Style::default().fg(theme.accent).bg(bg).add(Modifier::BOLD),
    ));
    f.buffer()
        .set_lines_on(inner, &[back_line], 0, Style::default().bg(bg));

    app.back_hit = Some(area);
    app.make_hit = None;
}

#[cfg(test)]
mod tests {
    use comb::{render_with_cursor, Size};

    use crate::app::state::{PlanAction, PlanCorrection};
    use crate::app::App;
    use crate::TuiInit;

    use super::PLAN_ACTION_COLS;

    #[test]
    fn plan_chip_width_is_roomy_and_even() {
        assert_eq!(PLAN_ACTION_COLS, 8);
    }

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
    fn subagent_back_has_no_hardware_cursor() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking".into(),
            prompt: "go".into(),
        });
        a.open_subagent_view("v1".into());

        let size = Size::new(80, 24);
        let (buf, cursor) = render_with_cursor(size, |f| crate::render::draw(f, &mut a));
        assert!(cursor.is_none(), "back control must not show a caret");
        assert!(a.back_hit.is_some(), "back hit target recorded");
        let text = buf.text();
        assert!(text.contains("← back"), "{text}");
        assert!(text.contains("esc"), "{text}");
        let last = size.height - 1;
        let mut bottom = String::new();
        for y in last.saturating_sub(1)..=last {
            for x in 0..size.width {
                bottom.push(buf.get(x, y).map(|c| c.ch).unwrap_or(' '));
            }
        }
        assert!(
            !bottom.contains("128.0k") && !bottom.contains("/tmp"),
            "no statusline under back: {bottom:?}"
        );
    }

    #[test]
    fn plan_toast_sits_in_the_bottom_right() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "S".into(),
            body: "# Plan\n\nstep one\nstep two\n".into(),
        });
        a.open_plan_view();
        a.flash("Ctrl+C again to quit");

        let size = Size::new(80, 24);
        let (buf, _) = render_with_cursor(size, |f| crate::render::draw(f, &mut a));
        let last = size.height - 1;
        let text = buf.text();
        assert!(text.contains("Ctrl+C again to quit"), "{text}");
        assert!(!text.contains("Info"), "no kind label: {text}");

        // Not on the footer row — that stays clean.
        let mut footer = String::new();
        for x in 0..size.width {
            footer.push(buf.get(x, last).map(|c| c.ch).unwrap_or(' '));
        }
        assert!(
            !footer.contains("Ctrl+C again to quit"),
            "toast left the last row: {footer:?}"
        );

        let mut found = false;
        for y in (0..size.height).rev() {
            let mut row = String::new();
            for x in 0..size.width {
                row.push(buf.get(x, y).map(|c| c.ch).unwrap_or(' '));
            }
            if let Some(at) = row.find("Ctrl+C") {
                found = true;
                let cell = buf.get(at as u16, y).unwrap();
                assert_eq!(cell.style.bg, Some(a.theme.strip), "filled chip: {row:?}");
                assert!(
                    at > size.width as usize / 2,
                    "bottom-right, not the left edge: {row:?}"
                );
                assert!(
                    y >= size.height / 2,
                    "lower half, not the top: y={y} {row:?}"
                );
                break;
            }
        }
        assert!(found, "notification missing from the bottom-right");

        // The InputZone (back / MARK / SEND) is raised to the chat Input height:
        // its strip band ends two rows above the bottom (same reserve as chat).
        let strip_bottom = (0..size.height)
            .rev()
            .find(|&y| buf.get(40, y).and_then(|c| c.style.bg).is_some())
            .expect("plan bar strip painted");
        assert_eq!(
            strip_bottom,
            size.height - 3,
            "InputZone bottom edge matches chat Input (2 rows reserved below)"
        );
    }

    #[test]
    fn plan_action_chips_are_equal_width_and_exactly_centered() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "Test".into(),
            body: "# Test\n\nhello world\n".into(),
        });
        a.open_plan_view();

        let (buf, _) = render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        let hit = a.make_hit.expect("MAKE");
        assert_eq!(
            hit.width, 8,
            "all action chips use the roomier 8-cell width"
        );
        assert_eq!(hit.height, 3, "hit column matches textarea strip height");
        assert_eq!(a.plan_action(), PlanAction::Make);
        assert!(a.back_hit.is_some(), "idle: back + MAKE");
        assert!(buf.text().contains("  MAKE  "), "{}", buf.text());

        // Composing a comment → MARK (not SEND yet).
        a.plan_view.corrections.push(PlanCorrection {
            start: 0,
            end: 5,
            excerpt: "# Tes".into(),
            note: String::new(),
        });
        a.plan_view.active = Some(0);
        assert!(a.plan_composing());
        assert_eq!(a.plan_action(), PlanAction::Add);
        assert_eq!(a.plan_action_word(), "MARK");

        let (buf, _) = render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert_eq!(a.make_hit.map(|r| r.width), Some(8));
        assert!(buf.text().contains("  MARK  "), "{}", buf.text());
        assert!(a.back_hit.is_none(), "composer hides back");

        // MARK commits → idle back + SEND (same slot as MAKE).
        a.input.insert_str("fix this");
        assert!(a.plan_commit_note());
        assert!(!a.plan_composing());
        assert_eq!(a.plan_action(), PlanAction::Send);
        assert_eq!(a.plan_action_word(), "SEND");
        assert!(!a.plan_view.corrections[0].note.is_empty());

        let (buf, _) = render_with_cursor(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        assert_eq!(a.make_hit.map(|r| r.width), Some(8));
        assert!(a.back_hit.is_some(), "idle comments: back + SEND");
        assert!(buf.text().contains("  SEND  "), "{}", buf.text());
        assert!(buf.text().contains("← back"), "{}", buf.text());
        assert!(!buf.text().contains("MAKE"), "{}", buf.text());

        // Another selection → compose again (multiple comments).
        a.plan_view.corrections.push(PlanCorrection {
            start: 6,
            end: 11,
            excerpt: "hello".into(),
            note: String::new(),
        });
        a.plan_view.active = Some(1);
        assert_eq!(a.plan_action(), PlanAction::Add);
    }
}
