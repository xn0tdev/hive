//! Plan preview, correction, and plan-selection methods.

use super::*;

impl App {
    /// Persist the composer into the active correction comment.
    pub fn plan_sync_active_note(&mut self) {
        let Some(i) = self.plan_view.active else {
            return;
        };
        if let Some(c) = self.plan_view.corrections.get_mut(i) {
            c.note = self.input.value.clone();
        }
    }

    /// True while writing/editing a comment (composer + MARK).
    /// Idle with saved comments shows ← back + SEND (like ← back + MAKE).
    pub fn plan_composing(&self) -> bool {
        self.plan_view
            .drag_range()
            .is_some_and(|(start, end)| start < end)
            || self.plan_view.active.is_some()
            || (!self.input.is_empty() && self.plan_view.has_corrections())
    }

    /// Enter / MARK: attach composer text to the selection, then idle → SEND.
    /// Returns whether anything changed.
    pub fn plan_commit_note(&mut self) -> bool {
        if !self.plan_composing() && !self.plan_view.has_corrections() {
            return false;
        }
        if self.plan_view.active.is_none() && self.plan_view.drag.is_none() {
            return false;
        }
        self.plan_sync_active_note();
        let _ = self.input.take();
        self.plan_view.active = None;
        self.blur_input();
        let n = self.plan_view.corrections.len();
        if n == 0 {
            self.flash("Nothing to add");
        } else if n == 1 {
            self.flash("Comment added · select more or SEND");
        } else {
            self.flash(format!("{n} comments · select more or SEND"));
        }
        true
    }

    /// Which right-chip: MAKE (no comments) / MARK (composing) / SEND (idle with comments).
    pub fn plan_action(&self) -> PlanAction {
        if self.plan_composing() {
            PlanAction::Add
        } else if !self.plan_view.has_corrections() {
            PlanAction::Make
        } else {
            PlanAction::Send
        }
    }

    /// Four-cell chip labels keep every action exactly centered in the same button.
    pub fn plan_action_word(&self) -> &'static str {
        match self.plan_action() {
            PlanAction::Make => "MAKE",
            PlanAction::Add => "MARK",
            PlanAction::Send => "SEND",
        }
    }

    /// Focus a correction and load its comment into the composer (read / edit).
    pub fn plan_focus_correction(&mut self, idx: usize) -> bool {
        if idx >= self.plan_view.corrections.len() {
            return false;
        }
        self.plan_sync_active_note();
        self.plan_view.active = Some(idx);
        let note = self.plan_view.corrections[idx].note.clone();
        self.input.clear();
        if !note.is_empty() {
            self.input.insert_str(&note);
        }
        self.focus_input();
        self.flash("Edit comment · MARK saves");
        true
    }

    /// Leave the composer; keep saved comments (back + SEND).
    pub fn plan_stop_composing(&mut self) -> bool {
        if !self.plan_composing() {
            return false;
        }
        // Drop an unfinished mark with no comment yet.
        if let Some(i) = self.plan_view.active {
            if self
                .plan_view
                .corrections
                .get(i)
                .is_some_and(|c| c.note.trim().is_empty() && self.input.is_empty())
            {
                self.plan_view.corrections.remove(i);
            } else {
                self.plan_sync_active_note();
            }
        }
        self.plan_view.active = None;
        self.plan_view.drag = None;
        let _ = self.input.take();
        self.blur_input();
        true
    }

    /// Finalize a drag range into a new correction (or focus an overlapping one).
    pub fn plan_finish_selection(&mut self, mut start: usize, mut end: usize) -> bool {
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        self.plan_view.drag = None;
        if start > end {
            return false;
        }
        let Some(body) = self.plan_card().map(|c| c.body.clone()) else {
            return false;
        };
        if end > body.len() || !body.is_char_boundary(start) || !body.is_char_boundary(end) {
            return false;
        }
        // Click or selection fully inside an existing mark → open note to read/edit.
        if let Some(i) = self
            .plan_view
            .corrections
            .iter()
            .position(|c| start >= c.start && start < c.end && end <= c.end)
        {
            return self.plan_focus_correction(i);
        }
        // Empty click outside any mark.
        if start == end {
            return false;
        }
        let slice = body.get(start..end).unwrap_or("");
        let Some(rel_start) = slice.find(|c: char| !c.is_whitespace()) else {
            return false;
        };
        let rel_end = slice
            .rfind(|c: char| !c.is_whitespace())
            .map(|i| i + slice[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1))
            .unwrap_or(rel_start);
        let new_start = start + rel_start;
        let new_end = start + rel_end;
        start = new_start;
        end = new_end;
        if start >= end || !body.is_char_boundary(start) || !body.is_char_boundary(end) {
            return false;
        }
        let excerpt = body.get(start..end).unwrap_or("").to_string();
        self.plan_sync_active_note();
        self.plan_view.corrections.push(PlanCorrection {
            start,
            end,
            excerpt,
            note: String::new(),
        });
        let idx = self.plan_view.corrections.len() - 1;
        self.plan_view.active = Some(idx);
        let _ = self.input.take();
        self.focus_input();
        true
    }

    /// Build the revise-plan user message from text corrections.
    pub fn plan_corrections_message(&self) -> String {
        let mut out = String::from(
            "Revise the plan in `.hive/Plan.md` based on these corrections. \
Keep everything else unless a note says otherwise.\n",
        );
        for (i, c) in self.plan_view.corrections.iter().enumerate() {
            out.push_str(&format!("\n### Correction {}\n\n", i + 1));
            for line in c.excerpt.lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            if !c.note.trim().is_empty() {
                out.push_str("\nNote: ");
                out.push_str(c.note.trim());
                out.push('\n');
            }
        }
        out
    }

    /// Clear corrections and leave the composer (stay in plan view).
    pub fn plan_clear_corrections(&mut self) {
        self.plan_view.corrections.clear();
        self.plan_view.active = None;
        self.plan_view.drag = None;
        let _ = self.input.take();
        self.blur_input();
    }

    /// Map a screen cell in the plan transcript to a source byte offset.
    pub fn plan_byte_at(&self, col: u16, row: u16) -> Option<usize> {
        use crate::render::tools::{col_to_offset, PlanRowSpan};

        let hit = self.transcript_hit?;
        if !hit.contains(col, row) {
            return None;
        }
        let scroll = self
            .transcript_max_scroll
            .saturating_sub(self.scroll_from_bottom);
        let line_idx = scroll + (row - hit.y) as usize;
        let body_i = line_idx.checked_sub(self.plan_view.body_line0)?;
        let &(start, end) = self.plan_view.row_spans.get(body_i)?;
        let body = self.plan_card()?.body.as_str();
        if start > body.len() || end > body.len() {
            return None;
        }
        let x = col.saturating_sub(hit.x) as usize;
        let content_col = x.saturating_sub(2);
        Some(col_to_offset(
            body,
            &PlanRowSpan { start, end },
            content_col,
        ))
    }

    /// Start / update / finish a drag selection over the plan body.
    pub fn plan_drag_to(&mut self, col: u16, row: u16, finish: bool) -> bool {
        let Some(off) = self.plan_byte_at(col, row) else {
            if finish {
                self.plan_view.drag = None;
            }
            return false;
        };
        match &mut self.plan_view.drag {
            Some(d) => d.current = off,
            None => {
                self.plan_view.drag = Some(crate::app::state::PlanDrag {
                    anchor: off,
                    current: off,
                });
            }
        }
        if finish {
            let (a, b) = self.plan_view.drag_range().unwrap_or((0, 0));
            self.plan_finish_selection(a, b)
        } else {
            true
        }
    }
}
