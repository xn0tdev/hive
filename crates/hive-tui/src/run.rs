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
use hive_core::AgentMode;

use crate::app::palette::PaletteMode;
use crate::app::state::{Block, ContextAction, TerminalViewPhase};
use crate::app::App;
use crate::commands::{self, CmdId};
use crate::render;
use crate::terminal_input::{terminal_key_bytes, terminal_paste_bytes};
use crate::{InputCommand, PrivateTerminalInput, TuiInit};

const UNKNOWN_HINT: &str = "Ctrl+P for commands · /about for About";

/// Poll interval while something is animating (spinner / shimmer).
const ANIM_TICK: Duration = Duration::from_millis(100);
/// Idle poll — long enough to skip needless redraws, short enough for input.
const IDLE_TICK: Duration = Duration::from_millis(250);
/// Keep a hot producer from starving keyboard and mouse polling.
const MAX_EVENTS_PER_TICK: usize = 128;

pub fn run(
    init: TuiInit,
    mut events: EventReceiver,
    input_tx: UnboundedSender<InputCommand>,
    interrupt: Arc<AtomicBool>,
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
    terminal.mouse_mode(MouseMode::Motion)?;
    let mut app = App::new(init);

    run_loop(&mut terminal, &mut app, &mut events, &input_tx, &interrupt)
}

fn run_loop(
    terminal: &mut Terminal,
    app: &mut App,
    events: &mut EventReceiver,
    input_tx: &UnboundedSender<InputCommand>,
    interrupt: &Arc<AtomicBool>,
) -> io::Result<()> {
    let mut dirty = true;
    let mut last_spinner = usize::MAX;
    loop {
        let content_dirty = drain_events(app, events, input_tx);

        if app.tick() {
            // Idle composer blur (or other tick-side visual change).
            dirty = true;
        }

        // Flush a deferred dispatch after the grace period (ESC recall window).
        if app.pending_dispatch_ready() {
            if let Some(pd) = app.take_pending_dispatch() {
                let _ = input_tx.send(InputCommand::User {
                    text: pd.agent_text,
                    images: pd.images,
                    mode: pd.mode,
                });
                dirty = true;
            }
        }
        let animating = app.needs_animation();
        let spinner_moved = animating && app.spinner != last_spinner;

        if app.take_repaint_request() {
            terminal.invalidate()?;
            dirty = true;
        }

        if dirty || content_dirty || spinner_moved {
            terminal.draw(|f| render::draw(f, app))?;
            if let Some((id, rows, cols)) = app.take_pending_terminal_resize() {
                if rows > 0 && cols > 0 {
                    let _ = input_tx.send(InputCommand::TerminalResize { id, rows, cols });
                }
            }
            last_spinner = app.spinner;
            dirty = false;
        }

        let wait = if animating { ANIM_TICK } else { IDLE_TICK };
        if let Some(ev) = terminal.read_event(wait)? {
            match ev {
                Event::Key(key) => {
                    if handle_key(app, key, input_tx, interrupt) {
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
                    if handle_paste(app, &text, input_tx) {
                        dirty = true;
                    }
                }
            }
        }
    }
    Ok(())
}

fn drain_events(
    app: &mut App,
    events: &mut EventReceiver,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    let mut dirty = false;
    for _ in 0..MAX_EVENTS_PER_TICK {
        let Ok(ev) = events.try_recv() else {
            break;
        };
        let finished = matches!(ev, hive_core::event::AgentEvent::TurnFinished);
        dirty |= app.apply(ev);
        if finished {
            dirty |= flush_follow_up(app, input_tx);
        }
    }
    if let Some(context) = app.take_pending_context_update() {
        let _ = input_tx.send(InputCommand::UpdateContextWindow { context });
    }
    dirty
}

/// Bracketed paste / clipboard paste. Returns whether the UI should redraw.
fn handle_paste(app: &mut App, text: &str, input_tx: &UnboundedSender<InputCommand>) -> bool {
    if app.in_terminal_view() {
        if app.terminal_view.phase != TerminalViewPhase::UserControl {
            return false;
        }
        let Some(id) = app.terminal_view_id().map(str::to_string) else {
            return false;
        };
        let bracketed = app
            .terminal_input_modes()
            .map(|(_, bracketed)| bracketed)
            .unwrap_or(false);
        let bytes = terminal_paste_bytes(text, bracketed);
        if bytes.is_empty() {
            return false;
        }
        let _ = input_tx.send(InputCommand::TerminalInput {
            id,
            input: PrivateTerminalInput::new(bytes),
        });
        return true;
    }
    if app.about_open() || app.settings_open() || app.goal_overlay_open() {
        return false;
    }
    if app.palette_open() {
        return paste_into_palette(app, text);
    }
    if app.in_special_view() && !(app.in_plan_view() && app.plan_composing()) {
        return false;
    }

    app.focus_input();

    // Drag-and-drop / lone path paste → attach as an @chip, exactly like the
    // `@` picker. `try_attach_pasted_path` only consumes the paste when it fully
    // resolves to existing file path(s) — handling quotes, escaped spaces and
    // `file://` URIs — otherwise we fall through to a normal text paste.
    if let Some(leftover) = app.try_attach_pasted_path(text) {
        if leftover.is_empty() {
            app.flash(format!("Attached {}", app.attachment_tags_line()));
            app.reset_menu();
            return true;
        }
    }

    app.input.insert_str(text);
    app.prompt_history.reset();
    app.reset_menu();
    true
}

fn sanitize_api_key(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn paste_into_palette(app: &mut App, text: &str) -> bool {
    let mode = app.palette.as_ref().map(|p| p.mode);
    match mode {
        // Pasting a key straight onto a highlighted provider skips the click.
        Some(PaletteMode::Connect) => {
            let idx = app
                .palette
                .as_ref()
                .and_then(|p| p.selected_connect(&app.connections))
                .and_then(|row| match row {
                    crate::app::palette::ConnectRow::Preset(i) => Some(i),
                    _ => None,
                });
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
                pal.insert_str(&cleaned, &choices, &connections, &app.saved_sessions);
            }
            true
        }
        Some(PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey) => {
            let cleaned = sanitize_api_key(text);
            if cleaned.is_empty() {
                return false;
            }
            let choices = app.model_choices.clone();
            let connections = app.connections.clone();
            if let Some(pal) = app.palette.as_mut() {
                pal.insert_str(&cleaned, &choices, &connections, &app.saved_sessions);
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
                pal.insert_str(text, &choices, &connections, &app.saved_sessions);
            }
            true
        }
        None => false,
    }
}

fn read_clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// A screenshot on the clipboard, as PNG bytes. Clipboards hand images over as
/// raw RGBA, so they have to be encoded before anything can be sent.
fn read_clipboard_image() -> Option<Vec<u8>> {
    let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
    let (w, h) = (image.width as u32, image.height as u32);
    if w == 0 || h == 0 {
        return None;
    }
    encode_png(w, h, &image.bytes)
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    if rgba.len() < (width as usize) * (height as usize) * 4 {
        return None;
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()?;
    Some(out)
}

/// Ctrl+V / Insert: text if there is any, otherwise an image. Copying a file in
/// a file manager puts its path on the clipboard as text, so the path-attach
/// route keeps working and only a bare image falls through to here.
fn paste_from_clipboard(app: &mut App, input_tx: &UnboundedSender<InputCommand>) -> bool {
    if let Some(text) = read_clipboard_text().filter(|t| !t.trim().is_empty()) {
        return handle_paste(app, &text, input_tx);
    }
    if !app.accepts_attachments() {
        return false;
    }
    let Some(png) = read_clipboard_image() else {
        return false;
    };
    app.focus_input();
    match app.attach_clipboard_image(png) {
        Ok(_) => {
            app.flash(format!("Attached {}", app.attachment_tags_line()));
            app.reset_menu();
        }
        Err(e) => app.flash(e),
    }
    true
}

fn write_clipboard_text(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}

fn handle_terminal_key(app: &mut App, key: Key, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let phase = app.terminal_view.phase;
    let Some(id) = app.terminal_view_id().map(str::to_string) else {
        return false;
    };
    let user_owned = app
        .viewed_terminal()
        .is_some_and(|card| card.controller == hive_core::TerminalController::User);

    if key.mods.ctrl && key.code == KeyCode::Char(']') {
        if phase == TerminalViewPhase::Attaching || user_owned {
            let _ = input_tx.send(InputCommand::TerminalDetach { id });
        }
        app.leave_terminal_view();
        return false;
    }

    if key.mods.shift && key.code == KeyCode::PageUp {
        let amount = app
            .terminal_view
            .body_rect
            .map(|rect| usize::from(rect.height.saturating_sub(1).max(1)))
            .unwrap_or(5);
        app.scroll_terminal(isize::try_from(amount).unwrap_or(isize::MAX));
        return false;
    }
    if key.mods.shift && key.code == KeyCode::PageDown {
        let amount = app
            .terminal_view
            .body_rect
            .map(|rect| usize::from(rect.height.saturating_sub(1).max(1)))
            .unwrap_or(5);
        app.scroll_terminal(-isize::try_from(amount).unwrap_or(isize::MAX));
        return false;
    }

    if phase != TerminalViewPhase::UserControl {
        return false;
    }
    let application_cursor = app
        .terminal_input_modes()
        .map(|(application_cursor, _)| application_cursor)
        .unwrap_or(false);
    let bytes = terminal_key_bytes(key, application_cursor);
    if !bytes.is_empty() {
        let _ = input_tx.send(InputCommand::TerminalInput {
            id,
            input: PrivateTerminalInput::new(bytes),
        });
    }
    false
}

/// Returns true when the user asked to quit.
fn handle_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
    interrupt: &Arc<AtomicBool>,
) -> bool {
    if app.in_terminal_view() {
        return handle_terminal_key(app, key, input_tx);
    }

    // Context menu captures all keys while open.
    if app.context_menu_open() {
        return handle_context_menu_key(app, key);
    }

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

    if app.goal_overlay_open() {
        return handle_goal_key(app, key, input_tx);
    }

    // Plan preview: section select / amend / Make / back.
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

    // Any key other than ESC flushes a deferred dispatch — the user is doing
    // something else, so start the agent now.
    if app.pending_dispatch.is_some() && key.code != KeyCode::Esc {
        if let Some(pd) = app.take_pending_dispatch() {
            let _ = input_tx.send(InputCommand::User {
                text: pd.agent_text,
                images: pd.images,
                mode: pd.mode,
            });
        }
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
        // Something wrote over the alternate screen (a subprocess prompt, a
        // kernel message): the frame diff can't know, so repaint everything.
        KeyCode::Char('l') if ctrl => {
            app.request_repaint();
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

    // An attachment chip has the keyboard: arrows walk the chips, backspace
    // drops one, anything else hands control back to the composer.
    if app.selected_attach().is_some() {
        match key.code {
            KeyCode::Left | KeyCode::Up => {
                app.move_attach_selection(-1);
                return false;
            }
            KeyCode::Right | KeyCode::Down => {
                app.move_attach_selection(1);
                return false;
            }
            KeyCode::Backspace | KeyCode::Delete => {
                if let Some(label) = app.remove_selected_attach() {
                    app.flash(format!("Removed @{label}"));
                }
                return false;
            }
            KeyCode::Esc => {
                app.clear_attach_selection();
                return false;
            }
            // Typing, Enter, chords: back to the composer, then handle normally.
            _ => {
                app.clear_attach_selection();
            }
        }
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
                // If a goal is active, pause it so the next turn doesn't auto-start.
                if app.goal_active() && !app.goal_paused() {
                    let _ = input_tx.send(InputCommand::PauseGoal);
                }
                interrupt.store(true, Ordering::Relaxed);
                return false;
            }
            KeyCode::Esc => {
                // Active goal: Esc pauses (or stops if already paused).
                if app.goal_active() && !app.goal_paused() {
                    let _ = input_tx.send(InputCommand::PauseGoal);
                    return false;
                }
                if app.goal_paused() {
                    let _ = input_tx.send(InputCommand::StopGoal);
                    return false;
                }
                // Recall a just-submitted prompt before the agent starts.
                if app.recall_pending_dispatch() {
                    app.focus_input();
                }
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
                return submit(app, input_tx);
            }
        }
        // Ctrl+J → newline when the host remaps it to Char('j')+CTRL.
        KeyCode::Char('j') if ctrl => {
            app.input.newline();
            composer_activity = true;
        }
        // Ctrl+V / Insert → system clipboard (bracketed paste is Event::Paste).
        KeyCode::Char('v') | KeyCode::Char('V') if ctrl => {
            paste_from_clipboard(app, input_tx);
            composer_activity = true;
        }
        KeyCode::Insert => {
            paste_from_clipboard(app, input_tx);
            composer_activity = true;
        }
        KeyCode::Char(ch) if !ctrl => {
            app.prompt_history.reset();
            app.input.insert(ch);
            app.reset_menu();
            composer_activity = true;
        }
        KeyCode::Backspace => {
            app.prompt_history.reset();
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
        // Up/down browse prompt history only when the input is empty.
        KeyCode::Up => {
            if app.prompt_history.is_browsing() {
                app.history_up();
                composer_activity = true;
            } else if (app.input.is_empty() && app.recall_follow_up())
                || (app.input.is_empty() && app.history_up())
                || app.input.up()
            {
                composer_activity = true;
            } else {
                app.scroll_up(1);
            }
        }
        KeyCode::Down => {
            if app.prompt_history.is_browsing() {
                app.history_down();
                composer_activity = true;
            } else if app.input.down() {
                composer_activity = true;
            } else if app.select_first_attach() {
                // Past the last row of text sit the @chips.
                composer_activity = true;
            } else {
                app.scroll_down(1);
            }
        }
        KeyCode::Esc => {
            if app.recall_pending_dispatch() {
                composer_activity = true;
            } else if !app.input.is_empty() {
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
/// chats, hits `← back` / Make, or bonks the logo.
///
/// Palette / About overlays are keyboard-only — mouse events are swallowed so
/// they don't leak through to the chat underneath.
/// Returns `true` when the UI should redraw.
fn handle_mouse(app: &mut App, m: Mouse, input_tx: &UnboundedSender<InputCommand>) -> bool {
    if app.about_open() || app.palette_open() || app.settings_open() || app.context_menu_open() {
        return false;
    }
    match m.kind {
        MouseKind::ScrollUp => {
            if app.in_terminal_view() {
                app.scroll_terminal(3);
            } else {
                app.scroll_up(3);
            }
            true
        }
        MouseKind::ScrollDown => {
            if app.in_terminal_view() {
                app.scroll_terminal(-3);
            } else {
                app.scroll_down(3);
            }
            true
        }
        MouseKind::Down(MouseButton::Left) => {
            if app.in_terminal_view() {
                if app
                    .terminal_stop_hit
                    .is_some_and(|hit| hit.contains(m.col, m.row))
                {
                    if let Some(id) = app.terminal_view_id() {
                        let _ = input_tx.send(InputCommand::TerminalStop { id: id.into() });
                    }
                    return true;
                }
                if app.back_hit.is_some_and(|hit| hit.contains(m.col, m.row)) {
                    let user_owned = app
                        .viewed_terminal()
                        .is_some_and(|card| card.controller == hive_core::TerminalController::User);
                    if app.terminal_view.phase == TerminalViewPhase::Attaching || user_owned {
                        if let Some(id) = app.terminal_view_id() {
                            let _ = input_tx.send(InputCommand::TerminalDetach { id: id.into() });
                        }
                    }
                    app.leave_terminal_view();
                    return true;
                }
                return false;
            }
            if app.is_empty_chat() && app.bonk_logo_at(m.col, m.row) {
                app.blur_input();
                return true;
            }
            if app.scroll_bottom_contains(m.col, m.row) {
                app.scroll_to_bottom();
                return true;
            }
            if let Some(hit) = app.sidebar_resize_hit {
                if hit.contains(m.col, m.row) {
                    app.clear_assistant_selection();
                    app.sidebar_resizing = true;
                    app.blur_input();
                    return true;
                }
            }
            if let Some(hit) = app.sidebar_toggle_hit {
                if hit.contains(m.col, m.row) {
                    app.clear_assistant_selection();
                    app.toggle_sidebar();
                    app.blur_input();
                    return true;
                }
            }
            for (hit, section) in &app.sidebar_section_hits {
                if hit.contains(m.col, m.row) {
                    let section = *section;
                    app.clear_assistant_selection();
                    app.toggle_sidebar_section(section);
                    app.blur_input();
                    return true;
                }
            }
            if let Some(item) = app
                .sidebar_item_hits
                .iter()
                .find(|(hit, _)| hit.contains(m.col, m.row))
                .map(|(_, item)| item.clone())
            {
                app.clear_assistant_selection();
                match item {
                    render::SidebarItem::Subagent(id) => app.open_subagent_view(id),
                    render::SidebarItem::Terminal(id) => app.open_terminal_view(id),
                }
                app.blur_input();
                return true;
            }
            if app.in_plan_view() {
                if let Some(hit) = app.make_hit {
                    if hit.contains(m.col, m.row) {
                        use crate::app::state::PlanAction;
                        match app.plan_action() {
                            PlanAction::Make => start_make_from_plan(app, input_tx),
                            PlanAction::Add => {
                                app.plan_commit_note();
                            }
                            PlanAction::Send => send_plan_corrections(app, input_tx),
                        }
                        return true;
                    }
                }
                if let Some(hit) = app.back_hit {
                    if hit.contains(m.col, m.row) {
                        app.leave_special_view();
                        return true;
                    }
                }
                if app.input_contains(m.col, m.row) && app.plan_composing() {
                    if !app.input_focused {
                        app.focus_input();
                        return true;
                    }
                    return false;
                }
                // Begin text selection on the plan body.
                return app.plan_drag_to(m.col, m.row, false);
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
            if app.start_assistant_selection(m.col, m.row) {
                app.blur_input();
                return true;
            }
            let cleared_selection = app.clear_assistant_selection();
            if let Some(idx) = app.expandable_at_row(m.row) {
                let terminal = app.blocks.get(idx).and_then(|block| match block {
                    crate::app::state::Block::Terminal(card)
                        if matches!(card.process, hive_core::TerminalProcessState::Running) =>
                    {
                        Some(card.id.clone())
                    }
                    _ => None,
                });
                app.activate_expandable_at(idx);
                if let Some(id) = terminal {
                    let _ = input_tx.send(InputCommand::TerminalAttach { id });
                }
                app.blur_input();
                return true;
            }
            if app.input_contains(m.col, m.row) {
                if !app.input_focused {
                    app.focus_input();
                    return true;
                }
                return cleared_selection;
            }
            // Empty transcript / chrome / elsewhere → drop composer focus.
            if app.input_focused {
                app.blur_input();
                return true;
            }
            cleared_selection
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
        MouseKind::Drag if app.in_plan_view() && app.plan_view.drag.is_some() => {
            app.plan_drag_to(m.col, m.row, false)
        }
        MouseKind::Drag if app.assistant_selection_active() => {
            app.update_assistant_selection(m.col, m.row)
        }
        MouseKind::Up if app.sidebar_resizing => {
            app.sidebar_resizing = false;
            app.ui.sidebar_width = render::clamp_width(app.ui.sidebar_width);
            let _ = input_tx.send(InputCommand::SaveUi(app.ui.clone()));
            true
        }
        MouseKind::Up if app.in_plan_view() && app.plan_view.drag.is_some() => {
            app.plan_drag_to(m.col, m.row, true)
        }
        MouseKind::Up if app.assistant_selection_active() => {
            if let Some(text) = app.finish_assistant_selection(m.col, m.row) {
                copy_to_clipboard(&text);
                app.flash("Copied");
            }
            true
        }
        MouseKind::Moved => {
            let mut dirty = false;
            dirty |= app.set_hover_scroll_bottom(app.scroll_bottom_contains(m.col, m.row));
            if app.in_special_view() {
                dirty |= app.set_hover_block(None);
                dirty |= app.set_hover_sidebar_item(None);
                let on_stop = app.in_terminal_view()
                    && app
                        .terminal_stop_hit
                        .is_some_and(|rect| rect.contains(m.col, m.row));
                let on_make = app
                    .make_hit
                    .map(|r| r.contains(m.col, m.row))
                    .unwrap_or(false);
                let on_back = !on_make
                    && !on_stop
                    && app
                        .back_hit
                        .map(|r| r.contains(m.col, m.row))
                        .unwrap_or(false);
                dirty |= app.set_hover_make(on_make);
                dirty |= app.set_hover_back(on_back);
                dirty |= app.set_hover_terminal_stop(on_stop);
            } else {
                dirty |= app.set_hover_back(false);
                dirty |= app.set_hover_make(false);
                dirty |= app.set_hover_terminal_stop(false);
                let sidebar_item = app
                    .sidebar_item_hits
                    .iter()
                    .find(|(hit, _)| hit.contains(m.col, m.row))
                    .map(|(_, item)| item.clone());
                dirty |= app.set_hover_sidebar_item(sidebar_item);
                let card = app.expandable_at_row(m.row).and_then(|i| {
                    matches!(
                        app.blocks.get(i),
                        Some(crate::app::state::Block::Plan(_))
                            | Some(crate::app::state::Block::Subagent(_))
                            | Some(crate::app::state::Block::Terminal(_))
                            | Some(crate::app::state::Block::User(_))
                            | Some(crate::app::state::Block::Tool(_))
                    )
                    .then_some(i)
                });
                dirty |= app.set_hover_block(card);
            }
            dirty
        }
        // Other releases must not thrash the redraw loop.
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

    // Composer open: typing goes to the comment (even if idle-blurred).
    if app.plan_composing() {
        match key.code {
            KeyCode::Enter if !key.mods.shift && !key.mods.alt && !key.mods.ctrl => {
                app.plan_commit_note();
                return false;
            }
            KeyCode::Enter if key.mods.shift => {
                app.focus_input();
                app.input.newline();
                app.note_input_activity();
                return false;
            }
            KeyCode::Esc => {
                if !app.input.is_empty() {
                    let _ = app.input.take();
                    app.note_input_activity();
                } else {
                    app.plan_stop_composing();
                }
                return false;
            }
            KeyCode::Char(ch) if !ctrl => {
                app.focus_input();
                app.input.insert(ch);
                app.note_input_activity();
                return false;
            }
            KeyCode::Backspace => {
                app.focus_input();
                app.input.backspace();
                app.note_input_activity();
                return false;
            }
            KeyCode::Left => {
                app.focus_input();
                app.input.left();
                app.note_input_activity();
                return false;
            }
            KeyCode::Right => {
                app.focus_input();
                app.input.right();
                app.note_input_activity();
                return false;
            }
            KeyCode::PageUp => app.scroll_up(5),
            KeyCode::PageDown => app.scroll_down(5),
            KeyCode::Up => app.scroll_up(1),
            KeyCode::Down => app.scroll_down(1),
            _ => {
                let _ = interrupt;
            }
        }
        return false;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => {
            app.leave_special_view();
        }
        KeyCode::Char('m') | KeyCode::Char('M') if !app.plan_view.has_corrections() => {
            start_make_from_plan(app, input_tx);
        }
        KeyCode::Char('s') | KeyCode::Char('S') if app.plan_view.has_corrections() => {
            send_plan_corrections(app, input_tx);
        }
        KeyCode::Enter
            if app.plan_view.has_corrections()
                && !key.mods.shift
                && !key.mods.alt
                && !key.mods.ctrl =>
        {
            send_plan_corrections(app, input_tx);
        }
        KeyCode::PageUp => app.scroll_up(5),
        KeyCode::PageDown => app.scroll_down(5),
        KeyCode::Up => app.scroll_up(1),
        KeyCode::Down => app.scroll_down(1),
        _ => {
            let _ = interrupt;
        }
    }
    false
}

fn send_plan_corrections(app: &mut App, input_tx: &UnboundedSender<InputCommand>) {
    if !app.plan_view.has_corrections() {
        return;
    }
    app.plan_sync_active_note();
    let msg = app.plan_corrections_message();
    app.plan_clear_corrections();
    // Stay in PLAN — agent revises the plan; user presses MAKE when ready.
    app.agent_mode = AgentMode::Plan;
    app.push_user(msg.clone());
    let _ = input_tx.send(InputCommand::User {
        text: msg,
        images: Vec::new(),
        mode: AgentMode::Plan,
    });
}

fn start_make_from_plan(app: &mut App, input_tx: &UnboundedSender<InputCommand>) {
    app.agent_mode = AgentMode::Make;
    app.leave_special_view();
    let text =
        "Implement the plan in `.hive/Plan.md`. Follow its sections in order and keep going until the work is done."
            .to_string();
    app.push_user(text.clone());
    let _ = input_tx.send(InputCommand::User {
        text,
        images: Vec::new(),
        mode: AgentMode::Make,
    });
}

/// Copy the last finished answer to the clipboard. Feedback goes to the footer,
/// not the chat. The text is cleaned first so the paste has no trailing padding.
fn copy_last_answer(app: &mut App) {
    let Some(text) = app.last_answer().map(clean_for_copy) else {
        app.flash("Nothing to copy");
        return;
    };
    if text.is_empty() {
        app.flash("Nothing to copy");
        return;
    }
    copy_to_clipboard(&text);
    app.flash("Copied");
}

/// Normalize an answer for copying: drop trailing whitespace on every line and
/// strip surrounding blank lines, while keeping leading indentation (code
/// blocks) and interior blank lines (paragraph breaks) intact.
fn clean_for_copy(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

/// Best-effort copy to the clipboard through two channels so it "just works":
/// the local OS clipboard (arboard) for a normal Ctrl/Cmd+V, and OSC 52 for
/// terminals over SSH / inside multiplexers.
fn copy_to_clipboard(text: &str) {
    // OSC 52 — understood by most modern terminals and forwarded over SSH/tmux.
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]52;c;{b64}\x07");
    let _ = out.flush();

    // Local OS clipboard. On X11/Wayland the selection is only served while the
    // owning connection lives, so hold it on a detached thread (`wait`) until
    // another app takes ownership; elsewhere `set_text` persists immediately.
    let owned = text.to_string();
    std::thread::spawn(move || {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return;
        };
        #[cfg(target_os = "linux")]
        {
            use arboard::SetExtLinux;
            let _ = clipboard.set().wait().text(owned);
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = clipboard.set_text(owned);
        }
    });
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

fn submit(app: &mut App, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let text = app.input.take();
    let trimmed = text.trim().to_string();

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

    // Record in prompt history (shell-style up/down recall).
    app.prompt_history.push(&prepared.composer);

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

/// Deferred dispatch: push the user block now but hold the driver send for a
/// brief grace period so ESC can recall the prompt before the agent starts.
fn dispatch_user(app: &mut App, _input_tx: &UnboundedSender<InputCommand>, prepared: PreparedUser) {
    let images = prepared.images();
    app.push_user(prepared.display);
    app.pending_dispatch = Some(crate::app::PendingDispatch {
        agent_text: prepared.agent_text,
        images,
        mode: prepared.mode,
        composer: prepared.composer,
        attaches: prepared.attaches,
        submitted_at: std::time::Instant::now(),
    });
}

/// Immediate dispatch (no grace period) — used for auto-flushed follow-ups.
fn dispatch_user_now(
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
    dispatch_user_now(
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
                let (vision, context, cost_input, cost_output) = app
                    .model_choices
                    .iter()
                    .find(|m| m.key == arg)
                    .map(|m| (m.vision, m.context, m.cost_input, m.cost_output))
                    .unwrap_or((false, 0, 0.0, 0.0));
                let _ = input_tx.send(InputCommand::SetModel {
                    id: arg.to_string(),
                    display,
                    connection_id: None,
                    vision,
                    context,
                    cost_input,
                    cost_output,
                });
            }
        }
        CmdId::Copy => copy_last_answer(app),
        CmdId::Cost => app.notice(format!(
            "tokens — prompt {} · completion {} · total {}",
            app.usage.prompt_tokens, app.usage.completion_tokens, app.usage.total_tokens
        )),
        CmdId::Connect => app.open_connect_picker(),
        CmdId::Goal => {
            if app.goal_active() {
                if let Some(g) = app.goal.as_ref() {
                    app.flash(format!("Goal: {} · {}", g.objective, g.timer_label()));
                }
            } else if !arg.is_empty() {
                let objective = arg.trim().to_string();
                let _ = input_tx.send(InputCommand::SetGoal {
                    objective,
                    duration: None,
                });
            } else {
                app.open_goal_overlay();
            }
        }
        CmdId::About => app.open_about(),
        CmdId::Settings => app.open_settings(),
        CmdId::Resume => {
            if arg.is_empty() {
                let _ = input_tx.send(InputCommand::ListSessions);
                app.open_sessions_picker();
            } else {
                let _ = input_tx.send(InputCommand::LoadSession {
                    id: arg.trim().to_string(),
                });
                app.flash("Loading session…");
            }
        }
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
            let width_row = app.settings.as_ref().is_some_and(|st| {
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
            let width_row = app.settings.as_ref().is_some_and(|st| {
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

fn handle_goal_key(app: &mut App, key: Key, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let ctrl = key.mods.ctrl;
    match key.code {
        KeyCode::Esc => {
            app.close_goal_overlay();
        }
        KeyCode::Backspace => {
            if let Some(st) = app.goal_overlay.as_mut() {
                st.backspace();
            }
        }
        KeyCode::Enter => {
            if let Some(st) = app.goal_overlay.as_ref() {
                let objective = st.objective.trim().to_string();
                if objective.is_empty() {
                    app.flash("Enter an objective first");
                    return false;
                }
                app.close_goal_overlay();
                let _ = input_tx.send(InputCommand::SetGoal {
                    objective,
                    duration: None,
                });
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if let Some(st) = app.goal_overlay.as_mut() {
                st.type_char(c);
            }
        }
        KeyCode::Char('p') if ctrl => {
            app.close_goal_overlay();
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

fn handle_context_menu_key(app: &mut App, key: Key) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Left => {
            app.close_context_menu();
        }
        KeyCode::Up => app.context_menu_up(),
        KeyCode::Down => app.context_menu_down(),
        KeyCode::Enter => {
            if let Some(action) = app.take_context_action() {
                app.close_context_menu();
                activate_context_action(app, action);
            } else {
                app.close_context_menu();
            }
        }
        _ => {}
    }
    false
}

fn activate_context_action(app: &mut App, action: ContextAction) {
    match action {
        ContextAction::CopyPrompt => {
            if let Some(Block::User(text)) = app.blocks.last() {
                write_clipboard_text(text);
                app.flash("Copied");
            }
        }
        ContextAction::RevertFile { path, content } => {
            let full = std::path::Path::new(&app.cwd).join(&path);
            match std::fs::write(&full, &content) {
                Ok(()) => app.flash(format!("Reverted {}", path)),
                Err(e) => app.flash(format!("Revert failed: {e}")),
            }
            app.project.invalidate();
            app.refresh_project();
        }
        ContextAction::CopyOutput => {
            if let Some(Block::Tool(card)) = app.blocks.last() {
                write_clipboard_text(&card.output);
                app.flash("Copied output");
            }
        }
    }
}

/// The `/connect` row under the cursor, if the providers list is open.
fn highlighted_provider<'a>(app: &'a App) -> Option<crate::app::palette::ConnectRow<'a>> {
    app.palette
        .as_ref()
        .filter(|p| p.mode == PaletteMode::Connect)
        .and_then(|p| p.selected_connect(&app.connections))
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
                    PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => {
                        app.open_connect_picker()
                    }
                    PaletteMode::Commands | PaletteMode::Sessions => app.close_palette(),
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
        // Set or replace the key of whatever provider is highlighted. Works on
        // both kinds of row so it never looks dead.
        KeyCode::Char('e') if ctrl => {
            use crate::app::palette::{ConnectRow, PaletteState};
            match highlighted_provider(app) {
                Some(ConnectRow::Profile(c)) => {
                    let id = c.id.clone();
                    app.palette = Some(PaletteState::edit_connection_key(id));
                }
                Some(ConnectRow::Preset(idx)) => {
                    app.palette = Some(PaletteState::connect_key(idx));
                }
                None => {}
            }
        }
        // Drop the highlighted provider.
        KeyCode::Char('r') if ctrl => {
            use crate::app::palette::ConnectRow;
            match highlighted_provider(app) {
                Some(ConnectRow::Profile(c)) => {
                    let (id, label) = (c.id.clone(), c.label.clone());
                    if app.connections.len() <= 1 {
                        app.flash("Can't remove the only provider");
                    } else {
                        let _ = input_tx.send(InputCommand::RemoveConnection { id });
                        app.flash(format!("Removing {label}…"));
                    }
                }
                Some(ConnectRow::Preset(_)) => app.flash("That provider isn't set up yet"),
                None => {}
            }
        }
        KeyCode::Up => {
            let visible = app.palette_list_visible as usize;
            if let Some(pal) = app.palette.as_mut() {
                if visible > 0 {
                    pal.move_up_visible(&choices, &connections, &app.saved_sessions, visible);
                } else {
                    pal.move_up(&choices, &connections, &app.saved_sessions);
                }
            }
        }
        KeyCode::Down => {
            let visible = app.palette_list_visible as usize;
            if let Some(pal) = app.palette.as_mut() {
                if visible > 0 {
                    pal.move_down_visible(&choices, &connections, &app.saved_sessions, visible);
                } else {
                    pal.move_down(&choices, &connections, &app.saved_sessions);
                }
            }
        }
        KeyCode::Enter => return activate_palette(app, input_tx),
        KeyCode::Backspace => {
            if let Some(pal) = app.palette.as_mut() {
                pal.backspace(&choices, &connections, &app.saved_sessions);
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
                pal.insert(ch, &choices, &connections, &app.saved_sessions);
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
                        c.vision,
                        c.context,
                        c.cost_input,
                        c.cost_output,
                    )
                });
            app.close_palette();
            if let Some((id, display, connection_id, vision, context, cost_input, cost_output)) =
                picked
            {
                let connection_id = (!connection_id.is_empty()).then_some(connection_id);
                let _ = input_tx.send(InputCommand::SetModel {
                    id,
                    display,
                    connection_id,
                    vision,
                    context,
                    cost_input,
                    cost_output,
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
                    ConnectRow::Preset(i) => ConnectPick::Preset(i),
                });
            match row {
                Some(ConnectPick::Profile(id)) => {
                    app.close_palette();
                    let _ = input_tx.send(InputCommand::SetConnection { id });
                    app.flash("Switching provider…");
                }
                Some(ConnectPick::Preset(idx)) => {
                    app.palette = Some(crate::app::palette::PaletteState::connect_key(idx));
                }
                None => {}
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
        Some(PaletteMode::EditConnectionKey) => {
            let key = app
                .palette
                .as_ref()
                .map(|p| p.query.trim().to_string())
                .unwrap_or_default();
            if key.is_empty() {
                app.flash("Paste an API key");
                return false;
            }
            let target = app.palette.as_ref().and_then(|p| p.edit_connection.clone());
            let connection = target
                .and_then(|id| app.connections.iter().find(|c| c.id == id))
                .cloned();
            let Some(connection) = connection else {
                app.open_connect_picker();
                app.flash("Provider is no longer available");
                return false;
            };
            app.close_palette();
            let _ = input_tx.send(InputCommand::UpdateConnectionKey {
                id: connection.id,
                api_key: key,
            });
            app.flash(format!("Updating {} key…", connection.label));
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
        Some(PaletteMode::Sessions) => {
            let picked = app
                .palette
                .as_ref()
                .and_then(|p| p.selected_session(&app.saved_sessions))
                .map(|s| s.id.clone());
            app.close_palette();
            if let Some(id) = picked {
                let _ = input_tx.send(InputCommand::LoadSession { id });
                app.flash("Loading session…");
            }
            false
        }
        None => false,
    }
}

/// What the highlighted `/connect` row does when you press Enter.
enum ConnectPick {
    /// Switch to a provider that's already set up.
    Profile(String),
    /// Ask for a key for a provider that isn't.
    Preset(usize),
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
                    vision: false,
                    context: 0,
                    cost_input: 0.0,
                    cost_output: 0.0,
                },
                crate::ModelChoice {
                    key: "fast".into(),
                    display: "Fast".into(),
                    detail: "fast".into(),
                    group: "Test".into(),
                    connection_id: String::new(),
                    vision: false,
                    context: 0,
                    cost_input: 0.0,
                    cost_output: 0.0,
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
            cost_input: 0.0,
            cost_output: 0.0,
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
        ));
        assert!(!handle_key(
            &mut app,
            Key {
                code: KeyCode::Char('m'),
                mods: KeyMods::NONE,
            },
            &tx,
            &interrupt,
        ));
        let pal = app.palette.as_ref().unwrap();
        assert!(pal.search_focused);
        assert_eq!(pal.query, "m");
    }

    #[test]
    fn picking_an_unconfigured_provider_asks_for_the_key() {
        use crate::app::palette::{ConnectRow, PaletteState};

        let mut app = test_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.connections = vec![hive_core::event::ConnectionInfo {
            id: "fireworks".into(),
            label: "Fireworks".into(),
            detail: "api.fireworks.ai".into(),
        }];
        app.active_connection = "fireworks".into();
        app.palette = Some(PaletteState::connect());

        // Row 0 is the configured provider: Enter switches to it.
        assert!(matches!(
            app.palette
                .as_ref()
                .and_then(|p| p.selected_connect(&app.connections)),
            Some(ConnectRow::Profile(_))
        ));
        assert!(!activate_palette(&mut app, &tx));
        assert!(matches!(
            rx.try_recv(),
            Ok(InputCommand::SetConnection { id }) if id == "fireworks"
        ));

        // Row 1 is a provider with no key: Enter goes straight to the key
        // prompt, with no "Add provider" detour in between.
        app.palette = Some(PaletteState::connect());
        if let Some(pal) = app.palette.as_mut() {
            pal.move_down(&[], &app.connections.clone(), &[]);
        }
        let idx = match app
            .palette
            .as_ref()
            .and_then(|p| p.selected_connect(&app.connections))
        {
            Some(ConnectRow::Preset(i)) => i,
            other => panic!("expected a preset row, got {other:?}"),
        };
        assert!(!activate_palette(&mut app, &tx));
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::ConnectKey { preset_idx: idx })
        );
        assert!(rx.try_recv().is_err(), "nothing sent until a key is typed");
    }

    #[test]
    fn ctrl_e_replaces_a_configured_provider_key() {
        let mut app = test_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        app.connections = vec![hive_core::event::ConnectionInfo {
            id: "openai".into(),
            label: "OpenAI".into(),
            detail: "api.openai.com".into(),
        }];
        app.palette = Some(crate::app::palette::PaletteState::connect());

        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('e')),
            &tx,
            &interrupt,
        ));
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::EditConnectionKey)
        );

        if let Some(pal) = app.palette.as_mut() {
            pal.query = "sk-new".into();
        }
        assert!(!activate_palette(&mut app, &tx));
        assert!(matches!(
            rx.try_recv(),
            Ok(InputCommand::UpdateConnectionKey { id, api_key })
                if id == "openai" && api_key == "sk-new"
        ));
    }

    /// Providers palette with one saved profile selected.
    fn provider_app() -> App {
        let mut app = test_app();
        app.connections = vec![
            hive_core::event::ConnectionInfo {
                id: "openai".into(),
                label: "OpenAI".into(),
                detail: "api.openai.com".into(),
            },
            hive_core::event::ConnectionInfo {
                id: "groq".into(),
                label: "Groq".into(),
                detail: "api.groq.com".into(),
            },
        ];
        app.active_connection = "openai".into();
        app.palette = Some(crate::app::palette::PaletteState::connect());
        app
    }

    /// Move the highlight onto the first provider that isn't set up.
    fn select_first_preset(app: &mut App) {
        use crate::app::palette::ConnectRow;
        for _ in 0..40 {
            if matches!(
                app.palette
                    .as_ref()
                    .and_then(|p| p.selected_connect(&app.connections)),
                Some(ConnectRow::Preset(_))
            ) {
                return;
            }
            let conns = app.connections.clone();
            if let Some(pal) = app.palette.as_mut() {
                pal.move_down(&[], &conns, &[]);
            }
        }
        panic!("no preset row to land on");
    }

    #[test]
    fn ctrl_e_on_an_unconfigured_provider_asks_for_its_key() {
        let mut app = provider_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        select_first_preset(&mut app);

        // It used to no-op here, which read as a dead binding.
        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('e')),
            &tx,
            &interrupt,
        ));
        assert!(matches!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::ConnectKey { .. })
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn ctrl_r_removes_the_highlighted_provider() {
        let mut app = provider_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('r')),
            &tx,
            &interrupt,
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(InputCommand::RemoveConnection { id }) if id == "openai"
        ));
    }

    #[test]
    fn ctrl_r_refuses_to_strand_you_without_a_provider() {
        let mut app = provider_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        app.connections.truncate(1);

        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('r')),
            &tx,
            &interrupt,
        ));
        assert!(rx.try_recv().is_err(), "nothing removed");

        // And on a provider that isn't set up there's nothing to remove.
        app.connections = provider_app().connections;
        select_first_preset(&mut app);
        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('r')),
            &tx,
            &interrupt,
        ));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_provider_list_update_cannot_redirect_a_key_edit() {
        let mut app = provider_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('e')),
            &tx,
            &interrupt,
        ));
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::EditConnectionKey)
        );

        // OpenAI slides to index 1 while the prompt is open.
        app.connections.insert(
            0,
            hive_core::event::ConnectionInfo {
                id: "fireworks".into(),
                label: "Fireworks".into(),
                detail: "api.fireworks.ai".into(),
            },
        );
        if let Some(pal) = app.palette.as_mut() {
            pal.query = "sk-new".into();
        }
        assert!(!activate_palette(&mut app, &tx));
        assert!(
            matches!(rx.try_recv(), Ok(InputCommand::UpdateConnectionKey { id, .. }) if id == "openai"),
            "the key must land on the provider that was highlighted"
        );
    }

    fn solid_rgba(w: usize, h: usize, px: [u8; 4]) -> Vec<u8> {
        px.iter().copied().cycle().take(w * h * 4).collect()
    }

    /// Composer with two attachments queued and focused.
    fn attached_app() -> App {
        let mut app = test_app();
        let png = encode_png(2, 2, &solid_rgba(2, 2, [0x40; 4])).expect("png");
        app.attach_clipboard_image(png.clone()).expect("first");
        app.attach_clipboard_image(png).expect("second");
        app.focus_input();
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        assert!(!handle_key(app, key(code), &tx, &interrupt));
    }

    fn press_ctrl(app: &mut App, code: KeyCode) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        assert!(!handle_key(app, ctrl(code), &tx, &interrupt));
    }

    #[test]
    fn down_steps_from_the_composer_onto_the_chips() {
        let mut app = attached_app();
        assert_eq!(app.selected_attach(), None);

        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_attach(), Some(0));

        // Arrows walk the chips and wrap.
        press(&mut app, KeyCode::Right);
        assert_eq!(app.selected_attach(), Some(1));
        press(&mut app, KeyCode::Right);
        assert_eq!(app.selected_attach(), Some(0));
        press(&mut app, KeyCode::Left);
        assert_eq!(app.selected_attach(), Some(1));

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.selected_attach(), None, "esc hands the keyboard back");
    }

    #[test]
    fn backspace_removes_the_highlighted_chip() {
        let mut app = attached_app();
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Right);
        assert_eq!(app.selected_attach(), Some(1));

        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.pending_attaches.len(), 1);
        assert_eq!(app.pending_attaches[0].label, "clipboard.png");
        assert_eq!(
            app.selected_attach(),
            Some(0),
            "selection lands on what's left"
        );

        // A second backspace clears the last one and returns to the composer.
        press(&mut app, KeyCode::Backspace);
        assert!(app.pending_attaches.is_empty());
        assert_eq!(app.selected_attach(), None);
    }

    #[test]
    fn backspace_still_edits_text_when_no_chip_is_selected() {
        let mut app = attached_app();
        app.input.value = "hi".into();
        app.input.cursor = 2;

        press(&mut app, KeyCode::Backspace);
        assert_eq!(app.input.value, "h", "composer keeps its own backspace");
        assert_eq!(app.pending_attaches.len(), 2, "nothing detached");
    }

    #[test]
    fn typing_returns_the_keyboard_to_the_composer() {
        let mut app = attached_app();
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_attach(), Some(0));

        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.selected_attach(), None);
        assert_eq!(app.input.value, "x", "the keystroke isn't swallowed");
        assert_eq!(app.pending_attaches.len(), 2);
    }

    #[test]
    fn down_still_scrolls_when_there_is_nothing_attached() {
        let mut app = test_app();
        app.focus_input();
        press(&mut app, KeyCode::Down);
        assert_eq!(app.selected_attach(), None);
    }

    #[test]
    fn ctrl_l_asks_for_a_full_repaint() {
        let mut app = test_app();
        assert!(!app.take_repaint_request());

        press_ctrl(&mut app, KeyCode::Char('l'));
        // Taken once by the draw loop, then it must not repaint forever.
        assert!(app.take_repaint_request());
        assert!(!app.take_repaint_request());
    }

    #[test]
    fn clipboard_rgba_becomes_a_readable_png() {
        let rgba = solid_rgba(3, 2, [0x11, 0x22, 0x33, 0xff]);
        let png = encode_png(3, 2, &rgba).expect("encoded");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "png magic");

        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
        let mut reader = decoder.read_info().expect("header");
        let mut out = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut out).expect("frame");
        assert_eq!((info.width, info.height), (3, 2));
        assert_eq!(&out[..info.buffer_size()], &rgba[..]);
    }

    #[test]
    fn a_truncated_clipboard_image_is_refused() {
        // Fewer bytes than width * height * 4 — encoding would panic or emit junk.
        assert!(encode_png(4, 4, &solid_rgba(2, 2, [0xff; 4])).is_none());
    }

    #[test]
    fn pasted_images_queue_up_instead_of_overwriting() {
        let mut app = test_app();
        let png = encode_png(2, 2, &solid_rgba(2, 2, [0xff; 4])).expect("png");

        assert_eq!(
            app.attach_clipboard_image(png.clone()),
            Ok("@clipboard.png".into())
        );
        assert_eq!(
            app.attach_clipboard_image(png.clone()),
            Ok("@clipboard-2.png".into())
        );
        assert_eq!(
            app.pending_attaches.len(),
            2,
            "second paste must not vanish"
        );
        assert!(app
            .pending_attaches
            .iter()
            .all(|a| matches!(&a.image, Some(hive_core::message::ImageSource::Base64 { media_type, data })
                if media_type == "image/png" && !data.is_empty())));
    }

    #[test]
    fn an_oversized_clipboard_image_is_reported_not_sent() {
        let mut app = test_app();
        let huge = vec![0u8; 11 * 1024 * 1024];
        let err = app.attach_clipboard_image(huge).expect_err("refused");
        assert!(err.contains("too large"), "{err}");
        assert!(app.pending_attaches.is_empty());
    }

    #[test]
    fn a_pasted_image_is_sent_as_a_vision_part() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let png = encode_png(2, 2, &solid_rgba(2, 2, [0x40; 4])).expect("png");
        app.attach_clipboard_image(png).expect("attached");
        app.input.value = "what is this".into();
        app.input.cursor = app.input.value.chars().count();

        assert!(!submit(&mut app, &tx));
        let pd = app.pending_dispatch.as_ref().expect("dispatched");
        assert_eq!(pd.images.len(), 1, "the image rides along with the prompt");
        assert!(
            !pd.agent_text.contains("[Attached file:"),
            "an image isn't a file note: {}",
            pd.agent_text
        );
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
                pal.move_down(
                    &app.model_choices.clone(),
                    &app.connections.clone(),
                    &app.saved_sessions,
                );
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
        assert!(matches!(rx.try_recv(), Ok(InputCommand::FetchModels)));
        app.models_catalog = crate::app::ModelsCatalogState::Ready;
        if let Some(pal) = app.palette.as_mut() {
            pal.clamp_selection(
                &app.model_choices.clone(),
                &app.connections.clone(),
                &app.saved_sessions,
            );
        }
        assert!(!activate_palette(&mut app, &tx));
        match rx.try_recv() {
            Ok(InputCommand::SetModel {
                id,
                display,
                connection_id,
                vision,
                context: _,
                cost_input: _,
                cost_output: _,
            }) => {
                assert_eq!(id, "default");
                assert_eq!(display, "Default");
                assert!(connection_id.is_none());
                assert!(!vision);
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
        assert!(!submit(&mut app, &tx));
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
            mode: AgentMode::Make,
        });
        assert!(app.recall_follow_up());
        assert!(!app.has_follow_up());
        assert_eq!(app.input.value, "queued text");
    }

    #[test]
    fn follow_up_second_enter_does_not_inject_mid_task() {
        let mut app = test_app();
        app.running = true;
        app.queue_follow_up(crate::app::QueuedFollowUp {
            display: "now".into(),
            text: "now".into(),
            composer: "now".into(),
            attaches: Vec::new(),
            mode: AgentMode::Make,
        });
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(app.input.is_empty());
        assert!(!submit(&mut app, &tx));
        assert!(
            app.has_follow_up(),
            "follow-up must stay queued while running"
        );
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
            items
                .iter()
                .map(|i| i.name().to_string())
                .collect::<Vec<_>>()
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
                pal.move_down(
                    &app.model_choices.clone(),
                    &app.connections.clone(),
                    &app.saved_sessions,
                );
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
        assert!(!handle_key(
            &mut app,
            Key {
                code: KeyCode::Esc,
                mods: KeyMods::NONE,
            },
            &tx,
            &interrupt,
        ));
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
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt,));
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
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt,));
        assert!(app.palette_open());
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::Commands)
        );
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt,));
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

    fn seed_assistant_mouse_row(app: &mut App) {
        use crate::app::state::{AssistantRowHit, AssistantRowJoin};

        app.assistant_row_hits = vec![AssistantRowHit {
            block: 2,
            response_row: 0,
            screen_row: 5,
            x: 2,
            text: "select this answer".into(),
            join_before: AssistantRowJoin::Hard,
        }];
    }

    #[test]
    fn assistant_mouse_drag_builds_selection() {
        let mut app = test_app();
        seed_assistant_mouse_row(&mut app);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 2,
                row: 5,
            },
            &tx
        ));
        assert!(handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Drag,
                col: 7,
                row: 5,
            },
            &tx
        ));
        let selection = app.assistant_selection.as_ref().expect("selection");
        assert!(selection.dragged);
        assert_eq!(selection.block, 2);
    }

    #[test]
    fn ordinary_click_clears_previous_assistant_selection() {
        let mut app = test_app();
        seed_assistant_mouse_row(&mut app);
        assert!(app.start_assistant_selection(2, 5));
        assert!(app.update_assistant_selection(7, 5));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 40,
                row: 20,
            },
            &tx
        ));
        assert!(app.assistant_selection.is_none());
    }

    #[test]
    fn assistant_selection_does_not_steal_sidebar_resize() {
        let mut app = test_app();
        seed_assistant_mouse_row(&mut app);
        app.sidebar_resize_hit = Some(comb::Rect::new(2, 5, 1, 1));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 2,
                row: 5,
            },
            &tx
        ));
        assert!(app.sidebar_resizing);
        assert!(app.assistant_selection.is_none());
    }

    #[test]
    fn sidebar_terminal_row_click_opens_terminal_view() {
        use hive_core::event::AgentEvent;
        let mut app = test_app();
        app.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "theme-installer".into(),
            description: String::new(),
            rows: 12,
            cols: 40,
        });
        app.sidebar_item_hits = vec![(
            comb::Rect::new(10, 8, 20, 1),
            render::SidebarItem::Terminal("term-1".into()),
        )];
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 12,
                row: 8,
            },
            &tx
        ));
        assert!(app.in_terminal_view());
        assert_eq!(app.terminal_view_id(), Some("term-1"));
    }

    #[test]
    fn ordinary_click_keeps_assistant_hit_map_available() {
        let mut app = test_app();
        seed_assistant_mouse_row(&mut app);
        app.input_hit = Some(comb::Rect::new(10, 20, 20, 2));
        app.focus_input();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(!handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 12,
                row: 20,
            },
            &tx
        ));
        assert!(app.start_assistant_selection(2, 5));
    }

    #[test]
    fn completed_assistant_selection_ignores_later_mouse_up() {
        let mut app = test_app();
        seed_assistant_mouse_row(&mut app);
        assert!(app.start_assistant_selection(2, 5));
        assert!(app.update_assistant_selection(7, 5));
        assert!(app.finish_assistant_selection(7, 5).is_some());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(!handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Up,
                col: 7,
                row: 5,
            },
            &tx
        ));
    }

    #[test]
    fn scrolling_keeps_an_active_assistant_drag() {
        let mut app = test_app();
        seed_assistant_mouse_row(&mut app);
        app.set_transcript_max_scroll(10);
        assert!(app.start_assistant_selection(2, 5));
        assert!(app.update_assistant_selection(7, 5));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::ScrollUp,
                col: 2,
                row: 5,
            },
            &tx
        ));
        assert!(app.assistant_selection_active());
        assert_eq!(app.scroll_from_bottom, 3);
    }

    #[test]
    fn paste_inserts_multiline_into_composer() {
        let mut app = test_app();
        app.blur_input();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(handle_paste(&mut app, "hello\r\nworld", &tx));
        assert!(app.input_focused);
        assert_eq!(app.input.value, "hello\nworld");
        assert_eq!(app.input.cursor, app.input.value.chars().count());
    }

    #[test]
    fn clean_for_copy_strips_padding_keeps_structure() {
        // Trailing spaces gone, surrounding blank lines gone, but interior blank
        // lines and leading indentation (code) preserved.
        let raw = "\n\nHello world   \n\n    let x = 1;  \n\n";
        assert_eq!(super::clean_for_copy(raw), "Hello world\n\n    let x = 1;");
        assert_eq!(super::clean_for_copy("   \n  \n"), "");
    }

    #[test]
    fn paste_into_connect_key_strips_whitespace() {
        let mut app = test_app();
        app.palette = Some(crate::app::palette::PaletteState::connect_key(0));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(handle_paste(&mut app, " sk-abc \n", &tx));
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
            cost_input: 0.0,
            cost_output: 0.0,
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
        assert!(!submit(&mut app, &tx));
        let _ = std::fs::remove_file(&path);
        // submit() defers the dispatch — flush it manually in tests.
        if let Some(pd) = app.take_pending_dispatch() {
            let _ = tx.send(InputCommand::User {
                text: pd.agent_text,
                images: pd.images,
                mode: pd.mode,
            });
        }
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
        ));
        assert!(app.input_focused);
        assert_eq!(app.input.value, "h");

        assert!(!handle_key(
            &mut app,
            key(KeyCode::Char('i')),
            &tx,
            &interrupt,
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

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt,));
        assert!(!app.input_focused);
        assert!(app.input.is_empty());
        assert_eq!(app.scroll_from_bottom, 1);

        assert!(!handle_key(&mut app, key(KeyCode::Down), &tx, &interrupt,));
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

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt,));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 1);

        assert!(!handle_key(&mut app, key(KeyCode::Down), &tx, &interrupt,));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 0);

        // Printable typing still auto-focuses after idle blur.
        assert!(!handle_key(
            &mut app,
            key(KeyCode::Char('x')),
            &tx,
            &interrupt,
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

        assert!(!handle_key(&mut app, key(KeyCode::Left), &tx, &interrupt,));
        assert!(app.input_focused, "Left should restore composer focus");
        assert_eq!(app.input.cursor, 4);

        app.blur_input();
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt,));
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

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt,));
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt,));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 2, "Up stops at top");

        // Further Up cannot scroll — hand focus back to the composer.
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt,));
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
        assert!(!handle_key(&mut app, key(KeyCode::Esc), &tx, &interrupt,));
        assert!(!app.about_open());
        assert!(!app.input_focused);

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt,));
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
        ));
        assert!(app.in_subagent_view());
        assert!(!app.input_focused);
        assert!(app.input.is_empty());
    }

    fn terminal_app(
        controller: hive_core::TerminalController,
        process: hive_core::TerminalProcessState,
    ) -> App {
        let mut app = test_app();
        app.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "cat".into(),
            description: String::new(),
            rows: 20,
            cols: 80,
        });
        app.apply(AgentEvent::TerminalState {
            id: "term-1".into(),
            controller,
            process,
            revision: 1,
        });
        app.open_terminal_view("term-1".into());
        app
    }

    fn terminal_input(rx: &mut tokio::sync::mpsc::UnboundedReceiver<InputCommand>) -> Vec<u8> {
        match rx.try_recv().expect("terminal input command") {
            InputCommand::TerminalInput { id, input } => {
                assert_eq!(id, "term-1");
                input.into_bytes()
            }
            other => panic!("expected private terminal input, got {other:?}"),
        }
    }

    #[test]
    fn ctrl_close_bracket_detaches_without_stopping() {
        let mut app = terminal_app(
            hive_core::TerminalController::User,
            hive_core::TerminalProcessState::Running,
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char(']')),
            &tx,
            &interrupt,
        ));
        assert!(!app.in_terminal_view());
        assert!(matches!(
            rx.try_recv(),
            Ok(InputCommand::TerminalDetach { id }) if id == "term-1"
        ));
        assert!(rx.try_recv().is_err(), "detach must not stop the process");
    }

    #[test]
    fn ctrl_close_bracket_hands_back_user_owned_exited_session() {
        let mut app = terminal_app(
            hive_core::TerminalController::User,
            hive_core::TerminalProcessState::Exited { code: 0 },
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        handle_key(&mut app, ctrl(KeyCode::Char(']')), &tx, &interrupt);
        assert!(matches!(
            rx.try_recv(),
            Ok(InputCommand::TerminalDetach { id }) if id == "term-1"
        ));
        assert!(!app.in_terminal_view());
    }

    #[test]
    fn escape_and_ctrl_c_go_to_pty_not_hive() {
        let mut app = terminal_app(
            hive_core::TerminalController::User,
            hive_core::TerminalProcessState::Running,
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(&mut app, key(KeyCode::Esc), &tx, &interrupt,));
        assert_eq!(terminal_input(&mut rx), b"\x1b");
        assert!(!handle_key(
            &mut app,
            ctrl(KeyCode::Char('c')),
            &tx,
            &interrupt,
        ));
        assert_eq!(terminal_input(&mut rx), b"\x03");
        assert!(!interrupt.load(Ordering::Relaxed));
        assert!(app.in_terminal_view());
    }

    #[test]
    fn paste_goes_to_pty_only_under_user_control() {
        let mut user = terminal_app(
            hive_core::TerminalController::User,
            hive_core::TerminalProcessState::Running,
        );
        user.apply(AgentEvent::TerminalOutput {
            id: "term-1".into(),
            frame: hive_core::TerminalOutputFrame::from_bytes(20, 80, 10_000, b"\x1b[?2004h", 2),
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(handle_paste(&mut user, "secret", &tx));
        assert_eq!(terminal_input(&mut rx), b"\x1b[200~secret\x1b[201~");

        let mut attaching = terminal_app(
            hive_core::TerminalController::Agent,
            hive_core::TerminalProcessState::Running,
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(!handle_paste(&mut attaching, "blocked", &tx));
        assert!(rx.try_recv().is_err());
        assert!(attaching.input.is_empty());
    }

    #[test]
    fn attaching_and_read_only_views_swallow_process_input() {
        for mut app in [
            terminal_app(
                hive_core::TerminalController::Agent,
                hive_core::TerminalProcessState::Running,
            ),
            terminal_app(
                hive_core::TerminalController::Agent,
                hive_core::TerminalProcessState::Exited { code: 0 },
            ),
        ] {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let interrupt = Arc::new(AtomicBool::new(false));
            assert!(!handle_key(
                &mut app,
                key(KeyCode::Char('x')),
                &tx,
                &interrupt,
            ));
            assert!(rx.try_recv().is_err());
            assert!(app.input.is_empty());
        }
    }

    #[test]
    fn mouse_back_detaches_and_stop_hit_stops() {
        let mut app = terminal_app(
            hive_core::TerminalController::User,
            hive_core::TerminalProcessState::Running,
        );
        app.back_hit = Some(comb::Rect::new(0, 20, 72, 3));
        app.terminal_stop_hit = Some(comb::Rect::new(72, 20, 8, 3));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 75,
                row: 21,
            },
            &tx,
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(InputCommand::TerminalStop { id }) if id == "term-1"
        ));
        assert!(app.in_terminal_view());

        assert!(handle_mouse(
            &mut app,
            Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 2,
                row: 21,
            },
            &tx,
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(InputCommand::TerminalDetach { id }) if id == "term-1"
        ));
        assert!(!app.in_terminal_view());
    }

    #[test]
    fn terminal_resize_uses_body_dimensions_and_is_deduplicated() {
        let mut app = terminal_app(
            hive_core::TerminalController::User,
            hive_core::TerminalProcessState::Running,
        );
        let size = comb::Size::new(100, 30);
        let _ = comb::render(size, |frame| crate::render::draw(frame, &mut app));
        let body = app.terminal_view.body_rect.unwrap();
        let first = app.take_pending_terminal_resize().unwrap();
        assert_eq!(first, ("term-1".into(), body.height, body.width));
        assert!(body.height > 0 && body.width > 0);

        let _ = comb::render(size, |frame| crate::render::draw(frame, &mut app));
        assert!(app.take_pending_terminal_resize().is_none());
    }

    #[test]
    fn agent_controlled_terminal_view_does_not_resize_pty() {
        let mut app = terminal_app(
            hive_core::TerminalController::Agent,
            hive_core::TerminalProcessState::Running,
        );
        let size = comb::Size::new(100, 30);
        let _ = comb::render(size, |frame| crate::render::draw(frame, &mut app));
        assert!(app.take_pending_terminal_resize().is_none());
    }

    #[test]
    fn shift_page_scrolls_history_but_plain_page_reaches_child() {
        let mut app = terminal_app(
            hive_core::TerminalController::User,
            hive_core::TerminalProcessState::Running,
        );
        app.apply(AgentEvent::TerminalOutput {
            id: "term-1".into(),
            frame: hive_core::TerminalOutputFrame::from_bytes(
                20,
                80,
                10_000,
                &(0..100)
                    .map(|line| format!("line {line}\r\n"))
                    .collect::<String>()
                    .into_bytes(),
                2,
            ),
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        let shift_page_up = Key {
            code: KeyCode::PageUp,
            mods: KeyMods::SHIFT,
        };
        assert!(!handle_key(&mut app, shift_page_up, &tx, &interrupt,));
        assert!(app.terminal_view.scrollback > 0);
        assert!(rx.try_recv().is_err());

        assert!(!handle_key(&mut app, key(KeyCode::PageUp), &tx, &interrupt,));
        assert_eq!(terminal_input(&mut rx), b"\x1b[5~");
    }

    #[test]
    fn direct_terminal_input_never_changes_composer_or_user_blocks() {
        let mut app = terminal_app(
            hive_core::TerminalController::User,
            hive_core::TerminalProcessState::Running,
        );
        app.input.insert_str("draft");
        let user_blocks = app
            .blocks
            .iter()
            .filter(|block| matches!(block, crate::app::state::Block::User(_)))
            .count();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        handle_key(&mut app, key(KeyCode::Char('x')), &tx, &interrupt);
        handle_paste(&mut app, "secret", &tx);

        assert_eq!(app.input.value, "draft");
        assert_eq!(
            app.blocks
                .iter()
                .filter(|block| matches!(block, crate::app::state::Block::User(_)))
                .count(),
            user_blocks
        );
    }

    #[test]
    fn event_drain_is_bounded_so_terminal_input_stays_responsive() {
        let mut app = test_app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for index in 0..300 {
            tx.send(AgentEvent::Notice(format!("event {index}")))
                .unwrap();
        }
        let (input_tx, _input_rx) = tokio::sync::mpsc::unbounded_channel();

        assert!(drain_events(&mut app, &mut rx, &input_tx));
        assert!(
            rx.try_recv().is_ok(),
            "one UI iteration drained an unbounded event stream"
        );
    }
}
