//! The OpenAI-compatible wire format, split by direction:
//! - `request`: what we send (messages, tools) + conversions from `hive-core`.
//! - `response`: the streamed chunk shapes we parse.

pub mod request;
pub mod response;
pub mod responses;

pub use request::build_request;
pub use response::ChatChunk;
pub use responses::{build_responses_request, ResponseEvent};
