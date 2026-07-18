use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;

use hive_core::config::AppConfig;
use hive_core::event::{AgentEvent, EventSender};
use hive_core::message::{ContentPart, ImageSource, Message};
use hive_core::provider::{ChatRequest, Delta, LlmProvider, ToolSpec, Usage};
use hive_core::skill::SkillSource;
use hive_core::spawner::SubagentSpawner;
use hive_core::tool::{Tool, ToolContext, ToolResult};
use hive_core::vision::VisionDescriber;

use crate::prompt::build_system_prompt;
use crate::session::Session;

/// Input for a single user turn: text plus any attached images.
pub struct UserInput {
    pub text: String,
    pub images: Vec<ImageSource>,
}

impl From<String> for UserInput {
    fn from(text: String) -> Self {
        UserInput {
            text,
            images: Vec::new(),
        }
    }
}

impl From<&str> for UserInput {
    fn from(text: &str) -> Self {
        UserInput::from(text.to_string())
    }
}

/// Holds the shared, cloneable ingredients needed to construct an agent. The
/// composition root builds one of these; the swarm reuses it to spin up
/// subagents. Note it deliberately does NOT hold a spawner, so there is no
/// reference cycle between agents and the swarm.
#[derive(Clone)]
pub struct AgentBuilder {
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub skills: Arc<dyn SkillSource>,
    pub vision: Arc<dyn VisionDescriber>,
    pub config: Arc<AppConfig>,
}

impl AgentBuilder {
    pub fn build(
        &self,
        events: EventSender,
        model: String,
        depth: usize,
        spawner: Arc<dyn SubagentSpawner>,
    ) -> Agent {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let system = build_system_prompt(&cwd, self.skills.as_ref(), depth > 0);
        Agent {
            provider: self.provider.clone(),
            tool_specs: tool_specs(&self.tools),
            tools: self.tools.clone(),
            skills: self.skills.clone(),
            vision: self.vision.clone(),
            config: self.config.clone(),
            spawner,
            events,
            session: Session::new(system),
            model,
            depth,
            cwd,
            max_steps: 50,
            vision_cache: HashMap::new(),
        }
    }
}

fn tool_specs(tools: &[Arc<dyn Tool>]) -> Vec<ToolSpec> {
    tools
        .iter()
        .map(|t| ToolSpec {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters(),
        })
        .collect()
}

/// One conversational agent: a provider, a tool set, and a running session.
/// Drives the YOLO loop and emits events for a frontend to render.
pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Arc<dyn Tool>>,
    tool_specs: Vec<ToolSpec>,
    skills: Arc<dyn SkillSource>,
    vision: Arc<dyn VisionDescriber>,
    config: Arc<AppConfig>,
    spawner: Arc<dyn SubagentSpawner>,
    events: EventSender,
    session: Session,
    model: String,
    depth: usize,
    cwd: PathBuf,
    max_steps: usize,
    vision_cache: HashMap<String, String>,
}

impl Agent {
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    pub fn usage(&self) -> Usage {
        self.session.usage
    }

    pub fn reset(&mut self) {
        self.session.reset();
        self.vision_cache.clear();
    }

    fn emit(&self, e: AgentEvent) {
        let _ = self.events.send(e);
    }

    /// Run one user turn to completion: stream the model, execute any tool calls
    /// (immediately, no confirmation), and repeat until the model stops calling
    /// tools. Returns the final assistant text.
    pub async fn run_turn(&mut self, input: UserInput, interrupt: Arc<AtomicBool>) -> String {
        self.emit(AgentEvent::TurnStarted);

        // Assemble the user message (text + any images).
        let mut parts: Vec<ContentPart> = Vec::new();
        if !input.text.is_empty() {
            parts.push(ContentPart::Text(input.text));
        }
        for img in input.images {
            parts.push(ContentPart::Image(img));
        }
        if !parts.is_empty() {
            self.session.push(Message::user_parts(parts));
        }

        let mut final_text = String::new();

        for step in 0..self.max_steps {
            if interrupt.load(Ordering::Relaxed) {
                self.emit(AgentEvent::Notice("Interrupted.".to_string()));
                break;
            }

            let messages = self.prepare_messages().await;
            let req = ChatRequest {
                model: self.model.clone(),
                messages,
                tools: self.tool_specs.clone(),
                temperature: Some(0.3),
                max_tokens: None,
            };

            self.emit(AgentEvent::AssistantStarted);

            let events = self.events.clone();
            let mut on_delta = move |d: Delta| match d {
                Delta::Text(t) => {
                    let _ = events.send(AgentEvent::AssistantTextDelta(t));
                }
                Delta::Reasoning(r) => {
                    let _ = events.send(AgentEvent::ReasoningDelta(r));
                }
            };

            let outcome = match self.provider.chat_stream(req, &mut on_delta).await {
                Ok(o) => o,
                Err(e) => {
                    self.emit(AgentEvent::Error(format!("model error: {e}")));
                    break;
                }
            };

            self.session.add_usage(outcome.usage);
            self.emit(AgentEvent::Usage(self.session.usage));

            let assistant_text = outcome.message.text();
            let tool_calls = outcome.message.tool_calls.clone();
            self.session.push(outcome.message);

            if !assistant_text.trim().is_empty() {
                self.emit(AgentEvent::AssistantMessage(assistant_text.clone()));
                final_text = assistant_text;
            }

            if tool_calls.is_empty() {
                break;
            }

            for tc in tool_calls {
                if interrupt.load(Ordering::Relaxed) {
                    self.emit(AgentEvent::Notice("Interrupted.".to_string()));
                    break;
                }
                self.run_tool(&tc.id, &tc.name, &tc.arguments).await;
            }

            if step + 1 == self.max_steps {
                self.emit(AgentEvent::Notice(
                    "Reached step limit for this turn.".to_string(),
                ));
            }
        }

        self.emit(AgentEvent::TurnFinished);
        final_text
    }

    /// Run a one-shot prompt with no external interrupt, returning the final
    /// text. Used by subagents.
    pub async fn run_headless(&mut self, prompt: impl Into<String>) -> String {
        let interrupt = Arc::new(AtomicBool::new(false));
        self.run_turn(UserInput::from(prompt.into()), interrupt).await
    }

    async fn run_tool(&mut self, id: &str, name: &str, arguments: &str) {
        self.emit(AgentEvent::ToolStarted {
            id: id.to_string(),
            name: name.to_string(),
            args_preview: preview_args(name, arguments),
        });

        let tool = self.tools.iter().find(|t| t.name() == name).cloned();
        let result = match tool {
            Some(t) => {
                let args: serde_json::Value =
                    serde_json::from_str(arguments).unwrap_or_else(|_| json!({}));
                let ctx = ToolContext {
                    cwd: self.cwd.clone(),
                    events: self.events.clone(),
                    spawner: self.spawner.clone(),
                    skills: self.skills.clone(),
                    config: self.config.clone(),
                    depth: self.depth,
                    call_id: id.to_string(),
                };
                t.execute(args, &ctx).await
            }
            None => ToolResult::error(format!("unknown tool: {name}")),
        };

        self.emit(AgentEvent::ToolFinished {
            id: id.to_string(),
            name: name.to_string(),
            ok: !result.is_error,
            summary: first_line(&result.content, 120),
        });

        self.session
            .push(Message::tool_result(id, name, result.content));

        if !result.images.is_empty() {
            let mut parts = vec![ContentPart::Text(format!(
                "[Image(s) produced by tool `{name}`]"
            ))];
            for img in result.images {
                parts.push(ContentPart::Image(img));
            }
            self.session.push(Message::user_parts(parts));
        }
    }

    /// Produce the message list to send. If the active model can't see images,
    /// replace each image with a cached textual description (the vision
    /// fallback).
    async fn prepare_messages(&mut self) -> Vec<Message> {
        if self.config.is_vision_capable(&self.model) {
            return self.session.messages.clone();
        }

        let vision = self.vision.clone();
        let msgs = self.session.messages.clone();
        let mut out = Vec::with_capacity(msgs.len());

        for m in &msgs {
            if m.images().is_empty() {
                out.push(m.clone());
                continue;
            }
            let mut new_parts = Vec::new();
            for part in &m.content {
                match part {
                    ContentPart::Text(t) => new_parts.push(ContentPart::Text(t.clone())),
                    ContentPart::Image(src) => {
                        let key = src.as_data_url();
                        let desc = if let Some(d) = self.vision_cache.get(&key) {
                            d.clone()
                        } else {
                            self.emit(AgentEvent::Notice(
                                "Describing image with vision model…".to_string(),
                            ));
                            let d = vision
                                .describe(src)
                                .await
                                .unwrap_or_else(|e| format!("[image description failed: {e}]"));
                            self.vision_cache.insert(key, d.clone());
                            d
                        };
                        new_parts.push(ContentPart::Text(format!(
                            "[Image described by vision model]:\n{desc}"
                        )));
                    }
                }
            }
            let mut nm = m.clone();
            nm.content = new_parts;
            out.push(nm);
        }

        out
    }
}

/// A short, human-friendly summary of a tool call — the one argument that
/// matters, not the raw JSON. Falls back to a compact key list.
fn preview_args(name: &str, arguments: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);

    let main = match name {
        "read_file" | "write_file" | "edit_file" | "list_dir" => s("path"),
        "glob" | "grep" => s("pattern"),
        "run_shell" => s("command"),
        "web_search" => s("query"),
        "read_skill" => s("name"),
        "spawn_subagent" | "spawn_swarm" => s("task"),
        "web_get_contents" => v
            .get("urls")
            .and_then(|u| u.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.as_str())
            .map(str::to_string),
        _ => None,
    };

    match main {
        Some(m) => truncate(&m, 80),
        None => match v.as_object() {
            Some(o) => o.keys().take(3).cloned().collect::<Vec<_>>().join(", "),
            None => String::new(),
        },
    }
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

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("");
    truncate(line, max)
}
