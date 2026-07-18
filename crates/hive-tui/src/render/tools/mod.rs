//! Transcript tool / subagent / plan card components.
//!
//! Lives in hive-tui (not comb): comb stays primitives; these compose them.

mod plan_card;
mod strip;
mod subagent_card;
pub(crate) mod tool_card;

pub(crate) use plan_card::{parse_sections, plan_card_lines};
pub(crate) use subagent_card::subagent_card_lines;
pub(crate) use tool_card::{format_tool_secs, tool_lines};
