use comb::{Color, Line, Modifier, Span, Style};

use hive_core::TerminalProcessState;

use crate::app::state::TerminalCard;
use crate::app::App;

/// Compact transcript row — same visual language as Thought headers.
pub(crate) fn terminal_card_lines(
    card: &TerminalCard,
    app: &App,
    width: usize,
    show_hint: bool,
    hovered: bool,
) -> Vec<Line> {
    let theme = &app.theme;
    let running = matches!(card.process, TerminalProcessState::Running);
    let hint_fg = if hovered { theme.dim } else { theme.faint };

    let mut spans: Vec<Span> = vec![Span::raw("  ")];
    if running {
        spans.extend(shimmer_bright("Terminal", app.spinner));
        spans.push(Span::styled(
            format!("  {:.0}s", card.secs()),
            Style::default().fg(theme.dim),
        ));
    } else {
        spans.push(Span::styled(
            format!("Terminal for {:.1}s", card.secs()),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        ));
        let failed = matches!(
            &card.process,
            TerminalProcessState::Exited { code } if *code != 0
        ) || matches!(&card.process, TerminalProcessState::Failed { .. });
        if failed {
            spans.push(Span::styled(
                format!(" · {}", truncate(&card.status_text(), 28)),
                Style::default().fg(theme.err),
            ));
        }
    }

    let detail = {
        let command = card.command.trim();
        if !command.is_empty() {
            command.to_string()
        } else {
            card.preview()
        }
    };
    if !detail.is_empty() {
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let budget = width
            .saturating_sub(used)
            .saturating_sub(if show_hint { 16 } else { 4 });
        if budget > 3 {
            spans.push(Span::styled(
                format!(" · {}", truncate(&detail, budget.saturating_sub(3))),
                Style::default().fg(theme.dim),
            ));
        }
    }

    if show_hint {
        spans.push(Span::styled(
            "  click to open",
            Style::default().fg(hint_fg),
        ));
    }

    vec![Line::from(spans)]
}

fn shimmer_bright(text: &str, tick: usize) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as i32;
    let head = (tick as i32 % (n + 6)) - 3;
    chars
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let dist = (i as i32 - head).abs() as f32;
            let t = (1.0 - dist / 3.0).max(0.0);
            let v = (0xb8 as f32 + (0xff - 0xb8) as f32 * t) as u8;
            Span::styled(
                c.to_string(),
                Style::default().fg(Color::Rgb(v, v, v)).add(Modifier::BOLD),
            )
        })
        .collect()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max <= 1 {
        return "…".into();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TuiInit, UiConfig};
    use hive_core::{TerminalController, TerminalProcessState};

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

    fn card(controller: TerminalController, process: TerminalProcessState) -> TerminalCard {
        let mut parser = vt100::Parser::new(4, 40, 100);
        parser.process(b"Choose preset:\r\n> graphite");
        TerminalCard {
            id: "term-1".into(),
            command: "theme-installer".into(),
            controller,
            process,
            revision: 2,
            screen: parser.screen().clone(),
            started: std::time::Instant::now(),
            elapsed_ms: Some(250),
        }
    }

    fn text(card: &TerminalCard) -> String {
        terminal_card_lines(card, &app(), 80, true, false)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_str())
            .collect()
    }

    #[test]
    fn terminal_card_renders_like_a_thought_row() {
        let running = text(&card(
            TerminalController::Agent,
            TerminalProcessState::Running,
        ));
        assert!(running.contains("Terminal"), "{running}");
        assert!(running.contains("theme-installer"), "{running}");
        assert!(running.contains("click to open"), "{running}");
        assert!(!running.contains("Choose preset"), "{running}");
        assert!(!running.contains("AGENT"), "{running}");

        let exited = text(&card(
            TerminalController::Agent,
            TerminalProcessState::Exited { code: 0 },
        ));
        assert!(exited.contains("Terminal for"), "{exited}");
        assert!(exited.contains("theme-installer"), "{exited}");
        assert!(!exited.contains("exited 0"), "{exited}");

        let failed = text(&card(
            TerminalController::Agent,
            TerminalProcessState::Failed {
                message: "boom".into(),
            },
        ));
        assert!(failed.contains("Terminal for"), "{failed}");
        assert!(failed.contains("boom"), "{failed}");
    }
}
