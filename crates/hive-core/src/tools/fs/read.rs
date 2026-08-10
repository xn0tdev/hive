//! Read-file tool implementation.

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

use super::super::resolve_workspace;
use super::super::str_arg;
use super::super::u64_arg;
use crate::message::ImageSource;
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

/// Recognized raster image extensions that `read_file` returns as images.
pub(super) fn image_media_type(path: &Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => return None,
    })
}

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a file. For UTF-8 text files, returns the text; optionally start at a 1-based line \
offset and limit the number of lines returned. For image files (png, jpg/jpeg, gif, webp, bmp) \
returns metadata, and attaches the image when the active model supports vision."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the current workspace. Absolute paths are rejected unless workspace_only=false."},
                "offset": {"type": "integer", "description": "1-based line to start reading from."},
                "limit": {"type": "integer", "description": "Maximum number of lines to return."}
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(path) = str_arg(&args, "path") else {
            return ToolResult::error("missing 'path'");
        };
        let full = match resolve_workspace(ctx, path) {
            Ok(path) => path,
            Err(error) => return ToolResult::error(error),
        };

        // Images: attach only for vision-capable models, and only under a size
        // cap so non-vision / huge files cannot blow up or break the request.
        const MAX_IMAGE_BYTES: usize = 768 * 1024;
        if let Some(media_type) = image_media_type(&full) {
            let bytes = match tokio::fs::read(&full).await {
                Ok(b) => b,
                Err(e) => return ToolResult::error(format!("cannot read {}: {e}", full.display())),
            };
            let note = format!(
                "Read image {} ({media_type}, {} bytes).",
                full.display(),
                bytes.len()
            );
            if !ctx.vision {
                return ToolResult::ok(format!(
                    "{note} Active model is not vision-capable — image not attached."
                ));
            }
            if bytes.len() > MAX_IMAGE_BYTES {
                return ToolResult::ok(format!(
                    "{note} Too large to attach (max {MAX_IMAGE_BYTES} bytes)."
                ));
            }
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return ToolResult::ok(format!("{note} Image attached.")).with_images(vec![
                ImageSource::Base64 {
                    media_type: media_type.to_string(),
                    data,
                },
            ]);
        }

        let content = match tokio::fs::read_to_string(&full).await {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("cannot read {}: {e}", full.display())),
        };

        let offset = u64_arg(&args, "offset");
        let limit = u64_arg(&args, "limit");
        let out = if offset.is_some() || limit.is_some() {
            let start = offset.unwrap_or(1).saturating_sub(1) as usize;
            let lines: Vec<&str> = content.lines().collect();
            let end = match limit {
                Some(l) => (start + l as usize).min(lines.len()),
                None => lines.len(),
            };
            lines.get(start..end).unwrap_or(&[]).join("\n")
        } else {
            content
        };

        const CAP: usize = 100_000;
        if out.len() > CAP {
            let mut s: String = out.chars().take(CAP).collect();
            s.push_str("\n… [truncated]");
            ToolResult::ok(s)
        } else {
            ToolResult::ok(out)
        }
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(ReadFile) as Arc<dyn Tool> } }
