//! Clipboard and paste handling for the TUI run loop.

use super::*;

/// Bracketed paste / clipboard paste. Returns whether the UI should redraw.
pub(super) fn handle_paste(
    app: &mut App,
    text: &str,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
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

pub(super) fn sanitize_api_key(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(super) fn paste_into_palette(app: &mut App, text: &str) -> bool {
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

pub(super) fn read_clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// A screenshot on the clipboard, as PNG bytes. Clipboards hand images over as
/// raw RGBA, so they have to be encoded before anything can be sent.
pub(super) fn read_clipboard_image() -> Option<Vec<u8>> {
    let image = arboard::Clipboard::new().ok()?.get_image().ok()?;
    let (w, h) = (image.width as u32, image.height as u32);
    if w == 0 || h == 0 {
        return None;
    }
    encode_png(w, h, &image.bytes)
}

pub(super) fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
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
pub(super) fn paste_from_clipboard(
    app: &mut App,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
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

pub(super) fn write_clipboard_text(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}
