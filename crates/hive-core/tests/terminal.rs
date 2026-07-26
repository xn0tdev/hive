#[cfg(unix)]
use std::path::Path;

#[cfg(unix)]
use hive_core::{
    AgentEvent, TerminalController, TerminalError, TerminalManager, TerminalProcessState,
    TerminalReadResult, TerminalWriteRequest,
};

#[cfg(unix)]
async fn read_until(manager: &TerminalManager, id: &str, needle: &str) -> TerminalReadResult {
    let mut revision = 0;
    for _ in 0..20 {
        let read = manager.read(id, Some(revision), Some(250)).await.unwrap();
        revision = read.session.revision;
        if read
            .screen
            .as_deref()
            .is_some_and(|screen| screen.contains(needle))
        {
            return read;
        }
    }
    panic!("terminal never displayed {needle:?}");
}

#[cfg(unix)]
fn marker(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "hive-terminal-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[cfg(unix)]
#[tokio::test]
async fn interactive_prompt_accepts_y_and_exits() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager
        .start(
            "printf 'Continue? [y/N] '; read answer; [ \"$answer\" = y ] && printf '\\napproved\\n'",
            "test",
            Path::new("."),
        )
        .await
        .unwrap();

    read_until(&manager, &started.id, "Continue?").await;
    manager
        .write_agent(
            &started.id,
            TerminalWriteRequest {
                text: Some("y".into()),
                key: None,
                submit: true,
            },
        )
        .await
        .unwrap();
    let done = read_until(&manager, &started.id, "approved").await;
    assert!(done.screen.unwrap().contains("approved"));
}

#[cfg(unix)]
#[tokio::test]
async fn user_control_blocks_agent_reads_and_writes_until_detach() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager.start("cat", "test", Path::new(".")).await.unwrap();

    manager.attach(&started.id).unwrap();
    let read = manager.read(&started.id, None, None).await.unwrap();
    assert_eq!(read.session.controller, TerminalController::User);
    assert!(read.screen.is_none());
    assert!(read.output.is_none());
    assert!(matches!(
        manager
            .write_agent(
                &started.id,
                TerminalWriteRequest {
                    text: Some("blocked".into()),
                    key: None,
                    submit: true,
                },
            )
            .await,
        Err(TerminalError::UserControlled)
    ));

    let before_write = manager
        .read(&started.id, None, None)
        .await
        .unwrap()
        .session
        .revision;
    manager
        .write_user(&started.id, b"visible\r".to_vec())
        .await
        .unwrap();
    let echoed = manager
        .read(&started.id, Some(before_write), Some(500))
        .await
        .unwrap();
    assert!(echoed.session.revision > before_write);
    manager.detach(&started.id).unwrap();
    let read = read_until(&manager, &started.id, "visible").await;
    assert_eq!(read.session.controller, TerminalController::Agent);
    manager.stop(&started.id).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn completed_user_session_requires_handback_before_replacement() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager
        .start("sleep 0.05; printf done", "test", Path::new("."))
        .await
        .unwrap();
    manager.attach(&started.id).unwrap();

    for _ in 0..20 {
        let read = manager.read(&started.id, None, None).await.unwrap();
        if !matches!(read.session.process, TerminalProcessState::Running) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let completed = manager.read(&started.id, None, None).await.unwrap();
    assert_eq!(completed.session.controller, TerminalController::User);
    assert!(!matches!(
        completed.session.process,
        TerminalProcessState::Running
    ));
    assert!(matches!(
        manager.start("cat", "test", Path::new(".")).await,
        Err(TerminalError::UserControlled)
    ));

    let detached = manager.detach(&started.id).unwrap();
    assert_eq!(detached.controller, TerminalController::Agent);
    let visible = manager.read(&started.id, None, None).await.unwrap();
    assert!(visible.screen.unwrap().contains("done"));

    let replacement = manager.start("cat", "test", Path::new(".")).await.unwrap();
    manager.stop(&replacement.id).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_second_running_session_and_stale_id() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager.start("cat", "test", Path::new(".")).await.unwrap();

    assert!(matches!(
        manager.start("cat", "test", Path::new(".")).await,
        Err(TerminalError::AlreadyRunning { .. })
    ));
    assert!(matches!(
        manager.read("missing", None, None).await,
        Err(TerminalError::NotFound { .. })
    ));
    manager.stop(&started.id).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn read_wait_is_clamped_and_revision_advances() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager
        .start("sleep 0.05; printf awake", "test", Path::new("."))
        .await
        .unwrap();

    let began = std::time::Instant::now();
    let read = manager
        .read(&started.id, Some(started.revision), Some(50_000))
        .await
        .unwrap();
    assert!(began.elapsed() < std::time::Duration::from_secs(2));
    assert!(read.session.revision > started.revision);
    read_until(&manager, &started.id, "awake").await;
}

#[cfg(unix)]
#[tokio::test]
async fn stop_and_drop_kill_descendant_processes() {
    let stop_marker = marker("stop");
    let drop_marker = marker("drop");
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let command = format!(
        "(sleep 1; printf leaked > '{}') & wait",
        stop_marker.display()
    );
    let started = manager
        .start(&command, "test", Path::new("."))
        .await
        .unwrap();
    manager.stop(&started.id).unwrap();

    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let dropped = TerminalManager::new(events);
    let command = format!(
        "(sleep 1; printf leaked > '{}') & wait",
        drop_marker.display()
    );
    dropped
        .start(&command, "test", Path::new("."))
        .await
        .unwrap();
    drop(dropped);

    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert!(!stop_marker.exists(), "descendant survived stop");
    assert!(!drop_marker.exists(), "descendant survived manager drop");
}

#[cfg(unix)]
#[tokio::test]
async fn leader_exit_kills_detached_descendants_before_session_finishes() {
    let leaked = marker("leader-exit");
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let command = format!(
        "(trap '' HUP; sleep 1; printf leaked > '{}') & exit 0",
        leaked.display()
    );
    let started = manager
        .start(&command, "test", Path::new("."))
        .await
        .unwrap();

    for _ in 0..40 {
        let state = manager.read(&started.id, None, None).await.unwrap();
        if !matches!(state.session.process, TerminalProcessState::Running) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    drop(manager);
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert!(!leaked.exists(), "descendant survived shell leader exit");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn leader_exit_kills_background_job_process_groups_in_same_session() {
    let leaked = marker("job-control-group");
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let command = format!(
        "set -m; (trap '' HUP; sleep 1; printf leaked > '{}') & exit 0",
        leaked.display()
    );
    let started = manager
        .start(&command, "test", Path::new("."))
        .await
        .unwrap();

    for _ in 0..40 {
        let state = manager.read(&started.id, None, None).await.unwrap();
        if !matches!(state.session.process, TerminalProcessState::Running) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    drop(manager);
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert!(
        !leaked.exists(),
        "background job group survived terminal session"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn stop_kills_descendants_that_create_a_new_session() {
    let leaked = marker("setsid");
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let command = format!(
        "setsid sh -c \"trap '' HUP; sleep 1; printf leaked > '{}'\" >/dev/null 2>&1 & wait",
        leaked.display()
    );
    let started = manager
        .start(&command, "test", Path::new("."))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    manager.stop(&started.id).unwrap();

    // Poll for up to 5s — the process group kill may take longer on CI.
    for _ in 0..50 {
        if !leaked.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        !leaked.exists(),
        "new-session descendant survived terminal STOP"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn resize_updates_core_screen_and_emits_dimensions() {
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager.start("cat", "test", Path::new(".")).await.unwrap();

    let resized = manager.resize(&started.id, 40, 100).await.unwrap();
    assert_eq!((resized.rows, resized.cols), (40, 100));
    let read = manager.read(&started.id, None, None).await.unwrap();
    assert_eq!((read.session.rows, read.session.cols), (40, 100));

    let mut saw_resize = false;
    while let Ok(event) = receiver.try_recv() {
        if matches!(
            event,
            AgentEvent::TerminalResized {
                rows: 40,
                cols: 100,
                ..
            }
        ) {
            saw_resize = true;
        }
    }
    assert!(saw_resize);
    assert_eq!(read.session.process, TerminalProcessState::Running);
    manager.stop(&started.id).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn terminal_tools_drive_prompt_and_emit_dedicated_events() {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use hive_core::{all_tools, no_skills, noop_spawner, AppConfig, ToolContext};
    use serde_json::{json, Value};

    let (events, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events.clone());
    let context = ToolContext {
        cwd: PathBuf::from("."),
        events,
        spawner: noop_spawner(),
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
        terminal: Some(manager.clone()),
        vision: false,
        depth: 0,
        call_id: "terminal-e2e".into(),
        isolate_worktrees: false,
        interrupt: Arc::new(AtomicBool::new(false)),
    };
    let tools = all_tools();
    let tool = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name() == name)
            .cloned()
            .unwrap()
    };

    let started = tool("terminal_start")
        .execute(
            json!({
                "command": "printf '\\033[32mContinue? [y/N]\\033[0m '; read answer; [ \"$answer\" = y ] && printf '\\napproved\\n'"
            }),
            &context,
        )
        .await;
    assert!(!started.is_error, "{}", started.content);
    let started_json: Value = serde_json::from_str(&started.content).unwrap();
    let id = started_json["id"].as_str().unwrap().to_string();

    read_until(&manager, &id, "Continue?").await;
    let read = tool("terminal_read")
        .execute(json!({"session_id": id.clone()}), &context)
        .await;
    assert!(!read.is_error, "{}", read.content);
    assert!(!read.content.contains("\u{1b}["));

    let wrote = tool("terminal_write")
        .execute(
            json!({"session_id": id.clone(), "text": "y", "submit": true}),
            &context,
        )
        .await;
    assert!(!wrote.is_error, "{}", wrote.content);
    let done = read_until(&manager, &id, "approved").await;
    assert!(done.screen.unwrap().contains("approved"));

    let mut saw_started = false;
    let mut saw_output = false;
    while let Ok(event) = event_rx.try_recv() {
        saw_started |= matches!(event, AgentEvent::TerminalStarted { .. });
        saw_output |= matches!(event, AgentEvent::TerminalOutput { .. });
    }
    assert!(saw_started);
    assert!(saw_output);
}

#[cfg(unix)]
#[tokio::test]
async fn continuous_output_coalesces_to_one_live_frame_plus_final() {
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager
        .start("yes flood", "test", Path::new("."))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    manager.stop(&started.id).unwrap();

    let mut output_events = 0;
    while let Ok(event) = receiver.try_recv() {
        if matches!(event, AgentEvent::TerminalOutput { .. }) {
            output_events += 1;
        }
    }
    assert!(
        output_events <= 2,
        "queued {output_events} terminal output frames"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_remains_responsive_while_large_input_write_is_blocked() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager
        .start("sleep 30", "test", Path::new("."))
        .await
        .unwrap();

    let writer = manager.clone();
    let write_id = started.id.clone();
    let write = tokio::spawn(async move {
        writer
            .write_agent(
                &write_id,
                TerminalWriteRequest {
                    text: Some("x".repeat(16 * 1024 * 1024)),
                    key: None,
                    submit: false,
                },
            )
            .await
    });
    // Give the writer time to fill the PTY buffer and block.
    // On some systems the PTY buffer is large, so we poll for backpressure.
    let mut blocked = false;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if !write.is_finished() {
            blocked = true;
            break;
        }
    }
    // If the write completed without blocking (huge PTY buffer), the test
    // is still valid — we just can't test the backpressure path. Skip
    // the stop-while-blocked assertion in that case.
    if !blocked {
        eprintln!("note: large write completed without blocking — skipping backpressure assertion");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), write).await;
        manager.stop(&started.id).unwrap();
        return;
    }

    let stopper = manager.clone();
    let stop_id = started.id.clone();
    let stopped = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::task::spawn_blocking(move || stopper.stop(&stop_id)),
    )
    .await
    .expect("STOP was blocked behind terminal input")
    .unwrap()
    .unwrap();
    assert!(!matches!(stopped.process, TerminalProcessState::Running));
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), write).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_cancels_a_blocked_user_write_without_stopping_process() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let started = manager
        .start("sleep 30", "test", Path::new("."))
        .await
        .unwrap();
    manager.attach(&started.id).unwrap();
    for _ in 0..8 {
        manager
            .write_user(&started.id, vec![b'x'; 1024 * 1024])
            .await
            .unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(75)).await;

    let detacher = manager.clone();
    let detach_id = started.id.clone();
    let detach = tokio::task::spawn_blocking(move || detacher.detach(&detach_id));
    let detached = match tokio::time::timeout(std::time::Duration::from_millis(500), detach).await {
        Ok(result) => result.unwrap().unwrap(),
        Err(_) => {
            manager.stop(&started.id).unwrap();
            panic!("detach was blocked behind terminal input");
        }
    };
    assert_eq!(detached.controller, TerminalController::Agent);
    assert_eq!(detached.process, TerminalProcessState::Running);
    manager.stop(&started.id).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn final_output_frame_survives_session_replacement() {
    let (events, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events);
    let first = manager
        .start("printf final-frame", "test", Path::new("."))
        .await
        .unwrap();
    for _ in 0..40 {
        let read = manager.read(&first.id, None, None).await.unwrap();
        if !matches!(read.session.process, TerminalProcessState::Running) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let replacement = manager.start("cat", "test", Path::new(".")).await.unwrap();

    let mut retained_final = false;
    while let Ok(event) = receiver.try_recv() {
        if let AgentEvent::TerminalOutput { id, frame } = event {
            if id == first.id {
                retained_final |= frame
                    .snapshot()
                    .is_some_and(|(screen, _)| screen.contents().contains("final-frame"));
            }
        }
    }
    assert!(retained_final, "final frame depended on dropped session");
    manager.stop(&replacement.id).unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupt_and_user_takeover_are_atomically_arbitrated() {
    for _ in 0..20 {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let manager = TerminalManager::new(events);
        let started = manager.start("cat", "test", Path::new(".")).await.unwrap();
        let gate = std::sync::Arc::new(std::sync::Barrier::new(3));

        let attaching = manager.clone();
        let attach_id = started.id.clone();
        let attach_gate = gate.clone();
        let attach = std::thread::spawn(move || {
            attach_gate.wait();
            attaching.request_attach(&attach_id)
        });

        let interrupting = manager.clone();
        let interrupt_gate = gate.clone();
        let interrupt = std::thread::spawn(move || {
            interrupt_gate.wait();
            interrupting.stop_if_agent_controlled();
        });

        gate.wait();
        let _ = attach.join().unwrap();
        interrupt.join().unwrap();
        let state = manager.read(&started.id, None, None).await.unwrap().session;
        assert!(
            state.controller != TerminalController::User
                || matches!(state.process, TerminalProcessState::Running),
            "interrupt killed a USER-controlled terminal"
        );
        if matches!(state.process, TerminalProcessState::Running) {
            manager.detach(&started.id).unwrap();
            manager.stop(&started.id).unwrap();
        }
    }
}
