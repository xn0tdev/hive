//! Tool for pulling a skill's full instructions into context on demand.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::str_arg;

pub struct ReadSkill;

#[async_trait]
impl Tool for ReadSkill {
    fn name(&self) -> &str {
        "read_skill"
    }

    fn description(&self) -> &str {
        "Load the full instructions of a named skill (from the skill catalogue in the system prompt) and follow them."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "The skill name to load."}
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(name) = str_arg(&args, "name") else {
            return ToolResult::error("missing 'name'");
        };
        match ctx.skills.read(name) {
            Some(content) => ToolResult::ok(content),
            None => {
                let available: Vec<String> =
                    ctx.skills.list().into_iter().map(|m| m.name).collect();
                ToolResult::error(format!(
                    "no skill named '{name}'. available: {}",
                    if available.is_empty() {
                        "(none)".to_string()
                    } else {
                        available.join(", ")
                    }
                ))
            }
        }
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(ReadSkill) as Arc<dyn Tool> } }
