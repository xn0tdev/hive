//! Transcript tool / subagent card components.
//!
//! Lives in hive-tui (not comb): comb stays primitives; these compose them.

mod subagent_card;
pub(crate) mod tool_card;

pub(crate) use subagent_card::subagent_card_lines;
pub(crate) use tool_card::{format_tool_secs, tool_lines};
