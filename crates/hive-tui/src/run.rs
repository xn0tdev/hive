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
    let menu_open = !special
        && !app.palette_open()
        && !app.about_open()
        && app.slash_prefix().is_some()
        && !app.menu_items().is_empty();

    // Any key other than ctrl+c cancels a pending quit confirmation.
    if !(ctrl && key.code == KeyCode::Char('c')) {
        app.disarm_quit();
    }

    // About overlay captures keys while open (esc / Ctrl+P).
    if app.about_open() {
        return handle_about_key(app, key);
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
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Home
            | KeyCode::End => {
                app.focus_input();
            }
            KeyCode::Esc if app.running => {
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
        KeyCode::Enter if menu_open => {
            // Commands with an argument get completed; the rest run at once.
            if let Some(cmd) = app.menu_selected() {
                if cmd.takes_arg {
                    complete_selected(app);
                    composer_activity = true;
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
        KeyCode::Up => {
            if app.input.up() {
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
            if app.running {
                // Stop the turn. The agent reports "Interrupted." in the chat
                // only once it has actually stopped.
                interrupt.store(true, Ordering::Relaxed);
            } else if !app.input.is_empty() {
                let _ = app.input.take();
                app.reset_menu();
                composer_activity = true;
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
fn handle_mouse(
    app: &mut App,
    m: Mouse,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    if app.about_open() || app.palette_open() {
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
    let images: Vec<ImageSource> = attaches.into_iter().filter_map(|a| a.image).collect();

    let display = match (tags.is_empty(), trimmed.is_empty()) {
        (true, true) => return false,
        (false, true) => tags,
        (true, false) => text.clone(),
        (false, false) => format!("{tags} {text}"),
    };

    let mut agent_text = text;
    if !file_notes.is_empty() {
        if !agent_text.is_empty() {
            agent_text.push('\n');
        }
        agent_text.push_str(&file_notes.join("\n"));
    }

    app.push_user(display);
    let mode = app.agent_mode;
    let _ = input_tx.send(InputCommand::User {
        text: agent_text,
        images,
        mode,
    });
    false
}

fn handle_slash(app: &mut App, cmd: &str, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let mut parts = cmd.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    // Removed command: steer users to attach / paste.
    if name.eq_ignore_ascii_case("image") {
        app.notice(" /image was removed — paste a path or use /attach <path>");
        return false;
    }

    let Some(def) = commands::resolve(name) else {
        app.notice(format!("unknown command: /{name}  — {UNKNOWN_HINT}"));
        return false;
    };

    run_command(app, def.id, arg, input_tx)
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
        CmdId::Model => {
            if arg.is_empty() {
                app.open_model_picker();
            } else {
                let _ = input_tx.send(InputCommand::SetModel(arg.to_string()));
            }
        }
        CmdId::Attach => match app.attach_path(arg) {
            Ok(tag) => app.flash(format!("Attached {tag}")),
            Err(e) => app.notice(e),
        },
        CmdId::Copy => copy_last_answer(app),
        CmdId::Cost => app.notice(format!(
            "tokens — prompt {} · completion {} · total {}",
            app.usage.prompt_tokens, app.usage.completion_tokens, app.usage.total_tokens
        )),
        CmdId::About => app.open_about(),
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

fn handle_palette_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    let ctrl = key.mods.ctrl;
    let choices = app.model_choices.clone();

    match key.code {
        KeyCode::Esc => {
            if let Some(pal) = app.palette.as_ref() {
                if pal.mode == PaletteMode::Models {
                    // Back to the main command list.
                    app.open_palette();
                } else {
                    app.close_palette();
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
                    pal.move_up_visible(&choices, visible);
                } else {
                    pal.move_up(&choices);
                }
            }
        }
        KeyCode::Down => {
            let visible = app.palette_list_visible as usize;
            if let Some(pal) = app.palette.as_mut() {
                if visible > 0 {
                    pal.move_down_visible(&choices, visible);
                } else {
                    pal.move_down(&choices);
                }
            }
        }
        KeyCode::Enter => return activate_palette(app, input_tx),
        KeyCode::Backspace => {
            if let Some(pal) = app.palette.as_mut() {
                pal.backspace(&choices);
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
                pal.insert(ch, &choices);
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
            let key = app
                .palette
                .as_ref()
                .and_then(|p| p.selected_model(&app.model_choices))
                .map(|c| c.key.clone());
            app.close_palette();
            if let Some(key) = key {
                let _ = input_tx.send(InputCommand::SetModel(key));
                app.flash("Switching model…");
            }
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
                    app.open_model_picker();
                    false
                }
                Some(CmdId::Attach) => {
                    app.focus_input();
                    app.input.value = "/attach ".into();
                    app.input.cursor = app.input.value.chars().count();
                    app.note_input_activity();
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
                },
                crate::ModelChoice {
                    key: "fast".into(),
                    display: "Fast".into(),
                    detail: "fast".into(),
                },
            ],
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
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
            &interrupt
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
            &interrupt
        ));
        assert!(!handle_key(
            &mut app,
            Key {
                code: KeyCode::Char('m'),
                mods: KeyMods::NONE,
            },
            &tx,
            &interrupt
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
                pal.move_down(&app.model_choices.clone());
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
        assert!(!activate_palette(&mut app, &tx));
        match rx.try_recv() {
            Ok(InputCommand::SetModel(k)) => assert_eq!(k, "default"),
            other => panic!("expected SetModel, got {other:?}"),
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
                pal.move_down(&app.model_choices.clone());
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
            &interrupt
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
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt));
        assert!(!app.palette_open());
    }

    #[test]
    fn palette_models_esc_returns_to_commands() {
        let mut app = test_app();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        app.open_model_picker();
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::Models)
        );
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt));
        assert!(app.palette_open());
        assert_eq!(
            app.palette.as_ref().map(|p| p.mode),
            Some(PaletteMode::Commands)
        );
        assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt));
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
    fn attach_image_path_without_image_command() {
        let path = std::env::temp_dir().join("hive-tui-attach-test.png");
        std::fs::write(&path, [0x89, 0x50, 0x4e, 0x47]).unwrap();
        let mut app = test_app();
        let tag = app.attach_path(path.to_str().unwrap()).unwrap();
        assert_eq!(tag, "[Image #1]");
        assert!(app.has_pending_attaches());
        assert!(commands::resolve("image").is_none());

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.input.value = "look".into();
        assert!(!submit(&mut app, &tx));
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
            &interrupt
        ));
        assert!(app.input_focused);
        assert_eq!(app.input.value, "h");

        assert!(!handle_key(
            &mut app,
            key(KeyCode::Char('i')),
            &tx,
            &interrupt
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

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt));
        assert!(!app.input_focused);
        assert!(app.input.is_empty());
        assert_eq!(app.scroll_from_bottom, 1);

        assert!(!handle_key(&mut app, key(KeyCode::Down), &tx, &interrupt));
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

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 1);

        assert!(!handle_key(&mut app, key(KeyCode::Down), &tx, &interrupt));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 0);

        // Printable typing still auto-focuses after idle blur.
        assert!(!handle_key(
            &mut app,
            key(KeyCode::Char('x')),
            &tx,
            &interrupt
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

        assert!(!handle_key(&mut app, key(KeyCode::Left), &tx, &interrupt));
        assert!(app.input_focused, "Left should restore composer focus");
        assert_eq!(app.input.cursor, 4);

        app.blur_input();
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt));
        assert!(
            app.input_focused,
            "Up with nothing to scroll should focus, not no-op"
        );
        assert_eq!(app.scroll_from_bottom, 0, "must not accumulate phantom scroll");
    }

    #[test]
    fn blurred_up_clamps_to_max_scroll() {
        let mut app = test_app();
        app.set_transcript_max_scroll(2);
        app.blur_input();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt));
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt));
        assert!(!app.input_focused);
        assert_eq!(app.scroll_from_bottom, 2, "Up stops at top");

        // Further Up cannot scroll — hand focus back to the composer.
        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt));
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
        assert!(!handle_key(&mut app, key(KeyCode::Esc), &tx, &interrupt));
        assert!(!app.about_open());
        assert!(!app.input_focused);

        assert!(!handle_key(&mut app, key(KeyCode::Up), &tx, &interrupt));
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
            &interrupt
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
            &interrupt
        ));
        assert!(app.in_subagent_view());
        assert!(!app.input_focused);
        assert!(app.input.is_empty());
    }
}
