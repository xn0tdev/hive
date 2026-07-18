use async_trait::async_trait;
use std::sync::Arc;

use crate::error::Result;
use crate::message::ImageSource;

/// Turns an image into a detailed textual description, used as a fallback so
/// non-vision models can still "see". Implemented in `hive-vision`.
#[async_trait]
pub trait VisionDescriber: Send + Sync {
    async fn describe(&self, image: &ImageSource) -> Result<String>;
}

/// Used when vision is not wired: returns a placeholder instead of a description.
pub struct NoVision;

#[async_trait]
impl VisionDescriber for NoVision {
    async fn describe(&self, _image: &ImageSource) -> Result<String> {
        Ok("[image omitted: vision is not enabled]".to_string())
    }
}

pub fn no_vision() -> Arc<dyn VisionDescriber> {
    Arc::new(NoVision)
}
