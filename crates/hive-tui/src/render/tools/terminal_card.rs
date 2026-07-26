//! Terminal card in the main transcript — same soft `strip` band as the
//! subagent card: title + duration, description under, blank pad rows.

use comb::{Color, Line, Modifier, Span, Style};

use hive_core::TerminalProcessState;

use crate::app::state::TerminalCard;
use crate::app::App;
use crate::render::tools::strip::{soft_bg_line, soft_bg_pad};
use crate::render::tools::tool_card::format_tool_secs;

/// Flat terminal card in the main transcript: title + duration, description
/// and status under. Click navigates into the dedicated terminal view.
/// Soft `strip` band with blank pad rows top/bottom so text isn't flush.
pub(crate) fn terminal_card_lines(
    card: &TerminalCard,
    app: &App,
    width: usize,
    show_hint: bool,
    hovered: bool,
) -> Vec<Line> {
    let theme = &app.theme;
    let bg: Color = if hovered {
        theme.strip_hover
    } else {
        theme.strip
    };
    let running = matches!(card.process, TerminalProcessState::Running);
    let failed = matches!(
        &card.process,
        TerminalProcessState::Exited { code } if *code != 0
    ) || matches!(&card.process, TerminalProcessState::Failed { .. });

    let (icon, icon_fg) = if running {
        (app.spinner_char().to_string(), theme.accent)
    } else if failed {
        ("✗".to_string(), theme.err)
    } else {
        ("✓".to_string(), theme.ok)
    };

    let title = {
        let command = card.command.trim();
        if command.is_empty() {
            "Terminal".to_string()
        } else {
            command.to_string()
        }
    };

    let mut title_spans = vec![
        Span::raw("  "),
        Span::styled(format!("{icon} "), Style::default().fg(icon_fg)),
        Span::styled(title, Style::default().fg(theme.fg).add(Modifier::BOLD)),
        Span::styled(
            format!(" · {}", format_tool_secs(card.secs())),
            Style::default().fg(theme.dim),
        ),
    ];
    if show_hint {
        title_spans.push(Span::styled(
            "  click to open",
            Style::default().fg(if hovered { theme.dim } else { theme.faint }),
        ));
    }

    let title_line = soft_bg_line(Line::from(title_spans), bg, width);

    // Description line: why the agent started this terminal + process state.
    let desc = card.description.trim();
    let status = card.status_text();
    let status_fg = if failed { theme.err } else { theme.dim };
    let detail_line = if desc.is_empty() {
        soft_bg_line(
            Line::from(vec![
                Span::raw("    "),
                Span::styled(status, Style::default().fg(status_fg)),
            ]),
            bg,
            width,
        )
    } else {
        let prefix = format!("    {desc} · ");
        let budget = width.saturating_sub(prefix.chars().count() + status.chars().count());
        let desc_text = if desc.chars().count() > budget && budget > 3 {
            let mut s: String = desc.chars().take(budget - 3).collect();
            s.push('…');
            s
        } else {
            desc.to_string()
        };
        let full = format!("    {desc_text} · {status}");
        soft_bg_line(
            Line::from(vec![Span::styled(full, Style::default().fg(theme.dim))]),
            bg,
            width,
        )
    };

    vec![
        soft_bg_pad(bg, width),
        title_line,
        detail_line,
        soft_bg_pad(bg, width),
    ]
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
            description: "Install a color theme".into(),
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
    fn terminal_card_shows_description_and_status() {
        let running = text(&card(
            TerminalController::Agent,
            TerminalProcessState::Running,
        ));
        assert!(running.contains("theme-installer"), "{running}");
        assert!(running.contains("Install a color theme"), "{running}");
        assert!(running.contains("running"), "{running}");
        assert!(running.contains("click to open"), "{running}");
        assert!(!running.contains("AGENT"), "{running}");
        assert!(!running.contains("Choose preset"), "{running}");

        let exited = text(&card(
            TerminalController::Agent,
            TerminalProcessState::Exited { code: 0 },
        ));
        assert!(exited.contains("theme-installer"), "{exited}");
        assert!(exited.contains("Install a color theme"), "{exited}");
        assert!(exited.contains("exited 0"), "{exited}");

        let failed = text(&card(
            TerminalController::Agent,
            TerminalProcessState::Failed {
                message: "boom".into(),
            },
        ));
        assert!(failed.contains("theme-installer"), "{failed}");
        assert!(failed.contains("Install a color theme"), "{failed}");
        assert!(failed.contains("failed: boom"), "{failed}");
    }
}
