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
                app.flash("Invalid time limit — try 30m or 2h");
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
                Err(e) => app.flash(format!("Revert failed: {e}")),
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

/// Whether a frame has been painted, so the centered overlays have geometry to
/// hit-test against. Guessing before that would dismiss them on a stray event.
pub(super) fn overlay_is_painted(app: &App) -> bool {
    app.overlay_area.width > 0 && app.overlay_area.height > 0
}

/// Mouse over the Settings overlay: hover highlights, click opens a page or
/// flips a toggle, wheel walks the rows, and a click off the panel closes it.
pub(super) fn handle_settings_mouse(
    app: &mut App,
    m: Mouse,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    if !overlay_is_painted(app) {
        return false;
    }
    let area = app.overlay_area;
    let row = render::settings::row_at(area, app, m.col, m.row);

    match m.kind {
        MouseKind::Moved => {
            let Some(row) = row else { return false };
            let Some(st) = app.settings.as_mut() else {
                return false;
            };
            if st.selected == row {
                return false;
            }
            st.selected = row;
            true
        }
        MouseKind::ScrollUp | MouseKind::ScrollDown => {
            let Some(st) = app.settings.as_mut() else {
                return false;
            };
            if matches!(m.kind, MouseKind::ScrollUp) {
                st.move_up();
            } else {
                st.move_down();
            }
            true
        }
        MouseKind::Down(MouseButton::Left) => {
            if let Some(row) = row {
                if let Some(st) = app.settings.as_mut() {
                    st.selected = row;
                }
                // Drilling into a page changes nothing to save; a toggle does.
                if crate::app::settings::activate(app) {
                    let _ = input_tx.send(InputCommand::SaveUi(app.ui.clone()));
                }
                return true;
            }
            let inside = render::settings::window_rect(area, app)
                .is_some_and(|win| win.contains(m.col, m.row));
            if !inside {
                app.close_settings();
                return true;
            }
            false
        }
        _ => false,
    }
}

/// Mouse over the `/goal` form: click a field to put the caret in it. The
/// overlay holds typed text, so a click off it is swallowed rather than
/// throwing that away.
pub(super) fn handle_goal_mouse(app: &mut App, m: Mouse) -> bool {
    if !overlay_is_painted(app) {
        return false;
    }
    if !matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
        return false;
    }
    let Some(field) = render::goal::field_at(app.overlay_area, app, m.col, m.row) else {
        return false;
    };
    let Some(st) = app.goal_overlay.as_mut() else {
        return false;
    };
    if st.focus == field {
        return false;
    }
    st.focus = field;
    true
}

/// Mouse over the palette overlay: hover highlights, click picks, wheel scrolls,
/// and a click outside the panel dismisses it.
pub(super) fn handle_palette_mouse(
    app: &mut App,
    m: Mouse,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    let area = app.overlay_area;
    // Opened but not painted yet — there is no geometry to hit-test against,
    // and guessing would dismiss the overlay on the first stray event.
    if area.width == 0 || area.height == 0 {
        return false;
    }
    let choices = app.model_choices.clone();
    let connections = app.connections.clone();
    let sessions = app.saved_sessions.clone();
    let visible = app.palette_list_visible as usize;
    let row = render::palette::row_at(area, app, m.col, m.row);

    match m.kind {
        MouseKind::Moved => {
            let Some(row) = row else { return false };
            app.palette
                .as_mut()
                .is_some_and(|pal| pal.select_row(&choices, &connections, &sessions, row))
        }
        MouseKind::ScrollUp | MouseKind::ScrollDown => {
            let delta = if matches!(m.kind, MouseKind::ScrollUp) {
                -3
            } else {
                3
            };
            app.palette.as_mut().is_some_and(|pal| {
                pal.scroll_list(&choices, &connections, &sessions, visible, delta)
            })
        }
        MouseKind::Down(MouseButton::Left) => {
            if let Some(row) = row {
                let picked = app
                    .palette
                    .as_mut()
                    .map(|pal| {
                        pal.select_row(&choices, &connections, &sessions, row);
                        pal.row_is_selectable(&choices, &connections, &sessions, row)
                    })
                    .unwrap_or(false);
                if picked {
                    if activate_palette(app, input_tx) {
                        app.request_quit();
                    }
                    return true;
                }
                // A group header — highlight nothing, swallow the click.
                return true;
            }
            let inside = render::palette::window_rect(area, app)
                .is_some_and(|win| win.contains(m.col, m.row));
            if !inside {
                app.close_palette();
                return true;
            }
            false
        }
        _ => false,
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
