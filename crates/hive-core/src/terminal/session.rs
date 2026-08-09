use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::event::{AgentEvent, EventSender};

use super::buffer::{PrintableDelta, TerminalBuffer};
use super::{
    terminal_input_request, TerminalController, TerminalError, TerminalProcessState,
    TerminalReadResult, TerminalSnapshot, TerminalWriteRequest, DEFAULT_COLS, DEFAULT_ROWS,
};

const MAX_QUEUED_WRITES: usize = 64;
const MAX_INPUT_BYTES: usize = 1024 * 1024;

#[cfg(target_os = "linux")]
struct LinuxCgroup {
    path: std::path::PathBuf,
}

#[cfg(target_os = "linux")]
impl LinuxCgroup {
    fn create(id: &str) -> std::io::Result<Self> {
        let membership = std::fs::read_to_string("/proc/self/cgroup")?;
        let relative = membership
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .ok_or_else(|| std::io::Error::other("cgroup v2 membership not found"))?;
        let parent = std::path::Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
        let path = parent.join(format!("hive-{id}"));
        std::fs::create_dir(&path)?;
        if !path.join("cgroup.kill").exists() {
            let _ = std::fs::remove_dir(&path);
            return Err(std::io::Error::other("cgroup.kill is unavailable"));
        }
        Ok(Self { path })
    }

    fn procs_path(&self) -> std::path::PathBuf {
        self.path.join("cgroup.procs")
    }

    fn add_pid(&self, pid: u32) -> std::io::Result<()> {
        std::fs::write(self.procs_path(), pid.to_string())
    }

    fn kill(&self) -> std::io::Result<()> {
        match std::fs::write(self.path.join("cgroup.kill"), "1") {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
        for _ in 0..100 {
            let empty = std::fs::read_to_string(self.path.join("cgroup.events"))
                .map(|events| events.lines().any(|line| line == "populated 0"))
                .unwrap_or(true);
            if empty {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = std::fs::remove_dir(&self.path);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for LinuxCgroup {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

#[derive(Clone)]
pub struct TerminalOutputFrame {
    source: Arc<TerminalOutputSource>,
}

enum TerminalOutputSource {
    Live {
        session: Weak<TerminalSession>,
        token: u64,
    },
    Fixed {
        screen: Box<vt100::Screen>,
        revision: u64,
    },
}

impl std::fmt::Debug for TerminalOutputFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TerminalOutputFrame(..)")
    }
}

impl TerminalOutputFrame {
    fn live(session: Weak<TerminalSession>, token: u64) -> Self {
        Self {
            source: Arc::new(TerminalOutputSource::Live { session, token }),
        }
    }

    pub fn from_bytes(
        rows: u16,
        cols: u16,
        scrollback: usize,
        bytes: &[u8],
        revision: u64,
    ) -> Self {
        let mut parser = vt100::Parser::new(rows.max(1), cols.max(1), scrollback);
        parser.process(bytes);
        Self {
            source: Arc::new(TerminalOutputSource::Fixed {
                screen: Box::new(parser.screen().clone()),
                revision,
            }),
        }
    }

    fn fixed(screen: vt100::Screen, revision: u64) -> Self {
        Self {
            source: Arc::new(TerminalOutputSource::Fixed {
                screen: Box::new(screen),
                revision,
            }),
        }
    }

    pub fn snapshot(&self) -> Option<(vt100::Screen, u64)> {
        match self.source.as_ref() {
            TerminalOutputSource::Fixed { screen, revision } => {
                Some((screen.as_ref().clone(), *revision))
            }
            TerminalOutputSource::Live { session, token } => {
                let session = session.upgrade()?;
                let mut state = session.lock_state();
                let snapshot = state.buffer.screen_snapshot();
                let revision = state.buffer.revision();
                if state.output_event_pending == Some(*token) {
                    state.output_event_pending = None;
                }
                Some((snapshot, revision))
            }
        }
    }
}

pub(crate) struct TerminalSession {
    pub id: String,
    pub command: String,
    state: Mutex<SessionState>,
    writer_tx: Sender<WriterCommand>,
    queued_writes: AtomicUsize,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    #[cfg(unix)]
    process_group: Option<i32>,
    #[cfg(target_os = "linux")]
    process_session: Option<i32>,
    #[cfg(target_os = "linux")]
    cgroup: Option<LinuxCgroup>,
    revision_tx: tokio::sync::watch::Sender<u64>,
    events: EventSender,
    process_changed: std::sync::Condvar,
}

enum WriterCommand {
    Write {
        bytes: Vec<u8>,
        controller: TerminalController,
        generation: u64,
        reply: Option<tokio::sync::oneshot::Sender<Result<(), TerminalError>>>,
    },
}

struct SessionState {
    controller: TerminalController,
    process: TerminalProcessState,
    buffer: TerminalBuffer,
    rows: u16,
    cols: u16,
    output_event_pending: Option<u64>,
    process_tree_cleaned: bool,
    controller_generation: u64,
    stopping: bool,
}

impl TerminalSession {
    pub(crate) fn spawn(
        id: String,
        command_text: &str,
        description: &str,
        cwd: &Path,
        events: EventSender,
    ) -> Result<Arc<Self>, TerminalError> {
        let pty = portable_pty::native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io_error)?;

        #[cfg(target_os = "linux")]
        let cgroup = LinuxCgroup::create(&id).ok();
        #[cfg(unix)]
        let mut command = CommandBuilder::new("sh");
        #[cfg(all(unix, not(target_os = "linux")))]
        command.args(["-c", command_text]);
        #[cfg(target_os = "linux")]
        if let Some(cgroup) = &cgroup {
            command.args([
                "-c",
                "hive_command=$HIVE_TERMINAL_COMMAND; \
                 printf '%s\\n' \"$$\" > \"$HIVE_TERMINAL_CGROUP_PROCS\" 2>/dev/null || :; \
                 unset HIVE_TERMINAL_COMMAND HIVE_TERMINAL_CGROUP_PROCS; \
                 exec sh -c \"$hive_command\"",
            ]);
            command.env("HIVE_TERMINAL_COMMAND", command_text);
            command.env(
                "HIVE_TERMINAL_CGROUP_PROCS",
                cgroup.procs_path().as_os_str(),
            );
        } else {
            command.args(["-c", command_text]);
        }
        #[cfg(windows)]
        let mut command = CommandBuilder::new("cmd.exe");
        #[cfg(windows)]
        command.args(["/C", command_text]);
        command.cwd(cwd);

        configure_nonblocking(pair.master.as_ref())?;
        let mut reader = pair.master.try_clone_reader().map_err(io_error)?;
        let writer = pair.master.take_writer().map_err(io_error)?;
        let mut child = pair.slave.spawn_command(command).map_err(io_error)?;
        drop(pair.slave);
        #[cfg(target_os = "linux")]
        if let (Some(cgroup), Some(pid)) = (&cgroup, child.process_id()) {
            let _ = cgroup.add_pid(pid);
        }
        let killer = child.clone_killer();
        #[cfg(unix)]
        let process_group = pair
            .master
            .process_group_leader()
            .or_else(|| child.process_id().and_then(|id| i32::try_from(id).ok()));
        #[cfg(target_os = "linux")]
        let process_session = child.process_id().and_then(|id| i32::try_from(id).ok());
        let (revision_tx, _) = tokio::sync::watch::channel(0);
        let (writer_tx, writer_rx) = std::sync::mpsc::channel();

        let session = Arc::new(Self {
            id,
            command: command_text.to_string(),
            state: Mutex::new(SessionState {
                controller: TerminalController::Agent,
                process: TerminalProcessState::Running,
                buffer: TerminalBuffer::new(DEFAULT_ROWS, DEFAULT_COLS),
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                output_event_pending: None,
                process_tree_cleaned: false,
                controller_generation: 0,
                stopping: false,
            }),
            writer_tx,
            queued_writes: AtomicUsize::new(0),
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
            #[cfg(unix)]
            process_group,
            #[cfg(target_os = "linux")]
            process_session,
            #[cfg(target_os = "linux")]
            cgroup,
            revision_tx,
            events,
            process_changed: std::sync::Condvar::new(),
        });

        let _ = session.events.send(AgentEvent::TerminalStarted {
            id: session.id.clone(),
            command: session.command.clone(),
            description: description.to_string(),
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
        });
        session.emit_state();

        let writer_session = Arc::downgrade(&session);
        std::thread::spawn(move || writer_loop(writer_session, writer, writer_rx));

        let (reader_done_tx, reader_done_rx) = std::sync::mpsc::sync_channel(1);
        let reader_session = Arc::clone(&session);
        std::thread::spawn(move || {
            let mut bytes = [0_u8; 8192];
            let mut outcome = Ok(());
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(read) => reader_session.record_output(&bytes[..read]),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => {
                        if error.raw_os_error() != Some(libc_eio()) {
                            let message = error.to_string();
                            let _ = reader_session.events.send(AgentEvent::TerminalError {
                                id: reader_session.id.clone(),
                                message: message.clone(),
                            });
                            let _ = reader_session.reserve_stop(None);
                            let _ = reader_session.kill_process_tree();
                            outcome = Err(message);
                        }
                        break;
                    }
                }
            }
            let _ = reader_done_tx.send(outcome);
        });

        let wait_session = Arc::clone(&session);
        std::thread::spawn(move || {
            let child_result = child.wait();
            let _ = wait_session.reserve_stop(None);
            let _ = wait_session.kill_process_tree();
            let reader_result = reader_done_rx
                .recv()
                .unwrap_or_else(|_| Err("terminal output reader stopped".into()));
            let process = match (child_result, reader_result) {
                (_, Err(message)) => TerminalProcessState::Failed { message },
                (Err(error), _) => TerminalProcessState::Failed {
                    message: error.to_string(),
                },
                (Ok(status), Ok(())) => {
                    let code = i32::try_from(status.exit_code()).unwrap_or(i32::MAX);
                    TerminalProcessState::Exited { code }
                }
            };
            wait_session.transition_process(process);
        });

        Ok(session)
    }

    pub(crate) fn snapshot(&self) -> TerminalSnapshot {
        let state = self.lock_state();
        self.snapshot_from(&state)
    }

    pub(crate) fn process_is_running(&self) -> bool {
        matches!(self.lock_state().process, TerminalProcessState::Running)
    }

    pub(crate) fn controller(&self) -> TerminalController {
        self.lock_state().controller
    }

    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.revision_tx.subscribe()
    }

    pub(crate) fn read_result(&self, after_revision: Option<u64>) -> TerminalReadResult {
        let state = self.lock_state();
        let session = self.snapshot_from(&state);
        if state.controller == TerminalController::User {
            return TerminalReadResult {
                session,
                screen: None,
                output: None,
                output_truncated: false,
                input_request: None,
            };
        }

        let PrintableDelta { text, truncated } =
            state.buffer.output_since(after_revision.unwrap_or(0));
        let screen = state.buffer.screen();
        let input_request = terminal_input_request(&screen);
        TerminalReadResult {
            session,
            screen: Some(screen),
            output: Some(text),
            output_truncated: truncated,
            input_request,
        }
    }

    pub(crate) async fn write_agent(
        &self,
        request: TerminalWriteRequest,
    ) -> Result<TerminalSnapshot, TerminalError> {
        let (bytes, generation) = {
            let state = self.lock_state();
            ensure_controller(&state, TerminalController::Agent)?;
            if terminal_input_request(&state.buffer.screen())
                .is_some_and(|input| input.is_private())
            {
                return Err(TerminalError::PrivateInputRequired);
            }
            (
                request.bytes(state.buffer.application_cursor())?,
                state.controller_generation,
            )
        };
        let (reply, result) = tokio::sync::oneshot::channel();
        self.queue_writer(WriterCommand::Write {
            bytes,
            controller: TerminalController::Agent,
            generation,
            reply: Some(reply),
        })?;
        result
            .await
            .map_err(|_| TerminalError::Io("terminal writer stopped".into()))??;
        Ok(self.snapshot())
    }

    pub(crate) fn write_user(&self, bytes: Vec<u8>) -> Result<TerminalSnapshot, TerminalError> {
        if bytes.is_empty() {
            return Err(TerminalError::EmptyInput);
        }
        let generation = {
            let state = self.lock_state();
            ensure_controller(&state, TerminalController::User)?;
            state.controller_generation
        };
        self.queue_writer(WriterCommand::Write {
            bytes,
            controller: TerminalController::User,
            generation,
            reply: None,
        })?;
        Ok(self.snapshot())
    }

    pub(crate) fn attach(&self) -> Result<TerminalSnapshot, TerminalError> {
        self.set_controller(TerminalController::User, true)
    }

    pub(crate) fn request_attach(&self) -> Result<TerminalSnapshot, TerminalError> {
        self.set_controller(TerminalController::User, true)
    }

    pub(crate) fn detach(&self) -> Result<TerminalSnapshot, TerminalError> {
        self.set_controller(TerminalController::Agent, false)
    }

    pub(crate) fn request_detach(&self) -> Result<TerminalSnapshot, TerminalError> {
        self.set_controller(TerminalController::Agent, false)
    }

    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<TerminalSnapshot, TerminalError> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let mut state = self.lock_state();
        ensure_active(&state)?;
        {
            let master = self.master.lock().unwrap_or_else(|e| e.into_inner());
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(io_error)?;
        }

        state.rows = rows;
        state.cols = cols;
        let revision = state.buffer.resize(rows, cols);
        self.publish_revision(revision);
        let _ = self.events.send(AgentEvent::TerminalResized {
            id: self.id.clone(),
            rows,
            cols,
        });
        self.emit_state_from(&state);
        Ok(self.snapshot_from(&state))
    }

    pub(crate) fn stop(&self) -> Result<TerminalSnapshot, TerminalError> {
        let should_kill = match self.reserve_stop(None) {
            Ok(should_kill) => should_kill,
            // Already finished — idempotent success for agent/UI STOP.
            Err(TerminalError::NotRunning) => return Ok(self.snapshot()),
            Err(error) => return Err(error),
        };
        if should_kill {
            if let Err(error) = self.kill_process_tree() {
                self.release_stop();
                return Err(error);
            }
        }
        Ok(self.wait_for_exit())
    }

    pub(crate) fn stop_if_agent_controlled(&self) {
        let Ok(should_kill) = self.reserve_stop(Some(TerminalController::Agent)) else {
            return;
        };
        if !should_kill {
            return;
        }
        if self.kill_process_tree().is_err() {
            self.release_stop();
            return;
        }
        let _ = self.wait_for_exit();
    }

    fn wait_for_exit(&self) -> TerminalSnapshot {
        let state = self.lock_state();
        let (state, timeout) = self
            .process_changed
            .wait_timeout_while(state, Duration::from_secs(2), |state| {
                matches!(state.process, TerminalProcessState::Running)
            })
            .unwrap_or_else(|error| error.into_inner());
        if timeout.timed_out() && matches!(state.process, TerminalProcessState::Running) {
            drop(state);
            self.transition_process(TerminalProcessState::Exited { code: 137 });
            return self.snapshot();
        }
        self.snapshot_from(&state)
    }

    pub(crate) fn kill_for_shutdown(&self) {
        let _ = self.reserve_stop(None);
        let _ = self.kill_process_tree();
    }

    fn reserve_stop(&self, controller: Option<TerminalController>) -> Result<bool, TerminalError> {
        let mut state = self.lock_state();
        ensure_running(&state)?;
        if controller.is_some_and(|controller| state.controller != controller) {
            return Ok(false);
        }
        if state.stopping {
            return Ok(false);
        }
        state.stopping = true;
        state.controller_generation = state.controller_generation.saturating_add(1);
        Ok(true)
    }

    fn release_stop(&self) {
        let mut state = self.lock_state();
        if matches!(state.process, TerminalProcessState::Running) {
            state.stopping = false;
        }
    }

    fn record_output(self: &Arc<Self>, bytes: &[u8]) {
        let mut state = self.lock_state();
        let revision = state.buffer.process(bytes);
        self.publish_revision(revision);
        if state.output_event_pending.is_none() {
            state.output_event_pending = Some(revision);
            let _ = self.events.send(AgentEvent::TerminalOutput {
                id: self.id.clone(),
                frame: TerminalOutputFrame::live(Arc::downgrade(self), revision),
            });
        }
    }

    fn transition_process(&self, process: TerminalProcessState) {
        let mut state = self.lock_state();
        if !matches!(state.process, TerminalProcessState::Running) {
            return;
        }
        state.stopping = true;
        state.controller_generation = state.controller_generation.saturating_add(1);
        state.process = process;
        let revision = state.buffer.touch();
        self.publish_revision(revision);
        state.output_event_pending = None;
        let _ = self.events.send(AgentEvent::TerminalOutput {
            id: self.id.clone(),
            frame: TerminalOutputFrame::fixed(state.buffer.screen_snapshot(), revision),
        });
        self.emit_state_from(&state);
        self.process_changed.notify_all();
    }

    fn set_controller(
        &self,
        controller: TerminalController,
        require_running: bool,
    ) -> Result<TerminalSnapshot, TerminalError> {
        self.apply_controller(controller, require_running)
    }

    fn apply_controller(
        &self,
        controller: TerminalController,
        require_running: bool,
    ) -> Result<TerminalSnapshot, TerminalError> {
        let mut state = self.lock_state();
        if require_running {
            ensure_running(&state)?;
        }
        if controller == TerminalController::User && state.stopping {
            return Err(TerminalError::NotRunning);
        }
        if state.controller != controller {
            state.controller = controller;
            state.controller_generation = state.controller_generation.saturating_add(1);
            let revision = state.buffer.touch();
            self.publish_revision(revision);
            self.emit_state_from(&state);
        }
        Ok(self.snapshot_from(&state))
    }

    fn queue_writer(&self, command: WriterCommand) -> Result<(), TerminalError> {
        let WriterCommand::Write { bytes, .. } = &command;
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(TerminalError::Io(format!(
                "terminal input exceeds {MAX_INPUT_BYTES} bytes"
            )));
        }
        self.queued_writes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                (queued < MAX_QUEUED_WRITES).then_some(queued + 1)
            })
            .map_err(|_| TerminalError::Io("terminal input queue is full".into()))?;
        if self.writer_tx.send(command).is_err() {
            self.queued_writes.fetch_sub(1, Ordering::AcqRel);
            return Err(TerminalError::Io("terminal writer stopped".into()));
        }
        Ok(())
    }

    fn snapshot_from(&self, state: &SessionState) -> TerminalSnapshot {
        TerminalSnapshot {
            id: self.id.clone(),
            command: self.command.clone(),
            controller: state.controller,
            process: state.process.clone(),
            revision: state.buffer.revision(),
            rows: state.rows,
            cols: state.cols,
        }
    }

    fn emit_state(&self) {
        let state = self.lock_state();
        self.emit_state_from(&state);
    }

    fn emit_state_from(&self, state: &SessionState) {
        let snapshot = self.snapshot_from(state);
        let _ = self.events.send(AgentEvent::TerminalState {
            id: snapshot.id,
            controller: snapshot.controller,
            process: snapshot.process,
            revision: snapshot.revision,
        });
    }

    fn publish_revision(&self, revision: u64) {
        self.revision_tx.send_replace(revision);
    }

    fn kill_process_tree(&self) -> Result<(), TerminalError> {
        {
            let mut state = self.lock_state();
            if state.process_tree_cleaned {
                return Ok(());
            }
            state.process_tree_cleaned = true;
        }

        let result = self.kill_process_tree_once();
        if result.is_err() {
            self.lock_state().process_tree_cleaned = false;
        }
        result
    }

    fn kill_process_tree_once(&self) -> Result<(), TerminalError> {
        #[cfg(unix)]
        {
            let mut attempted = false;
            let mut cleaned = false;
            let mut last_error = None;

            #[cfg(target_os = "linux")]
            if let Some(cgroup) = &self.cgroup {
                attempted = true;
                match cgroup.kill() {
                    Ok(()) => cleaned = true,
                    Err(error) => last_error = Some(error),
                }
            }

            #[cfg(target_os = "linux")]
            if let Some(session) = self.process_session {
                attempted = true;
                match kill_linux_session(session) {
                    Ok(()) => cleaned = true,
                    Err(error) => last_error = Some(error),
                }
            }

            if let Some(group) = self.process_group {
                attempted = true;
                let result = unsafe { libc::kill(-group, libc::SIGKILL) };
                if result == 0 {
                    cleaned = true;
                } else {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ESRCH) {
                        cleaned = true;
                    } else {
                        last_error = Some(error);
                    }
                }
            }

            if cleaned {
                return Ok(());
            }
            if attempted {
                return Err(io_error(last_error.unwrap_or_else(|| {
                    std::io::Error::other("terminal process cleanup failed")
                })));
            }
        }

        self.killer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .kill()
            .map_err(io_error)
    }

    fn lock_state(&self) -> MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(target_os = "linux")]
fn kill_linux_session(session: i32) -> Result<(), std::io::Error> {
    let mut first_error = None;
    for _ in 0..2 {
        for entry in std::fs::read_dir("/proc")? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    first_error.get_or_insert(error);
                    continue;
                }
            };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
            else {
                continue;
            };
            if proc_session_id(pid) != Some(session) {
                continue;
            }
            let result = unsafe { libc::kill(pid, libc::SIGKILL) };
            if result != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    first_error.get_or_insert(error);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(target_os = "linux")]
fn proc_session_id(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_whitespace().nth(3)?.parse().ok()
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.kill_process_tree();
    }
}

fn writer_loop(
    session: Weak<TerminalSession>,
    mut writer: Box<dyn Write + Send>,
    commands: Receiver<WriterCommand>,
) {
    while let Ok(command) = commands.recv() {
        let Some(session) = session.upgrade() else {
            // Session dropped; the queued_writes counter leaks by the
            // number of in-flight commands, but that is bounded by the
            // channel capacity and will not grow unboundedly.
            break;
        };
        match command {
            WriterCommand::Write {
                bytes,
                controller,
                generation,
                reply,
            } => {
                session.queued_writes.fetch_sub(1, Ordering::AcqRel);
                let result =
                    write_cancellable(&session, writer.as_mut(), &bytes, controller, generation);
                let report_error = result.is_err() && {
                    let state = session.lock_state();
                    write_is_authorized(&state, controller, generation)
                };
                if report_error {
                    let error = result.as_ref().unwrap_err();
                    let _ = session.events.send(AgentEvent::TerminalError {
                        id: session.id.clone(),
                        message: error.to_string(),
                    });
                    let _ = session.reserve_stop(None);
                    let _ = session.kill_process_tree();
                }
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
        }
    }
}

fn write_cancellable(
    session: &TerminalSession,
    writer: &mut dyn Write,
    bytes: &[u8],
    controller: TerminalController,
    generation: u64,
) -> Result<(), TerminalError> {
    let mut offset = 0;
    while offset < bytes.len() {
        let end = (offset + 4096).min(bytes.len());
        let write_result = {
            let state = session.lock_state();
            ensure_controller_generation(&state, controller, generation)?;
            drop(state);
            writer.write(&bytes[offset..end])
        };
        match write_result {
            Ok(0) => {
                return Err(TerminalError::Io(
                    "terminal writer closed before accepting input".into(),
                ));
            }
            Ok(written) => offset += written,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(io_error(error)),
        }
    }
    loop {
        let flush_result = {
            let state = session.lock_state();
            ensure_controller_generation(&state, controller, generation)?;
            drop(state);
            writer.flush()
        };
        match flush_result {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(io_error(error)),
        }
    }
}

fn write_is_authorized(
    state: &SessionState,
    controller: TerminalController,
    generation: u64,
) -> bool {
    !state.stopping
        && matches!(state.process, TerminalProcessState::Running)
        && state.controller == controller
        && state.controller_generation == generation
}

fn ensure_running(state: &SessionState) -> Result<(), TerminalError> {
    if matches!(state.process, TerminalProcessState::Running) {
        Ok(())
    } else {
        Err(TerminalError::NotRunning)
    }
}

fn ensure_active(state: &SessionState) -> Result<(), TerminalError> {
    ensure_running(state)?;
    if state.stopping {
        Err(TerminalError::NotRunning)
    } else {
        Ok(())
    }
}

fn ensure_controller(
    state: &SessionState,
    controller: TerminalController,
) -> Result<(), TerminalError> {
    ensure_active(state)?;
    if state.controller == controller {
        return Ok(());
    }
    match controller {
        TerminalController::Agent => Err(TerminalError::UserControlled),
        TerminalController::User => Err(TerminalError::Io(
            "terminal session is controlled by the agent".into(),
        )),
    }
}

fn ensure_controller_generation(
    state: &SessionState,
    controller: TerminalController,
    generation: u64,
) -> Result<(), TerminalError> {
    ensure_controller(state, controller)?;
    if state.controller_generation == generation {
        Ok(())
    } else {
        Err(TerminalError::Io("terminal control changed".into()))
    }
}

fn io_error(error: impl std::fmt::Display) -> TerminalError {
    TerminalError::Io(error.to_string())
}

#[cfg(unix)]
fn configure_nonblocking(master: &dyn MasterPty) -> Result<(), TerminalError> {
    let Some(fd) = master.as_raw_fd() else {
        return Ok(());
    };
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn configure_nonblocking(_master: &dyn MasterPty) -> Result<(), TerminalError> {
    Ok(())
}

#[cfg(unix)]
const fn libc_eio() -> i32 {
    libc::EIO
}

#[cfg(not(unix))]
const fn libc_eio() -> i32 {
    -1
}
