//! Settings, goal, palette, and context overlay handlers.

use super::*;

pub(super) fn handle_settings_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    let ctrl = key.mods.ctrl;
    match key.code {
        KeyCode::Esc => {
            app.close_settings();
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
                st.item() == Some(crate::app::settings::SettingsItem::SidebarWidth)
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
                st.item() == Some(crate::app::settings::SettingsItem::SidebarWidth)
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

pub(super) fn handle_goal_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
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
        KeyCode::Tab => {
            if let Some(st) = app.goal_overlay.as_mut() {
                st.toggle_focus();
            }
        }
        KeyCode::Enter => {
            let Some((objective, time_limit)) = app
                .goal_overlay
                .as_ref()
                .map(|st| (st.objective.trim().to_string(), st.time_limit.clone()))
            else {
                return false;
            };
            if objective.is_empty() {
                app.flash("Enter an objective first");
                return false;
            }
            let duration = if time_limit.trim().is_empty() {
                None
            } else if let Some(duration) = crate::app::goal::parse_duration(&time_limit) {
                Some(duration)
            } else {
                app.flash_error("Invalid time limit — try 30m or 2h");
                return false;
            };
            app.close_goal_overlay();
            let _ = input_tx.send(InputCommand::SetGoal {
                objective,
                duration,
            });
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

pub(super) fn handle_recap_key(app: &mut App, key: Key) -> bool {
    let ctrl = key.mods.ctrl;
    match key.code {
        KeyCode::Esc => app.close_recap(),
        KeyCode::Enter | KeyCode::Char(' ') => {
            app.reveal_recap_stream();
        }
        KeyCode::Up => {
            app.scroll_recap(-1);
        }
        KeyCode::Down => {
            app.scroll_recap(1);
        }
        KeyCode::PageUp => {
            app.scroll_recap(-8);
        }
        KeyCode::PageDown => {
            app.scroll_recap(8);
        }
        KeyCode::Char('p') if ctrl => {
            app.close_recap();
            app.open_palette();
        }
        KeyCode::Char('c') if ctrl => return app.arm_or_confirm_quit(),
        KeyCode::Char('q') if ctrl => return true,
        _ => {}
    }
    false
}

pub(super) fn handle_recap_mouse(app: &mut App, m: Mouse) -> bool {
    match m.kind {
        MouseKind::ScrollUp => app.scroll_recap(-3),
        MouseKind::ScrollDown => app.scroll_recap(3),
        MouseKind::Down(MouseButton::Left) => {
            if app
                .recap_generating_hit
                .is_some_and(|hit| hit.contains(m.col, m.row))
            {
                app.reveal_recap_stream();
            }
            true
        }
        _ => false,
    }
}

pub(super) fn handle_about_key(app: &mut App, key: Key) -> bool {
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

pub(super) fn handle_context_menu_key(app: &mut App, key: Key) -> bool {
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

pub(super) fn activate_context_action(app: &mut App, action: ContextAction) {
    match action {
        ContextAction::Copy(text) => {
            write_clipboard_text(&text);
            app.flash("Copied");
        }
        ContextAction::ToggleToolDetails { id } => {
            app.toggle_tool_details(&id);
        }
        ContextAction::RevertFile { path, content } => {
            let full = std::path::Path::new(&app.cwd).join(&path);
            match std::fs::write(&full, &content) {
                Ok(()) => app.flash(format!("Reverted {}", path)),
                Err(e) => app.flash_error(format!("Revert failed: {e}")),
            }
            app.project.invalidate();
            app.refresh_project();
        }
    }
}

/// The `/connect` row under the cursor, if the providers list is open.
pub(super) fn highlighted_provider<'a>(
    app: &'a App,
) -> Option<crate::app::palette::ConnectRow<'a>> {
    app.palette
        .as_ref()
        .filter(|p| p.mode == PaletteMode::Connect)
        .and_then(|p| p.selected_connect(&app.connections))
}

pub(super) fn handle_palette_key(
    app: &mut App,
    key: Key,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
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
                        app.flash_error("Can't remove the only provider");
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

/// Mouse inside the `/` or `@` composer menu: hover highlights, click picks,
/// wheel walks the list. Everything else in the panel is swallowed so a stray
/// click can't reach the transcript underneath.
pub(super) fn handle_menu_mouse(
    app: &mut App,
    m: Mouse,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    match m.kind {
        MouseKind::Moved => app
            .menu_item_at(m.col, m.row)
            .is_some_and(|idx| app.set_menu_index(idx)),
        MouseKind::ScrollUp => {
            app.menu_up();
            true
        }
        MouseKind::ScrollDown => {
            app.menu_down();
            true
        }
        MouseKind::Down(MouseButton::Left) => {
            let Some(idx) = app.menu_item_at(m.col, m.row) else {
                // The `↓ N more` footer — a click there shouldn't run anything.
                return false;
            };
            app.set_menu_index(idx);
            if let MenuPick::Ran { quit: true } = activate_menu_selection(app, input_tx) {
                app.request_quit();
            }
            true
        }
        _ => false,
    }
}

/// What accepting the composer menu's highlighted row did.
pub(super) enum MenuPick {
    /// Nothing was highlighted.
    Empty,
    /// The row was completed into the composer.
    Completed,
    /// A command ran; `quit` says whether it asked to exit.
    Ran { quit: bool },
}

/// Accept whatever the composer menu is pointing at. Shared by Enter and the
/// mouse so the two can't drift.
pub(super) fn activate_menu_selection(
    app: &mut App,
    input_tx: &UnboundedSender<InputCommand>,
) -> MenuPick {
    if app.slash_items().is_empty() {
        complete_selected(app);
        return MenuPick::Completed;
    }
    let Some(item) = app.menu_selected() else {
        return MenuPick::Empty;
    };
    // Commands with an argument get completed; the rest run at once.
    if item.takes_arg() {
        complete_selected(app);
        return MenuPick::Completed;
    }
    app.input.take();
    app.reset_menu();
    MenuPick::Ran {
        quit: handle_slash(app, item.name(), input_tx),
    }
}

pub(super) fn activate_palette(app: &mut App, input_tx: &UnboundedSender<InputCommand>) -> bool {
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
                .and_then(|p| p.selected_connect(&app.connections));
            match row {
                Some(ConnectRow::Profile(connection)) => {
                    app.flash(format!(
                        "{} is already added · ctrl+e edits its key",
                        connection.label
                    ));
                }
                Some(ConnectRow::Preset(idx)) => {
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
            app.close_palette();
            let _ = input_tx.send(InputCommand::UpsertConnection {
                id,
                label: preset.label.to_string(),
                base_url: preset.base_url.to_string(),
                api_key_env: preset.api_key_env.to_string(),
                api_key: key,
            });
            app.flash(format!("Adding {}…", preset.label));
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
                app.flash_error("Provider is no longer available");
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
