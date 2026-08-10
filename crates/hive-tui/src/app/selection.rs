//! Composer focus and assistant transcript selection methods.

use super::*;

impl App {
    pub fn focus_input(&mut self) {
        self.input_focused = true;
        self.input_last_activity = Some(std::time::Instant::now());
    }

    pub fn blur_input(&mut self) {
        self.input_focused = false;
        self.input_last_activity = None;
    }

    /// Record that the user touched the composer (typing, caret, slash menu).
    pub fn note_input_activity(&mut self) {
        if self.input_focused {
            self.input_last_activity = Some(std::time::Instant::now());
        }
    }

    /// Blur the composer when focused but idle past [`INPUT_IDLE_BLUR_MS`].
    /// Returns true when focus was dropped (caller should redraw).
    pub fn maybe_idle_blur_input(&mut self) -> bool {
        if !self.input_focused {
            return false;
        }
        let Some(at) = self.input_last_activity else {
            return false;
        };
        if at.elapsed().as_millis() < INPUT_IDLE_BLUR_MS {
            return false;
        }
        self.blur_input();
        true
    }

    /// True when `(col, row)` lands inside the input strip from the last draw.
    pub fn input_contains(&self, col: u16, row: u16) -> bool {
        self.input_hit
            .map(|r| r.contains(col, row))
            .unwrap_or(false)
    }

    /// Begin a selection only when the pointer is on visible assistant text.
    pub fn start_assistant_selection(&mut self, col: u16, row: u16) -> bool {
        let Some((block, point)) = self.assistant_point_at(col, row, None, false) else {
            return false;
        };
        self.assistant_selection = Some(AssistantSelection {
            block,
            anchor: point,
            current: point,
            dragged: false,
            active: true,
        });
        true
    }

    /// Update the active selection, clamping it to the response where it began.
    pub fn update_assistant_selection(&mut self, col: u16, row: u16) -> bool {
        let Some((block, active)) = self
            .assistant_selection
            .as_ref()
            .map(|selection| (selection.block, selection.active))
        else {
            return false;
        };
        if !active {
            return false;
        }
        let Some((_, point)) = self.assistant_point_at(col, row, Some(block), true) else {
            return false;
        };
        let selection = self.assistant_selection.as_mut().expect("checked above");
        let changed = selection.current != point;
        selection.current = point;
        selection.dragged |= selection.current != selection.anchor;
        changed
    }

    /// Finish a drag, consume its highlight, and return clean rendered text.
    /// A plain click is discarded.
    pub fn finish_assistant_selection(&mut self, col: u16, row: u16) -> Option<String> {
        if !self.assistant_selection_active() {
            return None;
        }
        let _ = self.update_assistant_selection(col, row);
        let selection = self.assistant_selection.clone()?;
        if !selection.dragged {
            self.assistant_selection = None;
            return None;
        }
        let text = self.assistant_selection_text(&selection);
        self.assistant_selection = None;
        text
    }

    pub fn assistant_selection_active(&self) -> bool {
        self.assistant_selection
            .as_ref()
            .is_some_and(|selection| selection.active)
    }

    /// Clear the current selection; returns whether a redraw is needed.
    pub fn clear_assistant_selection(&mut self) -> bool {
        self.assistant_selection.take().is_some()
    }

    pub(super) fn assistant_row_data(
        &self,
        block: usize,
        row: usize,
    ) -> Option<(&str, AssistantRowJoin)> {
        self.assistant_rows
            .iter()
            .find(|candidate| candidate.block == block && candidate.response_row == row)
            .map(|candidate| (candidate.text.as_str(), candidate.join_before))
            .or_else(|| {
                self.assistant_row_hits
                    .iter()
                    .find(|candidate| candidate.block == block && candidate.response_row == row)
                    .map(|candidate| (candidate.text.as_str(), candidate.join_before))
            })
    }

    /// Selected half-open display-column range for one rendered response row.
    pub fn assistant_selected_cols(&self, block: usize, row: usize) -> Option<(usize, usize)> {
        let selection = self.assistant_selection.as_ref()?;
        if selection.block != block {
            return None;
        }
        let (first, last) = normalized_points(selection.anchor, selection.current);
        if row < first.row || row > last.row {
            return None;
        }
        let (text, _) = self.assistant_row_data(block, row)?;
        assistant_row_selected_cols(first, last, row, text)
    }

    pub(super) fn assistant_point_from_rendered_rows(
        &self,
        col: u16,
        row: u16,
        block: usize,
    ) -> Option<AssistantPoint> {
        let viewport = self.transcript_hit?;
        let viewport_row = usize::from(row.saturating_sub(viewport.y))
            .min(usize::from(viewport.height.saturating_sub(1)));
        let line_idx = self
            .transcript_max_scroll
            .saturating_sub(self.scroll_from_bottom)
            .saturating_add(viewport_row);
        let first = self
            .assistant_rows
            .iter()
            .filter(|candidate| candidate.block == block)
            .min_by_key(|candidate| candidate.line_idx)?;
        let last = self
            .assistant_rows
            .iter()
            .filter(|candidate| candidate.block == block)
            .max_by_key(|candidate| candidate.line_idx)?;
        let candidate = if line_idx <= first.line_idx {
            first
        } else if line_idx >= last.line_idx {
            last
        } else {
            self.assistant_rows
                .iter()
                .filter(|candidate| candidate.block == block)
                .min_by_key(|candidate| candidate.line_idx.abs_diff(line_idx))?
        };
        let width = display_width(&candidate.text);
        let relative = if width == 0 || line_idx < candidate.line_idx {
            0
        } else if line_idx > candidate.line_idx {
            width - 1
        } else {
            usize::from(col.saturating_sub(viewport.x.saturating_add(2))).min(width - 1)
        };
        Some(AssistantPoint {
            row: candidate.response_row,
            col: display_cell_start(&candidate.text, relative),
        })
    }

    pub(super) fn assistant_point_at(
        &self,
        col: u16,
        row: u16,
        block: Option<usize>,
        clamp: bool,
    ) -> Option<(usize, AssistantPoint)> {
        let exact = self.assistant_row_hits.iter().find(|hit| {
            if block.is_some_and(|wanted| wanted != hit.block) || hit.screen_row != row {
                return false;
            }
            let width = display_width(&hit.text) as u16;
            width > 0 && col >= hit.x && col < hit.x.saturating_add(width)
        });
        let (hit, nearest) = match exact {
            Some(hit) => (hit, false),
            None if clamp => {
                if let Some(block) = block {
                    if let Some(point) = self.assistant_point_from_rendered_rows(col, row, block) {
                        return Some((block, point));
                    }
                }
                (
                    self.assistant_row_hits
                        .iter()
                        .filter(|hit| block.is_none_or(|wanted| wanted == hit.block))
                        .min_by_key(|hit| hit.screen_row.abs_diff(row))?,
                    true,
                )
            }
            None => return None,
        };
        let width = display_width(&hit.text);
        if width == 0 {
            return None;
        }
        let relative = if nearest && row < hit.screen_row {
            0
        } else if nearest && row > hit.screen_row {
            width - 1
        } else {
            usize::from(col.saturating_sub(hit.x)).min(width - 1)
        };
        Some((
            hit.block,
            AssistantPoint {
                row: hit.response_row,
                col: display_cell_start(&hit.text, relative),
            },
        ))
    }

    pub(super) fn assistant_selection_text(
        &self,
        selection: &AssistantSelection,
    ) -> Option<String> {
        let (first, last) = normalized_points(selection.anchor, selection.current);
        let has_full_rows = self
            .assistant_rows
            .iter()
            .any(|row| row.block == selection.block);
        let mut rows: Vec<(usize, &str, AssistantRowJoin)> = if has_full_rows {
            self.assistant_rows
                .iter()
                .filter(|row| {
                    row.block == selection.block
                        && row.response_row >= first.row
                        && row.response_row <= last.row
                })
                .map(|row| (row.response_row, row.text.as_str(), row.join_before))
                .collect()
        } else {
            self.assistant_row_hits
                .iter()
                .filter(|hit| {
                    hit.block == selection.block
                        && hit.response_row >= first.row
                        && hit.response_row <= last.row
                })
                .map(|hit| (hit.response_row, hit.text.as_str(), hit.join_before))
                .collect()
        };
        rows.sort_by_key(|(response_row, _, _)| *response_row);
        let expected_len = last.row.checked_sub(first.row)?.checked_add(1)?;
        if rows.len() != expected_len
            || rows
                .iter()
                .enumerate()
                .any(|(i, (response_row, _, _))| *response_row != first.row + i)
        {
            return None;
        }

        let mut out = String::new();
        for (i, (response_row, text, join_before)) in rows.into_iter().enumerate() {
            let (start, end) = assistant_row_selected_cols(first, last, response_row, text)?;
            let mut fragment = slice_display_range(text, start, end);
            if matches!(
                join_before,
                AssistantRowJoin::SoftSpace | AssistantRowJoin::SoftNone
            ) {
                fragment = fragment.trim_start_matches([' ', '\t']).to_string();
            }
            if i > 0 {
                match join_before {
                    AssistantRowJoin::Hard => {
                        trim_horizontal_end(&mut out);
                        out.push('\n');
                    }
                    AssistantRowJoin::SoftSpace => {
                        let separated = out.chars().last().is_some_and(char::is_whitespace)
                            || fragment.chars().next().is_some_and(char::is_whitespace);
                        if !separated && !out.is_empty() && !fragment.is_empty() {
                            out.push(' ');
                        }
                    }
                    AssistantRowJoin::SoftNone => {}
                }
            }
            out.push_str(&fragment);
        }
        let clean = clean_selected_text(&out);
        (!clean.is_empty()).then_some(clean)
    }
}
