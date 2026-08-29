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
fn connect_only_adds_new_providers_and_never_switches_existing_ones() {
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

    // Row 0 is already configured: Enter leaves the runtime alone and
    // points at the dedicated key-edit shortcut.
    assert!(matches!(
        app.palette
            .as_ref()
            .and_then(|p| p.selected_connect(&app.connections)),
        Some(ConnectRow::Profile(_))
    ));
    assert!(!activate_palette(&mut app, &tx));
    assert!(
        rx.try_recv().is_err(),
        "an existing row must not switch provider"
    );
    assert_eq!(
        app.palette.as_ref().map(|p| p.mode),
        Some(PaletteMode::Connect)
    );
    assert!(app
        .flash_text()
        .is_some_and(|text| text.contains("ctrl+e edits its key")));

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

// ── Pasted-text chips ────────────────────────────────────────────────────

fn pasted_app() -> App {
    let mut app = test_app();
    app.add_pasted_block("line one\nline two\nline three".into());
    app.add_pasted_block("just one line but long enough to chip".into());
    app.focus_input();
    app
}

#[test]
fn down_steps_onto_pasted_chips_after_attach_chips() {
    let mut app = pasted_app();
    app.attach_clipboard_image(b"png".to_vec()).expect("attach");

    press(&mut app, KeyCode::Down);
    assert_eq!(app.selected_attach(), Some(0), "attach chips come first");
    assert_eq!(app.selected_pasted(), None);

    // ↓ again walks off the attach chip onto the pasted chips.
    press(&mut app, KeyCode::Down);
    assert_eq!(app.selected_attach(), None);
    assert_eq!(app.selected_pasted(), Some(0));

    press(&mut app, KeyCode::Right);
    assert_eq!(app.selected_pasted(), Some(1));
    press(&mut app, KeyCode::Left);
    assert_eq!(app.selected_pasted(), Some(0));
    press(&mut app, KeyCode::Left);
    assert_eq!(app.selected_pasted(), Some(1), "wraps backwards");
}

#[test]
fn backspace_removes_the_highlighted_pasted_chip() {
    let mut app = pasted_app();
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.selected_pasted(), Some(1));

    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.pasted_blocks.len(), 1);
    assert_eq!(app.selected_pasted(), Some(0), "lands on what's left");

    press(&mut app, KeyCode::Backspace);
    assert!(app.pasted_blocks.is_empty());
    assert_eq!(app.selected_pasted(), None);
}

#[test]
fn enter_opens_the_pasted_viewer_and_esc_closes_it() {
    let mut app = pasted_app();
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.selected_pasted(), Some(1));

    press(&mut app, KeyCode::Enter);
    assert_eq!(app.pasted_view.as_ref().map(|v| v.idx), Some(1));
    assert_eq!(app.pasted_view.as_ref().map(|v| v.scroll), Some(0));

    // The viewer owns the keyboard: esc closes it, chips keep their state.
    press(&mut app, KeyCode::Esc);
    assert!(!app.pasted_view_open());
    assert_eq!(app.selected_pasted(), Some(1));
}

#[test]
fn pasted_viewer_scrolls_and_clamps() {
    let mut app = test_app();
    let body: String = (0..200).map(|i| format!("row {i}\n")).collect();
    app.add_pasted_block(body);
    app.open_pasted_view(0);

    assert!(app.pasted_view_scroll(5));
    assert_eq!(app.pasted_view.as_ref().unwrap().scroll, 5);
    assert!(app.pasted_view_scroll(-10));
    assert_eq!(app.pasted_view.as_ref().unwrap().scroll, 0, "clamps at 0");
    assert!(!app.pasted_view_scroll(-1), "no move → no redraw");

    // usize::MAX (End key) clamps to the last page at paint time.
    app.pasted_view.as_mut().unwrap().scroll = usize::MAX;
    let _ = comb::render(comb::Size::new(80, 24), |f| {
        crate::render::draw(f, &mut app)
    });
    let scroll = app.pasted_view.as_ref().unwrap().scroll;
    assert!(scroll < usize::MAX, "paint must clamp the scroll");
}

#[test]
fn large_paste_becomes_a_chip_small_paste_stays_text() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

    let big = "word ".repeat(120);
    assert!(handle_paste(&mut app, &big, &tx));
    assert_eq!(app.pasted_blocks.len(), 1);
    assert_eq!(
        app.input.value, "[ pasted text 1 ] ",
        "token instead of the pasted text"
    );

    assert!(handle_paste(&mut app, "short note", &tx));
    assert_eq!(app.pasted_blocks.len(), 1, "small paste stays inline");
    assert_eq!(app.input.value, "[ pasted text 1 ] short note");
}

#[test]
fn submit_sends_pasted_blocks_and_clears_the_chips() {
    let mut app = test_app();
    let token = app.add_pasted_block("alpha\nbeta".into());
    app.input.value = format!("{token} look at this");
    app.input.cursor = app.input.value.chars().count();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let interrupt = Arc::new(AtomicBool::new(false));

    assert!(!handle_key(&mut app, key(KeyCode::Enter), &tx, &interrupt));
    assert!(app.pasted_blocks.is_empty(), "tokens are consumed");
    let pd = app.pending_dispatch.expect("deferred dispatch");
    assert!(pd.agent_text.contains("look at this"), "{}", pd.agent_text);
    assert!(pd.agent_text.contains(&token), "{}", pd.agent_text);
    assert!(pd.agent_text.contains("alpha\nbeta"), "{}", pd.agent_text);
    assert!(
        app.blocks.iter().any(|b| matches!(
            b,
            crate::app::state::Block::User(d) if d.contains(&token)
        )),
        "transcript shows the token"
    );
}

#[test]
fn esc_recall_restores_pasted_blocks() {
    let mut app = test_app();
    let token = app.add_pasted_block("precious paste".into());
    app.input.value = format!("{token} with paste");
    app.input.cursor = app.input.value.chars().count();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let interrupt = Arc::new(AtomicBool::new(false));

    assert!(!handle_key(&mut app, key(KeyCode::Enter), &tx, &interrupt));
    assert!(app.pasted_blocks.is_empty());
    assert!(app.pending_dispatch.is_some());

    // ESC within the grace window pulls everything back.
    assert!(!handle_key(&mut app, key(KeyCode::Esc), &tx, &interrupt));
    assert!(app.pending_dispatch.is_none());
    assert_eq!(app.input.value, format!("{token} with paste"));
    assert_eq!(app.pasted_blocks.len(), 1);
    assert_eq!(app.pasted_blocks[0].content, "precious paste");
}

#[test]
fn new_chat_clears_pasted_state() {
    let mut app = pasted_app();
    app.open_pasted_view(0);
    app.new_chat();
    assert!(app.pasted_blocks.is_empty());
    assert_eq!(app.selected_pasted(), None);
    assert!(!app.pasted_view_open());
}

#[test]
fn pasted_viewer_keeps_a_blank_row_above_the_header() {
    let mut app = pasted_app();
    app.open_pasted_view(0);
    let buf = comb::render(comb::Size::new(80, 24), |f| {
        crate::render::draw(f, &mut app)
    });
    let rows: Vec<String> = buf.text().lines().map(|l| l.to_string()).collect();
    let header = rows
        .iter()
        .position(|r| r.contains("PASTED TEXT 1"))
        .expect("viewer header");
    assert!(
        header >= 1 && rows[header - 1].trim().is_empty(),
        "blank row expected above the header: {rows:?}"
    );
}

fn mouse(kind: MouseKind, col: u16, row: u16) -> Mouse {
    Mouse { kind, col, row }
}

/// Opening a context menu used to swallow every mouse event until you
/// found the keyboard — clicking anything looked like a dead mouse.
fn app_with_menu() -> App {
    use crate::app::state::{Block, FileSnapshot, ToolCard, ToolStatus};

    let mut app = test_app();
    app.blocks.clear();
    app.blocks.push(Block::Tool(ToolCard {
        id: "t1".into(),
        name: "write_file".into(),
        args: "src/main.rs".into(),
        output: "+1\tnew".into(),
        status: ToolStatus::Ok,
        started: std::time::Instant::now(),
        elapsed_ms: Some(1),
        details_open: false,
        snapshot: Some(FileSnapshot {
            path: "src/main.rs".into(),
            content: "old".into(),
        }),
    }));
    app.open_tool_menu(0);
    let _ = comb::render(comb::Size::new(80, 24), |f| {
        crate::render::draw(f, &mut app)
    });
    assert!(!app.context_menu_hits.is_empty(), "menu rows recorded");
    app
}

#[test]
fn clicking_away_dismisses_the_context_menu() {
    let mut app = app_with_menu();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

    // A corner far outside the panel.
    assert!(!app.context_menu_contains(0, 0));
    assert!(handle_mouse(
        &mut app,
        mouse(MouseKind::Down(MouseButton::Left), 0, 0),
        &tx
    ));
    assert!(!app.context_menu_open(), "click away closes it");

    // And the mouse works again right after.
    assert!(handle_mouse(
        &mut app,
        mouse(MouseKind::ScrollUp, 10, 5),
        &tx
    ));
}

#[test]
fn clicking_a_menu_row_runs_it() {
    let mut app = app_with_menu();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

    let (rect, idx) = app.context_menu_hits[0];
    assert_eq!(idx, 0);
    assert!(handle_mouse(
        &mut app,
        mouse(MouseKind::Down(MouseButton::Left), rect.x + 1, rect.y),
        &tx
    ));
    assert!(
        !app.context_menu_open(),
        "running an action closes the menu"
    );
}

#[test]
fn hovering_moves_the_menu_selection() {
    let mut app = app_with_menu();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    if app.context_menu_hits.len() < 2 {
        return; // only one action available
    }

    let (rect, idx) = app.context_menu_hits[1];
    assert!(handle_mouse(
        &mut app,
        mouse(MouseKind::Moved, rect.x + 1, rect.y),
        &tx
    ));
    assert_eq!(app.context_menu.as_ref().map(|m| m.selected), Some(idx));
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
    assert!(app.pending_attaches.iter().all(
        |a| matches!(&a.image, Some(hive_core::message::ImageSource::Base64 { media_type, data })
                if media_type == "image/png" && !data.is_empty())
    ));
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
fn choosing_a_model_is_the_only_provider_switch_path() {
    let mut app = test_app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.model_choices = vec![crate::ModelChoice {
        key: "llama".into(),
        display: "Llama".into(),
        detail: String::new(),
        group: "Groq".into(),
        connection_id: "groq".into(),
        vision: false,
        context: 128_000,
        cost_input: 0.0,
        cost_output: 0.0,
    }];
    let mut palette = crate::app::palette::PaletteState::models();
    palette.clamp_selection(&app.model_choices, &app.connections, &app.saved_sessions);
    app.palette = Some(palette);

    assert!(!activate_palette(&mut app, &tx));
    assert!(matches!(
        rx.try_recv(),
        Ok(InputCommand::SetModel {
            id,
            connection_id: Some(connection_id),
            ..
        }) if id == "llama" && connection_id == "groq"
    ));
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
        pasted: Vec::new(),
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
        pasted: Vec::new(),
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

fn click(col: u16, row: u16) -> Mouse {
    Mouse {
        kind: MouseKind::Down(MouseButton::Left),
        col,
        row,
    }
}

fn moved(col: u16, row: u16) -> Mouse {
    Mouse {
        kind: MouseKind::Moved,
        col,
        row,
    }
}

/// Paint a frame so the overlays have real geometry to hit-test against.
fn lay_out(app: &mut App) -> comb::Rect {
    let size = comb::Size::new(90, 30);
    let _ = comb::render(size, |f| render::draw(f, app));
    comb::Rect::new(0, 0, size.width, size.height)
}

/// Palette / Settings / Goal / About are keyboard-only: the pointer must not
/// pick, hover-highlight, or dismiss them — and must not leak into the chat.
#[test]
fn palette_ignores_the_mouse() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_palette();
    let area = lay_out(&mut app);
    let win = render::palette::window_rect(area, &app).expect("panel");
    let before = app.palette.as_ref().unwrap().selected;

    assert!(!handle_mouse(&mut app, click(0, 0), &tx));
    assert!(app.palette_open(), "click-away must not dismiss");

    assert!(!handle_mouse(&mut app, click(win.x + 2, win.y + 4), &tx));
    assert!(app.palette_open());
    assert_eq!(app.palette.as_ref().unwrap().selected, before);

    assert!(!handle_mouse(&mut app, moved(win.x + 2, win.y + 4), &tx));
    assert_eq!(app.palette.as_ref().unwrap().selected, before);
}

#[test]
fn settings_ignores_the_mouse() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_settings();
    let area = lay_out(&mut app);
    let win = render::settings::window_rect(area, &app).expect("panel");
    let before = app.settings.as_ref().unwrap().selected;
    let revert = app.ui.tool_revert;

    assert!(!handle_mouse(&mut app, click(0, 0), &tx));
    assert!(app.settings_open(), "click-away must not dismiss");

    assert!(!handle_mouse(&mut app, click(win.x + 2, win.y + 4), &tx));
    assert!(app.settings_open());
    assert_eq!(app.settings.as_ref().unwrap().selected, before);
    assert_eq!(app.ui.tool_revert, revert, "click must not toggle");

    assert!(!handle_mouse(&mut app, moved(win.x + 2, win.y + 6), &tx));
    assert_eq!(app.settings.as_ref().unwrap().selected, before);
}

fn seed_worked_for(app: &mut App, recap: crate::app::state::RecapBody) {
    use crate::app::state::{Block, WorkSummaryCard};
    app.blocks.clear();
    app.blocks.push(Block::User("fix the hover".into()));
    app.blocks.push(Block::Assistant {
        text: "hover is bold now".into(),
        streaming: false,
    });
    app.blocks.push(Block::WorkSummary(WorkSummaryCard {
        secs: 2,
        recap_id: 9,
        recap,
    }));
}

#[test]
fn clicking_worked_for_opens_recap_once() {
    use crate::app::state::RecapBody;

    let mut app = test_app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    seed_worked_for(&mut app, RecapBody::Idle);
    lay_out(&mut app);

    let (row, idx) = app
        .click_hits
        .iter()
        .copied()
        .find(|&(_, i)| {
            matches!(
                app.blocks.get(i),
                Some(crate::app::state::Block::WorkSummary(_))
            )
        })
        .expect("worked-for row");
    let col = app.transcript_hit.map(|r| r.x + 2).unwrap_or(4);
    assert!(handle_mouse(&mut app, click(col, row), &tx));
    assert!(app.recap_open());
    assert!(matches!(
        rx.try_recv(),
        Ok(InputCommand::Recap { id: 9, .. })
    ));
    assert!(matches!(
        app.blocks[idx],
        crate::app::state::Block::WorkSummary(ref c)
            if matches!(c.recap, RecapBody::Generating { .. })
    ));

    app.close_recap();
    assert!(!app.recap_open());
    lay_out(&mut app);
    let (row, _) = app
        .click_hits
        .iter()
        .copied()
        .find(|&(_, i)| {
            matches!(
                app.blocks.get(i),
                Some(crate::app::state::Block::WorkSummary(_))
            )
        })
        .expect("worked-for row");
    assert!(handle_mouse(&mut app, click(col, row), &tx));
    assert!(app.recap_open());
    assert!(rx.try_recv().is_err(), "reopening must not generate again");
}

#[test]
fn recap_esc_keeps_the_text() {
    use crate::app::state::RecapBody;
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    seed_worked_for(
        &mut app,
        RecapBody::Ready {
            text: "Made hover bold.".into(),
        },
    );
    app.open_recap(2, &tx);
    assert!(app.recap_open());
    assert!(!handle_recap_key(&mut app, key(KeyCode::Esc)));
    assert!(!app.recap_open());
    match &app.blocks[2] {
        crate::app::state::Block::WorkSummary(c) => {
            assert!(matches!(&c.recap, RecapBody::Ready { text } if text == "Made hover bold."));
        }
        _ => panic!("card gone"),
    }
}

/// Open the `/` menu and paint it, returning its panel rect and the item
/// its first row shows.
fn open_slash_menu(app: &mut App, typed: &str) -> (comb::Rect, usize) {
    app.input.value = typed.to_string();
    app.input.cursor = typed.chars().count();
    assert!(!app.slash_items().is_empty(), "menu should be open");
    lay_out(app);
    app.menu_hit.expect("menu was drawn")
}

#[test]
fn hovering_the_slash_menu_moves_the_highlight() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (rect, window) = open_slash_menu(&mut app, "/");

    assert!(handle_mouse(&mut app, moved(rect.x + 2, rect.y + 2), &tx));
    assert_eq!(app.menu_index, window + 2);
}

/// A command that takes an argument is completed into the composer, exactly
/// as Enter would.
#[test]
fn clicking_a_slash_row_with_an_argument_completes_it() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (rect, window) = open_slash_menu(&mut app, "/model");
    let item = app.slash_items()[window].clone();
    assert_eq!(item.name(), "model");
    assert!(item.takes_arg());

    assert!(handle_mouse(&mut app, click(rect.x + 2, rect.y), &tx));
    assert_eq!(app.input.value, "/model ");
}

/// A command that takes none runs on the click.
#[test]
fn clicking_a_slash_row_runs_it() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (rect, window) = open_slash_menu(&mut app, "/resume");
    assert_eq!(app.slash_items()[window].name(), "resume");

    assert!(handle_mouse(&mut app, click(rect.x + 2, rect.y), &tx));
    assert_eq!(app.input.value, "", "the composer is consumed");
    assert_eq!(
        app.palette.as_ref().map(|p| p.mode),
        Some(PaletteMode::Sessions),
        "the command actually ran"
    );
}

/// The `↓ N more` footer sits inside the panel but picks nothing.
#[test]
fn clicking_the_more_row_does_nothing() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (rect, _) = open_slash_menu(&mut app, "/");
    assert!(
        app.slash_items().len() > crate::app::files::MAX_MENU_ROWS,
        "need an overflowing menu for this test"
    );

    let footer = rect.y + crate::app::files::MAX_MENU_ROWS as u16;
    let before = app.input.value.clone();
    assert!(!handle_mouse(&mut app, click(rect.x + 2, footer), &tx));
    assert_eq!(app.input.value, before);
}

/// A click on the menu must not fall through to the transcript underneath.
#[test]
fn the_slash_menu_swallows_clicks() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (rect, _) = open_slash_menu(&mut app, "/");
    app.focus_input();

    handle_mouse(&mut app, click(rect.x + 2, rect.y), &tx);
    assert!(
        app.menu_hit.is_some() || !app.input.value.is_empty() || !app.input_focused,
        "click reached the chat underneath"
    );
}

#[test]
fn about_overlay_ignores_the_mouse() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_about();
    let area = lay_out(&mut app);
    let win = render::about::window_rect(area, &app).expect("card");
    assert!(!win.contains(0, 0));

    assert!(!handle_mouse(&mut app, click(0, 0), &tx));
    assert!(app.about_open(), "click-away must not dismiss");
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
