//! Terminal setup and the blocking event loop. Translates key events into
//! edits, scrolling, slash commands, and `InputCommand`s for the agent driver.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use comb::{Event, Key, KeyCode, Mouse, MouseButton, MouseKind, MouseMode, Terminal};
use tokio::sync::mpsc::UnboundedSender;

use hive_core::event::EventReceiver;
use hive_core::message::ImageSource;
use hive_core::{AgentMode, FollowUpSlot, UserInput};

use crate::app::palette::PaletteMode;
use crate::app::App;
use crate::commands::{self, CmdId};
use crate::render;
use crate::{InputCommand, TuiInit};

const UNKNOWN_HINT: &str = "Ctrl+P for commands · /about for About";

/// Poll interval while something is animating (spinner / shimmer).
const ANIM_TICK: Duration = Duration::from_millis(100);
/// Idle poll — long enough to skip needless redraws, short enough for input.
const IDLE_TICK: Duration = Duration::from_millis(250);

pub fn run(
    init: TuiInit,
    mut events: EventReceiver,
    input_tx: UnboundedSender<InputCommand>,
    interrupt: Arc<AtomicBool>,
    follow_up: FollowUpSlot,
) -> io::Result<()> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "hive must be run in an interactive terminal",
        ));
    }
    // comb enters raw mode + the alternate screen and turns on mouse reporting;
    // its `Drop` restores everything, so there's no manual teardown here.
    let mut terminal = Terminal::new()?;
    // Need button-drag reports so the sidebar left edge can be resized.
    terminal.mouse_mode(MouseMode::Drag)?;
    let mut app = App::new(init);

    run_loop(
        &mut terminal,
        &mut app,
        &mut events,
        &input_tx,
        &interrupt,
        &follow_up,
    )
}

fn run_loop(
    terminal: &mut Terminal,
    app: &mut App,
    events: &mut EventReceiver,
    input_tx: &UnboundedSender<InputCommand>,
    interrupt: &Arc<AtomicBool>,
    follow_up: &FollowUpSlot,
) -> io::Result<()> {
    let mut dirty = true;
    let mut last_spinner = usize::MAX;
    loop {
        let mut content_dirty = false;
        while let Ok(ev) = events.try_recv() {
            let finished = matches!(ev, hive_core::event::AgentEvent::TurnFinished);
            if app.apply(ev) {
                content_dirty = true;
            }
            if finished && flush_follow_up(app, input_tx) {
                content_dirty = true;
            }
        }

        if app.tick() {
            // Idle composer blur (or other tick-side visual change).
            dirty = true;
        }
        let animating = app.needs_animation();
        let spinner_moved = animating && app.spinner != last_spinner;

        if dirty || content_dirty || spinner_moved {
            terminal.draw(|f| render::draw(f, app))?;
            last_spinner = app.spinner;
            dirty = false;
        }

        let wait = if animating { ANIM_TICK } else { IDLE_TICK };
        if let Some(ev) = terminal.read_event(wait)? {
            match ev {
                Event::Key(key) => {
                    if handle_key(app, key, input_tx, interrupt, follow_up) {
                        break;
                    }
                    dirty = true;
                }
                Event::Mouse(m) => {
                    if handle_mouse(app, m, input_tx) {
                        dirty = true;
                    }
                }
                Event::Resize(_, _) => {
                    // Size is applied inside `Terminal::draw` (clear + full
                    // repaint). Flag dirty so idle sessions never skip it.
                    dirty = true;
                }
                Event::Paste(text) => {
                    if handle_paste(app, &text) {
                        dirty = true;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Bracketed paste / clipboard paste. Returns whether the UI should redraw.
fn handle_paste(app: &mut App, text: &str) -> bool {
    if app.about_open() || app.settings_open() {
        return false;
    }
    if app.palette_open() {
        return paste_into_palette(app, text);
    }
    if app.in_special_view() && !(app.in_plan_view() && app.plan_view.amending) {
        return false;
    }

    app.focus_input();

    let trimmed = text.trim();
    // Lone path paste → attach, same as submit-time path detect.
    if app.input.is_empty()
        && !trimmed.is_empty()
        && !trimmed.contains('\n')
        && !trimmed.contains(' ')
    {
        if let Some(leftover) = app.try_attach_pasted_path(trimmed) {
            if leftover.is_empty() {
                app.flash(format!("Attached {}", app.attachment_tags_line()));
                app.reset_menu();
                return true;
            }
        }
    }

    app.input.insert_str(text);
    app.reset_menu();
    true
}

fn sanitize_api_key(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn paste_into_palette(app: &mut App, text: &str) -> bool {
    let mode = app.palette.as_ref().map(|p| p.mode);
    match mode {
        Some(PaletteMode::ConnectPresets) => {
            let idx = app.palette.as_ref().and_then(|p| p.selected_preset_idx());
            let Some(idx) = idx else {
                return false;
            };
            let cleaned = sanitize_api_key(text);
            if cleaned.is_empty() {
                return false;
            }
            app.palette = Some(crate::app::palette::PaletteState::connect_key(idx));
            let choices = app.model_choices.clone();
            let connections = app.connections.clone();
            if let Some(pal) = app.palette.as_mut() {
                pal.insert_str(&cleaned, &choices, &connections);
            }
            true
        }
        Some(PaletteMode::ConnectKey { .. }) => {
            let cleaned = sanitize_api_key(text);
            if cleaned.is_empty() {
                return false;
            }
            let choices = app.model_choices.clone();
            let connections = app.connections.clone();
            if let Some(pal) = app.palette.as_mut() {
                pal.insert_str(&cleaned, &choices, &connections);
            }
            true
        }
        Some(_) => {
            if text.is_empty() {
                return false;
            }
            let choices = app.model_choices.clone();
            let connections = app.connections.clone();
            if let Some(pal) = app.palette.as_mut() {
                pal.insert_str(text, &choices, &connections);
            }
            true
        }
        None => false,
    }
}

fn read_clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Returns true when the user asked to quit.
fn handle_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
    interrupt: &Arc<AtomicBool>,
    follow_up: &FollowUpSlot,
) -> bool {
    let ctrl = key.mods.ctrl;
    let alt = key.mods.alt;
    let shift = key.mods.shift;

    let special = app.in_special_view();
    let slash_menu =
        !special && !app.palette_open() && !app.about_open() && !app.slash_items().is_empty();
    let file_menu = !special
        && !app.palette_open()
        && !app.about_open()
        && app.slash_items().is_empty()
        && app.at_mention().is_some()
        && !app.file_menu_items().is_empty();
    let menu_open = slash_menu || file_menu;

    // Any key other than ctrl+c cancels a pending quit confirmation.
    if !(ctrl && key.code == KeyCode::Char('c')) {
        app.disarm_quit();
    }

    // About overlay captures keys while open (esc / Ctrl+P).
    if app.about_open() {
        return handle_about_key(app, key);
    }

    if app.settings_open() {
        return handle_settings_key(app, key, input_tx);
    }

    // Plan preview: section select / amend / Build / back.
    if app.in_plan_view() {
        return handle_plan_key(app, key, input_tx, interrupt);
    }

    // Read-only subagent view: scroll + leave; no typing / submit.
    // Keys are handled at the app level — there is no focused text input.
    if app.in_subagent_view() {
        match key.code {
            KeyCode::Char('q') if ctrl => return true,
            KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
            KeyCode::Char('p') if ctrl => {
                app.open_palette();
            }
            KeyCode::Esc | KeyCode::Backspace => app.leave_subagent_view(),
            KeyCode::Left => app.leave_subagent_view(),
            KeyCode::PageUp => app.scroll_up(5),
            KeyCode::PageDown => app.scroll_down(5),
            KeyCode::Up => app.scroll_up(1),
            KeyCode::Down => app.scroll_down(1),
            _ => {}
        }
        return false;
    }

    // Command palette captures keys while open.
    if app.palette_open() {
        return handle_palette_key(app, key, input_tx);
    }

    // Global chords work whether or not the composer is focused.
    match key.code {
        KeyCode::Char('q') if ctrl => return true,
        KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
        KeyCode::Char('p') if ctrl => {
            app.open_palette();
            return false;
        }
        KeyCode::Char('y') if ctrl => {
            copy_last_answer(app);
            return false;
        }
        KeyCode::Char('t') if ctrl => {
            app.toggle_thoughts();
            return false;
        }
        KeyCode::PageUp => {
            app.scroll_up(5);
            return false;
        }
        KeyCode::PageDown => {
            app.scroll_down(5);
            return false;
        }
        _ => {}
    }

    // Blurred: Up/Down scroll the transcript when there is room; otherwise
    // (and for Left/Right/Home/End) focus the composer and fall through so
    // arrows stay useful after idle blur / click-away. Printable typing also
    // focuses and falls through. Subagent/plan views already returned above.
    if !app.input_focused {
        match key.code {
            KeyCode::Up if app.can_scroll_up() => {
                app.scroll_up(1);
                return false;
            }
            KeyCode::Down if app.can_scroll_down() => {
                app.scroll_down(1);
                return false;
            }
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End => {
                app.focus_input();
            }
            KeyCode::Esc if app.running => {
                // Same ladder as focused Esc: clear queue before interrupt.
                if app.clear_follow_up() {
                    return false;
                }
                interrupt.store(true, Ordering::Relaxed);
                return false;
            }
            KeyCode::Char(_) if !ctrl => {
                app.focus_input();
            }
            _ => return false,
        }
    }

    // Keys that edit or move within the composer refresh the idle-blur timer.
    // Pure transcript scroll (arrows at input edges, PageUp/Down above) does not.
    let mut composer_activity = false;
    match key.code {
        KeyCode::Tab if menu_open => {
            complete_selected(app);
            composer_activity = true;
        }
        KeyCode::Tab if !menu_open => {
            app.toggle_agent_mode();
        }
        KeyCode::Enter if file_menu => {
            complete_selected(app);
            composer_activity = true;
        }
        KeyCode::Enter if slash_menu => {
            // Commands with an argument get completed; the rest run at once.
            if let Some(item) = app.menu_selected() {
                if item.takes_arg() {
                    complete_selected(app);
                    composer_activity = true;
                } else {
                    app.input.take();
                    app.reset_menu();
                    return handle_slash(app, item.name(), input_tx);
                }
            }
        }
        KeyCode::Enter => {
            // Enter sends; Shift/Alt/Ctrl+Enter insert a newline.
            // (`\n` parses as Enter+SHIFT; kitty/xterm send CSI with mods.)
            if shift || alt || ctrl {
                app.input.newline();
                composer_activity = true;
            } else {
                return submit(app, input_tx, follow_up);
            }
        }
        // Ctrl+J → newline when the host remaps it to Char('j')+CTRL.
        KeyCode::Char('j') if ctrl => {
            app.input.newline();
            composer_activity = true;
        }
        // Ctrl+V / Insert → system clipboard (bracketed paste is Event::Paste).
        KeyCode::Char('v') | KeyCode::Char('V') if ctrl => {
            if let Some(text) = read_clipboard_text() {
                let _ = handle_paste(app, &text);
            }
            composer_activity = true;
        }
        KeyCode::Insert => {
            if let Some(text) = read_clipboard_text() {
                let _ = handle_paste(app, &text);
            }
            composer_activity = true;
        }
        KeyCode::Char(ch) if !ctrl => {
            app.input.insert(ch);
            app.reset_menu();
            composer_activity = true;
        }
        KeyCode::Backspace => {
            app.input.backspace();
            app.reset_menu();
            composer_activity = true;
        }
        KeyCode::Left => {
            app.input.left();
            composer_activity = true;
        }
        KeyCode::Right => {
            app.input.right();
            composer_activity = true;
        }
        KeyCode::Home => {
            app.input.home();
            composer_activity = true;
        }
        KeyCode::End => {
            app.input.end();
            composer_activity = true;
        }
        KeyCode::Up if menu_open => {
            app.menu_up();
            composer_activity = true;
        }
        KeyCode::Down if menu_open => {
            app.menu_down();
            composer_activity = true;
        }
        // Multiline: arrows move inside the input; at the edges they scroll chat.
        // Empty composer + queued follow-up: ↑ pulls it back for editing.
        KeyCode::Up => {
            if app.input.is_empty() && app.recall_follow_up() {
                composer_activity = true;
            } else if app.input.up() {
                composer_activity = true;
            } else {
                app.scroll_up(1);
            }
        }
        KeyCode::Down => {
            if app.input.down() {
                composer_activity = true;
            } else {
                app.scroll_down(1);
            }
        }
        KeyCode::Esc => {
            if !app.input.is_empty() {
                let _ = app.input.take();
                app.reset_menu();
                composer_activity = true;
            } else if app.clear_follow_up() {
                composer_activity = true;
            } else if app.running {
                // Stop the turn. The agent reports "Interrupted." in the chat
                // only once it has actually stopped.
                interrupt.store(true, Ordering::Relaxed);
            } else {
                app.blur_input();
            }
        }
        _ => {}
    }
    if composer_activity {
        app.note_input_activity();
    }
    false
}

/// Wheel scrolls the transcript; left-click toggles thoughts, opens subagent
/// chats, hits `← back` / Build, or bonks the logo.
///
/// Palette / About overlays are keyboard-only — mouse events are swallowed so
/// they don't leak through to the chat underneath.
/// Returns `true` when the UI should redraw.
fn handle_mouse(app: &mut App, m: Mouse, input_tx: &UnboundedSender<InputCommand>) -> bool {
    if app.about_open() || app.palette_open() || app.settings_open() {
        return false;
    }
    match m.kind {
        MouseKind::ScrollUp => {
            app.scroll_up(3);
            true
        }
        MouseKind::ScrollDown => {
            app.scroll_down(3);
            true
        }
        MouseKind::Down(MouseButton::Left) => {
            if app.is_empty_chat() && app.bonk_logo_at(m.col, m.row) {
                app.blur_input();
                return true;
            }
            if let Some(hit) = app.sidebar_resize_hit {
                if hit.contains(m.col, m.row) {
                    app.sidebar_resizing = true;
                    app.blur_input();
                    return true;
                }
            }
            if let Some(hit) = app.sidebar_toggle_hit {
                if hit.contains(m.col, m.row) {
                    app.toggle_sidebar();
                    app.blur_input();
                    return true;
                }
            }
            for (hit, section) in &app.sidebar_section_hits {
                if hit.contains(m.col, m.row) {
                    let section = *section;
                    app.toggle_sidebar_section(section);
                    app.blur_input();
                    return true;
                }
            }
            if app.in_plan_view() {
                if let Some(hit) = app.build_hit {
                    if hit.contains(m.col, m.row) {
                        start_build_from_plan(app, input_tx);
                        return true;
                    }
                }
                if let Some(hit) = app.back_hit {
                    if hit.contains(m.col, m.row) {
                        app.leave_special_view();
                        return true;
                    }
                }
                return false;
            }
            if app.in_subagent_view() {
                if let Some(hit) = app.back_hit {
                    if hit.contains(m.col, m.row) {
                        app.leave_subagent_view();
                        return true;
                    }
                }
                // No composer in subagent view — ignore leftover focus clicks.
                return false;
            }
            if let Some(idx) = app.expandable_at_row(m.row) {
                app.activate_expandable_at(idx);
                app.blur_input();
                return true;
            }
            if app.input_contains(m.col, m.row) {
                if !app.input_focused {
                    app.focus_input();
                    return true;
                }
                return false;
            }
            // Empty transcript / chrome / elsewhere → drop composer focus.
            if app.input_focused {
                app.blur_input();
                return true;
            }
            false
        }
        MouseKind::Drag if app.sidebar_resizing => {
            let edge = app.sidebar_right_edge;
            let next = render::clamp_width(edge.saturating_sub(m.col));
            if next != app.ui.sidebar_width {
                app.ui.sidebar_width = next;
                true
            } else {
                false
            }
        }
        MouseKind::Up if app.sidebar_resizing => {
            app.sidebar_resizing = false;
            app.ui.sidebar_width = render::clamp_width(app.ui.sidebar_width);
            let _ = input_tx.send(InputCommand::SaveUi(app.ui.clone()));
            true
        }
        // Motion / other releases must not thrash the redraw loop.
        _ => false,
    }
}

fn handle_plan_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
    interrupt: &Arc<AtomicBool>,
) -> bool {
    let ctrl = key.mods.ctrl;

    match key.code {
        KeyCode::Char('q') if ctrl => return true,
        KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
        _ => {}
    }

    if app.plan_view.amending && app.input_focused {
        match key.code {
            KeyCode::Enter if !key.mods.shift && !key.mods.alt && !key.mods.ctrl => {
                let notes = app.input.take();
                if notes.trim().is_empty() {
                    return false;
                }
                let msg = app.plan_amend_message(notes.trim());
                app.plan_view.selected.clear();
                app.plan_view.amending = false;
                app.blur_input();
                app.leave_special_view();
                app.agent_mode = AgentMode::Plan;
                app.push_user(msg.clone());
                let _ = input_tx.send(InputCommand::User {
                    text: msg,
                    images: Vec::new(),
                    mode: AgentMode::Plan,
                });
                return false;
            }
            KeyCode::Esc => {
                if !app.input.is_empty() {
                    let _ = app.input.take();
                    app.note_input_activity();
                } else {
                    app.plan_view.selected.clear();
                    app.plan_view.amending = false;
                    app.blur_input();
                }
                return false;
            }
            KeyCode::Char(ch) if !ctrl => {
                app.input.insert(ch);
                app.note_input_activity();
                return false;
            }
            KeyCode::Backspace => {
                app.input.backspace();
                app.note_input_activity();
                return false;
            }
            KeyCode::Left => {
                app.input.left();
                app.note_input_activity();
            }
            KeyCode::Right => {
                app.input.right();
                app.note_input_activity();
            }
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => {
            app.leave_special_view();
        }
        KeyCode::Char('b') | KeyCode::Char('B') => {
            start_build_from_plan(app, input_tx);
        }
        KeyCode::Char(' ') => app.plan_toggle_select(),
        KeyCode::Up => app.plan_cursor_up(),
        KeyCode::Down => app.plan_cursor_down(),
        KeyCode::PageUp => app.scroll_up(5),
        KeyCode::PageDown => app.scroll_down(5),
        _ => {
            let _ = interrupt;
        }
    }
    false
}

fn start_build_from_plan(app: &mut App, input_tx: &UnboundedSender<InputCommand>) {
    app.agent_mode = AgentMode::Build;
    app.leave_special_view();
    let text =
        "Implement the plan in `.hive/Plan.md`. Follow its sections in order and keep going until the work is done."
            .to_string();
    app.push_user(text.clone());
    let _ = input_tx.send(InputCommand::User {
        text,
        images: Vec::new(),
        mode: AgentMode::Build,
    });
}

/// Copy the last finished answer to the system clipboard via OSC 52 (works in
/// most modern terminals). Feedback goes to the footer, not the chat.
fn copy_last_answer(app: &mut App) {
    let Some(text) = app.last_answer().map(|s| s.to_string()) else {
        app.flash("Nothing to copy");
        return;
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]52;c;{b64}\x07");
    let _ = out.flush();
    app.flash("Copied");
}

/// Complete slash command or `@file` mention from the floating menu.
fn complete_selected(app: &mut App) {
    if !app.slash_items().is_empty() {
        if let Some(item) = app.menu_selected() {
            let text = if item.takes_arg() {
                format!("/{} ", item.name())
            } else {
                format!("/{}", item.name())
            };
            app.input.value = text;
            app.input.end();
            app.reset_menu();
        }
        return;
    }
    if let Some(path) = app.file_menu_selected() {
        app.complete_at_file(&path);
    }
}

fn submit(
    app: &mut App,
    input_tx: &UnboundedSender<InputCommand>,
    follow_up: &FollowUpSlot,
) -> bool {
    let text = app.input.take();
    let trimmed = text.trim().to_string();

    // Second Enter while a follow-up is queued: inject before the next tool round.
    if app.running && trimmed.is_empty() && !app.has_pending_attaches() {
        return inject_follow_up_now(app, follow_up);
    }

    // A lone pasted path becomes an attachment tag instead of a message.
    if !trimmed.starts_with('/') {
        if let Some(leftover) = app.try_attach_pasted_path(&trimmed) {
            if leftover.is_empty() {
                app.flash(format!("Attached {}", app.attachment_tags_line()));
                return false;
            }
        }
    }

    if trimmed.is_empty() && !app.has_pending_attaches() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix('/') {
        return handle_slash(app, rest, input_tx);
    }

    let Some(prepared) = prepare_user_message(app, text, trimmed) else {
        return false;
    };

    // While the agent is busy: queue — don't dump into chat / driver yet.
    if app.running {
        app.queue_follow_up(crate::app::QueuedFollowUp {
            display: prepared.display,
            text: prepared.agent_text,
            composer: prepared.composer,
            attaches: prepared.attaches,
            mode: prepared.mode,
        });
        return false;
    }

    dispatch_user(app, input_tx, prepared);
    false
}

/// Empty composer + queued follow-up + Enter → stage for the next agent step.
fn inject_follow_up_now(app: &mut App, slot: &FollowUpSlot) -> bool {
    let Some(fu) = app.follow_up.take() else {
        return false;
    };
    let images = fu
        .attaches
        .iter()
        .filter_map(|a| a.image.clone())
        .collect();
    app.push_user(fu.display);
    if let Ok(mut g) = slot.lock() {
        *g = Some(UserInput {
            text: fu.text,
            images,
            mode: fu.mode,
        });
    }
    app.flash("Follow-up → next step");
    false
}

struct PreparedUser {
    display: String,
    agent_text: String,
    /// Original composer text (for ↑ recall).
    composer: String,
    attaches: Vec<crate::app::PendingAttach>,
    mode: AgentMode,
}

impl PreparedUser {
    fn images(&self) -> Vec<ImageSource> {
        self.attaches
            .iter()
            .filter_map(|a| a.image.clone())
            .collect()
    }
}

fn prepare_user_message(app: &mut App, text: String, trimmed: String) -> Option<PreparedUser> {
    let attaches = app.take_pending_attaches();
    let tags = attaches
        .iter()
        .map(|a| a.tag())
        .collect::<Vec<_>>()
        .join(" ");
    let file_notes: Vec<String> = attaches
        .iter()
        .filter(|a| a.image.is_none())
        .map(|a| format!("[Attached file: {}]", a.path))
        .collect();

    let display = match (tags.is_empty(), trimmed.is_empty()) {
        (true, true) => return None,
        (false, true) => tags,
        (true, false) => text.clone(),
        (false, false) => format!("{tags} {text}"),
    };

    let composer = text.clone();
    let mut agent_text = text;
    if !file_notes.is_empty() {
        if !agent_text.is_empty() {
            agent_text.push('\n');
        }
        agent_text.push_str(&file_notes.join("\n"));
    }

    Some(PreparedUser {
        display,
        agent_text,
        composer,
        attaches,
        mode: app.agent_mode,
    })
}

fn dispatch_user(
    app: &mut App,
    input_tx: &UnboundedSender<InputCommand>,
    prepared: PreparedUser,
) {
    let images = prepared.images();
    app.push_user(prepared.display);
    let _ = input_tx.send(InputCommand::User {
        text: prepared.agent_text,
        images,
        mode: prepared.mode,
    });
}

/// After a turn ends, send any queued follow-up as the next user message.
fn flush_follow_up(app: &mut App, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let Some(fu) = app.follow_up.take() else {
        return false;
    };
    dispatch_user(
        app,
        input_tx,
        PreparedUser {
            display: fu.display,
            agent_text: fu.text,
            composer: fu.composer,
            attaches: fu.attaches,
            mode: fu.mode,
        },
    );
    true
}

fn handle_slash(app: &mut App, cmd: &str, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let mut parts = cmd.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    // Removed command: paste a path instead.
    if name.eq_ignore_ascii_case("image") || name.eq_ignore_ascii_case("attach") {
        app.notice(" /attach was removed — paste a file path into the composer");
        return false;
    }

    if let Some(def) = commands::resolve(name) {
        return run_command(app, def.id, arg, input_tx);
    }

    if let Some(skill) = app.find_skill(name).cloned() {
        return invoke_skill(app, &skill, arg, input_tx);
    }

    app.notice(format!("unknown command: /{name}  — {UNKNOWN_HINT}"));
    false
}

/// Run a skill: show `/name` in the transcript and send the skill body to the agent.
fn invoke_skill(
    app: &mut App,
    skill: &crate::SkillChoice,
    note: &str,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    let display = if note.is_empty() {
        format!("/{}", skill.name)
    } else {
        format!("/{} {}", skill.name, note)
    };

    let mut agent_text = format!(
        "The user invoked skill `{}` via the /{} command. \
Follow the skill instructions below immediately — do not ask whether to use it. \
You may call `read_skill` again if needed, but the full skill is already included.\n\n\
# Skill: {}\n\n{}\n",
        skill.name, skill.name, skill.name, skill.content
    );
    if !note.is_empty() {
        agent_text.push_str("\n## User note\n");
        agent_text.push_str(note);
        agent_text.push('\n');
    }

    app.push_user(display);
    app.flash(format!("Skill · {}", skill.name));
    let mode = app.agent_mode;
    let _ = input_tx.send(InputCommand::User {
        text: agent_text,
        images: Vec::new(),
        mode,
    });
    false
}

fn run_command(
    app: &mut App,
    id: CmdId,
    arg: &str,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    match id {
        CmdId::Quit => return true,
        CmdId::Clear => {
            app.new_chat();
            let _ = input_tx.send(InputCommand::Clear);
            app.flash("New chat");
        }
        CmdId::Compact => {
            let _ = input_tx.send(InputCommand::Compact);
            app.flash("Compacting…");
        }
        CmdId::Model => {
            if arg.is_empty() {
                app.open_model_picker(input_tx);
            } else {
                let display = arg.rsplit('/').next().unwrap_or(arg).to_string();
                let _ = input_tx.send(InputCommand::SetModel {
                    id: arg.to_string(),
                    display,
                    connection_id: None,
                });
            }
        }
        CmdId::Copy => copy_last_answer(app),
        CmdId::Cost => app.notice(format!(
            "tokens — prompt {} · completion {} · total {}",
            app.usage.prompt_tokens, app.usage.completion_tokens, app.usage.total_tokens
        )),
        CmdId::Connect => app.open_connect_picker(),
        CmdId::About => app.open_about(),
        CmdId::Settings => app.open_settings(),
    }
    false
}

fn handle_settings_key(app: &mut App, key: Key, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let ctrl = key.mods.ctrl;
    match key.code {
        KeyCode::Esc => {
            if let Some(st) = app.settings.as_mut() {
                if st.back() {
                    app.close_settings();
                }
            }
        }
        KeyCode::Up => {
            if let Some(st) = app.settings.as_mut() {
                st.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(st) = app.settings.as_mut() {
                st.move_down();
            }
        }
        KeyCode::Enter | KeyCode::Right => {
            let width_row = app
                .settings
                .as_ref()
                .is_some_and(|st| {
                    st.page == crate::app::settings::SettingsPage::Sidebar && st.selected == 2
                });
            let changed = if width_row {
                crate::app::settings::nudge_width(app, 2)
            } else {
                crate::app::settings::activate(app)
            };
            if changed {
                app.persist_ui(input_tx);
            }
        }
        KeyCode::Left => {
            let width_row = app
                .settings
                .as_ref()
                .is_some_and(|st| {
                    st.page == crate::app::settings::SettingsPage::Sidebar && st.selected == 2
                });
            if width_row && crate::app::settings::nudge_width(app, -2) {
                app.persist_ui(input_tx);
            }
        }
        KeyCode::Char('p') if ctrl => {
            app.close_settings();
            app.open_palette();
        }
        KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
        KeyCode::Char('q') if ctrl => return true,
        _ => {}
    }
    false
}

fn handle_about_key(app: &mut App, key: Key) -> bool {
    let ctrl = key.mods.ctrl;
    match key.code {
        KeyCode::Esc => app.close_about(),
        KeyCode::Char('p') if ctrl => {
            app.close_about();
            app.open_palette();
        }
        KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
        KeyCode::Char('q') if ctrl => return true,
        _ => {}
    }
    false
}

fn handle_palette_key(app: &mut App, key: Key, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let ctrl = key.mods.ctrl;
    let choices = app.model_choices.clone();
    let connections = app.connections.clone();

    // Ctrl+V / Insert / Shift+Insert → system clipboard (bracketed paste is Event::Paste).
    let wants_clipboard = (ctrl && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')))
        || matches!(key.code, KeyCode::Insert);
    if wants_clipboard {
        if let Some(text) = read_clipboard_text() {
            let _ = paste_into_palette(app, &text);
        }
        return false;
    }

    match key.code {
        KeyCode::Esc => {
            if let Some(pal) = app.palette.as_ref() {
                match pal.mode {
                    PaletteMode::Models | PaletteMode::Connect => app.open_palette(),
                    PaletteMode::ConnectPresets => app.open_connect_picker(),
                    PaletteMode::ConnectKey { .. } => {
                        app.palette = Some(crate::app::palette::PaletteState::connect_presets());
                    }
                    PaletteMode::Commands => app.close_palette(),
                }
            } else {
                app.close_palette();
            }
        }
        KeyCode::Char('p') if ctrl => {
            app.close_palette();
        }
        KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
        KeyCode::Char('q') if ctrl => return true,
        KeyCode::Up => {
            let visible = app.palette_list_visible as usize;
            if let Some(pal) = app.palette.as_mut() {
                if visible > 0 {
                    pal.move_up_visible(&choices, &connections, visible);
                } else {
                    pal.move_up(&choices, &connections);
                }
            }
        }
        KeyCode::Down => {
            let visible = app.palette_list_visible as usize;
            if let Some(pal) = app.palette.as_mut() {
                if visible > 0 {
                    pal.move_down_visible(&choices, &connections, visible);
                } else {
                    pal.move_down(&choices, &connections);
                }
            }
        }
        KeyCode::Enter => return activate_palette(app, input_tx),
        KeyCode::Backspace => {
            if let Some(pal) = app.palette.as_mut() {
                pal.backspace(&choices, &connections);
            }
        }
        KeyCode::Left => {
            if let Some(pal) = app.palette.as_mut() {
                pal.left();
            }
        }
        KeyCode::Right => {
            if let Some(pal) = app.palette.as_mut() {
                pal.right();
            }
        }
        KeyCode::Char(ch) if !ctrl => {
            if let Some(pal) = app.palette.as_mut() {
                pal.insert(ch, &choices, &connections);
            }
        }
        _ => {}
    }
    false
}

fn activate_palette(app: &mut App, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let mode = app.palette.as_ref().map(|p| p.mode);
    match mode {
        Some(PaletteMode::Models) => {
            let picked = app
                .palette
                .as_ref()
                .and_then(|p| p.selected_model(&app.model_choices))
                .map(|c| {
                    (
                        c.key.clone(),
                        c.display.clone(),
                        c.connection_id.clone(),
                    )
                });
            app.close_palette();
            if let Some((id, display, connection_id)) = picked {
                let connection_id = (!connection_id.is_empty()).then_some(connection_id);
                let _ = input_tx.send(InputCommand::SetModel {
                    id,
                    display,
                    connection_id,
                });
                app.flash("Switching model…");
            }
            false
        }
        Some(PaletteMode::Connect) => {
            use crate::app::palette::ConnectRow;
            let row = app
                .palette
                .as_ref()
                .and_then(|p| p.selected_connect(&app.connections))
                .map(|r| match r {
                    ConnectRow::Profile(c) => ConnectPick::Profile(c.id.clone()),
                    ConnectRow::Add => ConnectPick::Add,
                    ConnectRow::RemoveActive => ConnectPick::Remove,
                });
            match row {
                Some(ConnectPick::Profile(id)) => {
                    app.close_palette();
                    let _ = input_tx.send(InputCommand::SetConnection { id });
                    app.flash("Switching provider…");
                }
                Some(ConnectPick::Add) => {
                    app.palette = Some(crate::app::palette::PaletteState::connect_presets());
                }
                Some(ConnectPick::Remove) => {
                    let id = app.active_connection.clone();
                    if !id.is_empty() {
                        let _ = input_tx.send(InputCommand::RemoveConnection { id });
                        app.flash("Removing provider…");
                    }
                }
                None => {}
            }
            false
        }
        Some(PaletteMode::ConnectPresets) => {
            let idx = app.palette.as_ref().and_then(|p| p.selected_preset_idx());
            if let Some(idx) = idx {
                app.palette = Some(crate::app::palette::PaletteState::connect_key(idx));
            }
            false
        }
        Some(PaletteMode::ConnectKey { preset_idx }) => {
            let key = app
                .palette
                .as_ref()
                .map(|p| p.query.trim().to_string())
                .unwrap_or_default();
            if key.is_empty() {
                app.flash("Paste an API key");
                return false;
            }
            let preset = crate::PRESETS.get(preset_idx).copied();
            let Some(preset) = preset else {
                return false;
            };
            let id = unique_connection_id(preset.label, &app.connections);
            let model_id = app.model.clone();
            let model_name = app.model_display.clone();
            app.close_palette();
            let _ = input_tx.send(InputCommand::UpsertConnection {
                id,
                label: preset.label.to_string(),
                base_url: preset.base_url.to_string(),
                api_key_env: preset.api_key_env.to_string(),
                api_key: key,
                model_id,
                model_name,
            });
            app.flash(format!("Connecting {}…", preset.label));
            false
        }
        Some(PaletteMode::Commands) => {
            let id = app.palette.as_ref().and_then(|p| p.selected_cmd_id());
            let takes_arg = app
                .palette
                .as_ref()
                .and_then(|p| p.selected_command())
                .map(|c| c.takes_arg)
                .unwrap_or(false);
            let name = app
                .palette
                .as_ref()
                .and_then(|p| p.selected_command())
                .map(|c| c.name);
            app.close_palette();
            match id {
                Some(CmdId::Model) => {
                    app.open_model_picker(input_tx);
                    false
                }
                Some(CmdId::Connect) => {
                    app.open_connect_picker();
                    false
                }
                Some(_id) if takes_arg => {
                    if let Some(name) = name {
                        app.focus_input();
                        app.input.value = format!("/{name} ");
                        app.input.cursor = app.input.value.chars().count();
                        app.note_input_activity();
                    }
                    false
                }
                Some(id) => run_command(app, id, "", input_tx),
                None => false,
            }
        }
        None => false,
    }
}

enum ConnectPick {
    Profile(String),
    Add,
    Remove,
}

fn unique_connection_id(label: &str, existing: &[hive_core::event::ConnectionInfo]) -> String {
    let base: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() {
        "provider".into()
    } else {
        base
    };
    if !existing.iter().any(|c| c.id == base) {
        return base;
    }
    for n in 2..100 {
        let candidate = format!("{base}-{n}");
        if !existing.iter().any(|c| c.id == candidate) {
            return candidate;
        }
    }
    format!("{base}-x")
}

#[cfg(test)]
mod tests {
    use super::*;
    use comb::{Key, KeyCode, KeyMods};
    use hive_core::event::{AgentEvent, SubagentStatus};

    fn follow_slot() -> FollowUpSlot {
        Arc::new(std::sync::Mutex::new(None))
    }

    fn test_app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: vec![
                crate::ModelChoice {
                    key: "default".into(),
                    display: "Default".into(),
                    detail: "default".into(),
                    group: "Test".into(),
                    connection_id: String::new(),
                },
                crate::ModelChoice {
                    key: "fast".into(),
                    display: "Fast".into(),
                    detail: "fast".into(),
                    group: "Test".into(),
                    connection_id: String::new(),
                },
            ],
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
            ui: Default::default(),
            context_window: 128_000,
        })
    }

    fn ctrl(code: KeyCode) -> Key {
        Key {
            code,
            mods: KeyMods::CTRL,
        }
    }

    #[test]
    fn ctrl_p_opens_command_palette() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        assert!(!app.palette_open());
        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('p')),
            &tx,
            &interrupt,
            &follow_slot()
        ));
        assert!(app.palette_open());
        assert_eq!(
            app.palette.as_ref().unwrap().mode,
            crate::app::palette::PaletteMode::Commands
        );
        assert!(
            !app.palette.as_ref().unwrap().search_focused,
            "search starts unfocused"
        );
    }

    #[test]
    fn palette_typing_focuses_search() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('p')),
            &tx,
            &interrupt,
            &follow_slot()
        ));
        assert!(!handle_key(
            &mut app,
            Key {
                code: KeyCode::Char('m'),
                mods: KeyMods::NONE,
            },
            &tx,
            &interrupt,
            &follow_slot()
        ));
        let pal = app.palette.as_ref().unwrap();
        assert!(pal.search_focused);
        assert_eq!(pal.query, "m");
    }

    #[test]
    fn palette_switch_model_opens_picker_and_selects() {
        let mut app = test_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.open_palette();
        // Move selection to Switch model if needed and activate.
        if let Some(pal) = app.palette.as_mut() {
            // Ensure we land on Model.
            for _ in 0..8 {
                if pal.selected_cmd_id() == Some(CmdId::Model) {
                    break;
                }
                pal.move_down(&app.model_choices.clone(), &app.connections.clone());
            }
        }
        assert_eq!(
            app.palette.as_ref().and_then(|p| p.selected_cmd_id()),
            Some(CmdId::Model)
        );
        assert!(!activate_palette(&mut app, &tx));
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::Models)
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(InputCommand::FetchModels)
        ));
        app.models_catalog = crate::app::ModelsCatalogState::Ready;
        if let Some(pal) = app.palette.as_mut() {
            pal.clamp_selection(&app.model_choices.clone(), &app.connections.clone());
        }
        assert!(!activate_palette(&mut app, &tx));
        match rx.try_recv() {
            Ok(InputCommand::SetModel {
                id,
                display,
                connection_id,
            }) => {
                assert_eq!(id, "default");
                assert_eq!(display, "Default");
                assert!(connection_id.is_none());
            }
            other => panic!("expected SetModel, got {other:?}"),
        }
    }

    #[test]
    fn follow_up_queues_while_running_and_flushes() {
        let mut app = test_app();
        app.running = true;
        app.input.value = "do this next".into();
        app.input.cursor = app.input.value.chars().count();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(!submit(&mut app, &tx, &follow_slot()));
        assert!(app.has_follow_up());
        assert!(app.input.is_empty());
        assert!(rx.try_recv().is_err(), "must not send while running");

        app.running = false;
        assert!(flush_follow_up(&mut app, &tx));
        assert!(!app.has_follow_up());
        match rx.try_recv() {
            Ok(InputCommand::User { text, .. }) => assert_eq!(text, "do this next"),
            other => panic!("expected flushed User, got {other:?}"),
        }
    }

    #[test]
    fn follow_up_up_arrow_recalls_for_edit() {
        let mut app = test_app();
        app.queue_follow_up(crate::app::QueuedFollowUp {
            display: "queued text".into(),
            text: "queued text".into(),
            composer: "queued text".into(),
            attaches: Vec::new(),
            mode: AgentMode::Build,
        });
        assert!(app.recall_follow_up());
        assert!(!app.has_follow_up());
        assert_eq!(app.input.value, "queued text");
    }

    #[test]
    fn follow_up_second_enter_injects_for_next_step() {
        let mut app = test_app();
        app.running = true;
        app.queue_follow_up(crate::app::QueuedFollowUp {
            display: "now".into(),
            text: "now".into(),
            composer: "now".into(),
            attaches: Vec::new(),
            mode: AgentMode::Build,
        });
        let slot = follow_slot();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(app.input.is_empty());
        assert!(!submit(&mut app, &tx, &slot));
        assert!(!app.has_follow_up());
        assert!(rx.try_recv().is_err(), "inject uses slot, not InputCommand");
        let staged = slot.lock().unwrap().take().expect("staged follow-up");
        assert_eq!(staged.text, "now");
    }

    #[test]
    fn slash_menu_lists_skills_and_invokes() {
        let mut app = test_app();
        app.skills.push(crate::SkillChoice {
            name: "read-tweet".into(),
            description: "Fetch and summarize a tweet URL".into(),
            content: "1. Open the URL\n2. Summarize\n".into(),
        });
        app.input.value = "/read".into();
        app.input.cursor = app.input.value.chars().count();
        let items = app.slash_items();
        assert!(
            items.iter().any(|i| i.name() == "read-tweet"),
            "items={:?}",
            items.iter().map(|i| i.name().to_string()).collect::<Vec<_>>()
        );
        assert!(items.iter().any(|i| i.desc().contains("summarize")));

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(!handle_slash(&mut app, "read-tweet please", &tx));
        match rx.try_recv() {
            Ok(InputCommand::User { text, .. }) => {
                assert!(text.contains("Skill: read-tweet"));
                assert!(text.contains("Open the URL"));
                assert!(text.contains("User note"));
                assert!(text.contains("please"));
            }
            other => panic!("expected User with skill body, got {other:?}"),
        }
    }

    #[test]
    fn slash_aliases_resolve_clear_and_quit() {
        let mut app = test_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(!handle_slash(&mut app, "new", &tx));
        assert!(matches!(rx.try_recv(), Ok(InputCommand::Clear)));
        assert!(handle_slash(&mut app, "exit", &tx));
        assert!(handle_slash(&mut app, "quit", &tx));
    }

    #[test]
    fn slash_models_alias_opens_picker() {
        let mut app = test_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(!handle_slash(&mut app, "models", &tx));
        assert!(app.palette_open());
        assert!(matches!(
            app.palette.as_ref().map(|p| p.mode),
            Some(crate::app::palette::PaletteMode::Models)
        ));
        assert!(matches!(rx.try_recv(), Ok(InputCommand::FetchModels)));
    }

    #[test]
    fn slash_about_aliases_open_about() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        for name in ["about", "help", "version"] {
            app.close_about();
            assert!(!app.about_open());
            assert!(!handle_slash(&mut app, name, &tx));
            assert!(app.about_open(), "/{name} should open About");
        }
    }

    #[test]
    fn palette_about_opens_about_overlay() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        app.open_palette();
        if let Some(pal) = app.palette.as_mut() {
            for _ in 0..16 {
                if pal.selected_cmd_id() == Some(CmdId::About) {
                    break;
                }
                pal.move_down(&app.model_choices.clone(), &app.connections.clone());
            }
        }
        assert_eq!(
            app.palette.as_ref().and_then(|p| p.selected_cmd_id()),
            Some(CmdId::About)
        );
        assert!(!activate_palette(&mut app, &tx));
        assert!(!app.palette_open());
        assert!(app.about_open());
    }

    #[test]
    fn about_esc_closes() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        app.open_about();
        assert!(!handle_key(&mut app, Key {
                code: KeyCode::Esc,
                mods: KeyMods::NONE,
            }, &tx, &interrupt, &follow_slot()));
        assert!(!app.about_open());
    }

    fn esc_key() -> Key {
        Key {
            code: KeyCode::Esc,
            mods: KeyMods::NONE,
        }
    }

    #[test]
    fn palette_esc_closes() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        app.open_palette();
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt, &follow_slot()));
        assert!(!app.palette_open());
    }

    #[test]
    fn palette_models_esc_returns_to_commands() {
        let mut app = test_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        app.open_model_picker(&tx);
        let _ = rx.try_recv(); // FetchModels
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::Models)
        );
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt, &follow_slot()));
        assert!(app.palette_open());
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::Commands)
        );
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt, &follow_slot()));
        assert!(!app.palette_open());
    }

    #[test]
    fn overlay_mouse_is_ignored() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        app.open_palette();
        assert!(!handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 0,
                row: 0,
            },
            &tx
        ));
        assert!(app.palette_open());
        app.close_palette();
        app.open_about();
        assert!(!handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::ScrollDown,
                col: 40,
                row: 7,
            },
            &tx
        ));
        assert!(app.about_open());
    }

    #[test]
    fn paste_inserts_multiline_into_composer() {
        let mut app = test_app();
        app.blur_input();
        assert!(handle_paste(&mut app, "hello\r\nworld"));
        assert!(app.input_focused);
        assert_eq!(app.input.value, "hello\nworld");
        assert_eq!(app.input.cursor, app.input.value.chars().count());
    }

    #[test]
    fn paste_into_connect_key_strips_whitespace() {
        let mut app = test_app();
        app.palette = Some(crate::app::palette::PaletteState::connect_key(0));
        assert!(handle_paste(&mut app, " sk-abc \n"));
        let q = app.palette.as_ref().unwrap().query.clone();
        assert_eq!(q, "sk-abc");
    }

    #[test]
    fn at_mention_completes_file_into_input() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hive-at-run-{n}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/hello.rs"), "fn main() {}").unwrap();

        let mut app = App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: vec![],
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: dir.to_string_lossy().into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
            ui: Default::default(),
            context_window: 128_000,
        });
        app.input.value = "look @hel".into();
        app.input.cursor = app.input.value.chars().count();
        assert!(app.at_mention().is_some());
        let items = app.file_menu_items();
        assert!(items.iter().any(|p| p == "src/hello.rs"), "items={items:?}");
        app.complete_at_file("src/hello.rs");
        assert_eq!(app.input.value.trim_end(), "look");
        assert!(app.at_mention().is_none());
        assert!(app.has_pending_attaches());
        assert_eq!(app.attachment_tags_line(), "@src/hello.rs");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attach_image_path_without_image_command() {
        let path = std::env::temp_dir().join("hive-tui-attach-test.png");
        std::fs::write(&path, [0x89, 0x50, 0x4e, 0x47]).unwrap();
        let mut app = test_app();
        let tag = app.attach_path(path.to_str().unwrap()).unwrap();
        assert_eq!(tag, "@hive-tui-attach-test.png");
        assert!(app.has_pending_attaches());
        assert!(commands::resolve("image").is_none());

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.input.value = "look".into();
        assert!(!submit(&mut app, &tx, &follow_slot()));
        let _ = std::fs::remove_file(&path);
        match rx.try_recv() {
            Ok(InputCommand::User { text, images, .. }) => {
                assert_eq!(text, "look");
                assert_eq!(images.len(), 1);
            }
            other => panic!("expected User with image, got {other:?}"),
        }
    }

    fn key(code: KeyCode) -> Key {
        Key {
            code,
            mods: KeyMods::NONE,
        }
    }

    #[test]
    fn typing_while_blurred_focuses_and_inserts() {
        let mut app = test_app();
        app.blur_input();
        assert!(!app.input_focused);
        assert!(app.input.is_empty());

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(
            &mut app,
            key(KeyCode::Char('h')),
            &tx,
            &interrupt,
            &follow_slot()
        ));
        assert!(app.input_focused);
        assert_eq!(app.input.value, "h");

        assert!(!handle_key(
            &mut app,
            key(KeyCode::Char('i')),
            &tx,
            &interrupt,
            &follow_slot()
        ));
        assert_eq!(app.input.value, "hi");
    }

    #[test]
    fn arrows_while_blurred_scroll_without_focusing() {
        let mut app = test_app();
        // Pretend the last paint had room to scroll (active chat overflow).
        app.set_transcript_max_scroll(10);
        app.blur_input();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt, &follow_slot()));
        assert!(!app.input_focused);
        assert!(app.input.is_empty());
        assert_eq!(app.scroll_from_bottom, 1);

        assert!(!handle_key(&mut app, key(KeyCode::Down), &tx, &interrupt, &follow_slot()));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 0);
    }

    #[test]
    fn arrows_still_scroll_after_idle_blur() {
        let mut app = test_app();
        app.set_transcript_max_scroll(10);
        app.focus_input();
        app.input_last_activity = std::time::Instant::now().checked_sub(
            std::time::Duration::from_millis((crate::app::INPUT_IDLE_BLUR_MS + 50) as u64),
        );
        assert!(app.tick(), "idle blur should fire");
        assert!(!app.input_focused);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt, &follow_slot()));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 1);

        assert!(!handle_key(&mut app, key(KeyCode::Down), &tx, &interrupt, &follow_slot()));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 0);

        // Printable typing still auto-focuses after idle blur.
        assert!(!handle_key(
            &mut app,
            key(KeyCode::Char('x')),
            &tx,
            &interrupt,
            &follow_slot()
        ));
        assert!(app.input_focused);
        assert_eq!(app.input.value, "x");
    }

    #[test]
    fn blurred_arrows_focus_when_transcript_cannot_scroll() {
        // Fresh / short chat: max_scroll is 0. Blurred Up used to bump a
        // phantom offset with no visual change — felt broken until click.
        let mut app = test_app();
        app.set_transcript_max_scroll(0);
        app.blur_input();
        app.input.value = "hello".into();
        app.input.cursor = 5;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(&mut app, key(KeyCode::Left), &tx, &interrupt, &follow_slot()));
        assert!(app.input_focused, "Left should restore composer focus");
        assert_eq!(app.input.cursor, 4);

        app.blur_input();
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt, &follow_slot()));
        assert!(
            app.input_focused,
            "Up with nothing to scroll should focus, not no-op"
        );
        assert_eq!(
            app.scroll_from_bottom, 0,
            "must not accumulate phantom scroll"
        );
    }

    #[test]
    fn blurred_up_clamps_to_max_scroll() {
        let mut app = test_app();
        app.set_transcript_max_scroll(2);
        app.blur_input();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt, &follow_slot()));
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt, &follow_slot()));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 2, "Up stops at top");

        // Further Up cannot scroll — hand focus back to the composer.
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt, &follow_slot()));
        assert!(app.input_focused);
        assert_eq!(app.scroll_from_bottom, 2);
    }

    #[test]
    fn arrows_work_after_closing_about_while_blurred() {
        let mut app = test_app();
        app.set_transcript_max_scroll(5);
        app.blur_input();
        app.open_about();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        // Esc closes About; arrows must not stay swallowed afterward.
        assert!(!handle_key(&mut app, key(KeyCode::Esc), &tx, &interrupt, &follow_slot()));
        assert!(!app.about_open());
        assert!(!app.input_focused);

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt, &follow_slot()));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 1);
    }

    #[test]
    fn page_scroll_does_not_refresh_idle_blur_timer() {
        let mut app = test_app();
        app.focus_input();
        let before = app.input_last_activity;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(
            &mut app,
            key(KeyCode::PageDown),
            &tx,
            &interrupt,
            &follow_slot()
        ));
        assert_eq!(app.input_last_activity, before);
        assert!(app.input_focused);
    }

    #[test]
    fn typing_in_subagent_view_does_not_fill_main_input() {
        let mut app = test_app();
        app.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking".into(),
            prompt: "go".into(),
        });
        app.apply(AgentEvent::SubagentStatus {
            id: "v1".into(),
            status: SubagentStatus::Running,
            detail: "working".into(),
        });
        app.open_subagent_view("v1".into());
        assert!(!app.input_focused);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(
            &mut app,
            key(KeyCode::Char('x')),
            &tx,
            &interrupt,
            &follow_slot()
        ));
        assert!(app.in_subagent_view());
        assert!(!app.input_focused);
        assert!(app.input.is_empty());
    }
}
