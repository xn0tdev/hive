use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::event::{AgentEvent, EventSender};

use super::session::TerminalSession;
use super::{
    TerminalController, TerminalError, TerminalReadResult, TerminalSnapshot, TerminalWriteRequest,
    MAX_READ_WAIT_MS,
};

pub struct TerminalManager {
    events: EventSender,
    current: Mutex<Option<Arc<TerminalSession>>>,
}

impl TerminalManager {
    pub fn new(events: EventSender) -> Arc<Self> {
        Arc::new(Self {
            events,
            current: Mutex::new(None),
        })
    }

    pub async fn start(
        &self,
        command: &str,
        description: &str,
        cwd: &Path,
    ) -> Result<TerminalSnapshot, TerminalError> {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if current
            .as_ref()
            .is_some_and(|session| session.controller() == TerminalController::User)
        {
            return Err(TerminalError::UserControlled);
        }
        if let Some(session) = current
            .as_ref()
            .filter(|session| session.process_is_running())
        {
            return Err(TerminalError::AlreadyRunning {
                id: session.id.clone(),
            });
        }

        let id = format!("term-{}", uuid::Uuid::new_v4().simple());
        let session = match TerminalSession::spawn(
            id.clone(),
            command,
            description,
            cwd,
            self.events.clone(),
        ) {
            Ok(session) => session,
            Err(error) => {
                let _ = self.events.send(AgentEvent::TerminalStartFailed {
                    id,
                    command: command.to_string(),
                    description: description.to_string(),
                    message: error.to_string(),
                });
                return Err(error);
            }
        };
        let snapshot = session.snapshot();
        *current = Some(session);
        Ok(snapshot)
    }

    pub async fn read(
        &self,
        id: &str,
        after_revision: Option<u64>,
        wait_ms: Option<u64>,
    ) -> Result<TerminalReadResult, TerminalError> {
        let session = self.session(id)?;
        if let Some(wait_ms) = wait_ms.filter(|ms| *ms > 0) {
            let mut revisions = session.subscribe();
            let after = after_revision.unwrap_or_else(|| *revisions.borrow());
            if *revisions.borrow() <= after {
                let wait = Duration::from_millis(wait_ms.min(MAX_READ_WAIT_MS));
                let _ = tokio::time::timeout(wait, async {
                    loop {
                        if revisions.changed().await.is_err() || *revisions.borrow() > after {
                            break;
                        }
                    }
                })
                .await;
            }
        }
        Ok(session.read_result(after_revision))
    }

    pub async fn write_agent(
        &self,
        id: &str,
        request: TerminalWriteRequest,
    ) -> Result<TerminalSnapshot, TerminalError> {
        self.session(id)?.write_agent(request).await
    }

    pub fn attach(&self, id: &str) -> Result<TerminalSnapshot, TerminalError> {
        self.session(id)?.attach()
    }

    pub fn request_attach(&self, id: &str) -> Result<TerminalSnapshot, TerminalError> {
        self.session(id)?.request_attach()
    }

    pub fn detach(&self, id: &str) -> Result<TerminalSnapshot, TerminalError> {
        self.session(id)?.detach()
    }

    pub fn request_detach(&self, id: &str) -> Result<TerminalSnapshot, TerminalError> {
        self.session(id)?.request_detach()
    }

    pub async fn write_user(
        &self,
        id: &str,
        bytes: Vec<u8>,
    ) -> Result<TerminalSnapshot, TerminalError> {
        self.session(id)?.write_user(bytes)
    }

    pub async fn resize(
        &self,
        id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<TerminalSnapshot, TerminalError> {
        self.session(id)?.resize(rows, cols)
    }

    pub fn stop(&self, id: &str) -> Result<TerminalSnapshot, TerminalError> {
        self.session(id)?.stop()
    }

    pub fn stop_if_agent_controlled(&self) {
        if let Some(session) = self.current_session() {
            session.stop_if_agent_controlled();
        }
    }

    pub fn shutdown(&self) {
        if let Some(session) = self.current_session() {
            session.kill_for_shutdown();
        }
    }

    fn session(&self, id: &str) -> Result<Arc<TerminalSession>, TerminalError> {
        self.current_session()
            .filter(|session| session.id == id)
            .ok_or_else(|| TerminalError::NotFound { id: id.to_string() })
    }

    fn current_session(&self) -> Option<Arc<TerminalSession>> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}
