//! Web tools backed by the Exa API: search and full-content fetch. Both are
//! gracefully unavailable when no EXA_API_KEY is configured.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::{http_client, str_arg, u64_arg};

pub struct WebSearch;

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web with Exa. Returns titles, URLs, and the most relevant excerpts. Use for current information or docs."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "The search query. Rich, specific queries work best."},
                "num_results": {"type": "integer", "description": "How many results (1-10, default 5)."},
                "type": {"type": "string", "description": "Search type: auto (default), fast, or instant."}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(query) = str_arg(&args, "query") else {
            return ToolResult::error("missing 'query'");
        };
        let Some(key) = ctx.config.secrets.exa_api_key.clone() else {
            return ToolResult::error("web search unavailable: EXA_API_KEY is not set");
        };

        let num = u64_arg(&args, "num_results").unwrap_or(5).clamp(1, 10);
        let search_type = str_arg(&args, "type").unwrap_or("auto");
        let base = ctx.config.exa.base_url.trim_end_matches('/');

        let body = json!({
            "query": query,
            "type": search_type,
            "numResults": num,
            "contents": { "highlights": true, "text": { "maxCharacters": 800 } }
        });

        let resp = http_client()
            .post(format!("{base}/search"))
            .header("x-api-key", key)
            .json(&body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("request failed: {e}")),
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return ToolResult::error(format!("exa {status}: {text}"));
        }
        let v: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => return ToolResult::error(format!("bad response: {e}")),
        };

        ToolResult::ok(format_results(&v))
    }
}

fn format_results(v: &Value) -> String {
    let Some(results) = v.get("results").and_then(|r| r.as_array()) else {
        return "(no results)".to_string();
    };
    if results.is_empty() {
        return "(no results)".to_string();
    }
    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        let title = r
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("(untitled)");
        let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("");
        out.push_str(&format!("{}. {title}\n   {url}\n", i + 1));
        if let Some(hl) = r.get("highlights").and_then(|h| h.as_array()) {
            for h in hl.iter().filter_map(|x| x.as_str()).take(2) {
                out.push_str(&format!("   › {}\n", h.replace('\n', " ")));
            }
        }
        out.push('\n');
    }
    out
}

pub struct WebGetContents;

#[async_trait]
impl Tool for WebGetContents {
    fn name(&self) -> &str {
        "web_get_contents"
    }

    fn description(&self) -> &str {
        "Fetch the readable text of one or more URLs via Exa. Use after web_search to read a page in full."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "urls": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "URLs to fetch."
                }
            },
            "required": ["urls"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let urls: Vec<String> = match args.get("urls").and_then(|u| u.as_array()) {
            Some(arr) => arr
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
            None => return ToolResult::error("missing 'urls' array"),
        };
        if urls.is_empty() {
            return ToolResult::error("no urls provided");
        }
        let Some(key) = ctx.config.secrets.exa_api_key.clone() else {
            return ToolResult::error("web contents unavailable: EXA_API_KEY is not set");
        };
        let base = ctx.config.exa.base_url.trim_end_matches('/');

        let body = json!({ "ids": urls, "text": { "maxCharacters": 2000 } });

        let resp = http_client()
            .post(format!("{base}/contents"))
            .header("x-api-key", key)
            .json(&body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("request failed: {e}")),
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return ToolResult::error(format!("exa {status}: {text}"));
        }
        let v: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => return ToolResult::error(format!("bad response: {e}")),
        };

        let Some(results) = v.get("results").and_then(|r| r.as_array()) else {
            return ToolResult::ok("(no content)");
        };
        let mut out = String::new();
        for r in results {
            let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("");
            let text = r.get("text").and_then(|t| t.as_str()).unwrap_or("");
            out.push_str(&format!("## {url}\n{text}\n\n"));
        }
        if out.is_empty() {
            out.push_str("(no content)");
        }
        ToolResult::ok(out)
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(WebSearch) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(WebGetContents) as Arc<dyn Tool> } }
