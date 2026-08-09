//! Accumulation for OpenAI Responses API typed streaming events.

use std::collections::BTreeMap;

use hive_core::message::{ContentPart, Message, Role, ToolCall};
use hive_core::provider::{ChatOutcome, Delta, Usage};
use serde_json::Value;

use crate::wire::responses::{ResponseBody, ResponseError, ResponseEvent};

#[derive(Default)]
pub struct ResponsesAccumulator {
    text: String,
    output: BTreeMap<usize, Value>,
    usage: Usage,
    finish_reason: String,
}

impl ResponsesAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_event(&mut self, event: ResponseEvent) -> Result<Vec<Delta>, String> {
        if event.kind == "error" {
            return Err(event_error(&event));
        }

        let mut deltas = Vec::new();
        match event.kind.as_str() {
            "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(delta) = event.delta.filter(|delta| !delta.is_empty()) {
                    self.text.push_str(&delta);
                    deltas.push(Delta::Text(delta));
                }
            }
            "response.reasoning_summary_text.delta" => {
                if let Some(delta) = event.delta.filter(|delta| !delta.is_empty()) {
                    deltas.push(Delta::Reasoning(delta));
                }
            }
            "response.output_item.done" => {
                if let (Some(index), Some(item)) = (event.output_index, event.item) {
                    self.output.insert(index, item);
                    self.sync_text_from_output(&mut deltas);
                }
            }
            "response.completed" => {
                if let Some(response) = event.response {
                    self.apply_response(response, "stop", &mut deltas)?;
                }
            }
            "response.incomplete" => {
                if let Some(response) = event.response {
                    let reason = response
                        .incomplete_details
                        .as_ref()
                        .and_then(|details| details.get("reason"))
                        .and_then(Value::as_str)
                        .unwrap_or("incomplete")
                        .to_string();
                    self.apply_response(response, &reason, &mut deltas)?;
                }
            }
            "response.failed" => {
                let response = event.response;
                return Err(response
                    .as_ref()
                    .and_then(|response| response.error.as_ref())
                    .map(format_response_error)
                    .unwrap_or_else(|| "OpenAI response failed".to_string()));
            }
            _ => {}
        }

        Ok(deltas)
    }

    fn apply_response(
        &mut self,
        response: ResponseBody,
        default_reason: &str,
        deltas: &mut Vec<Delta>,
    ) -> Result<(), String> {
        if let Some(error) = response.error.as_ref() {
            return Err(format_response_error(error));
        }
        if let Some(usage) = response.usage {
            self.usage = Usage {
                prompt_tokens: usage.input_tokens,
                completion_tokens: usage.output_tokens,
                total_tokens: usage.total_tokens,
            };
        }
        if !response.output.is_empty() {
            self.output = response.output.into_iter().enumerate().collect();
        }
        self.finish_reason = response
            .status
            .unwrap_or_else(|| default_reason.to_string());
        if self.finish_reason == "completed" {
            self.finish_reason = default_reason.to_string();
        }
        self.sync_text_from_output(deltas);
        Ok(())
    }

    fn sync_text_from_output(&mut self, deltas: &mut Vec<Delta>) {
        let text = output_text(self.output.values());
        if !text.is_empty() {
            self.sync_text(text, deltas);
        }
    }

    fn sync_text(&mut self, complete: String, deltas: &mut Vec<Delta>) {
        if complete == self.text {
            return;
        }
        if let Some(missing) = complete.strip_prefix(&self.text) {
            if !missing.is_empty() {
                deltas.push(Delta::Text(missing.to_string()));
            }
        }
        self.text = complete;
    }

    pub fn finish(self) -> ChatOutcome {
        let mut provider_items: Vec<Value> = self.output.into_values().collect();
        let mut tool_calls = Vec::new();

        for item in &mut provider_items {
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            if object.get("type").and_then(Value::as_str) != Some("function_call") {
                continue;
            }

            let name = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let call_id = object
                .get("call_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()));
            if object
                .get("call_id")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                object.insert("call_id".to_string(), Value::String(call_id.clone()));
            }
            let arguments = object
                .get("arguments")
                .and_then(Value::as_str)
                .filter(|arguments| !arguments.is_empty())
                .unwrap_or("{}")
                .to_string();
            if object
                .get("arguments")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                object.insert("arguments".to_string(), Value::String(arguments.clone()));
            }
            tool_calls.push(ToolCall {
                id: call_id,
                name,
                arguments,
            });
        }

        let text = if self.text.is_empty() {
            output_text(provider_items.iter())
        } else {
            self.text
        };
        let mut content = Vec::new();
        if !text.is_empty() {
            content.push(ContentPart::Text(text));
        }

        let finish_reason = if !tool_calls.is_empty() {
            "tool_calls".to_string()
        } else if self.finish_reason.is_empty() {
            "stop".to_string()
        } else {
            self.finish_reason
        };

        ChatOutcome {
            message: Message {
                role: Role::Assistant,
                content,
                tool_calls,
                tool_call_id: None,
                name: None,
                provider_items,
            },
            usage: self.usage,
            finish_reason,
        }
    }
}

fn output_text<'a>(items: impl IntoIterator<Item = &'a Value>) -> String {
    let mut text = String::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for part in content {
            match part.get("type").and_then(Value::as_str) {
                Some("output_text") => {
                    if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                        text.push_str(part_text);
                    }
                }
                Some("refusal") => {
                    if let Some(refusal) = part.get("refusal").and_then(Value::as_str) {
                        text.push_str(refusal);
                    }
                }
                _ => {}
            }
        }
    }
    text
}

fn format_response_error(error: &ResponseError) -> String {
    match error.code.as_deref().filter(|code| !code.is_empty()) {
        Some(code) if !error.message.is_empty() => format!("{code}: {}", error.message),
        Some(code) => code.to_string(),
        None if !error.message.is_empty() => error.message.clone(),
        None => "OpenAI response failed".to_string(),
    }
}

fn event_error(event: &ResponseEvent) -> String {
    if let Some(error) = event.error.as_ref() {
        return format_response_error(error);
    }
    match (
        event.code.as_deref().filter(|code| !code.is_empty()),
        event
            .message
            .as_deref()
            .filter(|message| !message.is_empty()),
    ) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code.to_string(),
        (None, Some(message)) => message.to_string(),
        (None, None) => "OpenAI stream error".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(value: Value) -> ResponseEvent {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn completed_response_preserves_reasoning_tools_text_and_usage() {
        let reasoning = json!({
            "type": "reasoning",
            "id": "rs_1",
            "encrypted_content": "opaque"
        });
        let message = json!({
            "type": "message",
            "id": "msg_1",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "Checking", "annotations": []}]
        });
        let call = json!({
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_1",
            "name": "inspect",
            "arguments": "{\"path\":\".\"}"
        });

        let mut accumulator = ResponsesAccumulator::new();
        let deltas = accumulator
            .push_event(event(json!({
                "type": "response.output_text.delta",
                "delta": "Check"
            })))
            .unwrap();
        assert!(matches!(&deltas[0], Delta::Text(text) if text == "Check"));

        let deltas = accumulator
            .push_event(event(json!({
                "type": "response.completed",
                "response": {
                    "status": "completed",
                    "output": [reasoning, message, call],
                    "usage": {"input_tokens": 12, "output_tokens": 8, "total_tokens": 20}
                }
            })))
            .unwrap();
        assert!(matches!(&deltas[0], Delta::Text(text) if text == "ing"));

        let outcome = accumulator.finish();
        assert_eq!(outcome.message.text(), "Checking");
        assert_eq!(outcome.message.tool_calls.len(), 1);
        assert_eq!(outcome.message.tool_calls[0].id, "call_1");
        assert_eq!(outcome.message.tool_calls[0].name, "inspect");
        assert_eq!(outcome.message.provider_items.len(), 3);
        assert_eq!(
            outcome.message.provider_items[0]["encrypted_content"],
            "opaque"
        );
        assert_eq!(outcome.usage.prompt_tokens, 12);
        assert_eq!(outcome.usage.completion_tokens, 8);
        assert_eq!(outcome.usage.total_tokens, 20);
        assert_eq!(outcome.finish_reason, "tool_calls");
    }
}
