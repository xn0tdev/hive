//! Recap overlay: a short retelling of one turn, opened from "Worked for".

use tokio::sync::mpsc::UnboundedSender;

use super::state::{Block, RecapBody, WorkSummaryCard};
use super::App;
use crate::InputCommand;

const USER_CHARS: usize = 800;
const ASSISTANT_CHARS: usize = 1_200;
const TOOL_OUT_CHARS: usize = 200;
const TOTAL_CHARS: usize = 4_000;

/// Open overlay for the "Worked for" card at `block_idx`.
///
/// First open of an idle card starts generation. Later opens reuse the same
/// recap — closing the panel never deletes it.
pub struct RecapOverlay {
    pub recap_id: u64,
    /// User asked to watch the stream instead of the Generating label.
    pub reveal_stream: bool,
    pub scroll: usize,
    /// Body rows visible in the last paint (for clamping scroll).
    pub visible: usize,
}

impl App {
    pub fn recap_open(&self) -> bool {
        self.recap_overlay.is_some()
    }

    pub fn close_recap(&mut self) {
        self.recap_overlay = None;
        self.recap_generating_hit = None;
    }

    pub fn open_recap(&mut self, block_idx: usize, input_tx: &UnboundedSender<InputCommand>) {
        self.close_palette();
        self.close_settings();
        self.close_about();
        self.close_context_menu();

        let Some(Block::WorkSummary(card)) = self.blocks.get(block_idx) else {
            return;
        };
        let recap_id = card.recap_id;
        let already = !matches!(card.recap, RecapBody::Idle);
        let reveal_stream =
            matches!(&card.recap, RecapBody::Generating { text } if !text.is_empty());

        if !already {
            let context = turn_context(&self.blocks, block_idx);
            if let Some(Block::WorkSummary(card)) = self.blocks.get_mut(block_idx) {
                card.recap = RecapBody::Generating {
                    text: String::new(),
                };
            }
            let _ = input_tx.send(InputCommand::Recap {
                id: recap_id,
                context,
            });
        }

        self.recap_overlay = Some(RecapOverlay {
            recap_id,
            reveal_stream,
            scroll: 0,
            visible: 0,
        });
    }

    pub fn recap_card(&self, id: u64) -> Option<&WorkSummaryCard> {
        self.blocks.iter().find_map(|b| match b {
            Block::WorkSummary(c) if c.recap_id == id => Some(c),
            _ => None,
        })
    }

    pub fn recap_card_mut(&mut self, id: u64) -> Option<&mut WorkSummaryCard> {
        self.blocks.iter_mut().find_map(|b| match b {
            Block::WorkSummary(c) if c.recap_id == id => Some(c),
            _ => None,
        })
    }

    pub fn reveal_recap_stream(&mut self) -> bool {
        let Some(st) = self.recap_overlay.as_mut() else {
            return false;
        };
        if st.reveal_stream {
            return false;
        }
        st.reveal_stream = true;
        true
    }

    pub fn scroll_recap(&mut self, delta: isize) -> bool {
        let Some(st) = self.recap_overlay.as_ref() else {
            return false;
        };
        let vis = st.visible.max(1);
        let id = st.recap_id;
        let scroll = st.scroll;
        let Some(card) = self.recap_card(id) else {
            return false;
        };
        let lines = recap_body_line_count(card.recap.text());
        let max = lines.saturating_sub(vis);
        let next = (scroll as isize + delta).clamp(0, max as isize) as usize;
        if next == scroll {
            return false;
        }
        if let Some(st) = self.recap_overlay.as_mut() {
            st.scroll = next;
        }
        true
    }
}

/// Rough wrapped-line count so scroll clamps before the next paint.
fn recap_body_line_count(text: &str) -> usize {
    text.lines()
        .map(|l| l.chars().count().div_ceil(40).max(1))
        .sum::<usize>()
        .max(1)
}

/// Transcript slice for one turn: after the previous Worked-for line, up to
/// (not including) this one. That is the request, not the whole session.
pub fn turn_context(blocks: &[Block], summary_idx: usize) -> String {
    let start = blocks[..summary_idx.min(blocks.len())]
        .iter()
        .rposition(|b| matches!(b, Block::WorkSummary(_)))
        .map(|i| i + 1)
        .unwrap_or(0);
    let slice = &blocks[start..summary_idx.min(blocks.len())];

    let mut out = String::new();
    for block in slice {
        if out.len() >= TOTAL_CHARS {
            break;
        }
        let piece = match block {
            Block::User(text) => format!("User:\n{}\n", clip(text, USER_CHARS)),
            Block::Assistant { text, .. } if !text.trim().is_empty() => {
                format!("Assistant:\n{}\n", clip(text, ASSISTANT_CHARS))
            }
            Block::Tool(card) => {
                let mut line = format!("Tool {} {}", card.name, card.args);
                if !card.output.trim().is_empty() {
                    line.push('\n');
                    line.push_str(&clip(&card.output, TOOL_OUT_CHARS));
                }
                line.push('\n');
                line
            }
            Block::Explore(group) => group
                .tools
                .iter()
                .map(|card| format!("Tool {} {}", card.name, card.args))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Plan(card) => format!("Plan: {}\n", clip(&card.summary, 200)),
            Block::ModeSwitch(card) => format!("Mode: {}\n", card.reason),
            Block::Notice(s) | Block::Error(s) => format!("{s}\n"),
            _ => continue,
        };
        if out.len() + piece.len() > TOTAL_CHARS {
            let room = TOTAL_CHARS.saturating_sub(out.len());
            out.push_str(&piece.chars().take(room).collect::<String>());
            break;
        }
        out.push_str(&piece);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    if out.trim().is_empty() {
        "User asked something; the agent finished the turn with no visible text.".into()
    } else {
        out
    }
}

fn clip(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= max {
        trimmed.to_string()
    } else {
        let mut s: String = chars.into_iter().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(s: &str) -> Block {
        Block::User(s.into())
    }

    fn assistant(s: &str) -> Block {
        Block::Assistant {
            text: s.into(),
            streaming: false,
        }
    }

    fn summary() -> Block {
        Block::WorkSummary(WorkSummaryCard {
            secs: 2,
            recap_id: 1,
            recap: RecapBody::Idle,
        })
    }

    #[test]
    fn context_is_only_the_latest_turn() {
        let blocks = vec![
            user("first job"),
            assistant("did first"),
            Block::WorkSummary(WorkSummaryCard {
                secs: 1,
                recap_id: 0,
                recap: RecapBody::Idle,
            }),
            user("second job"),
            assistant("did second"),
            summary(),
        ];
        let ctx = turn_context(&blocks, 5);
        assert!(ctx.contains("second job"), "{ctx}");
        assert!(ctx.contains("did second"), "{ctx}");
        assert!(!ctx.contains("first job"), "{ctx}");
        assert!(!ctx.contains("did first"), "{ctx}");
    }

    #[test]
    fn first_turn_starts_at_the_user_message() {
        let blocks = vec![user("ship it"), assistant("shipped"), summary()];
        let ctx = turn_context(&blocks, 2);
        assert!(ctx.contains("ship it"), "{ctx}");
        assert!(ctx.contains("shipped"), "{ctx}");
    }
}
