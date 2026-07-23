//! "Switched to … Mode" card — soft strip band with an icon title + reason.
//! Emitted when the agent moves itself into another mode (e.g. PLAN for a big
//! feature) so the user sees why the mode changed.

use comb::{Color, Line, Modifier, Span, Style};

use crate::app::state::ModeSwitchCard;
use crate::app::App;
use crate::render::tools::strip::{soft_bg_line, soft_bg_pad};
use crate::render::wrap::wrap_lines;

pub(crate) fn mode_switch_card_lines(card: &ModeSwitchCard, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let bg: Color = theme.strip;

    let title = format!("Switched to {} Mode", card.mode.title());
    let title_line = soft_bg_line(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(title, Style::default().fg(theme.fg).add(Modifier::BOLD)),
        ]),
        bg,
        width,
    );

    let mut out = vec![soft_bg_pad(bg, width), title_line];

    let reason = card.reason.trim();
    if !reason.is_empty() {
        let content_w = width.saturating_sub(2).max(1);
        let raw: Vec<Line> = reason
            .split('\n')
            .map(|l| {
                Line::from(Span::styled(
                    l.trim().to_string(),
                    Style::default().fg(theme.dim),
                ))
            })
            .collect();
        for mut l in wrap_lines(raw, content_w) {
            l.spans.insert(0, Span::raw("  "));
            out.push(soft_bg_line(l, bg, width));
        }
    }

    out.push(soft_bg_pad(bg, width));
    out
}
