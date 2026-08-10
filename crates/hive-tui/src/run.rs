//! Terminal setup and the blocking event loop. Translates key events into
//! edits, scrolling, slash commands, and `InputCommand`s for the agent driver.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use comb::{Event, Key, KeyCode, Mouse, MouseButton, MouseKind, MouseMode, Terminal};
use tokio::sync::mpsc::UnboundedSender;

use hive_core::event::EventReceiver;
use hive_core::message::ImageSource;
use hive_core::AgentMode;

use crate::app::palette::PaletteMode;
use crate::app::state::{ContextAction, TerminalViewPhase};
use crate::app::App;
use crate::commands::{self, CmdId};
use crate::render;
use crate::terminal_input::{terminal_key_bytes, terminal_paste_bytes};
use crate::{InputCommand, PrivateTerminalInput, TuiInit};

const UNKNOWN_HINT: &str = "Ctrl+P for commands · /about for About";

/// Idle poll — long enough to skip needless redraws, short enough for input.
const IDLE_TICK: Duration = Duration::from_millis(250);
/// Keep a hot producer from starving keyboard and mouse polling.
const MAX_EVENTS_PER_TICK: usize = 128;

struct FrameProfiler {
    enabled: bool,
    window_started: Instant,
    frames: u64,
    total: Duration,
    max: Duration,
    changed_cells: u64,
    max_changed_cells: usize,
}

impl FrameProfiler {
    fn from_env() -> Self {
        Self {
            enabled: std::env::var_os("HIVE_TUI_PROFILE").is_some(),
            window_started: Instant::now(),
            frames: 0,
            total: Duration::ZERO,
            max: Duration::ZERO,
            changed_cells: 0,
            max_changed_cells: 0,
        }
    }

    fn start(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }

    fn finish(&mut self, started: Option<Instant>, changed_cells: usize) {
        let Some(started) = started else { return };
        let elapsed = started.elapsed();
        self.frames += 1;
        self.total += elapsed;
        self.max = self.max.max(elapsed);
        self.changed_cells = self.changed_cells.saturating_add(changed_cells as u64);
        self.max_changed_cells = self.max_changed_cells.max(changed_cells);
        if self.window_started.elapsed() >= Duration::from_secs(1) {
            let average_us = self.total.as_micros() / u128::from(self.frames.max(1));
            tracing::debug!(
                target: "hive_tui::perf",
                frames = self.frames,
                average_us,
                max_us = self.max.as_micros(),
                average_changed_cells = self.changed_cells / self.frames.max(1),
                max_changed_cells = self.max_changed_cells,
                "TUI frame timings"
            );
            self.window_started = Instant::now();
            self.frames = 0;
            self.total = Duration::ZERO;
            self.max = Duration::ZERO;
            self.changed_cells = 0;
            self.max_changed_cells = 0;
        }
    }
}

pub fn run(
    init: TuiInit,
    mut events: EventReceiver,
    input_tx: UnboundedSender<InputCommand>,
    interrupt: Arc<AtomicBool>,
) -> io::Result<()> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "hive must be run in an interactive terminal",
        ));
    }
    // comb enters raw mode + the alternate screen and turns on mouse reporting;
    // its `Drop` restores everything, so there's no manual teardown here.
    let mut terminal = Terminal::new()?;
    // Need button-drag reports so the sidebar left edge can be resized.
    terminal.mouse_mode(MouseMode::Motion)?;
    let mut app = App::new(init);

    run_loop(&mut terminal, &mut app, &mut events, &input_tx, &interrupt)
}

fn run_loop(
    terminal: &mut Terminal,
    app: &mut App,
    events: &mut EventReceiver,
    input_tx: &UnboundedSender<InputCommand>,
    interrupt: &Arc<AtomicBool>,
) -> io::Result<()> {
    let mut dirty = true;
    let mut last_animation_frame = u128::MAX;
    let mut profiler = FrameProfiler::from_env();
    loop {
        let content_dirty = drain_events(app, events, input_tx);

        if app.refresh_project() {
            dirty = true;
        }

        if app.tick() {
            // Idle composer blur (or other tick-side visual change).
            dirty = true;
        }

        // Flush a deferred dispatch after the grace period (ESC recall window).
        if app.pending_dispatch_ready() {
            if let Some(pd) = app.take_pending_dispatch() {
                let _ = input_tx.send(InputCommand::User {
                    text: pd.agent_text,
                    images: pd.images,
                    mode: pd.mode,
                });
                dirty = true;
            }
        }
        let animation_interval = app.animation_interval();
        let animation_frame = animation_interval.map(|step| app.animation_frame(step));
        let animation_moved = animation_frame.is_some_and(|frame| frame != last_animation_frame);

        if app.take_repaint_request() {
            terminal.invalidate()?;
            dirty = true;
        }

        if dirty || content_dirty || animation_moved {
            let frame_started = profiler.start();
            terminal.draw(|f| render::draw(f, app))?;
            profiler.finish(frame_started, terminal.last_changed_cells());
            if let Some((id, rows, cols)) = app.take_pending_terminal_resize() {
                if rows > 0 && cols > 0 {
                    let _ = input_tx.send(InputCommand::TerminalResize { id, rows, cols });
                }
            }
            last_animation_frame = animation_frame.unwrap_or(u128::MAX);
            dirty = false;
        }

        let mut wait = animation_interval.unwrap_or(IDLE_TICK).min(IDLE_TICK);
        if let Some(deadline) = app.next_visual_deadline() {
            wait = wait.min(deadline);
        }
        if let Some(ev) = terminal.read_event(wait)? {
            match ev {
                Event::Key(key) => {
                    if handle_key(app, key, input_tx, interrupt) {
                        break;
                    }
                    dirty = true;
                }
                Event::Mouse(m) => {
                    if handle_mouse(app, m, input_tx) {
                        dirty = true;
                    }
                    // Clicking `/quit` in a menu exits; the mouse handler only
                    // reports redraws, so the request is parked on the app.
                    if app.take_quit_request() {
                        break;
                    }
                }
                Event::Resize(_, _) => {
                    // Size is applied inside `Terminal::draw` (clear + full
                    // repaint). Flag dirty so idle sessions never skip it.
                    dirty = true;
                }
                Event::Paste(text) => {
                    if handle_paste(app, &text, input_tx) {
                        dirty = true;
                    }
                }
            }
        }
    }
    Ok(())
}

fn drain_events(
    app: &mut App,
    events: &mut EventReceiver,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    let mut dirty = false;
    for _ in 0..MAX_EVENTS_PER_TICK {
        let Ok(ev) = events.try_recv() else {
            break;
        };
        let finished = matches!(ev, hive_core::event::AgentEvent::TurnFinished);
        dirty |= app.apply(ev);
        if finished {
            dirty |= flush_follow_up(app, input_tx);
        }
    }
    if let Some(context) = app.take_pending_context_update() {
        let _ = input_tx.send(InputCommand::UpdateContextWindow { context });
    }
    dirty
}

#[path = "run/clipboard.rs"]
mod clipboard;
#[path = "run/keyboard.rs"]
mod keyboard;
#[path = "run/mouse.rs"]
mod mouse;
#[path = "run/overlays.rs"]
mod overlays;
#[path = "run/plan.rs"]
mod plan;
#[path = "run/session.rs"]
mod session;

use clipboard::*;
use keyboard::*;
use mouse::*;
use overlays::*;
use plan::*;
use session::*;

fn unique_connection_id(label: &str, existing: &[hive_core::event::ConnectionInfo]) -> String {
    let base: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() {
        "provider".into()
    } else {
        base
    };
    if !existing.iter().any(|c| c.id == base) {
        return base;
    }
    for n in 2..100 {
        let candidate = format!("{base}-{n}");
        if !existing.iter().any(|c| c.id == candidate) {
            return candidate;
        }
    }
    format!("{base}-x")
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
