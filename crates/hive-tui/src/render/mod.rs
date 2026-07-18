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

use comb::{Frame, Line, Rect, Span, Style};

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
fn draw_landing(f: &mut Frame, area: Rect, app: &mut App) {
    // A narrower, centered input — clamped so it looks good on any width.
    let inner_w = (area.width * 3 / 5)
        .clamp(36, 72)
        .min(area.width.saturating_sub(4));
    let ix = area.x + (area.width - inner_w) / 2;

    // Full input band (top pad + text + bottom pad). NEVER strip the band's
    // padding rows to remove space — trim the layout around it instead.
    let input_h = input_height(app, inner_w);
    let menu_h = menu::height(app);
    let theme = &app.theme;
    let gap: u16 = 1;

    // The menu is an overlay (see below), so it's NOT part of the centered
    // group — opening it doesn't move the wordmark or input.
    let group_h = mascot::HEIGHT + gap + 1 /*version*/ + gap + input_h + 2 /*status*/;
    let mut y = area.y + area.height.saturating_sub(group_h) / 2;

    // ASCII "HIVE", centered as one block so the letters stay aligned.
    let art_x = area.x + area.width.saturating_sub(mascot::WIDTH) / 2;
    f.buffer().set_lines(
        Rect::new(art_x, y, mascot::WIDTH, mascot::HEIGHT),
        &mascot::wordmark(theme),
        0,
    );
    y += mascot::HEIGHT + gap;

    // Version, centered under the wordmark.
    let version = Line::from(Span::styled(
        format!("v{}", app.version),
        Style::default().fg(theme.faint),
    ));
    let vw = version.width() as u16;
    let vx = area.x + area.width.saturating_sub(vw) / 2;
    f.buffer().set_line(vx, y, &version, vw);
    y += 1 + gap;

    // Centered, narrower input strip.
    let input_top = y;
    input_box::draw(f, Rect::new(ix, y, inner_w, input_h), app);
    y += input_h;

    // Model + cwd: left-aligned right under the input band, no empty gap.
    // A live flash (e.g. the ctrl+c confirmation) rides on the model row,
    // right-aligned, so the orange message shows on the right of the status.
    // Width matches the input's inner text span, so the left model text and the
    // right-aligned flash line up with the input band's edges (not the screen's).
    let sx = ix + 1;
    let sw = inner_w.saturating_sub(2);
    f.buffer().set_line(sx, y, &footer::model_line(app), sw);
    if let Some(msg) = app.flash_text() {
        let flash = Line::from(Span::styled(msg.to_string(), Style::default().fg(theme.warn)));
        let fw = flash.width() as u16;
        f.buffer().set_line(sx + sw.saturating_sub(fw), y, &flash, fw);
    }
    f.buffer().set_line(sx, y + 1, &footer::cwd_line(app), sw);

    // Slash menu overlay: floats above the input, layering over the space under
    // the wordmark without shifting anything.
    if menu_h > 0 {
        let top = input_top.saturating_sub(menu_h).max(area.y);
        menu::draw(
            f.buffer(),
            Rect::new(ix, top, inner_w, input_top - top),
            app,
        );
    }
}

/// The normal, bottom-anchored conversation layout.
fn draw_active(f: &mut Frame, area: Rect, app: &mut App) {
    let swarm_h = swarm::height(app);
    // blank + shimmering spinner + blank while running; nothing when idle
    let working_h: u16 = if app.running { 3 } else { 0 };
    let menu_h = menu::height(app);

    // Everything — transcript included — lives in a band a touch narrower than
    // the terminal, centered, so the chat doesn't sprawl across the screen.
    let inner_w = area.width.saturating_sub(6).max(20).min(area.width);
    let input_h = input_height(app, inner_w);
    let ix = area.x + (area.width - inner_w) / 2;

    // Manual vertical layout, bottom-anchored: the footer sits on the last two
    // rows, then the input, working, and swarm blocks stack upward, and the
    // transcript fills whatever is left. The slash menu is NOT part of this — it
    // floats above the input as an overlay, so opening it never shifts anything.
    let footer_y = area.bottom().saturating_sub(2);
    let input_y = footer_y.saturating_sub(input_h);
    let working_y = input_y.saturating_sub(working_h);
    let swarm_y = working_y.saturating_sub(swarm_h);
    let transcript = Rect::new(area.x, area.y, area.width, swarm_y.saturating_sub(area.y));
    let band = |y: u16, h: u16| Rect::new(ix, y, inner_w, h);
    let band_inner = |y: u16, h: u16| Rect::new(ix + 1, y, inner_w.saturating_sub(2), h);

    transcript::draw(f.buffer(), band(transcript.y, transcript.height), app);
    if swarm_h > 0 {
        swarm::draw(f.buffer(), band_inner(swarm_y, swarm_h), app);
    }
    if app.running {
        transcript::draw_working(f.buffer(), band_inner(working_y, working_h), app);
    }
    input_box::draw(f, band(input_y, input_h), app);
    footer::draw(f.buffer(), band_inner(footer_y, 2), app);

    // Menu overlay, drawn last so it layers over the transcript, its bottom
    // edge flush with the input's top. If it can't fit it clips at the top.
    if menu_h > 0 {
        let top = input_y.saturating_sub(menu_h).max(area.y);
        menu::draw(f.buffer(), Rect::new(ix, top, inner_w, input_y - top), app);
    }
}

fn input_height(app: &mut App, band_width: u16) -> u16 {
    // Persist wrap width so Up/Down between frames use the same soft-wrap.
    app.input.text_cols = input_box::text_cols(band_width);
    // text rows + one padding row above and below, inside the strip
    app.input.visible_line_count(app.input.text_cols) as u16 + 2
}
