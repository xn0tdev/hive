//! OpenAI Responses API request and streaming-event wire formats.

use hive_core::message::{ContentPart, Message, Role};
use hive_core::provider::{ChatRequest, ToolSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ResponsesTool>,
    pub stream: bool,
    pub store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ResponsesTool {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub strict: bool,
}

/// A single typed event from a streamed Responses API request.
#[derive(Debug, Deserialize)]
pub struct ResponseEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub output_index: Option<usize>,
    #[serde(default)]
    pub item: Option<Value>,
    #[serde(default)]
    pub response: Option<ResponseBody>,
    #[serde(default)]
    pub error: Option<ResponseError>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseBody {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub output: Vec<Value>,
    #[serde(default)]
    pub usage: Option<ResponsesUsage>,
    #[serde(default)]
    pub error: Option<ResponseError>,
    #[serde(default)]
    pub incomplete_details: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ResponsesUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

fn to_tool(spec: &ToolSpec) -> ResponsesTool {
    ResponsesTool {
        kind: "function",
        name: spec.name.clone(),
        description: spec.description.clone(),
        parameters: spec.parameters.clone(),
        // Hive's schemas are written for best-effort function calling and do
        // not universally satisfy OpenAI's strict-schema requirements.
        strict: false,
    }
}

fn input_message(message: &Message) -> Value {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "user",
    };

    if !message
        .content
        .iter()
        .any(|part| matches!(part, ContentPart::Image(_)))
    {
        return json!({"role": role, "content": message.text()});
    }

    let content: Vec<Value> = message
        .content
        .iter()
        .map(|part| match part {
            ContentPart::Text(text) => json!({"type": "input_text", "text": text}),
            ContentPart::Image(source) => {
                json!({"type": "input_image", "image_url": source.as_data_url()})
            }
        })
        .collect();
    json!({"role": role, "content": content})
}

fn append_message(input: &mut Vec<Value>, message: &Message) {
    match message.role {
        Role::System => {}
        Role::User => input.push(input_message(message)),
        Role::Assistant if !message.provider_items.is_empty() => {
            input.extend(message.provider_items.iter().cloned());
        }
        Role::Assistant => {
            let text = message.text();
            if !text.is_empty() || message.tool_calls.is_empty() {
                input.push(json!({"role": "assistant", "content": text}));
            }
            input.extend(message.tool_calls.iter().map(|call| {
                json!({
                    "type": "function_call",
                    "call_id": call.id,
                    "name": call.name,
                    "arguments": call.arguments,
                })
            }));
        }
        Role::Tool => {
            if let Some(call_id) = message.tool_call_id.as_deref() {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": message.text(),
                }));
            } else {
                // A malformed or legacy tool message is still useful context;
                // keep it as user text instead of silently dropping it.
                input.push(json!({
                    "role": "user",
                    "content": format!("Tool result: {}", message.text()),
                }));
            }
        }
    }
}

fn supports_temperature(model: &str) -> bool {
    let model = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    !model.starts_with("gpt-5")
        && !model.starts_with("o1")
        && !model.starts_with("o3")
        && !model.starts_with("o4")
        && !model.starts_with("o5")
        && !model.contains("codex")
}

pub fn build_responses_request(req: &ChatRequest) -> ResponsesRequest {
    let system_parts: Vec<String> = req
        .messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(Message::text)
        .filter(|text| !text.is_empty())
        .collect();
    let instructions = (!system_parts.is_empty()).then(|| system_parts.join("\n\n"));

    let mut input = Vec::new();
    for message in &req.messages {
        append_message(&mut input, message);
    }

    ResponsesRequest {
        model: req.model.clone(),
        instructions,
        input,
        tools: req.tools.iter().map(to_tool).collect(),
        stream: true,
        // Hive owns and trims its transcript, so it replays response items
        // locally rather than asking OpenAI to retain conversation state.
        store: false,
        temperature: supports_temperature(&req.model)
            .then_some(req.temperature)
            .flatten(),
        max_output_tokens: req.max_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_core::message::{ImageSource, ToolCall};

    fn tool() -> ToolSpec {
        ToolSpec {
            name: "lookup".into(),
            description: "Look something up".into(),
            parameters: json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            }),
        }
    }

    #[test]
    fn reasoning_tools_use_stateless_responses_items() {
        let reasoning = json!({
            "type": "reasoning",
            "id": "rs_1",
            "encrypted_content": "encrypted"
        });
        let function_call = json!({
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_1",
            "name": "lookup",
            "arguments": "{\"query\":\"hive\"}"
        });
        let mut assistant = Message::assistant("");
        assistant.content.clear();
        assistant.tool_calls.push(ToolCall {
            id: "call_1".into(),
            name: "lookup".into(),
            arguments: "{\"query\":\"hive\"}".into(),
        });
        assistant.provider_items = vec![reasoning.clone(), function_call.clone()];

        let request = ChatRequest {
            model: "gpt-5.6-luna".into(),
            messages: vec![
                Message::system("be useful"),
                Message::user("inspect it"),
                assistant,
                Message::tool_result("call_1", "lookup", "done"),
            ],
            tools: vec![tool()],
            temperature: Some(0.3),
            max_tokens: Some(4096),
        };

        let value = serde_json::to_value(build_responses_request(&request)).unwrap();
        assert_eq!(value["instructions"], "be useful");
        assert_eq!(value["store"], false);
        assert_eq!(value["stream"], true);
        assert_eq!(value["max_output_tokens"], 4096);
        assert!(value.get("temperature").is_none());
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(value["tools"][0]["name"], "lookup");
        assert_eq!(value["tools"][0]["strict"], false);
        assert!(value["tools"][0].get("function").is_none());

        let input = value["input"].as_array().unwrap();
        assert_eq!(input[0], json!({"role": "user", "content": "inspect it"}));
        assert_eq!(input[1], reasoning);
        assert_eq!(input[2], function_call);
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(input[3]["output"], "done");
    }

    #[test]
    fn image_inputs_use_responses_content_parts() {
        let request = ChatRequest {
            model: "gpt-4.1".into(),
            messages: vec![Message::user_parts(vec![
                ContentPart::Text("what is this?".into()),
                ContentPart::Image(ImageSource::Url("https://example.com/a.png".into())),
            ])],
            tools: Vec::new(),
            temperature: Some(0.2),
            max_tokens: None,
        };

        let value = serde_json::to_value(build_responses_request(&request)).unwrap();
        let temperature = value["temperature"].as_f64().unwrap();
        assert!((temperature - 0.2).abs() < 1e-6);
        assert_eq!(value["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(value["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(
            value["input"][0]["content"][1]["image_url"],
            "https://example.com/a.png"
        );
    }
}
