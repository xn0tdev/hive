use comb::{Color, Frame, Line, Modifier, Rect, Span, Style};

use crate::app::state::{Block, TerminalViewPhase};
use crate::app::App;

pub(crate) fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    frame.buffer().paint(area, Style::default());
    app.click_hits.clear();
    app.assistant_row_hits.clear();
    app.assistant_rows.clear();
    app.input_hit = None;
    app.make_hit = None;

    let Some(id) = app.terminal_view_id().map(str::to_string) else {
        app.terminal_view.body_rect = None;
        app.transcript_hit = None;
        return;
    };
    let Some(command) = app.terminal_card(&id).map(|card| card.command.clone()) else {
        app.terminal_view.phase = TerminalViewPhase::Failed;
        app.terminal_view.body_rect = None;
        app.transcript_hit = None;
        return;
    };

    if area.height > 1 {
        frame.buffer().set_line(
            area.x,
            area.y + 1,
            &Line::from(vec![
                Span::styled(
                    "Terminal",
                    Style::default().fg(app.theme.fg).add(Modifier::BOLD),
                ),
                Span::styled(" · ", Style::default().fg(app.theme.faint)),
                Span::styled(command, Style::default().fg(app.theme.dim)),
            ]),
            area.width,
        );
    }

    // Same rhythm as Plan.md: title, one blank row, then the body.
    let body_y = area.y.saturating_add(3).min(area.bottom());
    let body = Rect::new(
        area.x,
        body_y,
        area.width,
        area.bottom().saturating_sub(body_y),
    );
    app.terminal_view.body_rect = Some(body);
    app.transcript_hit = Some(body);

    if body.is_empty() {
        return;
    }

    let size = (body.height.max(1), body.width.max(1));
    // Only push PTY resizes while the human owns the session. Resizing an
    // agent-controlled PTY to the viewer's body makes agent reads nondeterministic.
    let user_owns_pty = app.terminal_view.phase == TerminalViewPhase::UserControl
        && app.terminal_card(&id).is_some_and(|card| {
            matches!(card.process, hive_core::TerminalProcessState::Running)
                && card.controller == hive_core::TerminalController::User
        });
    if user_owns_pty && app.terminal_view.last_size != Some(size) {
        app.terminal_view.last_size = Some(size);
        app.pending_terminal_resize = Some((id.clone(), size.0, size.1));
    }
    let scrollback = app.terminal_view.scrollback;
    let phase = app.terminal_view.phase;
    let default_style = Style::default().fg(app.theme.fg);
    let mut cursor = None;

    if let Some(card) = app.blocks.iter_mut().find_map(|block| match block {
        Block::Terminal(card) if card.id == id => Some(card.as_mut()),
        _ => None,
    }) {
        if card.screen.size() != size {
            card.screen.set_size(size.0, size.1);
        }
        card.screen.set_scrollback(scrollback);
        let screen = &card.screen;

        frame.buffer().paint(body, default_style);
        for row in 0..body.height {
            for col in 0..body.width {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let style = default_style.patch(cell_style(cell));
                let x = body.x + col;
                let y = body.y + row;
                if cell.has_contents() {
                    frame.buffer().set_str(x, y, cell.contents(), style);
                } else {
                    frame.buffer().set(x, y, ' ', style);
                }
            }
        }

        if phase == TerminalViewPhase::UserControl && scrollback == 0 && !screen.hide_cursor() {
            let (row, col) = screen.cursor_position();
            if row < body.height && col < body.width {
                cursor = Some((body.x + col, body.y + row));
            }
        }
    }

    if let Some((x, y)) = cursor {
        frame.set_cursor(x, y);
    }
}

fn vt_color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(xterm_index(index)),
        vt100::Color::Rgb(red, green, blue) => Some(Color::Rgb(red, green, blue)),
    }
}

fn xterm_index(index: u8) -> Color {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => {
            let (red, green, blue) = ANSI[usize::from(index)];
            Color::Rgb(red, green, blue)
        }
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let cube = index - 16;
            let red = LEVELS[usize::from(cube / 36)];
            let green = LEVELS[usize::from((cube % 36) / 6)];
            let blue = LEVELS[usize::from(cube % 6)];
            Color::Rgb(red, green, blue)
        }
        232..=255 => {
            let gray = 8 + 10 * (index - 232);
            Color::Rgb(gray, gray, gray)
        }
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style {
        fg: vt_color(cell.fgcolor()),
        bg: vt_color(cell.bgcolor()),
        ..Style::default()
    };
    if cell.bold() {
        style.mods |= Modifier::BOLD;
    }
    if cell.dim() {
        style.mods |= Modifier::DIM;
    }
    if cell.italic() {
        style.mods |= Modifier::ITALIC;
    }
    if cell.underline() {
        style.mods |= Modifier::UNDERLINE;
    }
    if cell.inverse() {
        style.mods |= Modifier::REVERSE;
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;
    use comb::core::buffer::WIDE_CONT;
    use comb::{render_with_cursor, Color, Modifier, Size};
    use hive_core::event::AgentEvent;
    use hive_core::{TerminalController, TerminalProcessState};

    use crate::app::state::TerminalViewPhase;
    use crate::{TuiInit, UiConfig};

    fn app() -> crate::app::App {
        crate::app::App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: Vec::new(),
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "test".into(),
            ui: UiConfig::default(),
            context_window: 128_000,
        })
    }

    fn start_terminal(app: &mut crate::app::App) {
        app.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "theme-installer".into(),
            rows: 20,
            cols: 80,
        });
    }

    #[test]
    fn terminal_card_opens_plan_style_view_without_sidebar() {
        let mut terminal = app();
        start_terminal(&mut terminal);
        terminal.open_terminal_view("term-1".into());
        let size = Size::new(100, 30);
        let (buffer, _) =
            render_with_cursor(size, |frame| crate::render::draw(frame, &mut terminal));
        let text = buffer.text();

        assert!(text.contains("Terminal · theme-installer"), "{text}");
        assert!(!text.contains("direct input is private"), "{text}");
        assert!(text.contains("← back"), "{text}");
        assert!(text.contains("AGENT"), "{text}");
        assert!(text.contains("STOP"), "{text}");
        assert!(terminal.sidebar_toggle_hit.is_none());
        assert!(terminal.sidebar_resize_hit.is_none());

        let terminal_body = terminal.terminal_view.body_rect.unwrap();
        let terminal_bar = terminal.back_hit.unwrap();

        let mut plan = app();
        plan.apply(AgentEvent::PlanUpdated {
            summary: "Plan".into(),
            body: "# Plan\n\nDo it".into(),
        });
        plan.open_plan_view();
        let _ = render_with_cursor(size, |frame| crate::render::draw(frame, &mut plan));
        let plan_body = plan.transcript_hit.unwrap();
        let plan_bar = plan.back_hit.unwrap();
        assert_eq!(
            (terminal_body.x, terminal_body.width),
            (plan_body.x, plan_body.width)
        );
        assert_eq!(terminal_bar, plan_bar);
    }

    #[test]
    fn terminal_view_maps_ansi_rgb_indexed_and_modifiers() {
        assert_eq!(vt_color(vt100::Color::Default), None);
        assert_eq!(
            vt_color(vt100::Color::Rgb(1, 2, 3)),
            Some(Color::Rgb(1, 2, 3))
        );
        assert_eq!(vt_color(vt100::Color::Idx(16)), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(
            vt_color(vt100::Color::Idx(231)),
            Some(Color::Rgb(255, 255, 255))
        );
        assert_eq!(vt_color(vt100::Color::Idx(232)), Some(Color::Rgb(8, 8, 8)));

        let mut parser = vt100::Parser::new(2, 20, 0);
        parser.process(b"\x1b[1;3;4;7;38;2;9;8;7;48;5;17mX");
        let style = cell_style(parser.screen().cell(0, 0).unwrap());
        assert_eq!(style.fg, Some(Color::Rgb(9, 8, 7)));
        assert_eq!(style.bg, Some(xterm_index(17)));
        for modifier in [
            Modifier::BOLD,
            Modifier::ITALIC,
            Modifier::UNDERLINE,
            Modifier::REVERSE,
        ] {
            assert!(style.mods.contains(modifier), "{modifier:?}");
        }
        let mut dim = vt100::Parser::new(1, 2, 0);
        dim.process(b"\x1b[2mX");
        assert!(cell_style(dim.screen().cell(0, 0).unwrap())
            .mods
            .contains(Modifier::DIM));
    }

    #[test]
    fn terminal_view_renders_wide_glyph_and_cursor() {
        let mut app = app();
        start_terminal(&mut app);
        app.apply(AgentEvent::TerminalOutput {
            id: "term-1".into(),
            frame: hive_core::TerminalOutputFrame::from_bytes(
                20,
                80,
                10_000,
                "界".as_bytes(),
                1,
            ),
        });
        app.apply(AgentEvent::TerminalState {
            id: "term-1".into(),
            controller: TerminalController::User,
            process: TerminalProcessState::Running,
            revision: 2,
        });
        app.open_terminal_view("term-1".into());

        let (buffer, cursor) = render_with_cursor(Size::new(80, 24), |frame| {
            crate::render::draw(frame, &mut app)
        });
        let body = app.terminal_view.body_rect.unwrap();
        assert_eq!(buffer.get(body.x, body.y).map(|cell| cell.ch), Some('界'));
        assert_eq!(
            buffer.get(body.x + 1, body.y).map(|cell| cell.ch),
            Some(WIDE_CONT)
        );
        assert_eq!(cursor, Some((body.x + 2, body.y)));
    }

    #[test]
    fn exited_terminal_view_is_read_only_without_stop() {
        let mut app = app();
        start_terminal(&mut app);
        app.apply(AgentEvent::TerminalState {
            id: "term-1".into(),
            controller: TerminalController::Agent,
            process: TerminalProcessState::Exited { code: 7 },
            revision: 1,
        });
        app.open_terminal_view("term-1".into());

        let (buffer, cursor) = render_with_cursor(Size::new(80, 24), |frame| {
            crate::render::draw(frame, &mut app)
        });
        assert_eq!(app.terminal_view.phase, TerminalViewPhase::ReadOnly);
        assert!(buffer.text().contains("exited 7"));
        assert!(!buffer.text().contains("STOP"));
        assert!(app.terminal_stop_hit.is_none());
        assert!(app.take_pending_terminal_resize().is_none());
        assert!(cursor.is_none());
    }
}
