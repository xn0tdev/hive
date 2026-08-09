//! Outbound request serialization and conversion from hive's internal types.

use serde::Serialize;

use hive_core::message::{ContentPart, Message, Role};
use hive_core::provider::{ChatRequest, ToolSpec};

#[derive(Serialize)]
pub struct WireRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<WireTool<'a>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream_options: StreamOptions,
}

#[derive(Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Serialize)]
pub struct WireMessage {
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<WireContent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<WireToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum WireContent {
    Text(String),
    Parts(Vec<WirePart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WirePart {
    Text { text: String },
    ImageUrl { image_url: WireImageUrl },
}

#[derive(Serialize)]
pub struct WireImageUrl {
    pub url: String,
}

#[derive(Serialize)]
pub struct WireToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: WireFn,
}

#[derive(Serialize)]
pub struct WireFn {
    pub name: String,
    pub arguments: String,
}

#[derive(Serialize)]
pub struct WireTool<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: WireToolFn<'a>,
}

#[derive(Serialize)]
pub struct WireToolFn<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub parameters: &'a serde_json::Value,
}

fn role_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn to_wire_message(m: &Message) -> WireMessage {
    let has_image = m.content.iter().any(|p| matches!(p, ContentPart::Image(_)));
    let content = if has_image {
        let parts = m
            .content
            .iter()
            .map(|p| match p {
                ContentPart::Text(t) => WirePart::Text { text: t.clone() },
                ContentPart::Image(src) => WirePart::ImageUrl {
                    image_url: WireImageUrl {
                        url: src.as_data_url(),
                    },
                },
            })
            .collect();
        Some(WireContent::Parts(parts))
    } else {
        let text = m.text();
        // Assistant messages that only call tools legitimately have no content.
        if text.is_empty() && !m.tool_calls.is_empty() {
            None
        } else {
            Some(WireContent::Text(text))
        }
    };

    let tool_calls = m
        .tool_calls
        .iter()
        .map(|tc| WireToolCall {
            id: tc.id.clone(),
            kind: "function",
            function: WireFn {
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            },
        })
        .collect();

    WireMessage {
        role: role_str(m.role),
        content,
        tool_calls,
        tool_call_id: m.tool_call_id.clone(),
        name: m.name.clone(),
    }
}

fn to_wire_tool(spec: &ToolSpec) -> WireTool<'_> {
    WireTool {
        kind: "function",
        function: WireToolFn {
            name: &spec.name,
            description: &spec.description,
            parameters: &spec.parameters,
        },
    }
}

pub fn build_request<'a>(req: &'a ChatRequest<'a>) -> WireRequest<'a> {
    WireRequest {
        model: req.model,
        messages: req.messages.iter().map(to_wire_message).collect(),
        tools: req.tools.iter().map(|spec| to_wire_tool(spec)).collect(),
        stream: true,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        stream_options: StreamOptions {
            include_usage: true,
        },
    }
}
