//! Turning a raw SSE chunk stream into a finished assistant message.

pub mod accumulate;
pub mod responses;

pub use accumulate::Accumulator;
pub use responses::ResponsesAccumulator;
