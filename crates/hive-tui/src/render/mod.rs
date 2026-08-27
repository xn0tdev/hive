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
//! - Plan view → markdown preview of Plan.md; bottom bar is back/Make or amend.

pub mod markdown;
pub mod spinner;
pub mod tools;
pub mod wordmark;
pub mod wrap;

pub(crate) mod about;
pub mod bot;
mod context_menu;
mod footer;
pub(crate) mod goal;
mod input_bars;
mod jump_bottom;
mod menu;
pub(crate) mod palette;
mod panel;
pub(crate) mod recap;
pub(crate) mod settings;
pub(crate) mod sidebar;
pub(crate) mod strip_paint;
mod terminal_view;
mod toast;
mod transcript;
pub(crate) mod two_col;

pub use sidebar::{
    clamp_width, ProjectRefresh, ProjectSnapshot, SidebarItem, SidebarSection, SidebarSections,
};

use comb::{Frame, Line, Rect, Span, Style};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    // comb supplies a freshly cleared retained back-buffer for every frame.
    // Only the chat layout re-arms this; landing and the special views have no
    // scrollback of their own to jump to.
    app.scroll_bottom_hit = None;
    // Re-armed by `menu::draw` when a composer menu is actually on screen.
    app.menu_hit = None;
    // Every centered overlay lays out against the whole frame.
    app.overlay_area = area;
    if app.is_empty_chat() {
        app.click_hits.clear();
        // Landing has no scrollable transcript — keep scroll state honest so
        // blurred arrows don't accumulate a phantom offset.
        app.set_transcript_max_scroll(0);
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
    // Copy colors before `input_bars::draw` needs `&mut App` for hit-testing.
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
    input_bars::draw(f, Rect::new(ix, y, inner_w, input_h), app);
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

    // Landing: notification chip above the input, bottom-right of that band.
    toast::draw(
        f,
        Rect::new(
            area.x,
            area.y,
            area.width,
            input_top.saturating_sub(area.y).max(1),
        ),
        app,
    );

    draw_palette(f, area, app);
    draw_about(f, area, app);
    draw_settings(f, area, app);
    draw_goal(f, area, app);
}

/// The normal, bottom-anchored conversation layout.
fn draw_active(f: &mut Frame, area: Rect, app: &mut App) {
    let special = app.in_special_view();
    let plan = app.in_plan_view();
    let terminal = app.in_terminal_view();
    let menu_h = if special { 0 } else { menu::height(app) };

    // Plan preview: hide sidebar; keep the same left/right page padding as chat.
    let side_w = if plan || terminal {
        0
    } else {
        sidebar::width_for(
            area.width,
            app.sidebar_open,
            app.ui.sidebar_mode,
            app.ui.sidebar_width,
        )
    };
    let gap: u16 = if side_w > 0 { 2 } else { 0 };
    let chat_avail = area.width.saturating_sub(side_w).saturating_sub(gap);
    // No hard 88-col ceiling — use the band left of the sidebar (with modest pad),
    // then center the column so leftover space isn't a dead left margin.
    let outer_pad: u16 = if area.width <= 60 { 6 } else { 2 };
    let (inner_w, ix) = if plan || terminal {
        // Same outer pad as a no-sidebar chat column (not flush to screen edges).
        let w = area.width.saturating_sub(outer_pad).max(40).min(area.width);
        (w, area.x + (area.width - w) / 2)
    } else if side_w > 0 {
        let w = chat_avail.saturating_sub(4).max(40).min(chat_avail);
        (w, area.x + chat_avail.saturating_sub(w) / 2)
    } else {
        let w = area.width.saturating_sub(outer_pad).max(20).min(area.width);
        (w, area.x + (area.width - w) / 2)
    };
    // Plan composer matches chat textarea height; chip width is reserved so
    // wrap/height don't shift when MARK/SEND appears. Subagent back stays 3.
    let input_h = if plan {
        let composer_w = inner_w.saturating_sub(input_bars::PLAN_ACTION_COLS);
        input_height(app, composer_w.max(8))
    } else if special {
        3
    } else {
        input_height(app, inner_w)
    };

    // Manual vertical layout, bottom-anchored. Plan / terminal / subagent
    // hide the status footer (model/cwd) — only the special bar matters —
    // but keep the same 2-row bottom reserve as chat so that bar lands at
    // the exact height of the chat Input.
    let footer_h: u16 = 2;
    let footer_y = area.bottom().saturating_sub(footer_h);
    let follow_h: u16 = if !special && app.has_follow_up() {
        1
    } else {
        0
    };
    let input_y = footer_y.saturating_sub(input_h);
    let follow_y = input_y.saturating_sub(follow_h);
    // One blank row between the transcript and the input/follow-up band.
    let transcript_h = follow_y.saturating_sub(area.y).saturating_sub(1);
    let transcript = Rect::new(ix, area.y, inner_w, transcript_h);
    let band = |y: u16, h: u16| Rect::new(ix, y, inner_w, h);

    if terminal {
        terminal_view::draw(f, transcript, app);
    } else {
        transcript::draw(f.buffer(), transcript, app);
    }
    if plan {
        input_bars::draw_plan_bar(f, band(input_y, input_h), app);
    } else if terminal {
        input_bars::draw_terminal_bar(f, band(input_y, input_h), app);
    } else if app.in_subagent_view() {
        app.input_hit = None;
        input_bars::draw_back(f, band(input_y, input_h), app);
    } else {
        app.back_hit = None;
        app.make_hit = None;
        if follow_h > 0 {
            input_bars::draw_follow_up(f, band(follow_y, follow_h), app);
        }
        input_bars::draw(f, band(input_y, input_h), app);
        // Full band width: text flush left with strip, chip flush right.
        footer::draw_with_mode(f, band(footer_y, 2), app);
        // Gap row above the composer: jump-to-bottom. Tasks live in the sidebar.
        let gap_y = follow_y.saturating_sub(1);
        jump_bottom::draw(f.buffer(), band(gap_y, 1), app);
    }

    if plan || terminal {
        app.sidebar_toggle_hit = None;
        app.sidebar_resize_hit = None;
        app.sidebar_section_hits.clear();
        app.sidebar_item_hits.clear();
    } else if side_w > 0 {
        let sx = area.right().saturating_sub(side_w);
        let side = Rect::new(sx, area.y + 1, side_w, area.height.saturating_sub(4));
        sidebar::draw(f.buffer(), side, app);
    } else if sidebar::available(area.width) && app.ui.sidebar_mode == hive_core::SidebarMode::Auto
    {
        sidebar::draw_collapsed_toggle(f.buffer(), area, app);
    } else {
        app.sidebar_toggle_hit = None;
        app.sidebar_resize_hit = None;
        app.sidebar_section_hits.clear();
        app.sidebar_item_hits.clear();
    }

    // Menu overlay, drawn last so it layers over the transcript, its bottom
    // edge flush with the follow-up banner (or input) top.
    if menu_h > 0 {
        let menu_bottom = if follow_h > 0 { follow_y } else { input_y };
        let top = menu_bottom.saturating_sub(menu_h).max(area.y);
        menu::draw(
            f.buffer(),
            Rect::new(ix, top, inner_w, menu_bottom - top),
            app,
        );
    }

    // Bottom-right of the chat column, above the composer / back bar.
    let toast_w = area.width.saturating_sub(side_w);
    let toast_bottom = if follow_h > 0 { follow_y } else { input_y };
    toast::draw(
        f,
        Rect::new(
            area.x,
            area.y,
            toast_w.max(8),
            toast_bottom.saturating_sub(area.y).max(1),
        ),
        app,
    );

    draw_palette(f, area, app);
    draw_about(f, area, app);
    draw_settings(f, area, app);
    draw_goal(f, area, app);
    draw_recap(f, area, app);
    draw_context_menu(f, area, app);
}

fn draw_settings(f: &mut Frame, area: Rect, app: &App) {
    if app.settings_open() {
        settings::draw(f.buffer(), area, app);
    }
}

fn draw_goal(f: &mut Frame, area: Rect, app: &App) {
    if app.goal_overlay_open() {
        goal::draw(f.buffer(), area, app);
    }
}

fn draw_recap(f: &mut Frame, area: Rect, app: &mut App) {
    if app.recap_open() {
        recap::draw(f.buffer(), area, app);
    }
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

fn draw_context_menu(f: &mut Frame, area: Rect, app: &mut App) {
    if app.context_menu_open() {
        context_menu::draw(f.buffer(), area, app);
    }
}

fn input_height(app: &mut App, band_width: u16) -> u16 {
    // Persist wrap width so Up/Down between frames use the same soft-wrap.
    app.input.text_cols = input_bars::text_cols(band_width);
    // text rows + one padding row above and below, inside the strip
    let tag = u16::from(app.has_pending_attaches());
    app.input.visible_line_count(app.input.text_cols) as u16 + 2 + tag
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Block;
    use crate::TuiInit;
    use comb::{render, Size};

    fn chat_app() -> App {
        let mut a = App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: Vec::new(),
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
            ui: Default::default(),
            context_window: 128_000,
            cost_input: 0.0,
            cost_output: 0.0,
        });
        a.blocks.clear();
        for i in 0..40 {
            a.blocks.push(Block::User(format!("question {i}")));
            a.blocks.push(Block::Assistant {
                text: format!("answer {i}"),
                streaming: false,
            });
        }
        a
    }

    #[test]
    fn jump_chip_only_shows_when_scrolled_up() {
        let mut a = chat_app();
        let size = Size::new(80, 24);

        let buf = render(size, |f| draw(f, &mut a));
        assert!(!buf.text().contains('↓'), "at the bottom: no chip");
        assert!(a.scroll_bottom_hit.is_none());

        a.scroll_up(4);
        let buf = render(size, |f| draw(f, &mut a));
        assert!(buf.text().contains('↓'), "scrolled up: chip");
        let hit = a.scroll_bottom_hit.expect("chip");

        // The chip lives on the gap row, clear of the composer strip below it.
        let input = a.input_hit.expect("input strip");
        assert!(
            hit.y < input.y,
            "chip row {} must sit above the composer at {}",
            hit.y,
            input.y
        );
        assert_eq!(hit.y + 1, input.y, "chip belongs on the gap row");
    }

    #[test]
    fn tasks_do_not_replace_the_jump_chip() {
        let mut a = chat_app();
        a.todos = vec![hive_core::TodoItem {
            text: "do the thing".into(),
            done: false,
        }];
        let _ = render(Size::new(80, 24), |f| draw(f, &mut a));
        a.scroll_up(4);
        let buf = render(Size::new(80, 24), |f| draw(f, &mut a));
        assert!(buf.text().contains('↓'), "jump chip still shows with tasks");
        assert!(
            !buf.text().contains("→ Tasks"),
            "no task chrome above the composer: {}",
            buf.text()
        );
        let input = a.input_hit.expect("input strip");
        let hit = a.scroll_bottom_hit.expect("chip");
        assert_eq!(hit.y + 1, input.y);
    }

    #[test]
    fn jump_chip_click_returns_to_the_bottom() {
        let mut a = chat_app();
        // A first paint teaches the app how far the transcript can scroll;
        // `scroll_up` clamps to that, so scrolling before it is a no-op.
        let _ = render(Size::new(80, 24), |f| draw(f, &mut a));
        a.scroll_up(4);
        let _ = render(Size::new(80, 24), |f| draw(f, &mut a));

        let hit = a.scroll_bottom_hit.expect("chip");
        assert!(a.scroll_bottom_contains(hit.x, hit.y));
        assert!(!a.scroll_bottom_contains(hit.x, hit.y + 1), "composer row");

        a.scroll_to_bottom();
        let buf = render(Size::new(80, 24), |f| draw(f, &mut a));
        assert!(!buf.text().contains('↓'), "chip retires at the bottom");
    }

    #[test]
    fn landing_has_no_jump_chip() {
        let mut a = chat_app();
        a.blocks.clear();
        a.scroll_up(4);
        let buf = render(Size::new(80, 24), |f| draw(f, &mut a));
        assert!(!buf.text().contains('↓'));
        assert!(a.scroll_bottom_hit.is_none());
    }
}
