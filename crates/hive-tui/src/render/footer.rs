//! Footer content: `model · tokens` and the working directory. Exposed as
//! reusable lines so both the bottom bar and the centered landing can use them.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// `model · tokens · attachments` as a single line.
pub fn model_line(app: &crate::app::App) -> Line<'static> {
    let theme = &app.theme;
    let mut spans = vec![Span::styled(
        short_model(&app.model),
        Style::default().fg(theme.dim),
    )];
    if app.usage.total_tokens > 0 {
        spans.push(Span::styled(" · ", Style::default().fg(theme.faint)));
        spans.push(Span::styled(
            format_tokens(app.usage.total_tokens),
            Style::default().fg(theme.faint),
        ));
    }
    if !app.pending_images.is_empty() {
        spans.push(Span::styled(" · ", Style::default().fg(theme.faint)));
        spans.push(Span::styled(
            format!("{} image(s) attached", app.pending_images.len()),
            Style::default().fg(theme.warn),
        ));
    }
    Line::from(spans)
}

pub fn cwd_line(app: &crate::app::App) -> Line<'static> {
    Line::from(Span::styled(
        tilde(&app.cwd),
        Style::default().fg(app.theme.faint),
    ))
}

pub fn draw(f: &mut Frame, area: Rect, app: &crate::app::App) {
    let theme = &app.theme;
    if area.height < 2 {
        return;
    }

    let row1 = Rect { y: area.y, height: 1, ..area };
    let row2 = Rect { y: area.y + 1, height: 1, ..area };

    f.render_widget(Paragraph::new(model_line(app)), row1);

    // Ephemeral status (copy feedback, quit confirmation), right-aligned.
    if let Some(msg) = app.flash_text() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg.to_string(),
                Style::default().fg(theme.warn),
            )))
            .alignment(ratatui::layout::Alignment::Right),
            row1,
        );
    }

    f.render_widget(Paragraph::new(cwd_line(app)), row2);
}

fn short_model(model: &str) -> String {
    model.rsplit('/').next().unwrap_or(model).to_string()
}

fn format_tokens(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k tokens", n as f64 / 1000.0)
    } else {
        format!("{n} tokens")
    }
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}
