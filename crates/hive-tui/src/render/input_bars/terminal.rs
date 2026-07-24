use comb::{Frame, Line, Modifier, Rect, Span, Style};
use hive_core::{TerminalController, TerminalProcessState};

use crate::app::App;

use super::BACK;

const STOP_COLS: u16 = 8;

pub fn draw_terminal_bar(frame: &mut Frame, area: Rect, app: &mut App) {
    let status = app.viewed_terminal().map(|card| {
        let controller = match card.controller {
            TerminalController::Agent => "AGENT",
            TerminalController::User => "USER",
        };
        let process = match &card.process {
            TerminalProcessState::Running => "running".to_string(),
            TerminalProcessState::Exited { code } => format!("exited {code}"),
            TerminalProcessState::Failed { message } => format!("failed: {message}"),
        };
        (
            format!("Ctrl+] · {controller} · {process}"),
            matches!(card.process, TerminalProcessState::Running),
        )
    });
    let (status, running) = status.unwrap_or_else(|| ("Ctrl+] · unavailable".into(), false));
    let stop = running && area.width > STOP_COLS + BACK.chars().count() as u16 + 8;
    let stop_rect = stop.then(|| Rect {
        x: area.right().saturating_sub(STOP_COLS),
        y: area.y,
        width: STOP_COLS,
        height: area.height,
    });
    let left_width = stop_rect
        .map(|rect| rect.x.saturating_sub(area.x))
        .unwrap_or(area.width);
    let left = Rect::new(area.x, area.y, left_width, area.height);
    let bg = if app.hover_back {
        app.theme.strip_hover
    } else {
        app.theme.strip
    };
    frame.buffer().paint(left, Style::default().bg(bg));

    let y = area.y + area.height.saturating_sub(1) / 2;
    frame.buffer().set_line(
        left.x + 1,
        y,
        &Line::from(Span::styled(
            BACK,
            Style::default()
                .fg(app.theme.accent)
                .bg(bg)
                .add(Modifier::BOLD),
        )),
        left.width.saturating_sub(2),
    );

    let status_width = status.chars().count() as u16;
    let back_width = BACK.chars().count() as u16;
    if left.width > status_width + back_width + 4 {
        let x = left.right().saturating_sub(status_width + 1);
        frame.buffer().set_line(
            x,
            y,
            &Line::from(Span::styled(
                status,
                Style::default().fg(app.theme.dim).bg(bg),
            )),
            status_width,
        );
    }

    app.back_hit = Some(left);
    app.input_hit = None;
    app.make_hit = None;
    if let Some(stop_rect) = stop_rect {
        let stop_bg = if app.hover_terminal_stop {
            app.theme.del_fg
        } else {
            app.theme.err
        };
        frame.buffer().put_label(
            stop_rect,
            "STOP",
            Style::default()
                .fg(app.theme.sel_fg)
                .bg(stop_bg)
                .add(Modifier::BOLD),
        );
        app.terminal_stop_hit = Some(stop_rect);
    } else {
        app.terminal_stop_hit = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use comb::{render_with_cursor, Size};
    use hive_core::event::AgentEvent;
    use hive_core::{TerminalController, TerminalProcessState};

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
            cost_input: 0.0,
            cost_output: 0.0,
        })
    }

    #[test]
    fn terminal_bar_shows_back_controller_status_and_stop() {
        let mut app = app();
        app.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "installer".into(),
            rows: 20,
            cols: 80,
        });
        app.apply(AgentEvent::TerminalState {
            id: "term-1".into(),
            controller: TerminalController::User,
            process: TerminalProcessState::Running,
            revision: 1,
        });
        app.open_terminal_view("term-1".into());

        let (buffer, cursor) = render_with_cursor(Size::new(80, 3), |frame| {
            draw_terminal_bar(frame, frame.area(), &mut app);
        });
        let text = buffer.text();
        assert!(text.contains("← back"), "{text}");
        assert!(text.contains("Ctrl+]"), "{text}");
        assert!(text.contains("USER"), "{text}");
        assert!(text.contains("running"), "{text}");
        assert!(text.contains("STOP"), "{text}");
        assert!(app.back_hit.is_some());
        assert_eq!(app.terminal_stop_hit.map(|hit| hit.width), Some(8));
        assert!(cursor.is_none());
    }
}
