//! Footer content: `model · tokens` and the working directory. Exposed as
//! reusable lines so both the bottom bar and the centered landing can use them.

use comb::{Buffer, Line, Rect, Span, Style};

/// `model · tokens · attachments` as a single line.
pub fn model_line(app: &crate::app::App) -> Line {
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

pub fn cwd_line(app: &crate::app::App) -> Line {
    Line::from(Span::styled(
        tilde(&app.cwd),
        Style::default().fg(app.theme.faint),
    ))
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &crate::app::App) {
    let theme = &app.theme;
    if area.height < 2 {
        return;
    }

    buf.set_line(area.x, area.y, &model_line(app), area.width);

    // Ephemeral status (copy feedback, quit confirmation), right-aligned.
    if let Some(msg) = app.flash_text() {
        let line = Line::from(Span::styled(msg.to_string(), Style::default().fg(theme.warn)));
        let w = line.width() as u16;
        let x = area.x + area.width.saturating_sub(w);
        buf.set_line(x, area.y, &line, w);
    }

    buf.set_line(area.x, area.y + 1, &cwd_line(app), area.width);
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
