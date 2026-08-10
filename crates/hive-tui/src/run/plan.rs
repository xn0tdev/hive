//! Plan-mode and selection actions for the TUI run loop.

use super::*;

pub(super) fn handle_plan_key(
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

pub(super) fn send_plan_corrections(app: &mut App, input_tx: &UnboundedSender<InputCommand>) {
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

pub(super) fn start_make_from_plan(app: &mut App, input_tx: &UnboundedSender<InputCommand>) {
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
pub(super) fn copy_last_answer(app: &mut App) {
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
pub(super) fn clean_for_copy(text: &str) -> String {
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
pub(super) fn copy_to_clipboard(text: &str) {
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
pub(super) fn complete_selected(app: &mut App) {
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
