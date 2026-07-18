//! The describe-and-inject vision fallback: send an image to a vision-capable
//! model and get back an exhaustive textual description that a non-vision model
//! can consume as ordinary text. Implements `hive_core::VisionDescriber`.

use async_trait::async_trait;
use std::sync::Arc;

use hive_core::error::Result;
use hive_core::message::{ContentPart, ImageSource, Message};
use hive_core::provider::{ChatRequest, Delta, LlmProvider};
use hive_core::vision::VisionDescriber;

const PROMPT: &str = "Describe this image in exhaustive detail so that someone who cannot see it \
can fully understand what it shows. Transcribe ALL visible text verbatim. Describe layout, UI \
elements, colors, code, diagrams, charts, and any notable details. Be thorough and precise.";

pub struct DescribeVision {
    provider: Arc<dyn LlmProvider>,
    model: String,
}

impl DescribeVision {
    pub fn new(provider: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        DescribeVision {
            provider,
            model: model.into(),
        }
    }
}

#[async_trait]
impl VisionDescriber for DescribeVision {
    async fn describe(&self, image: &ImageSource) -> Result<String> {
        let user = Message::user_parts(vec![
            ContentPart::Text(PROMPT.to_string()),
            ContentPart::Image(image.clone()),
        ]);
        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                Message::system("You are a meticulous vision assistant."),
                user,
            ],
            tools: Vec::new(),
            temperature: Some(0.2),
            max_tokens: Some(1500),
        };

        let mut sink = |_d: Delta| {};
        let outcome = self.provider.chat_stream(req, &mut sink).await?;
        Ok(outcome.message.text())
    }
}
