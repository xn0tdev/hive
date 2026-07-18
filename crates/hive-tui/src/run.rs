//! Terminal setup and the blocking event loop. Translates key events into
//! edits, scrolling, slash commands, and `InputCommand`s for the agent driver.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use comb::{Event, Key, KeyCode, Mouse, MouseButton, MouseKind, Terminal};
use tokio::sync::mpsc::UnboundedSender;

use hive_core::event::EventReceiver;
use hive_core::message::ImageSource;
use hive_core::AgentMode;

use crate::app::App;
use crate::render;
use crate::{InputCommand, TuiInit};

const HELP: &str =
    "commands: /model <id-or-role> · /image <path> · /copy · /new · /clear · /cost · /quit — tab plan/build, enter send, shift+enter newline, esc stops, ctrl+t thoughts, double ctrl+c quits";

/// Poll interval while something is animating (spinner / shimmer).
const ANIM_TICK: Duration = Duration::from_millis(100);
/// Idle poll — long enough to skip needless redraws, short enough for input.
const IDLE_TICK: Duration = Duration::from_millis(250);

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
        let mut content_dirty = false;
        while let Ok(ev) = events.try_recv() {
            if app.apply(ev) {
                content_dirty = true;
            }
        }

        app.tick();
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
            }
        }
    }
    Ok(())
}

/// Returns true when the user asked to quit.
fn handle_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
    interrupt: &Arc<AtomicBool>,
) -> bool {
    let ctrl = key.mods.ctrl;
    let alt = key.mods.alt;
    let shift = key.mods.shift;

    let special = app.in_special_view();
    let menu_open = !special && app.slash_prefix().is_some() && !app.menu_items().is_empty();

    // Any key other than ctrl+c cancels a pending quit confirmation.
    if !(ctrl && key.code == KeyCode::Char('c')) {
        app.disarm_quit();
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

    // Global chords work whether or not the composer is focused.
    match key.code {
        KeyCode::Char('q') if ctrl => return true,
        KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
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

    // Blurred: no caret, ignore typing / submit / cursor motion. Arrows scroll.
    if !app.input_focused {
        match key.code {
            KeyCode::Up => app.scroll_up(1),
            KeyCode::Down => app.scroll_down(1),
            KeyCode::Esc if app.running => {
                interrupt.store(true, Ordering::Relaxed);
            }
            _ => {}
        }
        return false;
    }

    match key.code {
        KeyCode::Tab if menu_open => complete_selected(app),
        KeyCode::Tab if !menu_open => {
            app.toggle_agent_mode();
        }
        KeyCode::Enter if menu_open => {
            // Commands with an argument get completed; the rest run at once.
            if let Some(cmd) = app.menu_selected() {
                if cmd.takes_arg {
                    complete_selected(app);
                } else {
                    app.input.take();
                    app.reset_menu();
                    return handle_slash(app, cmd.name, input_tx);
                }
            }
        }
        KeyCode::Enter => {
            // Enter sends; Shift/Alt/Ctrl+Enter insert a newline.
            // (`\n` parses as Enter+SHIFT; kitty/xterm send CSI with mods.)
            if shift || alt || ctrl {
                app.input.newline();
            } else {
                return submit(app, input_tx);
            }
        }
        // Ctrl+J → newline when the host remaps it to Char('j')+CTRL.
        KeyCode::Char('j') if ctrl => {
            app.input.newline();
        }
        KeyCode::Char(ch) if !ctrl => {
            app.input.insert(ch);
            app.reset_menu();
        }
        KeyCode::Backspace => {
            app.input.backspace();
            app.reset_menu();
        }
        KeyCode::Left => app.input.left(),
        KeyCode::Right => app.input.right(),
        KeyCode::Home => app.input.home(),
        KeyCode::End => app.input.end(),
        KeyCode::Up if menu_open => app.menu_up(),
        KeyCode::Down if menu_open => app.menu_down(),
        // Multiline: arrows move inside the input; at the edges they scroll chat.
        KeyCode::Up => {
            if !app.input.up() {
                app.scroll_up(1);
            }
        }
        KeyCode::Down => {
            if !app.input.down() {
                app.scroll_down(1);
            }
        }
        KeyCode::Esc => {
            if app.running {
                // Stop the turn. The agent reports "Interrupted." in the chat
                // only once it has actually stopped.
                interrupt.store(true, Ordering::Relaxed);
            } else if !app.input.is_empty() {
                let _ = app.input.take();
                app.reset_menu();
            } else {
                app.blur_input();
            }
        }
        _ => {}
    }
    false
}

/// Wheel scrolls the transcript; left-click toggles thoughts, opens subagent
/// chats, hits `← back` / Build, or bonks the logo.
/// Returns `true` when state changed and a redraw is needed.
fn handle_mouse(app: &mut App, m: Mouse, input_tx: &UnboundedSender<InputCommand>) -> bool {
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
            if let Some(hit) = app.sidebar_toggle_hit {
                if hit.contains(m.col, m.row) {
                    app.toggle_sidebar();
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
        // Motion / drag / release must not thrash the redraw loop.
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
                app.input_focused = false;
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
                } else {
                    app.plan_view.selected.clear();
                    app.plan_view.amending = false;
                    app.input_focused = false;
                }
                return false;
            }
            KeyCode::Char(ch) if !ctrl => {
                app.input.insert(ch);
                return false;
            }
            KeyCode::Backspace => {
                app.input.backspace();
                return false;
            }
            KeyCode::Left => app.input.left(),
            KeyCode::Right => app.input.right(),
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

/// Replace the input with the selected command (plus a space if it wants an
/// argument), keeping the user typing.
fn complete_selected(app: &mut App) {
    if let Some(cmd) = app.menu_selected() {
        let text = if cmd.takes_arg {
            format!("/{} ", cmd.name)
        } else {
            format!("/{}", cmd.name)
        };
        app.input.value = text;
        app.input.end();
        app.reset_menu();
    }
}

fn submit(app: &mut App, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let text = app.input.take();
    let trimmed = text.trim().to_string();

    if trimmed.is_empty() && app.pending_images.is_empty() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix('/') {
        return handle_slash(app, rest, input_tx);
    }

    let display = if trimmed.is_empty() {
        "[image]".to_string()
    } else {
        text.clone()
    };
    app.push_user(display);
    let images = app.take_pending_images();
    let mode = app.agent_mode;
    let _ = input_tx.send(InputCommand::User { text, images, mode });
    false
}

fn handle_slash(app: &mut App, cmd: &str, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let mut parts = cmd.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    match name {
        "quit" | "q" | "exit" => return true,
        "clear" | "reset" | "new" => {
            // Full new chat → centered Home landing (no leftover Notice blocks).
            app.new_chat();
            let _ = input_tx.send(InputCommand::Clear);
            app.flash("New chat");
        }
        "model" => {
            if arg.is_empty() {
                app.notice("usage: /model <id-or-role>  (roles: default, smart, fast, vision)");
            } else {
                let _ = input_tx.send(InputCommand::SetModel(arg.to_string()));
            }
        }
        "image" => load_image(app, arg),
        "copy" => copy_last_answer(app),
        "cost" => app.notice(format!(
            "tokens — prompt {} · completion {} · total {}",
            app.usage.prompt_tokens, app.usage.completion_tokens, app.usage.total_tokens
        )),
        "help" => app.notice(HELP),
        other => app.notice(format!("unknown command: /{other}  ({HELP})")),
    }
    false
}

fn load_image(app: &mut App, path: &str) {
    if path.is_empty() {
        app.notice("usage: /image <path>");
        return;
    }
    match std::fs::read(path) {
        Ok(bytes) => {
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            app.add_pending_image(ImageSource::Base64 {
                media_type: media_type(path),
                data,
            });
            app.notice(format!("Attached image: {path}"));
        }
        Err(e) => app.notice(format!("cannot read image {path}: {e}")),
    }
}

fn media_type(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    }
    .to_string()
}
