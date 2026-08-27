//! Event-specific transcript and card update helpers.

use super::*;

impl App {
    pub(super) fn append_assistant(&mut self, t: &str) {
        self.close_thought();
        if let Some(Block::Assistant { text, streaming }) = self.blocks.last_mut() {
            if *streaming {
                text.push_str(t);
                return;
            }
        }
        self.blocks.push(Block::Assistant {
            text: t.to_string(),
            streaming: true,
        });
    }

    pub(super) fn append_reasoning(&mut self, t: &str) {
        if let Some(Block::Reasoning(th)) = self.blocks.last_mut() {
            if th.elapsed_ms.is_none() {
                th.text.push_str(t);
                return;
            }
        }
        let mut th = Thought::new();
        th.open = self.ui.thoughts_always_open;
        th.text.push_str(t);
        self.blocks.push(Block::Reasoning(th));
    }

    /// Stamp the duration on the most recent still-open thought, if any. A
    /// thought that never got any text is dropped instead — an empty
    /// "Thought for 0.0s" between two tool cards is just a gap.
    pub(super) fn close_thought(&mut self) {
        for (i, block) in self.blocks.iter_mut().enumerate().rev() {
            if let Block::Reasoning(th) = block {
                if th.elapsed_ms.is_none() {
                    if th.text.trim().is_empty() {
                        self.blocks.remove(i);
                    } else {
                        th.elapsed_ms = Some(th.started.elapsed().as_millis());
                    }
                }
                return;
            }
        }
    }

    /// What the agent is visibly busy with, for the live status line. A running
    /// terminal is the honest answer while one is open — "Thinking" is not.
    pub fn activity_label(&self) -> &'static str {
        let mut terminal_running = false;
        for block in &self.blocks {
            if let Block::Terminal(card) = block {
                if card.awaiting_user().is_some() {
                    return "Waiting for you";
                }
                terminal_running |=
                    matches!(card.process, hive_core::TerminalProcessState::Running);
            }
        }
        if terminal_running {
            return "Working in terminal";
        }
        let tool_running = self.blocks.iter().any(|b| match b {
            Block::Tool(c) => c.status == ToolStatus::Running,
            Block::Subagent(c) => c.status == SubagentStatus::Running,
            _ => false,
        });
        if tool_running {
            "Working"
        } else {
            "Thinking"
        }
    }

    /// ctrl+t: if any thought is open, close them all; otherwise open them all.
    pub fn toggle_thoughts(&mut self) {
        let any_open = self.blocks.iter().any(|b| match b {
            Block::Reasoning(th) => th.open,
            _ => false,
        });
        for block in self.blocks.iter_mut() {
            if let Block::Reasoning(th) = block {
                th.open = !any_open;
            }
        }
        self.invalidate_transcript();
        self.flash(if any_open {
            "Thoughts hidden"
        } else {
            "Thoughts shown"
        });
    }

    /// Activate an expandable header by index — thoughts toggle; subagents /
    /// plan cards open their dedicated views.
    pub fn activate_expandable_at(&mut self, block_idx: usize) {
        match self.blocks.get(block_idx) {
            Some(Block::Reasoning(_)) => {
                if let Some(Block::Reasoning(th)) = self.blocks.get_mut(block_idx) {
                    th.open = !th.open;
                    self.invalidate_transcript();
                }
            }
            Some(Block::Subagent(card)) => {
                let id = card.id.clone();
                self.open_subagent_view(id);
            }
            Some(Block::Plan(_)) => {
                self.open_plan_view();
            }
            Some(Block::Terminal(card)) => {
                let id = card.id.clone();
                self.open_terminal_view(id);
            }
            Some(Block::User(_)) => {
                self.open_prompt_menu(block_idx);
            }
            Some(Block::Tool(_)) => {
                self.open_tool_menu(block_idx);
            }
            _ => {}
        }
    }

    /// Index of the most recent plan card in the transcript.
    pub(super) fn last_plan_idx(&self) -> Option<usize> {
        self.blocks
            .iter()
            .rposition(|b| matches!(b, Block::Plan(_)))
    }

    pub(super) fn mark_plan_writing(&mut self) {
        // Rewriting a finished plan moves the single card to the bottom of the
        // transcript (with the UPDATED badge) so the change is visible next to
        // the latest messages — one Plan.md card total, never a stack of copies.
        // A still-empty / in-progress card is reused in place.
        if let Some(i) = self.last_plan_idx() {
            {
                let Block::Plan(card) = &mut self.blocks[i] else {
                    return;
                };
                let rewrite = card.status == PlanStatus::Ready && !card.body.is_empty();
                card.status = PlanStatus::Writing;
                if !rewrite {
                    return;
                }
                card.revised = true;
            }
            self.clear_assistant_selection();
            let card = self.blocks.remove(i);
            self.blocks.push(card);
            self.scroll_from_bottom = 0;
            return;
        }
        self.blocks.push(Block::Plan(PlanCard {
            summary: String::new(),
            body: String::new(),
            status: PlanStatus::Writing,
            revised: false,
        }));
    }

    pub(super) fn upsert_plan(&mut self, summary: String, body: String) {
        let summary = summary.trim().to_string();
        let revised;
        match self.last_plan_idx() {
            None => {
                self.blocks.push(Block::Plan(PlanCard {
                    summary,
                    body,
                    status: PlanStatus::Ready,
                    revised: false,
                }));
                revised = false;
            }
            Some(i) => {
                // Rewrite arriving without a "writing…" placeholder (e.g. a
                // direct event) — same rule as mark_plan_writing: move the one
                // card to the bottom and badge it.
                let rewrite = {
                    let Block::Plan(card) = &self.blocks[i] else {
                        return;
                    };
                    card.status == PlanStatus::Ready && !card.body.is_empty()
                };
                {
                    let Block::Plan(card) = &mut self.blocks[i] else {
                        return;
                    };
                    card.summary = summary;
                    card.body = body;
                    card.status = PlanStatus::Ready;
                    card.revised = card.revised || rewrite;
                    revised = card.revised;
                }
                if rewrite {
                    self.clear_assistant_selection();
                    let card = self.blocks.remove(i);
                    self.blocks.push(card);
                    self.scroll_from_bottom = 0;
                }
            }
        }
        // Body rewrite invalidates highlight ranges — drop them.
        if matches!(self.view, ChatView::Plan) {
            self.plan_clear_corrections();
            // Show the revised plan from the top, like a fresh open, so
            // the reader isn't stranded mid-scroll over new content.
            self.scroll_from_bottom = usize::MAX;
            if revised {
                self.flash("plan revised");
            }
        }
    }

    /// The expandable card (if any) drawn at this screen cell in the last frame.
    ///
    /// The column matters as much as the row: cards are only as wide as the chat
    /// column, so the page margins beside it — and the sidebar — are not the
    /// card, even on the card's own rows.
    pub fn expandable_at(&self, col: u16, row: u16) -> Option<usize> {
        if !self
            .transcript_hit
            .is_some_and(|band| band.contains(col, row))
        {
            return None;
        }
        self.click_hits
            .iter()
            .find(|(r, _)| *r == row)
            .map(|(_, idx)| *idx)
    }

    pub(super) fn finalize_assistant(&mut self, final_text: String) {
        self.close_thought();
        for block in self.blocks.iter_mut().rev() {
            if let Block::Assistant { text, streaming } = block {
                if *streaming {
                    *text = final_text;
                    *streaming = false;
                    return;
                }
            }
        }
        self.blocks.push(Block::Assistant {
            text: final_text,
            streaming: false,
        });
    }

    pub(super) fn finalize_streaming(&mut self) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Assistant { streaming, .. } = block {
                if *streaming {
                    *streaming = false;
                    return;
                }
            }
        }
    }

    pub(super) fn append_tool_output(&mut self, id: &str, chunk: &str) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool(card) = block {
                if card.id == id {
                    card.output.push_str(chunk);
                    const CAP: usize = 8000;
                    if card.output.len() > CAP {
                        let mut start = card.output.len() - CAP;
                        while !card.output.is_char_boundary(start) {
                            start += 1;
                        }
                        card.output = card.output.split_off(start);
                    }
                    return;
                }
            }
        }
    }

    pub(super) fn finish_tool(&mut self, id: &str, ok: bool) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool(card) = block {
                if card.id == id {
                    card.status = if ok { ToolStatus::Ok } else { ToolStatus::Err };
                    if card.elapsed_ms.is_none() {
                        card.elapsed_ms = Some(card.started.elapsed().as_millis());
                    }
                    return;
                }
            }
        }
    }

    pub(super) fn collapse_completed_tool_details(&mut self) {
        for block in &mut self.blocks {
            match block {
                Block::Tool(card) if card.status == ToolStatus::Ok => {
                    card.details_open = false;
                }
                _ => {}
            }
        }
    }

    pub(super) fn attach_snapshot(&mut self, id: &str, snapshot: FileSnapshot) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool(card) = block {
                if card.id == id {
                    card.snapshot = Some(snapshot);
                    return;
                }
            }
        }
    }

    pub(super) fn update_subagent(&mut self, id: &str, status: SubagentStatus, detail: String) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Subagent(card) = block {
                if card.id == id {
                    card.status = status;
                    if !detail.is_empty() {
                        card.detail = detail;
                    }
                    if status != SubagentStatus::Running && card.elapsed_ms.is_none() {
                        card.elapsed_ms = Some(card.started.elapsed().as_millis());
                    }
                    return;
                }
            }
        }
    }

    pub(super) fn update_subagent_usage(&mut self, id: &str, usage: Usage) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Subagent(card) = block {
                if card.id == id {
                    card.usage = usage;
                    return;
                }
            }
        }
    }

    /// Subagent cards for the sidebar (running first, then recent finished).
    pub fn sidebar_subagents(&self) -> Vec<&SubagentCard> {
        let mut cards: Vec<&SubagentCard> = self
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Subagent(c) => Some(c),
                _ => None,
            })
            .collect();
        cards.sort_by_key(|c| match c.status {
            SubagentStatus::Running => 0u8,
            SubagentStatus::Failed => 1,
            SubagentStatus::Done => 2,
        });
        cards
    }

    /// Terminal cards for the sidebar (running first, then failed, then exited).
    pub fn sidebar_terminals(&self) -> Vec<&TerminalCard> {
        let mut cards: Vec<&TerminalCard> = self
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Terminal(c) => Some(c.as_ref()),
                _ => None,
            })
            .collect();
        cards.sort_by_key(|c| match &c.process {
            TerminalProcessState::Running => 0u8,
            TerminalProcessState::Failed { .. } => 1,
            TerminalProcessState::Exited { code } if *code != 0 => 2,
            TerminalProcessState::Exited { .. } => 3,
        });
        cards
    }

    pub(super) fn append_subagent_line(&mut self, id: &str, line: SubagentLine) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Subagent(card) = block {
                if card.id != id {
                    continue;
                }
                match line {
                    SubagentLine::Thinking(t) => {
                        if let Some(SubagentLine::Thinking(prev)) = card.lines.last_mut() {
                            prev.push_str(&t);
                        } else {
                            card.lines.push(SubagentLine::Thinking(t));
                        }
                    }
                    SubagentLine::Tool {
                        name,
                        args,
                        summary,
                        ok: Some(ok),
                    } => {
                        let open = card.lines.iter_mut().rev().find(|l| {
                            matches!(
                                l,
                                SubagentLine::Tool { ok: None, name: n, .. } if *n == name
                            )
                        });
                        if let Some(SubagentLine::Tool {
                            args: a,
                            summary: s,
                            ok: running,
                            ..
                        }) = open
                        {
                            if a.is_empty() && !args.is_empty() {
                                *a = args;
                            }
                            if !summary.is_empty() {
                                *s = summary;
                            }
                            *running = Some(ok);
                        } else {
                            card.lines.push(SubagentLine::Tool {
                                name,
                                args,
                                summary,
                                ok: Some(ok),
                            });
                        }
                    }
                    other => card.lines.push(other),
                }
                return;
            }
        }
    }
}
