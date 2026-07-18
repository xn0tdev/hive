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

use crate::app::App;
use crate::render;
use crate::{InputCommand, TuiInit};

const HELP: &str =
    "commands: /model <id-or-role> · /image <path> · /copy · /new · /clear · /cost · /quit — enter send, shift+enter newline, esc stops, ctrl+t thoughts, double ctrl+c quits";

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
    let tick = Duration::from_millis(90);
    loop {
        while let Ok(ev) = events.try_recv() {
            app.apply(ev);
        }

        terminal.draw(|f| render::draw(f, app))?;

        if let Some(ev) = terminal.read_event(tick)? {
            match ev {
                Event::Key(key) => {
                    if handle_key(app, key, input_tx, interrupt) {
                        break;
                    }
                }
                Event::Mouse(m) => handle_mouse(app, m),
                Event::Resize(_, _) => {}
            }
        }

        app.tick();
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

    let subagent = app.in_subagent_view();
    let menu_open =
        !subagent && app.slash_prefix().is_some() && !app.menu_items().is_empty();

    // Any key other than ctrl+c cancels a pending quit confirmation.
    if !(ctrl && key.code == KeyCode::Char('c')) {
        app.disarm_quit();
    }

    // Read-only subagent view: scroll + leave; no typing / submit.
    if subagent {
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

    match key.code {
        KeyCode::Char('q') if ctrl => return true,
        KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
        KeyCode::Char('y') if ctrl => copy_last_answer(app),
        KeyCode::Char('t') if ctrl => app.toggle_thoughts(),
        KeyCode::Tab if menu_open => complete_selected(app),
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
        KeyCode::PageUp => app.scroll_up(5),
        KeyCode::PageDown => app.scroll_down(5),
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
            }
        }
        _ => {}
    }
    false
}

/// Wheel scrolls the transcript; left-click toggles thoughts, opens subagent
/// chats, hits `← back`, or bonks the logo.
fn handle_mouse(app: &mut App, m: Mouse) {
    match m.kind {
        MouseKind::ScrollUp => app.scroll_up(3),
        MouseKind::ScrollDown => app.scroll_down(3),
        MouseKind::Down(MouseButton::Left) => {
            if app.is_empty_chat() && app.bonk_logo_at(m.col, m.row) {
                return;
            }
            if app.in_subagent_view() && app.back_hit_row == Some(m.row) {
                app.leave_subagent_view();
                return;
            }
            if let Some(idx) = app.expandable_at_row(m.row) {
                app.activate_expandable_at(idx);
            }
        }
        _ => {}
    }
}

/// Copy the last finished answer to the system clipboard via OSC 52 (works in
/// most modern terminals). Feedback goes to the footer, not the chat.
fn copy_last_answer(app: &mut App) {
    let Some(text) = app.last_answer().map(|s| s.to_string()) else {
        app.flash("nothing to copy yet");
        return;
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]52;c;{b64}\x07");
    let _ = out.flush();
    app.flash("answer copied to clipboard");
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
    let _ = input_tx.send(InputCommand::User { text, images });
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
            app.flash("new chat");
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
