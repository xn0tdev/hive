//! Background subagent jobs with a slot cap, optional hidden worktrees,
//! and observe / message / wait / close.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;

use crate::agent::{AgentBuilder, FollowUpSlot, UserInput};
use crate::config::MAX_AGENT_SLOTS;
use crate::event::{AgentEvent, EventSender, SubagentLine, SubagentStatus};
use crate::spawner::{
    JobSnapshot, JobWake, JobWakeSender, SubagentSpawner, SubagentTask, ToolRecord, WaitOutcome,
    WaitSpec,
};
use crate::worktree;

const THINK_COALESCE_MS: u64 = 24;
const CLOSE_JOIN_SECS: u64 = 8;

struct Job {
    id: String,
    label: String,
    paths: Vec<String>,
    status: Mutex<SubagentStatus>,
    interrupt: Arc<AtomicBool>,
    follow_up: FollowUpSlot,
    next_tx: mpsc::UnboundedSender<UserInput>,
    closing: Arc<AtomicBool>,
    tools: Mutex<Vec<ToolRecord>>,
    last_text: Mutex<String>,
    worktree: Option<(PathBuf, String)>,
    root_cwd: PathBuf,
}

struct Inner {
    builder: AgentBuilder,
    events: EventSender,
    max_slots: usize,
    max_depth: usize,
    root_cwd: PathBuf,
    wake: Option<JobWakeSender>,
    me: Weak<Inner>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    handles: Mutex<HashMap<String, JoinHandle<()>>>,
    notify: Notify,
}

pub fn new_spawner(
    builder: AgentBuilder,
    events: EventSender,
    max_concurrent: usize,
    max_depth: usize,
) -> Arc<dyn SubagentSpawner> {
    new_spawner_in(
        builder,
        events,
        max_concurrent,
        max_depth,
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        None,
    )
}

pub fn new_spawner_in(
    builder: AgentBuilder,
    events: EventSender,
    max_concurrent: usize,
    max_depth: usize,
    root_cwd: PathBuf,
    wake: Option<JobWakeSender>,
) -> Arc<dyn SubagentSpawner> {
    Arc::new_cyclic(|me| Inner {
        builder,
        events,
        max_slots: max_concurrent.clamp(1, MAX_AGENT_SLOTS),
        max_depth: max_depth.max(1),
        root_cwd,
        wake,
        me: me.clone(),
        jobs: Mutex::new(HashMap::new()),
        handles: Mutex::new(HashMap::new()),
        notify: Notify::new(),
    })
}

impl Inner {
    fn occupied(&self) -> usize {
        self.jobs.lock().map(|g| g.len()).unwrap_or(0)
    }

    fn snapshot_job(job: &Job) -> JobSnapshot {
        JobSnapshot {
            id: job.id.clone(),
            label: job.label.clone(),
            status: job
                .status
                .lock()
                .map(|s| *s)
                .unwrap_or(SubagentStatus::Failed),
            last_text: job.last_text.lock().map(|t| t.clone()).unwrap_or_default(),
            tools: job.tools.lock().map(|t| t.clone()).unwrap_or_default(),
            paths: job.paths.clone(),
        }
    }

    fn set_status(job: &Job, status: SubagentStatus) {
        if let Ok(mut g) = job.status.lock() {
            *g = status;
        }
    }

    fn overlap_with(&self, paths: &[String]) -> Option<String> {
        if paths.is_empty() {
            return None;
        }
        let jobs = self.jobs.lock().ok()?;
        for job in jobs.values() {
            if paths_overlap(paths, &job.paths) {
                return Some(job.id.clone());
            }
        }
        None
    }

    fn wake_parent(&self, id: &str, ok: bool, summary: &str) {
        if let Some(tx) = &self.wake {
            let _ = tx.send(JobWake {
                id: id.to_string(),
                ok,
                summary: summary.to_string(),
            });
        }
        self.notify.notify_waiters();
    }

    async fn run_job(
        self: Arc<Self>,
        job: Arc<Job>,
        mut next_rx: mpsc::UnboundedReceiver<UserInput>,
        prompt: String,
        depth: usize,
        isolate: bool,
        model_role: crate::config::ModelRole,
    ) {
        let id = job.id.clone();
        let label = if job.label.trim().is_empty() {
            truncate(&prompt, 44)
        } else {
            job.label.clone()
        };
        let _ = self.events.send(AgentEvent::SubagentSpawned {
            id: id.clone(),
            label,
            prompt: prompt.clone(),
        });

        let child_spawner: Arc<dyn SubagentSpawner> = if depth < self.max_depth {
            match self.me.upgrade() {
                Some(inner) => inner,
                None => crate::noop_spawner(),
            }
        } else {
            crate::noop_spawner()
        };

        let (tx, rx) = mpsc::unbounded_channel();
        let fwd = self.events.clone();
        let fwd_id = id.clone();
        let job_tools = job.clone();
        let forward = tokio::spawn(async move {
            forward_child_events(rx, fwd, fwd_id, job_tools).await;
        });

        let model = self.builder.config.model(model_role).to_string();
        let child_cwd = job
            .worktree
            .as_ref()
            .map(|(p, _)| p.clone())
            .unwrap_or_else(|| job.root_cwd.clone());

        let extra = if isolate {
            "You are a focused worker. Complete the assigned task in this workspace. \
Your final message is the entire return value. Do not spawn other agents."
                .to_string()
        } else {
            "You are a focused worker. Complete the assigned task. \
Your final message is the entire return value. Do not spawn other agents."
                .to_string()
        };
        let mut first = Some(UserInput::from(format!(
            "{extra}\n\n## Your task\n{prompt}"
        )));

        {
            let mut agent = self
                .builder
                .build_in(tx, model, depth, child_spawner, child_cwd);
            loop {
                if job.closing.load(Ordering::Relaxed) {
                    break;
                }
                let input = if let Some(first) = first.take() {
                    first
                } else {
                    Self::set_status(&job, SubagentStatus::Done);
                    let last = job.last_text.lock().map(|t| t.clone()).unwrap_or_default();
                    let ok = !last.trim().is_empty();
                    let _ = self.events.send(AgentEvent::SubagentStatus {
                        id: id.clone(),
                        status: if ok {
                            SubagentStatus::Done
                        } else {
                            SubagentStatus::Failed
                        },
                        detail: if ok { "done".into() } else { "failed".into() },
                    });
                    self.wake_parent(&id, ok, &truncate(&last, 400));
                    tokio::select! {
                        biased;
                        _ = wait_flag(&job.closing) => break,
                        next = next_rx.recv() => match next {
                            Some(input) => {
                                Self::set_status(&job, SubagentStatus::Running);
                                let _ = self.events.send(AgentEvent::SubagentStatus {
                                    id: id.clone(),
                                    status: SubagentStatus::Running,
                                    detail: "follow-up".into(),
                                });
                                input
                            }
                            None => break,
                        },
                    }
                };

                job.interrupt.store(false, Ordering::Relaxed);
                let text = agent
                    .run_turn(input, job.interrupt.clone(), job.follow_up.clone())
                    .await;
                if let Ok(mut g) = job.last_text.lock() {
                    *g = text;
                }
                if job.closing.load(Ordering::Relaxed) || job.interrupt.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
        let _ = forward.await;
        self.notify.notify_waiters();
    }
}

async fn wait_flag(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn forward_child_events(
    mut rx: mpsc::UnboundedReceiver<AgentEvent>,
    events: EventSender,
    id: String,
    job: Arc<Job>,
) {
    let mut pending_think = String::new();

    while let Some(ev) = rx.recv().await {
        let mut next = Some(ev);
        while let Some(ev) = next.take() {
            match ev {
                AgentEvent::ReasoningDelta(t) if !t.is_empty() => {
                    pending_think.push_str(&t);
                    if let Some(other) = drain_thinking(&mut rx, &mut pending_think) {
                        flush_thinking(&events, &id, &mut pending_think);
                        next = Some(other);
                        continue;
                    }
                    tokio::time::sleep(Duration::from_millis(THINK_COALESCE_MS)).await;
                    if let Some(other) = drain_thinking(&mut rx, &mut pending_think) {
                        flush_thinking(&events, &id, &mut pending_think);
                        next = Some(other);
                        continue;
                    }
                    flush_thinking(&events, &id, &mut pending_think);
                }
                other => {
                    flush_thinking(&events, &id, &mut pending_think);
                    record_tool(&job, &other);
                    forward_child(&events, &id, other);
                }
            }
        }
    }
    flush_thinking(&events, &id, &mut pending_think);
}

fn record_tool(job: &Job, ev: &AgentEvent) {
    let Ok(mut tools) = job.tools.lock() else {
        return;
    };
    match ev {
        AgentEvent::ToolStarted {
            name, args_preview, ..
        } => {
            tools.push(ToolRecord {
                name: name.clone(),
                args: args_preview.clone(),
                summary: String::new(),
                ok: None,
            });
        }
        AgentEvent::ToolFinished {
            name, ok, summary, ..
        } => {
            if let Some(rec) = tools
                .iter_mut()
                .rev()
                .find(|t| t.name == *name && t.ok.is_none())
            {
                rec.ok = Some(*ok);
                rec.summary = summary.clone();
            } else {
                tools.push(ToolRecord {
                    name: name.clone(),
                    args: String::new(),
                    summary: summary.clone(),
                    ok: Some(*ok),
                });
            }
        }
        _ => {}
    }
}

fn drain_thinking(
    rx: &mut mpsc::UnboundedReceiver<AgentEvent>,
    pending: &mut String,
) -> Option<AgentEvent> {
    loop {
        match rx.try_recv() {
            Ok(AgentEvent::ReasoningDelta(t)) => {
                if !t.is_empty() {
                    pending.push_str(&t);
                }
            }
            Ok(other) => return Some(other),
            Err(_) => return None,
        }
    }
}

fn flush_thinking(events: &EventSender, id: &str, pending: &mut String) {
    if pending.is_empty() {
        return;
    }
    let _ = events.send(AgentEvent::SubagentTranscript {
        id: id.to_string(),
        line: SubagentLine::Thinking(std::mem::take(pending)),
    });
}

fn forward_child(events: &EventSender, id: &str, ev: AgentEvent) {
    match ev {
        AgentEvent::ToolStarted {
            name, args_preview, ..
        } => {
            let detail = tool_detail(&name, &args_preview);
            let _ = events.send(AgentEvent::SubagentStatus {
                id: id.to_string(),
                status: SubagentStatus::Running,
                detail: detail.clone(),
            });
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Tool {
                    name,
                    args: args_preview,
                    summary: String::new(),
                    ok: None,
                },
            });
        }
        AgentEvent::ToolFinished {
            name, ok, summary, ..
        } => {
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Tool {
                    name,
                    args: String::new(),
                    summary,
                    ok: Some(ok),
                },
            });
        }
        AgentEvent::ReasoningDelta(t) => {
            if t.is_empty() {
                return;
            }
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Thinking(t),
            });
        }
        AgentEvent::AssistantMessage(t) => {
            if t.trim().is_empty() {
                return;
            }
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Assistant(t),
            });
        }
        AgentEvent::Notice(t) => {
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Notice(t),
            });
        }
        AgentEvent::Usage(usage) => {
            let _ = events.send(AgentEvent::SubagentUsage {
                id: id.to_string(),
                usage,
            });
        }
        _ => {}
    }
}

fn tool_detail(name: &str, args: &str) -> String {
    let s = if args.trim().is_empty() {
        name.to_string()
    } else if name == "run_shell" {
        format!("$ {args}")
    } else {
        format!("{name} · {args}")
    };
    truncate(&s, 56)
}

fn collect_wait(jobs: &HashMap<String, Arc<Job>>, spec: &WaitSpec) -> Option<Vec<WaitOutcome>> {
    let idle = |job: &Job| {
        !matches!(
            job.status
                .lock()
                .map(|s| *s)
                .unwrap_or(SubagentStatus::Failed),
            SubagentStatus::Running
        )
    };
    let to_outcome = |job: &Job| WaitOutcome {
        id: job.id.clone(),
        status: job
            .status
            .lock()
            .map(|s| *s)
            .unwrap_or(SubagentStatus::Failed),
        last_text: job.last_text.lock().map(|t| t.clone()).unwrap_or_default(),
    };

    match spec {
        WaitSpec::Id(id) => {
            let job = jobs.get(id)?;
            idle(job).then(|| vec![to_outcome(job)])
        }
        WaitSpec::Any => {
            if jobs.is_empty() {
                return Some(Vec::new());
            }
            let done: Vec<_> = jobs
                .values()
                .filter(|j| idle(j))
                .map(|j| to_outcome(j))
                .collect();
            if done.is_empty() {
                None
            } else {
                Some(done)
            }
        }
        WaitSpec::All => {
            if jobs.is_empty() {
                return Some(Vec::new());
            }
            if jobs.values().all(|j| idle(j)) {
                Some(jobs.values().map(|j| to_outcome(j)).collect())
            } else {
                None
            }
        }
    }
}

#[async_trait]
impl SubagentSpawner for Inner {
    async fn start(&self, task: SubagentTask) -> Result<String, String> {
        if task.depth > self.max_depth {
            return Err(format!(
                "maximum subagent depth ({}) reached; cannot spawn deeper",
                self.max_depth
            ));
        }
        if self.occupied() >= self.max_slots {
            return Err(format!(
                "all {} agent slots are occupied — close a finished worker first",
                self.max_slots
            ));
        }

        let cwd = task.cwd.clone().unwrap_or_else(|| self.root_cwd.clone());
        let paths = normalize_paths(&cwd, &task.paths);
        if let Some(other) = self.overlap_with(&paths) {
            return Err(format!(
                "paths overlap with running worker `{other}` — pick different files"
            ));
        }

        let id = if task.id.trim().is_empty() {
            format!("sub_{}", uuid::Uuid::new_v4().simple())
        } else {
            task.id.clone()
        };

        let worktree_meta = if task.isolate_worktree {
            match worktree::create(&cwd, &id) {
                Ok((path, branch)) => Some((path, branch)),
                Err(e) => return Err(e),
            }
        } else {
            None
        };

        let (next_tx, next_rx) = mpsc::unbounded_channel();
        let job = Arc::new(Job {
            id: id.clone(),
            label: task.label.clone(),
            paths,
            status: Mutex::new(SubagentStatus::Running),
            interrupt: Arc::new(AtomicBool::new(false)),
            follow_up: Arc::new(Mutex::new(None)),
            next_tx,
            closing: Arc::new(AtomicBool::new(false)),
            tools: Mutex::new(Vec::new()),
            last_text: Mutex::new(String::new()),
            worktree: worktree_meta,
            root_cwd: cwd,
        });

        {
            let mut jobs = self
                .jobs
                .lock()
                .map_err(|_| "job registry poisoned".to_string())?;
            jobs.insert(id.clone(), job.clone());
        }

        let Some(inner) = self.me.upgrade() else {
            return Err("spawner is shutting down".into());
        };
        let handle = tokio::spawn(inner.run_job(
            job,
            next_rx,
            task.prompt,
            task.depth,
            task.isolate_worktree,
            task.model_role,
        ));
        if let Ok(mut handles) = self.handles.lock() {
            handles.insert(id.clone(), handle);
        }
        Ok(id)
    }

    async fn observe(&self, id: &str) -> Result<JobSnapshot, String> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| "job registry poisoned".to_string())?;
        let job = jobs.get(id).ok_or_else(|| format!("no worker `{id}`"))?;
        Ok(Self::snapshot_job(job))
    }

    async fn message(&self, id: &str, text: String) -> Result<(), String> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| "job registry poisoned".to_string())?;
        let job = jobs.get(id).ok_or_else(|| format!("no worker `{id}`"))?;
        let status = job
            .status
            .lock()
            .map(|s| *s)
            .unwrap_or(SubagentStatus::Failed);
        let input = UserInput::from(text);
        if status == SubagentStatus::Running {
            if let Ok(mut g) = job.follow_up.lock() {
                *g = Some(input);
            }
        } else {
            Self::set_status(job, SubagentStatus::Running);
            if job.next_tx.send(input).is_err() {
                return Err(format!("worker `{id}` is no longer accepting messages"));
            }
            self.notify.notify_waiters();
        }
        Ok(())
    }

    async fn wait(
        &self,
        spec: WaitSpec,
        timeout: Option<Duration>,
    ) -> Result<Vec<WaitOutcome>, String> {
        let deadline = timeout.map(|d| tokio::time::Instant::now() + d);
        loop {
            {
                let jobs = self
                    .jobs
                    .lock()
                    .map_err(|_| "job registry poisoned".to_string())?;
                if let Some(out) = collect_wait(&jobs, &spec) {
                    return Ok(out);
                }
                if matches!(spec, WaitSpec::Id(ref id) if !jobs.contains_key(id)) {
                    return Err(format!(
                        "no worker `{}`",
                        match spec {
                            WaitSpec::Id(id) => id,
                            _ => String::new(),
                        }
                    ));
                }
            }
            if let Some(deadline) = deadline {
                let left = deadline.saturating_duration_since(tokio::time::Instant::now());
                if left.is_zero() {
                    return Err("wait timed out".into());
                }
                tokio::select! {
                    _ = self.notify.notified() => {}
                    _ = tokio::time::sleep(left) => return Err("wait timed out".into()),
                }
            } else {
                self.notify.notified().await;
            }
        }
    }

    async fn close(&self, id: &str, discard: bool) -> Result<String, String> {
        let job = {
            let jobs = self
                .jobs
                .lock()
                .map_err(|_| "job registry poisoned".to_string())?;
            jobs.get(id)
                .cloned()
                .ok_or_else(|| format!("no worker `{id}`"))?
        };

        job.closing.store(true, Ordering::Relaxed);
        job.interrupt.store(true, Ordering::Relaxed);
        let _ = job.next_tx.send(UserInput::from(""));

        let handle = self.handles.lock().ok().and_then(|mut h| h.remove(id));
        if let Some(handle) = handle {
            let _ = tokio::time::timeout(Duration::from_secs(CLOSE_JOIN_SECS), handle).await;
        }

        let was_running = matches!(
            job.status
                .lock()
                .map(|s| *s)
                .unwrap_or(SubagentStatus::Failed),
            SubagentStatus::Running
        );

        let msg = if discard || was_running {
            if let Some((_, _)) = &job.worktree {
                let _ = worktree::remove(&job.root_cwd, id);
            }
            if was_running {
                "Closed. Worker was still running — work discarded.".to_string()
            } else {
                "Closed. Work discarded.".to_string()
            }
        } else if job.worktree.is_some() {
            match worktree::integrate(&job.root_cwd, id) {
                Ok(text) => format!("Closed. {text}"),
                Err(e) => {
                    job.closing.store(false, Ordering::Relaxed);
                    job.interrupt.store(false, Ordering::Relaxed);
                    return Err(e);
                }
            }
        } else {
            "Closed.".to_string()
        };

        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(id);
        }
        let _ = self.events.send(AgentEvent::SubagentStatus {
            id: id.to_string(),
            status: SubagentStatus::Done,
            detail: "closed".into(),
        });
        self.notify.notify_waiters();
        Ok(msg)
    }

    fn shutdown(&self) {
        if let Ok(jobs) = self.jobs.lock() {
            for job in jobs.values() {
                job.closing.store(true, Ordering::Relaxed);
                job.interrupt.store(true, Ordering::Relaxed);
            }
        }
        if let Ok(mut handles) = self.handles.lock() {
            for (_, handle) in handles.drain() {
                handle.abort();
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn normalize_paths(cwd: &Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|p| {
            let joined = if Path::new(p).is_absolute() {
                PathBuf::from(p)
            } else {
                cwd.join(p)
            };
            lexical_normalize(&joined)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .filter(|p| !p.is_empty())
        .collect()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(std::path::MAIN_SEPARATOR_STR),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

pub(crate) fn paths_overlap(a: &[String], b: &[String]) -> bool {
    for x in a {
        for y in b {
            if x == y || x.starts_with(&format!("{y}/")) || y.starts_with(&format!("{x}/")) {
                return true;
            }
        }
    }
    false
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::error::Result;
    use crate::provider::{ChatOutcome, ChatRequest, Delta, LlmProvider};
    use crate::skill::no_skills;
    use crate::{AgentBuilder, Message};
    use async_trait::async_trait;

    struct PromptProvider;

    #[async_trait]
    impl LlmProvider for PromptProvider {
        async fn chat_stream(
            &self,
            req: ChatRequest<'_>,
            on_delta: &mut (dyn FnMut(Delta) + Send),
        ) -> Result<ChatOutcome> {
            let last = req.messages.last().map(|m| m.text()).unwrap_or_default();
            let reply = if last.contains("follow") {
                "got follow-up"
            } else {
                "done"
            };
            on_delta(Delta::Text(reply.into()));
            Ok(ChatOutcome {
                message: Message::assistant(reply),
                usage: Default::default(),
                finish_reason: "stop".into(),
            })
        }
    }

    fn builder() -> AgentBuilder {
        AgentBuilder {
            provider: Arc::new(PromptProvider),
            tools: Vec::new(),
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
        }
    }

    fn task(id: &str, prompt: &str, paths: &[&str]) -> SubagentTask {
        SubagentTask {
            id: id.into(),
            label: id.into(),
            prompt: prompt.into(),
            model_role: crate::config::ModelRole::Default,
            depth: 1,
            isolate_worktree: false,
            cwd: Some(std::env::temp_dir()),
            paths: paths.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn claimed_paths_overlap() {
        assert!(paths_overlap(
            &["/tmp/a/src/main.rs".into()],
            &["/tmp/a/src/main.rs".into()]
        ));
        assert!(paths_overlap(
            &["/tmp/a/src".into()],
            &["/tmp/a/src/main.rs".into()]
        ));
        assert!(!paths_overlap(
            &["/tmp/a/src/a.rs".into()],
            &["/tmp/a/src/b.rs".into()]
        ));
    }

    #[tokio::test]
    async fn start_observe_wait_close() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let spawner = new_spawner_in(builder(), tx, 3, 1, std::env::temp_dir(), None);
        let id = spawner
            .start(task("sub_one", "say hi", &[]))
            .await
            .expect("start");
        let waited = spawner
            .wait(WaitSpec::Id(id.clone()), Some(Duration::from_secs(5)))
            .await
            .expect("wait");
        assert_eq!(waited[0].id, id);
        let snap = spawner.observe(&id).await.expect("observe");
        assert!(snap.last_text.contains("done"), "{}", snap.last_text);
        let msg = spawner.close(&id, true).await.expect("close");
        assert!(msg.contains("Closed"));
        assert!(spawner.observe(&id).await.is_err());
    }

    #[tokio::test]
    async fn slot_cap_and_path_overlap() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let spawner = new_spawner_in(builder(), tx, 2, 1, std::env::temp_dir(), None);
        let a = spawner
            .start(task("sub_a", "a", &["src/a.rs"]))
            .await
            .unwrap();
        let overlap = spawner
            .start(task("sub_b", "b", &["src/a.rs"]))
            .await
            .expect_err("overlap");
        assert!(overlap.contains("overlap"), "{overlap}");
        let b = spawner
            .start(task("sub_c", "c", &["src/c.rs"]))
            .await
            .unwrap();
        let third = spawner
            .start(task("sub_d", "d", &["src/d.rs"]))
            .await
            .expect_err("slot cap");
        assert!(third.contains("slots"), "expected slot cap, got {third}");
        let _ = spawner
            .wait(WaitSpec::All, Some(Duration::from_secs(5)))
            .await;
        spawner.close(&a, true).await.unwrap();
        spawner.close(&b, true).await.unwrap();
    }

    #[tokio::test]
    async fn message_reaches_idle_worker() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let spawner = new_spawner_in(builder(), tx, 3, 1, std::env::temp_dir(), None);
        let id = spawner.start(task("sub_m", "first", &[])).await.unwrap();
        let _ = spawner
            .wait(WaitSpec::Id(id.clone()), Some(Duration::from_secs(5)))
            .await
            .unwrap();
        spawner
            .message(&id, "please follow up".into())
            .await
            .unwrap();
        let _ = spawner
            .wait(WaitSpec::Id(id.clone()), Some(Duration::from_secs(5)))
            .await
            .unwrap();
        // Give the child a tick to store last_text after the follow-up turn.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let snap = spawner.observe(&id).await.unwrap();
        assert!(
            snap.last_text.contains("follow"),
            "expected follow-up reply, got {}",
            snap.last_text
        );
        spawner.close(&id, true).await.unwrap();
    }

    #[tokio::test]
    async fn forward_loop_rewrites_usage_to_subagent() {
        use crate::provider::Usage;

        let (child_tx, child_rx) = mpsc::unbounded_channel();
        let (ui_tx, mut ui_rx) = mpsc::unbounded_channel();
        let job = Arc::new(Job {
            id: "s1".into(),
            label: "s1".into(),
            paths: Vec::new(),
            status: Mutex::new(SubagentStatus::Running),
            interrupt: Arc::new(AtomicBool::new(false)),
            follow_up: Arc::new(Mutex::new(None)),
            next_tx: mpsc::unbounded_channel().0,
            closing: Arc::new(AtomicBool::new(false)),
            tools: Mutex::new(Vec::new()),
            last_text: Mutex::new(String::new()),
            worktree: None,
            root_cwd: std::env::temp_dir(),
        });

        let fwd = tokio::spawn({
            let job = job.clone();
            async move {
                forward_child_events(child_rx, ui_tx, "s1".into(), job).await;
            }
        });

        let _ = child_tx.send(AgentEvent::Usage(Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        }));
        drop(child_tx);
        let _ = fwd.await;

        let mut saw = false;
        while let Ok(ev) = ui_rx.try_recv() {
            if let AgentEvent::SubagentUsage { id, usage } = ev {
                assert_eq!(id, "s1");
                assert_eq!(usage.total_tokens, 15);
                saw = true;
            }
        }
        assert!(saw, "expected SubagentUsage event");
    }
}
