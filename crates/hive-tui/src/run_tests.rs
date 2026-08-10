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

fn with_goal(app: &mut App) {
    app.goal = Some(crate::app::goal::GoalStatus {
        objective: "keep going".into(),
        deadline: None,
        paused: false,
        circle: 0,
    });
    app.focus_input();
}

fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<InputCommand>) -> Vec<InputCommand> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// Esc from the composer — the normal place to press it — never reached the
/// goal, so the loop just started the next turn again.
#[test]
fn esc_pauses_then_stops_the_goal() {
    let mut app = test_app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let interrupt = Arc::new(AtomicBool::new(false));
    with_goal(&mut app);

    assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt));
    assert!(
        drain(&mut rx)
            .iter()
            .any(|c| matches!(c, InputCommand::PauseGoal)),
        "first esc pauses"
    );

    // The driver answers by flipping the flag; mirror that here.
    if let Some(g) = app.goal.as_mut() {
        g.paused = true;
    }
    assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt));
    assert!(
        drain(&mut rx)
            .iter()
            .any(|c| matches!(c, InputCommand::StopGoal)),
        "second esc stops"
    );
}

/// Mid-turn, Esc has to do both: end the turn and stop the loop from
/// starting another one.
#[test]
fn esc_during_a_goal_turn_interrupts_and_pauses() {
    let mut app = test_app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let interrupt = Arc::new(AtomicBool::new(false));
    with_goal(&mut app);
    app.running = true;

    assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt));
    assert!(
        interrupt.load(Ordering::Relaxed),
        "the turn was interrupted"
    );
    assert!(
        drain(&mut rx)
            .iter()
            .any(|c| matches!(c, InputCommand::PauseGoal)),
        "and the goal was paused"
    );
}

/// Typed text still wins the first Esc — the ladder is unchanged.
#[test]
fn esc_clears_the_composer_before_touching_the_goal() {
    let mut app = test_app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let interrupt = Arc::new(AtomicBool::new(false));
    with_goal(&mut app);
    app.input.value = "half-typed".into();
    app.input.cursor = 10;

    assert!(!handle_key(&mut app, esc_key(), &tx, &interrupt));
    assert_eq!(app.input.value, "");
    assert!(
        !drain(&mut rx)
            .iter()
            .any(|c| matches!(c, InputCommand::PauseGoal)),
        "the goal is not touched while there is text to clear"
    );
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

/// Before the first paint there is no geometry, so the mouse must not guess.
#[test]
fn palette_mouse_waits_for_a_frame() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_palette();
    assert!(!handle_mouse(&mut app, click(0, 0), &tx));
    assert!(
        app.palette_open(),
        "an unpainted palette can't be dismissed"
    );
}

#[test]
fn clicking_away_from_the_palette_closes_it() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_palette();
    let area = lay_out(&mut app);
    let win = render::palette::window_rect(area, &app).expect("panel");
    assert!(!win.contains(0, 0), "probe must be outside the panel");

    assert!(handle_mouse(&mut app, click(0, 0), &tx));
    assert!(!app.palette_open());
}

#[test]
fn hovering_the_palette_moves_the_highlight() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_palette();
    let area = lay_out(&mut app);

    // Find a row the pointer can actually land on.
    let win = render::palette::window_rect(area, &app).expect("panel");
    let before = app.palette.as_ref().unwrap().selected;
    let mut moved_to = None;
    for row in win.y..win.bottom() {
        let Some(idx) = render::palette::row_at(area, &app, win.x + 2, row) else {
            continue;
        };
        let selectable = app.palette.as_ref().unwrap().row_is_selectable(
            &app.model_choices,
            &app.connections,
            &app.saved_sessions,
            idx,
        );
        if selectable && idx != before {
            handle_mouse(&mut app, moved(win.x + 2, row), &tx);
            moved_to = Some(idx);
            break;
        }
    }
    let moved_to = moved_to.expect("a selectable row under the pointer");
    assert_eq!(app.palette.as_ref().unwrap().selected, moved_to);
    assert!(app.palette_open(), "hover must not activate anything");
}

/// Hovering a provider header should do nothing — it picks nothing.
#[test]
fn palette_headers_do_not_take_the_highlight() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_palette();
    let area = lay_out(&mut app);
    let win = render::palette::window_rect(area, &app).expect("panel");

    for row in win.y..win.bottom() {
        let Some(idx) = render::palette::row_at(area, &app, win.x + 2, row) else {
            continue;
        };
        let selectable = app.palette.as_ref().unwrap().row_is_selectable(
            &app.model_choices,
            &app.connections,
            &app.saved_sessions,
            idx,
        );
        if selectable {
            continue;
        }
        let before = app.palette.as_ref().unwrap().selected;
        handle_mouse(&mut app, moved(win.x + 2, row), &tx);
        assert_eq!(
            app.palette.as_ref().unwrap().selected,
            before,
            "header row {idx} stole the highlight"
        );
        return;
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
    let (rect, window) = open_slash_menu(&mut app, "/goal");
    let item = app.slash_items()[window].clone();
    assert_eq!(item.name(), "goal");
    assert!(item.takes_arg());

    assert!(handle_mouse(&mut app, click(rect.x + 2, rect.y), &tx));
    assert_eq!(app.input.value, "/goal ");
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

/// Row rect for a settings row, so tests click where `draw` actually paints.
fn settings_row_xy(app: &App, idx: usize) -> (u16, u16) {
    let area = app.overlay_area;
    let win = render::settings::window_rect(area, app).expect("panel");
    for row in win.y..win.bottom() {
        if render::settings::row_at(area, app, win.x + 2, row) == Some(idx) {
            return (win.x + 2, row);
        }
    }
    panic!("row {idx} is not on screen");
}

#[test]
fn clicking_a_settings_row_opens_its_page() {
    use crate::app::settings::SettingsPage;
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_settings();
    lay_out(&mut app);

    // Root row 2 is Tools.
    let (col, row) = settings_row_xy(&app, 2);
    assert!(handle_mouse(&mut app, click(col, row), &tx));
    assert_eq!(
        app.settings.as_ref().map(|s| s.page),
        Some(SettingsPage::Tools)
    );
}

/// The pages behind the root menu are where the mouse was dead.
#[test]
fn clicking_a_row_on_a_nested_page_toggles_it() {
    let mut app = test_app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_settings();
    if let Some(st) = app.settings.as_mut() {
        st.enter_tools();
    }
    lay_out(&mut app);
    let before = app.ui.tool_revert;

    // Row 1 is "Revert file" (row 0 is "Completed tools").
    let (col, row) = settings_row_xy(&app, 1);
    assert!(handle_mouse(&mut app, click(col, row), &tx));
    assert_eq!(app.ui.tool_revert, !before, "the toggle flipped");
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).any(|cmd| matches!(cmd, InputCommand::SaveUi(_))),
        "a changed setting must be persisted"
    );
}

#[test]
fn hovering_settings_moves_the_highlight() {
    use crate::app::settings::SettingsPage;
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_settings();
    lay_out(&mut app);
    assert_eq!(app.settings.as_ref().unwrap().selected, 0);

    let (col, row) = settings_row_xy(&app, 1);
    assert!(handle_mouse(&mut app, moved(col, row), &tx));
    assert_eq!(app.settings.as_ref().unwrap().selected, 1);
    assert_eq!(
        app.settings.as_ref().map(|s| s.page),
        Some(SettingsPage::Root),
        "hover must not open anything"
    );
}

#[test]
fn clicking_away_from_settings_closes_it() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_settings();
    let area = lay_out(&mut app);
    let win = render::settings::window_rect(area, &app).expect("panel");
    assert!(!win.contains(0, 0));

    assert!(handle_mouse(&mut app, click(0, 0), &tx));
    assert!(!app.settings_open());
}

#[test]
fn clicking_a_goal_field_focuses_it() {
    use crate::app::goal::GoalField;

    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_goal_overlay();
    let area = lay_out(&mut app);
    assert_eq!(
        app.goal_overlay.as_ref().unwrap().focus,
        GoalField::Objective
    );

    // Find the time-limit row and click it.
    let mut hit = None;
    for row in area.y..area.bottom() {
        if render::goal::field_at(area, &app, area.x + area.width / 2, row)
            == Some(GoalField::TimeLimit)
        {
            hit = Some(row);
            break;
        }
    }
    let row = hit.expect("time limit row");
    assert!(handle_mouse(
        &mut app,
        click(area.x + area.width / 2, row),
        &tx
    ));
    assert_eq!(
        app.goal_overlay.as_ref().unwrap().focus,
        GoalField::TimeLimit
    );
}

#[test]
fn goal_time_limit_is_parsed_and_sent() {
    use crate::app::goal::GoalField;

    let mut app = test_app();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_goal_overlay();
    assert!(!handle_goal_key(&mut app, key(KeyCode::Tab), &tx));
    let st = app.goal_overlay.as_mut().unwrap();
    assert_eq!(st.focus, GoalField::TimeLimit);
    st.objective = "ship it".into();
    st.time_limit = "1h30m".into();

    assert!(!handle_goal_key(&mut app, key(KeyCode::Enter), &tx));
    let command = rx.try_recv().expect("goal command");
    assert!(matches!(
        command,
        InputCommand::SetGoal { objective, duration }
            if objective == "ship it"
                && duration == Some(std::time::Duration::from_secs(5_400))
    ));
    assert!(!app.goal_overlay_open());
}

/// Typed text must survive a stray click off the goal panel.
#[test]
fn clicking_away_from_the_goal_form_keeps_it_open() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_goal_overlay();
    lay_out(&mut app);
    if let Some(st) = app.goal_overlay.as_mut() {
        st.objective = "ship the thing".into();
    }

    assert!(!handle_mouse(&mut app, click(0, 0), &tx));
    assert!(app.goal_overlay_open());
    assert_eq!(
        app.goal_overlay.as_ref().unwrap().objective,
        "ship the thing"
    );
}

#[test]
fn clicking_away_from_about_closes_it() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.open_about();
    let area = lay_out(&mut app);
    let win = render::about::window_rect(area, &app).expect("card");
    assert!(!win.contains(0, 0));

    assert!(handle_mouse(&mut app, click(0, 0), &tx));
    assert!(!app.about_open());
}

#[test]
fn about_overlay_still_ignores_the_mouse() {
    let mut app = test_app();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
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
