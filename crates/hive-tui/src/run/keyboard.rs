//! Keyboard handling for the TUI run loop.

use super::*;

pub(super) fn handle_terminal_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
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
pub(super) fn handle_key(
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

    if app.recap_open() {
        return handle_recap_key(app, key);
    }

    let ctrl = key.mods.ctrl;
    let alt = key.mods.alt;
    let shift = key.mods.shift;

    let special = app.in_special_view();
    let slash_menu = !special
        && !app.palette_open()
        && !app.about_open()
        && !app.recap_open()
        && !app.slash_items().is_empty();
    let file_menu = !special
        && !app.palette_open()
        && !app.about_open()
        && !app.recap_open()
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
        KeyCode::Enter if file_menu || slash_menu => match activate_menu_selection(app, input_tx) {
            MenuPick::Ran { quit } => return quit,
            MenuPick::Completed => composer_activity = true,
            MenuPick::Empty => {}
        },
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
                // Interrupting only ends the current turn — an active goal would
                // start the next one right back up, so pause it on the way out.
                if app.goal_active() && !app.goal_paused() {
                    let _ = input_tx.send(InputCommand::PauseGoal);
                }
                // Stop the turn. The agent reports "Interrupted." in the chat
                // only once it has actually stopped.
                interrupt.store(true, Ordering::Relaxed);
            } else if app.goal_active() && !app.goal_paused() {
                let _ = input_tx.send(InputCommand::PauseGoal);
            } else if app.goal_paused() {
                let _ = input_tx.send(InputCommand::StopGoal);
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
