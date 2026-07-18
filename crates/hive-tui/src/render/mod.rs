//! Rendering: turning `App` state into a frame.
//!
//! Two modes:
//! - Empty chat → a centered landing: an ASCII "HIVE" wordmark + a narrower,
//!   centered input, all vertically centered, with the model/cwd status
//!   left-aligned right under the input.
//! - Active chat → the flowing bottom-anchored layout (transcript, swarm,
//!   working spinner, input strip, slash menu, footer).

pub mod markdown;
pub mod mascot;
pub mod spinner;
pub mod wrap;

mod footer;
mod input_box;
mod menu;
mod swarm;
mod transcript;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if app.is_empty_chat() {
        app.thought_hits.clear();
        draw_landing(f, area, app);
    } else {
        draw_active(f, area, app);
    }
}

/// Centered landing screen for a fresh, empty chat.
fn draw_landing(f: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;

    // A narrower, centered input — clamped so it looks good on any width.
    let inner_w = (area.width * 3 / 5).clamp(36, 72).min(area.width.saturating_sub(4));
    let ix = area.x + (area.width - inner_w) / 2;

    // Full input band (top pad + text + bottom pad). NEVER strip the band's
    // padding rows to remove space — trim the layout around it instead.
    let input_h = input_height(app);
    let menu_h = menu::height(app);
    let gap: u16 = 1;

    // The menu is an overlay (see below), so it's NOT part of the centered
    // group — opening it doesn't move the wordmark or input.
    let group_h = mascot::HEIGHT + gap + 1 /*version*/ + gap + input_h + 2 /*status*/;
    let mut y = area.y + area.height.saturating_sub(group_h) / 2;

    let full = |y: u16, h: u16| Rect {
        x: area.x,
        y,
        width: area.width,
        height: h,
    };

    // ASCII "HIVE", centered as one block so the letters stay aligned.
    let art_x = area.x + (area.width.saturating_sub(mascot::WIDTH)) / 2;
    f.render_widget(
        Paragraph::new(mascot::wordmark(theme)),
        Rect {
            x: art_x,
            y,
            width: mascot::WIDTH,
            height: mascot::HEIGHT,
        },
    );
    y += mascot::HEIGHT + gap;

    // Version, centered under the wordmark.
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("v{}", app.version),
            Style::default().fg(theme.faint),
        )))
        .alignment(Alignment::Center),
        full(y, 1),
    );
    y += 1 + gap;

    // Centered, narrower input strip.
    let input_top = y;
    input_box::draw(
        f,
        Rect {
            x: ix,
            y,
            width: inner_w,
            height: input_h,
        },
        app,
    );
    y += input_h;

    // Model + cwd: left-aligned right under the input band, no empty gap.
    // A live flash (e.g. the ctrl+c confirmation) rides on the model row,
    // right-aligned, so the orange message shows on the right of the status.
    // Width matches the input's inner text span, so the left model text and the
    // right-aligned flash line up with the input band's edges (not the screen's).
    let sx = ix + 1;
    let sw = inner_w.saturating_sub(2);
    let status = |y: u16| Rect { x: sx, y, width: sw, height: 1 };
    f.render_widget(Paragraph::new(footer::model_line(app)), status(y));
    if let Some(msg) = app.flash_text() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg.to_string(),
                Style::default().fg(theme.warn),
            )))
            .alignment(Alignment::Right),
            status(y),
        );
    }
    f.render_widget(Paragraph::new(footer::cwd_line(app)), status(y + 1));

    // Slash menu overlay: floats above the input, layering over the space under
    // the wordmark without shifting anything.
    if menu_h > 0 {
        let top = input_top.saturating_sub(menu_h).max(area.y);
        menu::draw(
            f,
            Rect {
                x: ix,
                y: top,
                width: inner_w,
                height: input_top - top,
            },
            app,
        );
    }
}

/// The normal, bottom-anchored conversation layout.
fn draw_active(f: &mut Frame, area: Rect, app: &mut App) {
    let swarm_h = swarm::height(app);
    // blank + shimmering spinner + blank while running; nothing when idle
    let working_h: u16 = if app.running { 3 } else { 0 };
    let input_h = input_height(app);
    let menu_h = menu::height(app);

    // The slash menu is NOT part of this layout — it floats above the input as
    // an overlay, so opening it never pushes the transcript up.
    let rows = Layout::vertical([
        Constraint::Min(1),            // transcript
        Constraint::Length(swarm_h),   // swarm (0 when idle)
        Constraint::Length(working_h), // spinner block (0 when idle)
        Constraint::Length(input_h),   // input strip
        Constraint::Length(2),         // footer: model row + cwd row
    ])
    .split(area);

    // Everything — transcript included — lives in a band a touch narrower than
    // the terminal, centered, so the chat doesn't sprawl across the screen.
    let inner_w = area.width.saturating_sub(6).max(20).min(area.width);
    let ix = area.x + (area.width - inner_w) / 2;
    let band = |r: Rect| Rect {
        x: ix,
        y: r.y,
        width: inner_w,
        height: r.height,
    };
    let band_inner = |r: Rect| Rect {
        x: ix + 1,
        y: r.y,
        width: inner_w.saturating_sub(2),
        height: r.height,
    };

    transcript::draw(f, band(rows[0]), app);
    if swarm_h > 0 {
        swarm::draw(f, band_inner(rows[1]), app);
    }
    if app.running {
        transcript::draw_working(f, band_inner(rows[2]), app);
    }
    input_box::draw(f, band(rows[3]), app);
    footer::draw(f, band_inner(rows[4]), app);

    // Menu overlay, drawn last so it layers over the transcript, its bottom
    // edge flush with the input's top. If it can't fit it clips at the top.
    if menu_h > 0 {
        let input_top = rows[3].y;
        let top = input_top.saturating_sub(menu_h).max(area.y);
        menu::draw(
            f,
            Rect {
                x: ix,
                y: top,
                width: inner_w,
                height: input_top - top,
            },
            app,
        );
    }
}

fn input_height(app: &App) -> u16 {
    // text rows + one padding row above and below, inside the strip
    let lines = app.input.line_count().clamp(1, 6) as u16;
    lines + 2
}
