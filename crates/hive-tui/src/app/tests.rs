use super::*;
use crate::TuiInit;
use hive_core::event::{AgentEvent, SubagentLine, SubagentStatus};
use hive_core::{TerminalController, TerminalProcessState};

fn app() -> App {
    App::new(TuiInit {
        model: "m".into(),
        model_display: "m".into(),
        model_choices: Vec::new(),
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

#[test]
fn reopening_models_keeps_a_ready_catalog_visible_while_refreshing() {
    let mut a = app();
    a.models_catalog = ModelsCatalogState::Ready;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    a.open_model_picker(&tx);

    assert_eq!(a.models_catalog, ModelsCatalogState::Ready);
    assert!(matches!(
        rx.try_recv(),
        Ok(crate::InputCommand::FetchModels)
    ));
}

#[test]
fn failed_background_refresh_keeps_the_last_catalog() {
    let mut a = app();
    a.models_catalog = ModelsCatalogState::Ready;
    a.palette = Some(PaletteState::models());

    a.apply(AgentEvent::ModelsListFailed("offline".into()));

    assert_eq!(a.models_catalog, ModelsCatalogState::Ready);
    assert!(a
        .error_toast()
        .is_some_and(|text| text.contains("Couldn't refresh models")));
}

#[test]
fn catalog_refresh_keeps_the_same_provider_model_selected() {
    let mut a = app();
    a.model_choices = vec![ModelChoice {
        key: "llama".into(),
        display: "Llama".into(),
        detail: String::new(),
        group: "Groq".into(),
        connection_id: "groq".into(),
        vision: false,
        context: 0,
        cost_input: 0.0,
        cost_output: 0.0,
    }];
    let mut palette = PaletteState::models();
    palette.clamp_selection(&a.model_choices, &[], &[]);
    a.palette = Some(palette);

    a.apply(AgentEvent::ModelsListed {
        models: vec![
            hive_core::event::CatalogModel {
                id: "claude".into(),
                name: "Claude".into(),
                detail: String::new(),
                group: "Anthropic".into(),
                connection_id: "anthropic".into(),
                vision: false,
                context: 0,
                cost_input: 0.0,
                cost_output: 0.0,
            },
            hive_core::event::CatalogModel {
                id: "llama".into(),
                name: "Llama".into(),
                detail: String::new(),
                group: "Groq".into(),
                connection_id: "groq".into(),
                vision: false,
                context: 0,
                cost_input: 0.0,
                cost_output: 0.0,
            },
        ],
    });

    let selected = a
        .palette
        .as_ref()
        .and_then(|palette| palette.selected_model(&a.model_choices))
        .expect("selected model");
    assert_eq!(selected.connection_id, "groq");
    assert_eq!(selected.key, "llama");
}

#[test]
fn subagent_transcript_does_not_dirty_main_view() {
    let mut a = app();
    assert!(a.apply(AgentEvent::SubagentSpawned {
        id: "v1".into(),
        label: "Checking".into(),
        prompt: "go".into(),
    }));
    // Body updates are stored but not painted on the main chat.
    assert!(!a.apply(AgentEvent::SubagentTranscript {
        id: "v1".into(),
        line: SubagentLine::Thinking("hmm".into()),
    }));
    // Status line is visible on the card.
    assert!(a.apply(AgentEvent::SubagentStatus {
        id: "v1".into(),
        status: SubagentStatus::Running,
        detail: "$ cargo check".into(),
    }));
}

#[test]
fn subagent_transcript_dirties_open_subagent_view() {
    let mut a = app();
    a.apply(AgentEvent::SubagentSpawned {
        id: "v1".into(),
        label: "Checking".into(),
        prompt: "go".into(),
    });
    a.open_subagent_view("v1".into());
    assert!(a.apply(AgentEvent::SubagentTranscript {
        id: "v1".into(),
        line: SubagentLine::Thinking("hmm".into()),
    }));
    // A different subagent's transcript shouldn't force a redraw.
    assert!(!a.apply(AgentEvent::SubagentTranscript {
        id: "other".into(),
        line: SubagentLine::Thinking("nope".into()),
    }));
}

#[test]
fn needs_animation_while_subagent_running() {
    let mut a = app();
    a.apply(AgentEvent::SubagentSpawned {
        id: "v1".into(),
        label: "Checking".into(),
        prompt: "go".into(),
    });
    assert!(a.needs_animation());
    a.apply(AgentEvent::SubagentStatus {
        id: "v1".into(),
        status: SubagentStatus::Done,
        detail: "done".into(),
    });
    assert!(!a.needs_animation());
}

#[test]
fn deadline_only_state_does_not_request_fast_animation() {
    let mut a = app();
    a.flash("saved");
    assert!(a.animation_interval().is_none());
    assert!(a.next_visual_deadline().is_some());

    a.apply(AgentEvent::GoalSet {
        objective: "wait".into(),
        deadline: None,
    });
    a.running = false;
    assert_eq!(
        a.animation_interval(),
        Some(std::time::Duration::from_secs(1))
    );
}

#[test]
fn completed_todos_retire_after_a_short_acknowledgement() {
    let mut a = app();
    a.apply(AgentEvent::TodosUpdated {
        items: vec![hive_core::TodoItem {
            text: "Ship it".into(),
            done: true,
        }],
    });

    assert_eq!(a.todos.len(), 1);
    assert!(a.todos_completed_at.is_some());
    assert!(
        a.needs_animation(),
        "the retirement timer must keep ticking"
    );
    assert!(a
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Todos(_))));

    a.todos_completed_at = std::time::Instant::now().checked_sub(std::time::Duration::from_millis(
        (TASKS_DONE_VISIBLE_MS + 1) as u64,
    ));
    assert!(a.tick());
    assert!(a.todos.is_empty());
    assert!(a.todos_completed_at.is_none());
    assert!(!a
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Todos(_))));
}

#[test]
fn active_todos_do_not_expire() {
    let mut a = app();
    a.apply(AgentEvent::TodosUpdated {
        items: vec![hive_core::TodoItem {
            text: "Keep working".into(),
            done: false,
        }],
    });

    assert!(a.todos_completed_at.is_none());
    assert_eq!(a.todos.len(), 1);
}

#[test]
fn running_terminal_animates_spinner() {
    let mut a = app();
    a.apply(AgentEvent::TerminalStarted {
        id: "term-1".into(),
        command: "sleep 999".into(),
        description: "Wait for server".into(),
        rows: 20,
        cols: 80,
    });
    a.apply(AgentEvent::TerminalState {
        id: "term-1".into(),
        controller: TerminalController::Agent,
        process: TerminalProcessState::Running,
        revision: 1,
    });
    assert!(a.needs_animation());
}

#[test]
fn subagent_usage_updates_card_and_sidebar_list() {
    let mut a = app();
    a.apply(AgentEvent::SubagentSpawned {
        id: "v1".into(),
        label: "Checking".into(),
        prompt: "go".into(),
    });
    assert!(a.apply(AgentEvent::SubagentUsage {
        id: "v1".into(),
        usage: Usage {
            prompt_tokens: 20,
            completion_tokens: 10,
            total_tokens: 30,
        },
    }));
    let cards = a.sidebar_subagents();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].usage.total_tokens, 30);
}

#[test]
fn tab_toggles_agent_mode() {
    use hive_core::AgentMode;

    let mut a = app();
    assert_eq!(a.agent_mode, AgentMode::Make);
    a.toggle_agent_mode();
    assert_eq!(a.agent_mode, AgentMode::Plan);
    a.toggle_agent_mode();
    assert_eq!(a.agent_mode, AgentMode::Multitask);
    a.toggle_agent_mode();
    assert_eq!(a.agent_mode, AgentMode::Make);
}

#[test]
fn mode_switched_updates_chip_and_adds_card() {
    use hive_core::AgentMode;

    let mut a = app();
    assert_eq!(a.agent_mode, AgentMode::Make);
    assert!(a.apply(AgentEvent::ModeSwitched {
        mode: AgentMode::Plan,
        reason: "This is a large multi-part feature.".into(),
    }));
    // Footer chip / next-turn default now follow the agent's choice.
    assert_eq!(a.agent_mode, AgentMode::Plan);
    let Some(Block::ModeSwitch(card)) = a.blocks.last() else {
        panic!("expected a mode-switch card");
    };
    assert_eq!(card.mode, AgentMode::Plan);
    assert!(card.reason.contains("large multi-part"));

    // The card renders the title and the reason in the transcript.
    let buf = comb::render(comb::Size::new(120, 40), |f| crate::render::draw(f, &mut a));
    let text = buf.text();
    assert!(text.contains("Switched to Plan Mode"), "{text}");
    assert!(text.contains("large multi-part"), "{text}");
}

#[test]
fn loop_detected_adds_card_and_flash() {
    let mut a = app();
    assert!(a.apply(AgentEvent::LoopDetected {
        tool: "read_file".into(),
    }));
    assert!(matches!(a.blocks.last(), Some(Block::LoopDetected(_))));

    let buf = comb::render(comb::Size::new(120, 40), |f| crate::render::draw(f, &mut a));
    let text = buf.text();
    assert!(text.contains("Loop detected"), "{text}");
    assert!(text.contains("restarting task"), "{text}");
}

#[test]
fn compacted_card_shows_token_reduction() {
    let mut a = app();
    // In-progress: spinner.
    a.apply(AgentEvent::Compacted {
        before: None,
        after: 0,
    });
    assert!(a.needs_animation());
    let buf = comb::render(comb::Size::new(120, 40), |f| crate::render::draw(f, &mut a));
    let text = buf.text();
    assert!(text.contains("Compacting context"), "{text}");

    // Done: token reduction.
    a.apply(AgentEvent::Compacted {
        before: Some(200_000),
        after: 15_000,
    });
    // Flash from the event keeps animation alive briefly; clear it.
    a.flash_msg = None;
    a.ctrl_c_armed = None;
    assert!(!a.needs_animation());
    let buf = comb::render(comb::Size::new(120, 40), |f| crate::render::draw(f, &mut a));
    let text = buf.text();
    assert!(text.contains("Compacted context"), "{text}");
    assert!(text.contains("200k"), "{text}");
    assert!(text.contains("15k"), "{text}");
}

#[test]
fn terminal_events_update_one_persistent_card() {
    let mut app = app();
    app.apply(AgentEvent::TerminalStarted {
        id: "term-1".into(),
        command: "theme-installer".into(),
        description: "Install a color theme".into(),
        rows: 30,
        cols: 120,
    });
    app.apply(AgentEvent::TerminalOutput {
        id: "term-1".into(),
        frame: hive_core::TerminalOutputFrame::from_bytes(
            30,
            120,
            10_000,
            b"Choose preset:\r\n",
            1,
        ),
    });
    app.apply(AgentEvent::TerminalState {
        id: "term-1".into(),
        controller: TerminalController::Agent,
        process: TerminalProcessState::Running,
        revision: 1,
    });

    let cards: Vec<_> = app
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Terminal(card) => Some(card),
            _ => None,
        })
        .collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].description, "Install a color theme");
}

#[test]
fn private_terminal_prompt_gets_one_clear_handoff() {
    let mut app = app();
    app.apply(AgentEvent::TerminalStarted {
        id: "term-1".into(),
        command: "sudo apt upgrade".into(),
        description: "Upgrade packages".into(),
        rows: 20,
        cols: 80,
    });
    app.apply(AgentEvent::TerminalOutput {
        id: "term-1".into(),
        frame: hive_core::TerminalOutputFrame::from_bytes(
            20,
            80,
            10_000,
            b"[sudo] password for user:",
            1,
        ),
    });

    assert!(app
        .flash_text()
        .is_some_and(|text| text.contains("private input")));
    assert!(app
        .terminal_card("term-1")
        .and_then(|card| card.input_request())
        .is_some_and(|request| request.is_private()));

    // Coalesced/repeated frames for the same prompt do not nag again.
    app.flash_msg = None;
    app.apply(AgentEvent::TerminalOutput {
        id: "term-1".into(),
        frame: hive_core::TerminalOutputFrame::from_bytes(
            20,
            80,
            10_000,
            b"[sudo] password for user:",
            2,
        ),
    });
    assert!(app.flash_text().is_none());
}

#[test]
fn failed_terminal_start_creates_a_visible_failed_card() {
    let mut app = app();
    app.apply(AgentEvent::TerminalStartFailed {
        id: "term-failed".into(),
        command: "installer".into(),
        description: "Install something".into(),
        message: "cannot spawn".into(),
    });

    let card = app.terminal_card("term-failed").unwrap();
    assert!(matches!(card.process, TerminalProcessState::Failed { .. }));
    assert!(card.status_text().contains("cannot spawn"));
}

#[test]
fn terminal_tool_calls_do_not_create_generic_tool_cards() {
    let mut app = app();
    for name in [
        "terminal_start",
        "terminal_read",
        "terminal_write",
        "terminal_stop",
    ] {
        app.apply(AgentEvent::ToolStarted {
            id: format!("{name}-call"),
            name: name.into(),
            args_preview: "term-1".into(),
        });
    }
    assert!(!app
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Tool(_))));
}

#[test]
fn tool_output_truncation_preserves_utf8_boundaries() {
    let mut a = app();
    a.apply(AgentEvent::ToolStarted {
        id: "shell".into(),
        name: "run_shell".into(),
        args_preview: String::new(),
    });
    a.apply(AgentEvent::ToolOutput {
        id: "shell".into(),
        chunk: format!("{}x", "я".repeat(4000)),
    });

    let Block::Tool(card) = a.blocks.last().expect("tool card") else {
        panic!("expected tool card");
    };
    assert!(card.output.len() <= 8000);
    assert!(card.output.is_char_boundary(0));
}

#[test]
fn plan_rewrite_moves_single_card_to_bottom_with_updated_badge() {
    use hive_core::event::AgentEvent;

    let mut a = app();
    let plans = |a: &App| {
        a.blocks
            .iter()
            .filter(|b| matches!(b, Block::Plan(_)))
            .count()
    };

    // First plan: one card, no badge.
    a.apply(AgentEvent::ToolStarted {
        id: "t1".into(),
        name: "write_plan".into(),
        args_preview: "Initial plan".into(),
    });
    a.apply(AgentEvent::PlanUpdated {
        summary: "Initial".into(),
        body: "## Alpha\n\nx\n".into(),
    });
    assert_eq!(plans(&a), 1);

    // Conversation continues, then the agent rewrites the plan — twice.
    a.push_user("note: change it".into());
    for (id, summary, body) in [
        ("t2", "Revised", "## Alpha\n\nrewritten\n"),
        ("t3", "Revised again", "## Alpha\n\nrewritten again\n"),
    ] {
        a.apply(AgentEvent::ToolStarted {
            id: id.into(),
            name: "write_plan".into(),
            args_preview: summary.into(),
        });
        a.apply(AgentEvent::PlanUpdated {
            summary: summary.into(),
            body: body.into(),
        });
    }

    // Still exactly one card; it moved below the user note and is badged.
    assert_eq!(plans(&a), 1, "rewrites must not stack extra plan cards");
    let Some(Block::Plan(card)) = a.blocks.last() else {
        panic!("expected the plan card at the bottom");
    };
    assert!(card.revised, "card carries the UPDATED badge");
    assert_eq!(card.summary, "Revised again");
    assert_eq!(a.plan_card().unwrap().summary, "Revised again");
}

#[test]
fn plan_rewrite_in_view_pins_top_and_flashes() {
    use hive_core::event::AgentEvent;

    let mut a = app();
    a.apply(AgentEvent::PlanUpdated {
        summary: "A".into(),
        body: "## Alpha\n\nx\n".into(),
    });
    a.open_plan_view();
    a.scroll_from_bottom = 0; // pretend the reader scrolled to the bottom
    a.apply(AgentEvent::PlanUpdated {
        summary: "A2".into(),
        body: "## Alpha\n\nrewritten\n".into(),
    });
    assert_eq!(a.scroll_from_bottom, usize::MAX, "rewrite pins to top");
    assert_eq!(a.flash_text(), Some("plan revised"));
}

#[test]
fn plan_corrections_message_lists_excerpts() {
    use hive_core::event::AgentEvent;

    let mut a = app();
    a.apply(AgentEvent::PlanUpdated {
        summary: "A".into(),
        body: "## Alpha\n\nx\n\n## Beta\n\ny\n".into(),
    });
    a.open_plan_view();
    a.plan_view.corrections.push(PlanCorrection {
        start: 0,
        end: 8,
        excerpt: "## Alpha".into(),
        note: "make it shorter".into(),
    });
    let msg = a.plan_corrections_message();
    assert!(msg.contains("## Alpha"), "{msg}");
    assert!(msg.contains("make it shorter"), "{msg}");
    assert!(msg.contains(".hive/Plan.md"), "{msg}");
}

#[test]
fn idle_timeout_blurs_focused_input() {
    let mut a = app();
    assert!(a.input_focused);
    a.input_last_activity = std::time::Instant::now().checked_sub(
        std::time::Duration::from_millis((INPUT_IDLE_BLUR_MS + 50) as u64),
    );
    assert!(a.tick(), "idle blur should request a redraw");
    assert!(!a.input_focused);
    assert!(a.input_last_activity.is_none());
}

#[test]
fn transcript_scroll_clamps_to_max() {
    let mut a = app();
    a.set_transcript_max_scroll(3);
    a.scroll_up(100);
    assert_eq!(a.scroll_from_bottom, 3);
    a.set_transcript_max_scroll(1);
    assert_eq!(
        a.scroll_from_bottom, 1,
        "shrinking max must unpin phantom offset"
    );
    assert!(!a.can_scroll_up());
    assert!(a.can_scroll_down());
    a.scroll_down(1);
    assert_eq!(a.scroll_from_bottom, 0);
    assert!(!a.can_scroll_down());
}

#[test]
fn recent_composer_activity_keeps_focus() {
    let mut a = app();
    a.focus_input();
    assert!(!a.tick());
    assert!(a.input_focused);

    a.input_last_activity = std::time::Instant::now().checked_sub(
        std::time::Duration::from_millis((INPUT_IDLE_BLUR_MS + 50) as u64),
    );
    a.note_input_activity();
    assert!(!a.tick());
    assert!(a.input_focused);
}

#[test]
fn percent_decode_handles_spaces_and_partials() {
    assert_eq!(super::percent_decode("a%20b"), "a b");
    assert_eq!(super::percent_decode("%2Fx%2Fy"), "/x/y");
    // Incomplete / invalid escapes are left untouched.
    assert_eq!(super::percent_decode("a%2"), "a%2");
    assert_eq!(super::percent_decode("a%zz"), "a%zz");
}

#[test]
fn strips_quotes_and_escapes() {
    assert_eq!(super::strip_matching_quotes("\"/a b/c.png\""), "/a b/c.png");
    assert_eq!(super::strip_matching_quotes("'/a/c.png'"), "/a/c.png");
    assert_eq!(super::strip_matching_quotes("/a/c.png"), "/a/c.png");
    assert_eq!(
        super::unescape_backslashes("/a/My\\ Photos/c.png"),
        "/a/My Photos/c.png"
    );
}

#[test]
fn drop_candidates_cover_terminal_quirks() {
    // Plain path.
    assert_eq!(
        super::drop_path_candidates("/a/b.png"),
        Some(vec!["/a/b.png".to_string()])
    );
    // Quoted path with spaces.
    assert_eq!(
        super::drop_path_candidates("\"/a/My Photos/b.png\""),
        Some(vec!["/a/My Photos/b.png".to_string()])
    );
    // Backslash-escaped spaces (kitty / iTerm2).
    assert_eq!(
        super::drop_path_candidates("/a/My\\ Photos/b.png"),
        Some(vec!["/a/My Photos/b.png".to_string()])
    );
    // Percent-encoded file:// URI (GNOME / VTE).
    assert_eq!(
        super::drop_path_candidates("file:///a/My%20Photos/b.png"),
        Some(vec!["/a/My Photos/b.png".to_string()])
    );
    // Multiple file:// URIs from a multi-file drop.
    assert_eq!(
        super::drop_path_candidates("file:///a/x.png\nfile:///a/y.png"),
        Some(vec!["/a/x.png".to_string(), "/a/y.png".to_string()])
    );
    // Multi-line prose is not treated as a path.
    assert_eq!(super::drop_path_candidates("hello\nworld"), None);
    assert_eq!(super::drop_path_candidates("   "), None);
}

#[test]
fn dropped_path_with_spaces_attaches_as_chip() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir()
        .join(format!("hive-drop-{n}"))
        .join("My Photos");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("shot.png");
    std::fs::write(&file, b"fakepng").unwrap();

    let mut a = app();
    // Quoted absolute path, as many terminals emit on drop.
    let quoted = format!("\"{}\"", file.display());
    assert_eq!(a.try_attach_pasted_path(&quoted), Some(String::new()));
    assert!(a.has_pending_attaches());
    // Chip shows the filename (label ends with the dropped file name).
    assert!(a.attachment_tags_line().ends_with("shot.png"));

    // Non-file text is left for the composer.
    assert_eq!(a.try_attach_pasted_path("just some text"), None);

    let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("hive-drop-{n}")));
}

fn assistant_hit(
    block: usize,
    response_row: usize,
    screen_row: u16,
    text: &str,
    join_before: crate::app::state::AssistantRowJoin,
) -> crate::app::state::AssistantRowHit {
    crate::app::state::AssistantRowHit {
        block,
        response_row,
        screen_row,
        x: 2,
        text: text.into(),
        join_before,
    }
}

#[test]
fn assistant_selection_extracts_partial_visible_text() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![assistant_hit(
        3,
        0,
        8,
        "hello world",
        AssistantRowJoin::Hard,
    )];

    assert!(a.start_assistant_selection(2, 8));
    assert!(a.update_assistant_selection(6, 8));
    assert_eq!(a.finish_assistant_selection(6, 8).as_deref(), Some("hello"));
}

#[test]
fn assistant_selection_reconstructs_soft_and_hard_breaks() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![
        assistant_hit(3, 0, 8, "hello", AssistantRowJoin::Hard),
        assistant_hit(3, 1, 9, "world", AssistantRowJoin::SoftSpace),
        assistant_hit(3, 2, 10, "code", AssistantRowJoin::Hard),
    ];

    assert!(a.start_assistant_selection(2, 8));
    assert!(a.update_assistant_selection(5, 10));
    assert_eq!(
        a.finish_assistant_selection(5, 10).as_deref(),
        Some("hello world\ncode")
    );
}

#[test]
fn assistant_selection_preserves_blank_paragraph_rows() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![
        assistant_hit(3, 0, 8, "hello", AssistantRowJoin::Hard),
        assistant_hit(3, 1, 9, "", AssistantRowJoin::Hard),
        assistant_hit(3, 2, 10, "world", AssistantRowJoin::Hard),
    ];

    assert!(a.start_assistant_selection(2, 8));
    assert!(a.update_assistant_selection(6, 10));
    assert_eq!(
        a.finish_assistant_selection(6, 10).as_deref(),
        Some("hello\n\nworld")
    );
}

#[test]
fn assistant_selection_drops_visual_indent_on_soft_wraps() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![
        assistant_hit(3, 0, 8, "  bullet text", AssistantRowJoin::Hard),
        assistant_hit(3, 1, 9, "  continues", AssistantRowJoin::SoftSpace),
    ];

    assert!(a.start_assistant_selection(2, 8));
    assert!(a.update_assistant_selection(12, 9));
    assert_eq!(
        a.finish_assistant_selection(12, 9).as_deref(),
        Some("  bullet text continues")
    );
}

#[test]
fn assistant_selection_supports_reverse_drag() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![
        assistant_hit(3, 0, 8, "hello", AssistantRowJoin::Hard),
        assistant_hit(3, 1, 9, "world", AssistantRowJoin::SoftSpace),
    ];

    assert!(a.start_assistant_selection(6, 9));
    assert!(a.update_assistant_selection(2, 8));
    assert_eq!(
        a.finish_assistant_selection(2, 8).as_deref(),
        Some("hello world")
    );
}

#[test]
fn assistant_selection_clamps_to_starting_response() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![
        assistant_hit(3, 0, 8, "first", AssistantRowJoin::Hard),
        assistant_hit(3, 1, 9, "answer", AssistantRowJoin::Hard),
        assistant_hit(4, 0, 10, "other", AssistantRowJoin::Hard),
    ];

    assert!(a.start_assistant_selection(2, 8));
    assert!(a.update_assistant_selection(6, 10));
    assert_eq!(
        a.finish_assistant_selection(6, 10).as_deref(),
        Some("first\nanswer")
    );
}

#[test]
fn assistant_selection_rejects_incomplete_row_coverage() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![
        assistant_hit(3, 0, 8, "first", AssistantRowJoin::Hard),
        assistant_hit(3, 2, 9, "third", AssistantRowJoin::Hard),
    ];

    assert!(a.start_assistant_selection(2, 8));
    assert!(a.update_assistant_selection(6, 9));
    assert_eq!(a.finish_assistant_selection(6, 9), None);
    assert!(a.assistant_selection.is_none());
}

#[test]
fn assistant_click_without_drag_does_not_copy() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![assistant_hit(3, 0, 8, "hello", AssistantRowJoin::Hard)];

    assert!(a.start_assistant_selection(2, 8));
    assert_eq!(a.finish_assistant_selection(2, 8), None);
    assert!(a.assistant_selection.is_none());
}

#[test]
fn completed_assistant_selection_is_consumed_and_cannot_be_reused() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![assistant_hit(
        3,
        0,
        8,
        "hello world",
        AssistantRowJoin::Hard,
    )];

    assert!(a.start_assistant_selection(2, 8));
    assert!(a.update_assistant_selection(6, 8));
    assert_eq!(a.finish_assistant_selection(6, 8).as_deref(), Some("hello"));
    assert!(!a.update_assistant_selection(7, 8));
    assert_eq!(a.finish_assistant_selection(7, 8), None);
    assert!(
        a.assistant_selection.is_none(),
        "highlight disappears after copying"
    );
}

#[test]
fn moving_a_plan_card_clears_block_indexed_assistant_selection() {
    use crate::app::state::{AssistantRowJoin, Block};
    use hive_core::event::AgentEvent;

    let mut a = app();
    a.apply(AgentEvent::PlanUpdated {
        summary: "first".into(),
        body: "# Plan\n\nOld".into(),
    });
    a.blocks.push(Block::Assistant {
        text: "answer".into(),
        streaming: false,
    });
    let block = a.blocks.len() - 1;
    a.assistant_row_hits = vec![assistant_hit(block, 0, 8, "answer", AssistantRowJoin::Hard)];
    assert!(a.start_assistant_selection(2, 8));
    assert!(a.update_assistant_selection(4, 8));

    a.apply(AgentEvent::PlanUpdated {
        summary: "second".into(),
        body: "# Plan\n\nRewritten".into(),
    });
    assert!(a.assistant_selection.is_none());
}

#[test]
fn assistant_selection_uses_display_columns_for_wide_glyphs() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![assistant_hit(3, 0, 8, "a界b", AssistantRowJoin::Hard)];

    assert!(a.start_assistant_selection(3, 8));
    assert!(a.update_assistant_selection(5, 8));
    assert_eq!(a.finish_assistant_selection(5, 8).as_deref(), Some("界b"));
}

#[test]
fn assistant_selection_width_matches_renderer_for_zwj_sequences() {
    use crate::app::state::AssistantRowJoin;

    let mut a = app();
    a.assistant_row_hits = vec![assistant_hit(3, 0, 8, "👩‍💻x", AssistantRowJoin::Hard)];

    // comb renders the two emoji scalars as two wide glyphs and skips ZWJ,
    // so `x` starts four display columns after the row origin.
    assert!(a.start_assistant_selection(6, 8));
    assert_eq!(
        a.assistant_selection.as_ref().map(|s| s.anchor.col),
        Some(4)
    );
}

#[test]
fn prompt_history_up_down_cycles() {
    let mut a = app();
    a.prompt_history.push("first");
    a.prompt_history.push("second");
    a.prompt_history.push("third");

    // Up on empty input → most recent entry.
    assert!(a.history_up());
    assert_eq!(a.input.value, "third");

    // Up again → previous entry.
    assert!(a.history_up());
    assert_eq!(a.input.value, "second");

    // Up again → oldest entry.
    assert!(a.history_up());
    assert_eq!(a.input.value, "first");

    // Up at oldest → stays (no-op, returns true to prevent scroll).
    assert!(a.history_up());
    assert_eq!(a.input.value, "first");

    // Down → next entry.
    assert!(a.history_down());
    assert_eq!(a.input.value, "second");

    // Down → most recent.
    assert!(a.history_down());
    assert_eq!(a.input.value, "third");

    // Down past end → exit browsing, restore draft (was empty).
    assert!(a.history_down());
    assert!(a.input.value.is_empty());
    assert!(!a.prompt_history.is_browsing());
}

#[test]
fn prompt_history_saves_and_restores_draft() {
    let mut a = app();
    a.prompt_history.push("old prompt");
    a.input.value = "my draft".into();

    // Up enters history (cursor at top of non-empty input).
    assert!(a.history_up());
    assert_eq!(a.input.value, "old prompt");

    // Down past end restores the draft.
    assert!(a.history_down());
    assert_eq!(a.input.value, "my draft");
}

#[test]
fn prompt_history_push_dedups_last() {
    let mut a = app();
    a.prompt_history.push("hello");
    a.prompt_history.push("hello");
    assert_eq!(a.prompt_history.entries.len(), 1);
    a.prompt_history.push("world");
    assert_eq!(a.prompt_history.entries.len(), 2);
}

#[test]
fn context_menu_copies_the_block_that_opened_it() {
    let mut a = app();
    a.blocks = vec![Block::User("older".into()), Block::User("newer".into())];

    a.open_prompt_menu(0);
    let action = a.take_context_action().expect("copy action");

    assert!(matches!(action, ContextAction::Copy(text) if text == "older"));
}

#[test]
fn turn_end_collapses_tool_details_and_menu_can_restore_them() {
    let mut a = app();
    a.apply(AgentEvent::ToolStarted {
        id: "edit-1".into(),
        name: "edit_file".into(),
        args_preview: "src/main.rs".into(),
    });
    a.apply(AgentEvent::ToolOutput {
        id: "edit-1".into(),
        chunk: "+1\tnew line".into(),
    });
    a.apply(AgentEvent::ToolFinished {
        id: "edit-1".into(),
        name: "edit_file".into(),
        ok: true,
        summary: "done".into(),
    });
    let tool_idx = a
        .blocks
        .iter()
        .position(|block| matches!(block, Block::Tool(_)))
        .expect("tool card");
    assert!(matches!(
        &a.blocks[tool_idx],
        Block::Tool(card) if card.details_open
    ));

    a.apply(AgentEvent::TurnFinished);
    assert!(matches!(
        &a.blocks[tool_idx],
        Block::Tool(card) if !card.details_open
    ));

    a.open_tool_menu(tool_idx);
    let action = a.take_context_action().expect("details action");
    let ContextAction::ToggleToolDetails { id } = action else {
        panic!("first action should reveal details");
    };
    a.toggle_tool_details(&id);
    assert!(matches!(
        &a.blocks[tool_idx],
        Block::Tool(card) if card.details_open
    ));
}

#[test]
fn pending_dispatch_recall_restores_input_and_removes_block() {
    let mut a = app();
    a.blocks.clear();
    a.input.value = "test prompt".into();
    a.pending_dispatch = Some(PendingDispatch {
        agent_text: "test prompt".into(),
        images: Vec::new(),
        mode: AgentMode::Make,
        composer: "test prompt".into(),
        attaches: Vec::new(),
        submitted_at: std::time::Instant::now(),
    });
    a.push_user("test prompt".into());
    assert_eq!(a.blocks.len(), 1);

    assert!(a.recall_pending_dispatch());
    assert!(a.pending_dispatch.is_none());
    assert_eq!(a.input.value, "test prompt");
    assert!(a.blocks.is_empty(), "user block should be removed");
}

#[test]
fn resume_restores_every_parallel_tool_output() {
    use hive_core::message::{Message, ToolCall};

    let mut assistant = Message::assistant("running two");
    assistant.tool_calls = vec![
        ToolCall {
            id: "call_a".into(),
            name: "read_file".into(),
            arguments: "{\"path\":\"a\"}".into(),
        },
        ToolCall {
            id: "call_b".into(),
            name: "read_file".into(),
            arguments: "{\"path\":\"b\"}".into(),
        },
    ];

    let mut a = app();
    a.apply(AgentEvent::SessionLoaded {
        title: "two tools".into(),
        model: "acc/models/test".into(),
        context_window: 64_000,
        messages: vec![
            Message::system("sys"),
            Message::user("go"),
            assistant,
            Message::tool_result("call_a", "read_file", "output A"),
            Message::tool_result("call_b", "read_file", "output B"),
        ],
        usage: Default::default(),
    });

    let cards: Vec<&ToolCard> = a
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Tool(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].id, "call_a");
    assert_eq!(cards[0].args, "a", "resume should not show raw JSON");
    assert_eq!(cards[0].output, "output A");
    assert!(cards[0].elapsed_ms.is_none(), "unknown timing stays hidden");
    assert!(!cards[0].details_open);
    assert_eq!(cards[1].id, "call_b");
    assert_eq!(cards[1].output, "output B");
    assert_eq!(a.context_window, 64_000);
}

#[test]
fn resume_skips_tools_with_dedicated_ui() {
    use hive_core::message::{Message, ToolCall};

    let mut assistant = Message::assistant("");
    assistant.tool_calls = vec![
        ToolCall {
            id: "todos".into(),
            name: "set_todos".into(),
            arguments: r#"{"todos":[]}"#.into(),
        },
        ToolCall {
            id: "terminal".into(),
            name: "terminal_start".into(),
            arguments: r#"{"command":"cargo test"}"#.into(),
        },
        ToolCall {
            id: "read".into(),
            name: "read_file".into(),
            arguments: r#"{"path":"README.md"}"#.into(),
        },
    ];

    let mut a = app();
    a.apply(AgentEvent::SessionLoaded {
        title: "clean resume".into(),
        model: "test".into(),
        context_window: 0,
        messages: vec![assistant],
        usage: Default::default(),
    });

    let tools: Vec<&ToolCard> = a
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Tool(card) => Some(card),
            _ => None,
        })
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].id, "read");
    assert_eq!(tools[0].args, "README.md");
}

#[test]
fn resume_does_not_fill_the_context_gauge_from_session_totals() {
    use hive_core::message::Message;
    use hive_core::provider::Usage;

    let mut a = app();
    a.context_tokens = 4_000;
    a.apply(AgentEvent::SessionLoaded {
        title: "long chat".into(),
        model: "acc/models/test".into(),
        context_window: 0,
        messages: vec![Message::user("hi")],
        // Cumulative session spend, far above the live context.
        usage: Usage {
            prompt_tokens: 900_000,
            completion_tokens: 100_000,
            total_tokens: 1_000_000,
        },
    });

    // The gauge waits for ContextTokens instead of showing the total.
    assert_eq!(a.context_tokens, 0);
    assert_eq!(
        a.context_window, 128_000,
        "unknown window keeps the old one"
    );
    assert_eq!(a.usage.total_tokens, 1_000_000);

    a.apply(AgentEvent::ContextTokens(12_345));
    assert_eq!(a.context_tokens, 12_345);
}
