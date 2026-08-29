//! User submission, follow-ups, slash commands, and session dispatch.

use super::*;

pub(super) fn submit(app: &mut App, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let text = app.input.take();
    let trimmed = text.trim().to_string();

    // A lone pasted path becomes an attachment tag instead of a message.
    if !trimmed.starts_with('/') {
        if let Some(leftover) = app.try_attach_pasted_path(&trimmed) {
            if leftover.is_empty() {
                app.flash(format!("Attached {}", app.attachment_tags_line()));
                return false;
            }
        }
    }

    if trimmed.is_empty() && !app.has_pending_attaches() && !app.has_pasted_blocks() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix('/') {
        return handle_slash(app, rest, input_tx);
    }

    let Some(prepared) = prepare_user_message(app, text, trimmed) else {
        return false;
    };

    // Record in prompt history (shell-style up/down recall).
    app.prompt_history.push(&prepared.composer);

    // While the agent is busy: queue — don't dump into chat / driver yet.
    if app.running {
        app.queue_follow_up(crate::app::QueuedFollowUp {
            display: prepared.display,
            text: prepared.agent_text,
            composer: prepared.composer,
            attaches: prepared.attaches,
            pasted: prepared.pasted,
            mode: prepared.mode,
        });
        return false;
    }

    dispatch_user(app, input_tx, prepared);
    false
}

pub(super) struct PreparedUser {
    display: String,
    agent_text: String,
    /// Original composer text (for ↑ recall).
    composer: String,
    attaches: Vec<crate::app::PendingAttach>,
    pasted: Vec<crate::app::pasted::PastedBlock>,
    mode: AgentMode,
}

impl PreparedUser {
    fn images(&self) -> Vec<ImageSource> {
        self.attaches
            .iter()
            .filter_map(|a| a.image.clone())
            .collect()
    }
}

pub(super) fn prepare_user_message(
    app: &mut App,
    text: String,
    trimmed: String,
) -> Option<PreparedUser> {
    let attaches = app.take_pending_attaches();
    let mut pasted = app.take_pasted_blocks();
    // The token lives in the composer text now — deleting it by hand takes
    // the paste with it.
    pasted.retain(|p| text.contains(&p.token()));
    let tags = attaches
        .iter()
        .map(|a| a.tag())
        .collect::<Vec<_>>()
        .join(" ");
    let file_notes: Vec<String> = attaches
        .iter()
        .filter(|a| a.image.is_none())
        .map(|a| match &a.content {
            Some(content) => format!(
                "[Attached file: {}]\n```\n{}\n```",
                a.path,
                truncate_attach(content)
            ),
            None => format!("[Attached file: {}]", a.path),
        })
        .collect();
    let pasted_notes: Vec<String> = pasted
        .iter()
        .map(|p| format!("{}\n```\n{}\n```", p.token(), truncate_attach(&p.content)))
        .collect();

    let all_tags = tags;
    let display = match (all_tags.is_empty(), trimmed.is_empty()) {
        (true, true) => return None,
        (false, true) => all_tags,
        (true, false) => text.clone(),
        (false, false) => format!("{all_tags} {text}"),
    };

    let composer = text.clone();
    let mut agent_text = text;
    for notes in [file_notes, pasted_notes] {
        if notes.is_empty() {
            continue;
        }
        if !agent_text.is_empty() {
            agent_text.push('\n');
        }
        agent_text.push_str(&notes.join("\n"));
    }

    Some(PreparedUser {
        display,
        agent_text,
        composer,
        attaches,
        pasted,
        mode: app.agent_mode,
    })
}

/// Cap attached file content so a huge file can't blow up the context.
const ATTACH_MAX_CHARS: usize = 100_000;

pub(super) fn truncate_attach(content: &str) -> &str {
    if content.len() <= ATTACH_MAX_CHARS {
        return content;
    }
    let mut end = ATTACH_MAX_CHARS;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}

/// Deferred dispatch: push the user block now but hold the driver send for a
/// brief grace period so ESC can recall the prompt before the agent starts.
pub(super) fn dispatch_user(
    app: &mut App,
    _input_tx: &UnboundedSender<InputCommand>,
    prepared: PreparedUser,
) {
    let images = prepared.images();
    app.push_user(prepared.display);
    app.pending_dispatch = Some(crate::app::PendingDispatch {
        agent_text: prepared.agent_text,
        images,
        mode: prepared.mode,
        composer: prepared.composer,
        attaches: prepared.attaches,
        pasted: prepared.pasted,
        submitted_at: std::time::Instant::now(),
    });
}

/// Immediate dispatch (no grace period) — used for auto-flushed follow-ups.
pub(super) fn dispatch_user_now(
    app: &mut App,
    input_tx: &UnboundedSender<InputCommand>,
    prepared: PreparedUser,
) {
    let images = prepared.images();
    app.push_user(prepared.display);
    let _ = input_tx.send(InputCommand::User {
        text: prepared.agent_text,
        images,
        mode: prepared.mode,
    });
}

/// After a turn ends, send any queued follow-up as the next user message.
pub(super) fn flush_follow_up(app: &mut App, input_tx: &UnboundedSender<InputCommand>) -> bool {
    let Some(fu) = app.follow_up.take() else {
        return false;
    };
    dispatch_user_now(
        app,
        input_tx,
        PreparedUser {
            display: fu.display,
            agent_text: fu.text,
            composer: fu.composer,
            attaches: fu.attaches,
            pasted: fu.pasted,
            mode: fu.mode,
        },
    );
    true
}

pub(super) fn handle_slash(
    app: &mut App,
    cmd: &str,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    let mut parts = cmd.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();

    // Removed command: paste a path instead.
    if name.eq_ignore_ascii_case("image") || name.eq_ignore_ascii_case("attach") {
        app.notice(" /attach was removed — paste a file path into the composer");
        return false;
    }

    if let Some(def) = commands::resolve(name) {
        return run_command(app, def.id, arg, input_tx);
    }

    if let Some(skill) = app.find_skill(name).cloned() {
        return invoke_skill(app, &skill, arg, input_tx);
    }

    app.notice(format!("unknown command: /{name}  — {UNKNOWN_HINT}"));
    false
}

/// Run a skill: show `/name` in the transcript and send the skill body to the agent.
pub(super) fn invoke_skill(
    app: &mut App,
    skill: &crate::SkillChoice,
    note: &str,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    let display = if note.is_empty() {
        format!("/{}", skill.name)
    } else {
        format!("/{} {}", skill.name, note)
    };

    let mut agent_text = format!(
        "The user invoked skill `{}` via the /{} command. \
Follow the skill instructions below immediately — do not ask whether to use it. \
You may call `read_skill` again if needed, but the full skill is already included.\n\n\
# Skill: {}\n\n{}\n",
        skill.name, skill.name, skill.name, skill.content
    );
    if !note.is_empty() {
        agent_text.push_str("\n## User note\n");
        agent_text.push_str(note);
        agent_text.push('\n');
    }

    app.push_user(display);
    app.flash(format!("Skill · {}", skill.name));
    let mode = app.agent_mode;
    let _ = input_tx.send(InputCommand::User {
        text: agent_text,
        images: Vec::new(),
        mode,
    });
    false
}

pub(super) fn run_command(
    app: &mut App,
    id: CmdId,
    arg: &str,
    input_tx: &UnboundedSender<InputCommand>,
) -> bool {
    match id {
        CmdId::Quit => return true,
        CmdId::Clear => {
            app.new_chat();
            let _ = input_tx.send(InputCommand::Clear);
            app.flash("New chat");
        }
        CmdId::Compact => {
            let _ = input_tx.send(InputCommand::Compact);
            app.flash("Compacting…");
        }
        CmdId::Model => {
            if arg.is_empty() {
                app.open_model_picker(input_tx);
            } else {
                let display = arg.rsplit('/').next().unwrap_or(arg).to_string();
                let (vision, context, cost_input, cost_output) = app
                    .model_choices
                    .iter()
                    .find(|m| m.key == arg)
                    .map(|m| (m.vision, m.context, m.cost_input, m.cost_output))
                    .unwrap_or((false, 0, 0.0, 0.0));
                let _ = input_tx.send(InputCommand::SetModel {
                    id: arg.to_string(),
                    display,
                    connection_id: None,
                    vision,
                    context,
                    cost_input,
                    cost_output,
                });
            }
        }
        CmdId::Copy => copy_last_answer(app),
        CmdId::Cost => app.notice(format!(
            "tokens — prompt {} · completion {} · total {}",
            app.usage.prompt_tokens, app.usage.completion_tokens, app.usage.total_tokens
        )),
        CmdId::Connect => app.open_connect_picker(),
        CmdId::About => app.open_about(),
        CmdId::Settings => app.open_settings(),
        CmdId::Resume => {
            if arg.is_empty() {
                let _ = input_tx.send(InputCommand::ListSessions);
                app.open_sessions_picker();
            } else {
                let _ = input_tx.send(InputCommand::LoadSession {
                    id: arg.trim().to_string(),
                });
                app.flash("Loading session…");
            }
        }
    }
    false
}
