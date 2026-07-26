//! "Compacted" card — soft strip band showing context compaction.
//! Spinner while in progress, token reduction when done.

use comb::{Color, Line, Modifier, Span, Style};

use crate::app::state::CompactedCard;
use crate::app::App;
use crate::render::tools::strip::{soft_bg_line, soft_bg_pad};

fn short_tokens(n: u64) -> String {
    if n >= 1000 {
        format!("{:.0}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

pub(crate) fn compacted_card_lines(card: &CompactedCard, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let bg: Color = theme.strip;

    let (title_text, detail_spans) = match card.before {
        // In progress: spinner.
        None => (
            format!("{} Compacting context", app.spinner_char()),
            vec![Span::styled(
                "clearing conversation history…",
                Style::default().fg(theme.dim),
            )],
        ),
        // Done: show token reduction.
        Some(before) => {
            let title = "Compacted context".to_string();
            let detail = format!(
                "{} → {} tokens",
                short_tokens(before),
                short_tokens(card.after)
            );
            (
                title,
                vec![Span::styled(detail, Style::default().fg(theme.dim))],
            )
        }
    };

    let title_line = soft_bg_line(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                title_text,
                Style::default().fg(theme.fg).add(Modifier::BOLD),
            ),
        ]),
        bg,
        width,
    );

    let detail_line = soft_bg_line(
        Line::from({
            let mut v = vec![Span::raw("    ")];
            v.extend(detail_spans);
            v
        }),
        bg,
        width,
    );

    vec![
        soft_bg_pad(bg, width),
        title_line,
        detail_line,
        soft_bg_pad(bg, width),
    ]
}
