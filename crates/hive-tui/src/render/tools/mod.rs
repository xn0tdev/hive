//! Transcript tool / subagent / plan card components.
//!
//! Lives in hive-tui (not comb): comb stays primitives; these compose them.

mod mode_switch_card;
mod plan_card;
mod plan_layout;
mod strip;
mod subagent_card;
mod terminal_card;
pub(crate) mod tool_card;

pub(crate) use mode_switch_card::mode_switch_card_lines;
pub(crate) use plan_card::plan_card_lines;
pub(crate) use plan_layout::{
    col_to_offset, layout_plan_body, paint_highlights, MarkTone, PlanRowSpan,
};
pub(crate) use subagent_card::subagent_card_lines;
pub(crate) use terminal_card::terminal_card_lines;
pub(crate) use tool_card::{format_tool_secs, tool_lines};
