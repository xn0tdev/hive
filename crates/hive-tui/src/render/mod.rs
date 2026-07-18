//! Rendering: turning `App` state into a frame.
//!
//! Modes:
//! - Empty chat → a centered landing: an ASCII "HIVE" wordmark + a narrower,
//!   centered input, all vertically centered, with the model/cwd status
//!   left-aligned right under the input.
//! - Active chat → the flowing bottom-anchored layout (transcript with inline
//!   subagent cards, input strip, slash menu, footer; working cubes by the mode chip).
//! - Subagent view → same layout, but the transcript shows that agent's thread
//!   and the input strip is replaced by a clickable `← back` button (no caret).
//! - Plan view → markdown preview of Plan.md; bottom bar is back/Build or amend.

pub mod bee;
pub mod markdown;
pub mod spinner;
pub mod tools;
pub mod wordmark;
pub mod wrap;

mod about;
mod footer;
mod input_box;
mod menu;
mod palette;
mod sidebar;
mod strip_paint;
mod toast;
mod transcript;

pub use sidebar::ProjectSnapshot;

use comb::{Frame, Line, Rect, Span, Style};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    // Full-frame clear first: landing ↔ active, sidebar open/close, and
    // resize all shift rects; without this, vacated columns/rows keep ghosts.
    f.buffer().paint(area, Style::default());
    if app.is_empty_chat() {
        app.click_hits.clear();
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
    let gap: u16 = 1;
    // Copy colors before `input_box::draw` needs `&mut App` for hit-testing.
    let faint = app.theme.faint;

    // The menu is an overlay (see below), so it's NOT part of the centered
    // group — opening it doesn't move the wordmark or input.
    let group_h = wordmark::HEIGHT + gap + 1 /*version*/ + gap + input_h + 2 /*status*/;
    let mut y = area.y + area.height.saturating_sub(group_h) / 2;

    // Block "HIVE", centered as one block so the letters stay aligned.
    let art_x = area.x + area.width.saturating_sub(wordmark::WIDTH) / 2;
    let logo = Rect::new(art_x, y, wordmark::WIDTH, wordmark::HEIGHT);
    app.logo_hit = Some(logo);
    let bonk = app.logo_bonk.clone();
    f.buffer()
        .set_lines(logo, &wordmark::lines(&app.theme, bonk.as_ref()), 0);
    y += wordmark::HEIGHT + gap;

    // Version, centered under the wordmark.
    let version = Line::from(Span::styled(
        format!("v{}", app.version),
        Style::default().fg(faint),
    ));
    let vw = version.width() as u16;
    let vx = area.x + area.width.saturating_sub(vw) / 2;
    f.buffer().set_line(vx, y, &version, vw);
    y += 1 + gap;

    // Centered, narrower input strip.
    let input_top = y;
    input_box::draw(f, Rect::new(ix, y, inner_w, input_h), app);
    y += input_h;

    // Model + cwd flush with the input strip's outer left; chip on the right.
    footer::draw_with_mode(f, Rect::new(ix, y, inner_w, 2), app);

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

    // Toasts sit centered on the bottom edge of the screen (not on the model row).
    toast::draw(f, area, app);

    draw_palette(f, area, app);
    draw_about(f, area, app);
}

/// The normal, bottom-anchored conversation layout.
fn draw_active(f: &mut Frame, area: Rect, app: &mut App) {
    let special = app.in_special_view();
    let plan = app.in_plan_view();
    let menu_h = if special { 0 } else { menu::height(app) };

    app.refresh_project();

    // On wide screens, reserve a right project sidebar; chat stays in a left band.
    let side_w = sidebar::width_for(area.width, app.sidebar_open);
    let gap: u16 = if side_w > 0 { 3 } else { 0 };
    let chat_avail = area.width.saturating_sub(side_w).saturating_sub(gap);
    let inner_w = if side_w > 0 {
        chat_avail.saturating_sub(4).clamp(40, 88)
    } else {
        area.width.saturating_sub(6).max(20).min(area.width)
    };
    let input_h = if plan && app.plan_view.amending {
        input_height(app, inner_w)
    } else if special {
        3 // pad + ← back (+ Build) + pad
    } else {
        input_height(app, inner_w)
    };
    let ix = if side_w > 0 {
        area.x + 2
    } else {
        area.x + (area.width - inner_w) / 2
    };

    // Manual vertical layout, bottom-anchored: footer (model+cwd, chip on the
    // model row) on the last two rows, then input, transcript above.
    // Working cubes sit just left of the mode chip on the model row.
    let footer_y = area.bottom().saturating_sub(2);
    let input_y = footer_y.saturating_sub(input_h);
    let transcript_h = input_y.saturating_sub(area.y);
    let transcript = Rect::new(ix, area.y, inner_w, transcript_h);
    let band = |y: u16, h: u16| Rect::new(ix, y, inner_w, h);

    transcript::draw(f.buffer(), transcript, app);
    if plan {
        input_box::draw_plan_bar(f, band(input_y, input_h), app);
        footer::draw(f.buffer(), band(footer_y, 2), app);
    } else if app.in_subagent_view() {
        app.input_hit = None;
        input_box::draw_back(f, band(input_y, input_h), app);
        footer::draw(f.buffer(), band(footer_y, 2), app);
    } else {
        app.back_hit = None;
        app.build_hit = None;
        input_box::draw(f, band(input_y, input_h), app);
        // Full band width: text flush left with strip, chip flush right.
        footer::draw_with_mode(f, band(footer_y, 2), app);
    }

    if side_w > 0 {
        let sx = area.right().saturating_sub(side_w).saturating_sub(1);
        let side = Rect::new(sx, area.y + 1, side_w, area.height.saturating_sub(4));
        sidebar::draw(f.buffer(), side, app);
    } else if sidebar::available(area.width) {
        sidebar::draw_collapsed_toggle(f.buffer(), area, app);
    } else {
        app.sidebar_toggle_hit = None;
    }

    // Menu overlay, drawn last so it layers over the transcript, its bottom
    // edge flush with the input's top. If it can't fit it clips at the top.
    if menu_h > 0 {
        let top = input_y.saturating_sub(menu_h).max(area.y);
        menu::draw(f.buffer(), Rect::new(ix, top, inner_w, input_y - top), app);
    }

    // Centered toast on the bottom edge of the screen (same as landing).
    toast::draw(f, area, app);

    draw_palette(f, area, app);
    draw_about(f, area, app);
}

fn draw_palette(f: &mut Frame, area: Rect, app: &mut App) {
    if !app.palette_open() {
        app.palette_list_visible = 0;
        return;
    }
    app.palette_list_visible = palette::list_visible(area, app);
    palette::draw(f.buffer(), area, app);
    if let Some((x, y)) = palette::search_cursor(area, app) {
        f.set_cursor(x, y);
    }
}

fn draw_about(f: &mut Frame, area: Rect, app: &mut App) {
    if !app.about_open() {
        return;
    }
    about::draw(f.buffer(), area, app);
}

fn input_height(app: &mut App, band_width: u16) -> u16 {
    // Persist wrap width so Up/Down between frames use the same soft-wrap.
    app.input.text_cols = input_box::text_cols(band_width);
    // text rows + one padding row above and below, inside the strip
    let tag = u16::from(app.has_pending_attaches() && !app.input.is_empty());
    app.input.visible_line_count(app.input.text_cols) as u16 + 2 + tag
}
