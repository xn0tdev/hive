//! Mouse handling for the TUI run loop.

use super::*;

/// Wheel scrolls the transcript; left-click toggles thoughts, opens subagent
/// chats, hits `← back` / Make, or bonks the logo.
///
/// Palette / Settings / About are keyboard-only — mouse events are
/// swallowed so they don't leak through to the chat underneath. Recap takes
/// clicks on Generating and wheel-scrolls the body.
/// Returns `true` when the UI should redraw.
pub(super) fn handle_mouse(
    app: &mut App,
    m: Mouse,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    // An open menu owns the pointer: pick an action, or dismiss by clicking
    // away. Swallowing every mouse event instead left the mouse looking dead
    // until you found the keyboard.
    if app.context_menu_open() {
        return match m.kind {
            MouseKind::Moved => app.context_menu_hover(m.col, m.row),
            MouseKind::Down(MouseButton::Left) => {
                if app.context_menu_hover(m.col, m.row) || app.context_menu_on_item(m.col, m.row) {
                    if let Some(action) = app.take_context_action() {
                        app.close_context_menu();
                        activate_context_action(app, action);
                    } else {
                        app.close_context_menu();
                    }
                } else if !app.context_menu_contains(m.col, m.row) {
                    app.close_context_menu();
                }
                true
            }
            _ => false,
        };
    }
    if app.recap_open() {
        return handle_recap_mouse(app, m);
    }
    if app.palette_open() || app.settings_open() || app.about_open() {
        return false;
    }
    // The composer menu is on top of the transcript, so it gets first refusal
    // on anything inside its panel.
    if app.menu_contains(m.col, m.row) {
        return handle_menu_mouse(app, m, input_tx);
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
            if let Some(idx) = app.expandable_at(m.col, m.row) {
                if matches!(
                    app.blocks.get(idx),
                    Some(crate::app::state::Block::WorkSummary(_))
                ) {
                    app.clear_assistant_selection();
                    app.open_recap(idx, input_tx);
                    app.blur_input();
                    return true;
                }
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
                let card = app.expandable_at(m.col, m.row).and_then(|i| {
                    matches!(
                        app.blocks.get(i),
                        Some(crate::app::state::Block::Plan(_))
                            | Some(crate::app::state::Block::Subagent(_))
                            | Some(crate::app::state::Block::Terminal(_))
                            | Some(crate::app::state::Block::User(_))
                            | Some(crate::app::state::Block::Tool(_))
                            | Some(crate::app::state::Block::Explore(_))
                            | Some(crate::app::state::Block::WorkSummary(_))
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
