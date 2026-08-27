//! Conversation compaction: shrink history while keeping task continuity.
//!
//! Triggered manually (`/compact`) or automatically when the last request's
//! prompt tokens reach [`COMPACT_RATIO`] of the configured context window.
//! Safe to run mid-turn at the top of the tool loop (after tool results land).

use crate::message::{ContentPart, ImageSource, Message, Role};

/// Compact when prompt tokens ≥ this fraction of the context window.
pub const COMPACT_RATIO: f64 = 0.75;
/// Skip compact when there is almost nothing to shrink.
pub const MIN_MESSAGES_TO_COMPACT: usize = 4;
/// Cap transcript fed to the summarizer (characters).
const MAX_TRANSCRIPT_CHARS: usize = 120_000;
/// Truncate individual tool results in the transcript.
const MAX_TOOL_CHARS: usize = 1_200;
/// Truncate long assistant/user blobs in the transcript.
const MAX_TEXT_CHARS: usize = 4_000;

/// True when the last request filled enough of the context window.
pub fn should_compact(last_prompt_tokens: u64, context_window: u64) -> bool {
    if context_window == 0 || last_prompt_tokens == 0 {
        return false;
    }
    last_prompt_tokens as f64 >= context_window as f64 * COMPACT_RATIO
}

/// Rough token estimate when the provider has not reported usage yet.
pub fn estimate_tokens(messages: &[Message]) -> u64 {
    let chars: usize = messages.iter().map(message_chars).sum();
    (chars as u64 / 4).max(1)
}

fn message_chars(m: &Message) -> usize {
    if !m.provider_items.is_empty() {
        return m.provider_items.iter().map(json_chars).sum();
    }

    let mut n = m
        .content
        .iter()
        .map(|part| match part {
            ContentPart::Text(text) => text.len(),
            ContentPart::Image(ImageSource::Url(url)) => url.len(),
            // Image bytes are billed as image input, not tokenized base64 text.
            ContentPart::Image(ImageSource::Base64 { media_type, .. }) => media_type.len(),
        })
        .sum();
    for tc in &m.tool_calls {
        n += tc.name.len() + tc.arguments.len();
    }
    n
}

/// Approximate serialized JSON size without allocating a temporary string.
fn json_chars(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(true) => 4,
        serde_json::Value::Bool(false) => 5,
        serde_json::Value::Number(_) => 24,
        serde_json::Value::String(value) => value.len() + 2,
        serde_json::Value::Array(values) => {
            2 + values.iter().map(json_chars).sum::<usize>() + values.len().saturating_sub(1)
        }
        serde_json::Value::Object(values) => {
            2 + values
                .iter()
                .map(|(key, value)| key.len() + 3 + json_chars(value))
                .sum::<usize>()
                + values.len().saturating_sub(1)
        }
    }
}

/// Build a plain-text transcript for the summarizer (system message omitted).
pub fn format_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for m in messages {
        if matches!(m.role, Role::System) {
            continue;
        }
        let role = match m.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
            Role::System => continue,
        };
        out.push_str("## ");
        out.push_str(role);
        if let Some(name) = m.name.as_deref() {
            out.push_str(" (");
            out.push_str(name);
            out.push(')');
        }
        out.push('\n');

        let text = m.text();
        if !text.is_empty() {
            let cap = if matches!(m.role, Role::Tool) {
                MAX_TOOL_CHARS
            } else {
                MAX_TEXT_CHARS
            };
            out.push_str(&truncate_chars(&text, cap));
            out.push('\n');
        }
        for tc in &m.tool_calls {
            out.push_str(&format!(
                "- call {} `{}` {}\n",
                tc.id,
                tc.name,
                truncate_chars(&tc.arguments, 400)
            ));
        }
        out.push('\n');

        if out.len() > MAX_TRANSCRIPT_CHARS {
            out.truncate(MAX_TRANSCRIPT_CHARS);
            out.push_str("\n…[transcript truncated]\n");
            break;
        }
    }
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Prompt asking the model for a continuity summary (no fluff).
pub fn summarize_request(transcript: &str) -> Vec<Message> {
    let system = Message::system(
        "You compress agent chat history into a continuity brief. \
Output ONLY the brief — no preamble, no offers to help.",
    );
    let user = Message::user(format!(
        "Compress the transcript below into a continuity brief the same agent will read next.

Preserve:
- The user's current goal and hard constraints
- Decisions already made
- Files/paths created, edited, or deleted (be specific)
- Errors hit and how they were fixed
- Exact current status and the immediate next step

Omit:
- Raw tool dumps and full file contents
- Repetition, pleasantries, and speculation

Use short markdown headings exactly:
## Goal
## Done
## Files
## State
## Next

Transcript:

{transcript}"
    ));
    vec![system, user]
}

/// Replace history with system + continuity brief + the recent unsummarized tail.
pub fn compacted_messages(system: &str, summary: &str, recent: Vec<Message>) -> Vec<Message> {
    let summary = summary.trim();
    let body = format!(
        "# Session context (compacted)\n\n{summary}\n\n\
Continue from **State** / **Next** above. \
Do not mention compaction or summarization. \
Act as if you already had the full history and keep working."
    );
    let mut out = vec![
        Message::system(system.to_string()),
        Message {
            role: Role::User,
            content: vec![ContentPart::Text(body)],
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            provider_items: Vec::new(),
        },
    ];
    out.extend(recent);
    out
}

/// Index where the kept tail starts (after the system message). Tool-result
/// groups stay attached to their assistant call so the next request stays valid.
pub fn tail_start(messages: &[Message]) -> usize {
    const KEEP_RECENT: usize = 12;
    if messages.len() <= KEEP_RECENT + 1 {
        return 1;
    }
    let mut i = messages.len().saturating_sub(KEEP_RECENT);
    if i < 1 {
        i = 1;
    }
    while i > 1 && matches!(messages[i].role, Role::Tool) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ToolCall;

    #[test]
    fn threshold_at_75_percent() {
        assert!(!should_compact(74_999, 100_000));
        assert!(should_compact(75_000, 100_000));
        assert!(!should_compact(0, 100_000));
        assert!(!should_compact(90_000, 0));
    }

    #[test]
    fn transcript_truncates_tool_blobs() {
        let big = "x".repeat(5_000);
        let msgs = vec![
            Message::system("sys"),
            Message::user("do the thing"),
            Message::tool_result("c1", "read_file", big),
        ];
        let t = format_transcript(&msgs);
        assert!(t.contains("User"));
        assert!(t.contains("do the thing"));
        assert!(t.contains('…'));
        assert!(!t.contains(&"x".repeat(2_000)));
    }

    #[test]
    fn compacted_history_keeps_the_recent_tail() {
        let recent = vec![Message::user("still going")];
        let msgs = compacted_messages("sys", "## Goal\nShip it\n## Next\nKeep going", recent);
        assert!(msgs.len() >= 3);
        assert!(matches!(msgs[0].role, Role::System));
        assert!(matches!(msgs[1].role, Role::User));
        assert!(msgs[1].text().contains("Ship it"));
        assert!(msgs[1].text().contains("Do not mention compaction"));
        assert_eq!(msgs[2].text(), "still going");
    }

    #[test]
    fn tail_start_does_not_split_tool_results() {
        let msgs = vec![
            Message::system("sys"),
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
            Message::assistant("d"),
            {
                let mut m = Message::assistant("");
                m.tool_calls.push(ToolCall {
                    id: "1".into(),
                    name: "read_file".into(),
                    arguments: "{}".into(),
                });
                m
            },
            Message::tool_result("1", "read_file", "ok"),
        ];
        let start = tail_start(&msgs);
        assert!(
            start == msgs.len() || !matches!(msgs[start].role, Role::Tool),
            "tail must not start on a tool result"
        );
    }

    #[test]
    fn estimate_counts_tool_calls() {
        let mut m = Message::assistant("");
        m.tool_calls.push(ToolCall {
            id: "1".into(),
            name: "shell".into(),
            arguments: r#"{"cmd":"ls"}"#.into(),
        });
        let n = estimate_tokens(&[Message::system("a"), m]);
        assert!(n >= 1);
    }

    #[test]
    fn estimate_counts_provider_native_items() {
        let mut message = Message::assistant("short");
        message.provider_items = vec![serde_json::json!({
            "type": "reasoning",
            "encrypted_content": "x".repeat(4_000),
        })];

        assert!(estimate_tokens(&[message]) >= 1_000);
    }
}
