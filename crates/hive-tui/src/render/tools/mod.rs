//! Transcript tool / subagent / plan card components.
//!
//! Lives in hive-tui (not comb): comb stays primitives; these compose them.

mod compacted_card;
mod goal_card;
mod loop_detected_card;
mod mode_switch_card;
mod plan_card;
mod plan_layout;
mod strip;
mod subagent_card;
mod terminal_card;
mod todo_card;
pub(crate) mod tool_card;
mod work_summary_card;

pub(crate) use compacted_card::compacted_card_lines;
pub(crate) use goal_card::goal_card_lines;
pub(crate) use loop_detected_card::loop_detected_card_lines;
pub(crate) use mode_switch_card::mode_switch_card_lines;
pub(crate) use plan_card::plan_card_lines;
pub(crate) use plan_layout::{
    col_to_offset, layout_plan_body, paint_highlights, MarkTone, PlanRowSpan,
};
pub(crate) use subagent_card::subagent_card_lines;
pub(crate) use terminal_card::terminal_card_lines;
pub(crate) use todo_card::todo_card_lines;
pub(crate) use tool_card::{format_tool_secs, tool_lines};
pub(crate) use work_summary_card::work_summary_card_lines;
