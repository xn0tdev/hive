//! Composer menus, history, attachments, and follow-up state.

use super::*;

impl App {
    /// The last finished assistant answer, for copying.
    pub fn last_answer(&self) -> Option<&str> {
        self.blocks.iter().rev().find_map(|b| match b {
            Block::Assistant { text, streaming } if !streaming && !text.trim().is_empty() => {
                Some(text.as_str())
            }
            _ => None,
        })
    }

    // --- slash / @file menus ---

    /// The prefix typed after `/`, if the menu should be open: input starts
    /// with `/`, is a single line, and has no space yet.
    pub fn slash_prefix(&self) -> Option<&str> {
        let v = self.input.value.as_str();
        let rest = v.strip_prefix('/')?;
        if rest.contains(' ') || rest.contains('\n') {
            return None;
        }
        Some(rest)
    }

    pub fn slash_items(&self) -> Vec<SlashItem> {
        let Some(prefix) = self.slash_prefix() else {
            return Vec::new();
        };
        let prefix_l = prefix.to_ascii_lowercase();
        let mut items: Vec<SlashItem> = commands::filtered(prefix)
            .into_iter()
            .map(SlashItem::Command)
            .collect();
        for s in &self.skills {
            if commands::is_builtin_name(&s.name) {
                continue;
            }
            let name_l = s.name.to_ascii_lowercase();
            let desc_l = s.description.to_ascii_lowercase();
            if prefix_l.is_empty()
                || name_l.starts_with(&prefix_l)
                || name_l.contains(&prefix_l)
                || desc_l.contains(&prefix_l)
            {
                items.push(SlashItem::Skill(s.clone()));
            }
        }
        items
    }

    /// Active `@path` query at the cursor (disabled while slash menu owns input).
    pub fn at_mention(&self) -> Option<AtQuery> {
        if self.slash_prefix().is_some() {
            return None;
        }
        files::at_query(&self.input.value, self.input.cursor)
    }

    /// Ensure the project file index is loaded (lazy, once per session).
    pub fn ensure_file_index(&mut self) {
        if self.file_index.is_some() {
            return;
        }
        self.file_index = Some(files::index_files(std::path::Path::new(&self.cwd)));
    }

    /// Filtered file paths for the `@` menu (empty when menu closed).
    pub fn file_menu_items(&mut self) -> Vec<String> {
        let Some(q) = self.at_mention() else {
            return Vec::new();
        };
        self.ensure_file_index();
        let index = self.file_index.as_deref().unwrap_or(&[]);
        files::filter_files(index, &q.query)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    pub fn menu_up(&mut self) {
        let n = if !self.slash_items().is_empty() {
            self.slash_items().len()
        } else {
            self.file_menu_items().len()
        };
        if n > 0 {
            self.menu_index = (self.menu_index + n - 1) % n;
        }
    }

    pub fn menu_down(&mut self) {
        let n = if !self.slash_items().is_empty() {
            self.slash_items().len()
        } else {
            self.file_menu_items().len()
        };
        if n > 0 {
            self.menu_index = (self.menu_index + 1) % n;
        }
    }

    pub fn menu_selected(&self) -> Option<SlashItem> {
        let items = self.slash_items();
        if items.is_empty() {
            return None;
        }
        Some(items[self.menu_index.min(items.len() - 1)].clone())
    }

    pub fn find_skill(&self, name: &str) -> Option<&SkillChoice> {
        let name = name.to_ascii_lowercase();
        self.skills
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(&name))
    }

    pub fn file_menu_selected(&mut self) -> Option<String> {
        let items = self.file_menu_items();
        if items.is_empty() {
            return None;
        }
        Some(items[self.menu_index.min(items.len() - 1)].clone())
    }

    /// Turn `@query` into a pending `@file` chip (removes the typed mention).
    pub fn complete_at_file(&mut self, rel: &str) {
        let Some(q) = self.at_mention() else {
            return;
        };
        let end = self.input.cursor;
        self.input.replace_chars(q.at, end, "");
        // Collapse a double space left when `@…` sat mid-sentence.
        let v = self.input.value.clone();
        if let Some(i) = v.find("  ") {
            let chars: Vec<char> = v.chars().collect();
            if i + 1 < chars.len() {
                self.input.replace_chars(i, i + 2, " ");
            }
        }
        match self.attach_rel(rel) {
            Ok(_) => self.reset_menu(),
            Err(e) => {
                self.flash_error(e);
                self.reset_menu();
            }
        }
    }

    pub fn reset_menu(&mut self) {
        self.menu_index = 0;
    }

    /// How many rows the composer menu currently offers.
    pub(super) fn menu_len(&mut self) -> usize {
        let slash = self.slash_items().len();
        if slash > 0 {
            slash
        } else {
            self.file_menu_items().len()
        }
    }

    pub fn menu_contains(&self, col: u16, row: u16) -> bool {
        self.menu_hit
            .is_some_and(|(rect, _)| rect.contains(col, row))
    }

    /// Which menu item sits under the pointer, if any. The `↓ N more` footer
    /// row is inside the panel but picks nothing, so it answers `None`.
    pub fn menu_item_at(&mut self, col: u16, row: u16) -> Option<usize> {
        let (rect, window) = self.menu_hit?;
        if !rect.contains(col, row) {
            return None;
        }
        let idx = window + usize::from(row - rect.y);
        (idx < self.menu_len() && idx < window + crate::app::files::MAX_MENU_ROWS).then_some(idx)
    }

    /// Point the menu at `idx`. Returns whether anything moved.
    pub fn set_menu_index(&mut self, idx: usize) -> bool {
        if idx >= self.menu_len() || self.menu_index == idx {
            return false;
        }
        self.menu_index = idx;
        true
    }

    pub fn request_quit(&mut self) {
        self.quit_requested = true;
    }

    pub fn take_quit_request(&mut self) -> bool {
        std::mem::take(&mut self.quit_requested)
    }

    pub fn scroll_up(&mut self, n: usize) {
        let next = self.scroll_from_bottom.saturating_add(n);
        self.scroll_from_bottom = next.min(self.transcript_max_scroll);
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(n);
    }

    /// Snap the transcript back to the newest content.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_from_bottom = 0;
    }

    /// True when Up can move the transcript toward older content.
    pub fn can_scroll_up(&self) -> bool {
        self.scroll_from_bottom < self.transcript_max_scroll
    }

    /// True when Down can move the transcript back toward the bottom.
    pub fn can_scroll_down(&self) -> bool {
        self.scroll_from_bottom > 0
    }

    /// Remember how far the transcript can scroll (from the last paint) and
    /// pin `scroll_from_bottom` so phantom Ups on a short view don't stick.
    pub fn set_transcript_max_scroll(&mut self, max: usize) {
        self.transcript_max_scroll = max;
        if self.scroll_from_bottom > max {
            self.scroll_from_bottom = max;
        }
    }

    pub fn push_user(&mut self, text: String) {
        self.blocks.push(Block::User(text));
        self.scroll_from_bottom = 0;
        self.invalidate_transcript();
    }

    pub fn notice(&mut self, text: impl Into<String>) {
        self.blocks.push(Block::Notice(text.into()));
        self.invalidate_transcript();
    }

    pub(super) fn clear_todos(&mut self) {
        self.todos.clear();
        self.todos_completed_at = None;
        self.blocks
            .retain(|block| !matches!(block, Block::Todos(_)));
        self.invalidate_transcript();
    }

    /// Start a fresh chat — back to the centered Home/landing screen.
    pub fn new_chat(&mut self) {
        self.blocks.clear();
        self.blocks.push(Block::Welcome);
        self.usage = Usage::default();
        self.context_tokens = 0;
        self.scroll_from_bottom = 0;
        self.pending_attaches.clear();
        self.attach_selected = None;
        self.pasted_blocks.clear();
        self.pasted_selected = None;
        self.pasted_view = None;
        self.next_pasted_id = 1;
        self.follow_up = None;
        self.prompt_history.reset();
        self.pending_dispatch = None;
        self.input.clear();
        self.reset_menu();
        self.close_palette();
        self.close_about();
        self.close_context_menu();
        self.close_recap();
        self.clear_todos();
        self.running = false;
        self.click_hits.clear();
        self.assistant_row_hits.clear();
        self.assistant_rows.clear();
        self.assistant_selection = None;
        self.assistant_selection_width = 0;
        self.hover_block = None;
        self.hover_sidebar_item = None;
        self.hover_back = false;
        self.hover_make = false;
        self.hover_terminal_stop = false;
        self.view = ChatView::Main;
        self.back_hit = None;
        self.make_hit = None;
        self.terminal_stop_hit = None;
        self.plan_view = PlanViewState::default();
        self.terminal_view = TerminalViewState::default();
        self.pending_terminal_resize = None;
        self.pending_context_update = None;
        self.md_cache = MdCache::default();
        self.transcript_cache = TranscriptCache::default();
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
    }

    pub(crate) fn invalidate_transcript(&mut self) {
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
        self.transcript_cache.valid = false;
    }

    /// True when the composer can take an attachment: plain chat, no overlay,
    /// no pty, no read-only view.
    pub fn accepts_attachments(&self) -> bool {
        !self.in_terminal_view()
            && !self.in_special_view()
            && !self.about_open()
            && !self.settings_open()
            && !self.recap_open()
            && !self.palette_open()
    }

    /// True just after an image paste, while the composer acknowledges it.
    pub fn just_pasted_image(&self) -> bool {
        self.image_pasted_at
            .is_some_and(|at| at.elapsed().as_millis() < TOAST_MS)
    }

    /// Ask for a full repaint — the screen has something on it we didn't draw.
    pub fn request_repaint(&mut self) {
        self.repaint_requested = true;
    }

    pub fn take_repaint_request(&mut self) -> bool {
        std::mem::take(&mut self.repaint_requested)
    }

    pub fn has_pending_attaches(&self) -> bool {
        !self.pending_attaches.is_empty()
    }

    pub fn has_follow_up(&self) -> bool {
        self.follow_up.is_some()
    }

    /// Queue (or replace) a follow-up while the agent is busy.
    pub fn queue_follow_up(&mut self, fu: QueuedFollowUp) {
        self.follow_up = Some(fu);
        self.flash("Follow-up queued · sent when task finishes");
    }

    /// Pull the queued follow-up into the composer for editing (↑).
    pub fn recall_follow_up(&mut self) -> bool {
        let Some(fu) = self.follow_up.take() else {
            return false;
        };
        self.pending_attaches = fu.attaches;
        self.pasted_blocks = fu.pasted;
        self.agent_mode = fu.mode;
        self.input.value = fu.composer;
        self.input.end();
        self.reset_menu();
        self.flash("Edit follow-up · Enter to re-queue");
        true
    }

    /// Clear a queued follow-up without sending.
    pub fn clear_follow_up(&mut self) -> bool {
        if self.follow_up.take().is_some() {
            self.flash("Follow-up cleared");
            true
        } else {
            false
        }
    }

    // -- Prompt history (shell-style up/down recall) --

    /// Step back through prompt history. Saves the current composer text as a
    /// draft on first entry. Returns `false` if there is no history.
    pub fn history_up(&mut self) -> bool {
        if self.prompt_history.entries.is_empty() {
            return false;
        }
        match self.prompt_history.index {
            None => {
                self.prompt_history.draft = self.input.value.clone();
                let i = self.prompt_history.entries.len() - 1;
                self.prompt_history.index = Some(i);
                self.input.value = self.prompt_history.entries[i].clone();
                self.input.end();
                true
            }
            Some(0) => true,
            Some(i) => {
                let i = i - 1;
                self.prompt_history.index = Some(i);
                self.input.value = self.prompt_history.entries[i].clone();
                self.input.end();
                true
            }
        }
    }

    /// Step forward through prompt history. Past the last entry, restores the
    /// draft that was saved on entry. Returns `false` when not browsing.
    pub fn history_down(&mut self) -> bool {
        let Some(i) = self.prompt_history.index else {
            return false;
        };
        if i + 1 >= self.prompt_history.entries.len() {
            self.prompt_history.index = None;
            self.input.value = self.prompt_history.draft.clone();
            self.input.end();
            self.prompt_history.draft.clear();
        } else {
            let next = i + 1;
            self.prompt_history.index = Some(next);
            self.input.value = self.prompt_history.entries[next].clone();
            self.input.end();
        }
        true
    }

    // -- Pending dispatch (ESC recall before the agent starts) --

    /// True when the grace period has elapsed and the deferred message should
    /// be flushed to the driver.
    pub fn pending_dispatch_ready(&self) -> bool {
        match &self.pending_dispatch {
            Some(pd) => pd.submitted_at.elapsed().as_millis() >= DISPATCH_GRACE_MS,
            None => false,
        }
    }

    /// Take the pending dispatch out for flushing to the driver.
    pub fn take_pending_dispatch(&mut self) -> Option<PendingDispatch> {
        self.pending_dispatch.take()
    }

    /// Recall a deferred message back into the composer (ESC before the agent
    /// starts). Removes the last `Block::User` from the transcript, restores
    /// the composer text and attachments, and flashes a hint.
    pub fn recall_pending_dispatch(&mut self) -> bool {
        let Some(pd) = self.pending_dispatch.take() else {
            return false;
        };
        if matches!(self.blocks.last(), Some(Block::User(_))) {
            self.blocks.pop();
        }
        self.input.value = pd.composer;
        self.input.end();
        self.pending_attaches = pd.attaches;
        self.pasted_blocks = pd.pasted;
        self.agent_mode = pd.mode;
        self.reset_menu();
        self.scroll_from_bottom = 0;
        self.invalidate_transcript();
        self.flash("Prompt recalled — edit and resend");
        true
    }

    /// One-line preview for the banner above the input.
    pub fn follow_up_preview(&self, max_chars: usize) -> Option<String> {
        let fu = self.follow_up.as_ref()?;
        let t = fu.display.replace('\n', " ");
        let t = t.trim();
        if t.chars().count() <= max_chars {
            return Some(t.to_string());
        }
        let mut s: String = t.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        Some(s)
    }

    pub fn attachment_tags_line(&self) -> String {
        self.pending_attaches
            .iter()
            .map(PendingAttach::tag)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Attach a project-relative path from the `@` picker.
    pub fn attach_rel(&mut self, rel: &str) -> Result<String, String> {
        let rel = rel.trim().trim_start_matches("./");
        if rel.is_empty() {
            return Err("empty path".into());
        }
        let abs = std::path::Path::new(&self.cwd).join(rel);
        self.attach_path_inner(&abs, rel)
    }

    /// Attach a path: images become vision sources; other files are path notes.
    pub fn attach_path(&mut self, path: &str) -> Result<String, String> {
        let path = path.trim();
        if path.is_empty() {
            return Err("paste a file path to attach".into());
        }
        let abs = std::path::Path::new(path);
        let abs = if abs.is_absolute() {
            abs.to_path_buf()
        } else {
            std::path::Path::new(&self.cwd).join(abs)
        };
        let label = abs
            .strip_prefix(&self.cwd)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| {
                abs.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| abs.to_string_lossy().into_owned())
            });
        self.attach_path_inner(&abs, &label)
    }

    /// Attach an image pasted from the clipboard. There's no file behind it,
    /// so the label doubles as its identity in the pending list.
    pub fn attach_clipboard_image(&mut self, png: Vec<u8>) -> Result<String, String> {
        /// Roughly what vision endpoints accept once base64 inflates it.
        const MAX_BYTES: usize = 10 * 1024 * 1024;

        if png.is_empty() {
            return Err("clipboard image is empty".into());
        }
        if png.len() > MAX_BYTES {
            return Err(format!(
                "image is {:.1} MB — too large to send",
                png.len() as f64 / (1024.0 * 1024.0)
            ));
        }

        let label = self.next_clipboard_label();
        let tag = format!("@{label}");
        self.attach_selected = None;
        self.image_pasted_at = Some(std::time::Instant::now());
        self.pending_attaches.push(PendingAttach {
            path: label.clone(),
            label,
            image: Some(ImageSource::Base64 {
                media_type: "image/png".into(),
                data: base64_encode(&png),
            }),
            content: None,
        });
        Ok(tag)
    }

    /// Pasting twice must queue two images, not silently drop the second.
    pub(super) fn next_clipboard_label(&self) -> String {
        let taken = |name: &str| self.pending_attaches.iter().any(|a| a.path == name);
        if !taken("clipboard.png") {
            return "clipboard.png".into();
        }
        (2..)
            .map(|n| format!("clipboard-{n}.png"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| "clipboard.png".into())
    }

    pub(super) fn attach_path_inner(
        &mut self,
        abs: &std::path::Path,
        label: &str,
    ) -> Result<String, String> {
        let bytes =
            std::fs::read(abs).map_err(|e| format!("cannot read {}: {e}", abs.display()))?;
        let is_image = is_image_path(abs.to_string_lossy().as_ref());
        let image = if is_image {
            Some(ImageSource::Base64 {
                media_type: media_type_for(abs.to_string_lossy().as_ref()),
                data: base64_encode(&bytes),
            })
        } else {
            None
        };
        let content = if !is_image {
            String::from_utf8(bytes).ok()
        } else {
            None
        };
        let label = label.trim().trim_start_matches("./").to_string();
        let path = label.clone();
        let tag = format!("@{label}");
        // Skip duplicates (same path already queued).
        if self.pending_attaches.iter().any(|a| a.path == path) {
            return Ok(tag);
        }
        self.attach_selected = None;
        self.pending_attaches.push(PendingAttach {
            label,
            path,
            image,
            content,
        });
        Ok(tag)
    }

    /// Attach existing file path(s) parsed from pasted / drag-and-dropped `text`
    /// as `@chips`, exactly like the `@` picker. Handles the quirks terminals
    /// apply on drop: surrounding quotes, backslash-escaped spaces, and
    /// (percent-encoded) `file://` URIs — including a multi-file drop. Returns
    /// `Some("")` when at least one file was attached, or `None` when the text
    /// isn't purely path(s) to existing file(s) (so the caller keeps it as text).
    pub fn try_attach_pasted_path(&mut self, text: &str) -> Option<String> {
        let candidates = drop_path_candidates(text)?;
        let mut attached = false;
        for cand in candidates {
            if std::path::Path::new(&cand).is_file() && self.attach_path(&cand).is_ok() {
                attached = true;
            }
        }
        attached.then_some(String::new())
    }

    pub fn take_pending_attaches(&mut self) -> Vec<PendingAttach> {
        self.attach_selected = None;
        std::mem::take(&mut self.pending_attaches)
    }

    // ── Attachment chips: ↓ out of the composer, ⌫ to drop one ───────────

    /// Chip the keyboard is on, if it stepped out of the composer.
    pub fn selected_attach(&self) -> Option<usize> {
        self.attach_selected
            .filter(|i| *i < self.pending_attaches.len())
    }

    /// Step from the composer onto the first chip. False when there are none,
    /// so the caller can fall back to scrolling.
    pub fn select_first_attach(&mut self) -> bool {
        if self.pending_attaches.is_empty() {
            return false;
        }
        self.attach_selected = Some(0);
        true
    }

    /// Step onto the last chip — ↑ from the pasted chips walks back onto the
    /// @chips. False when there are none.
    pub fn select_last_attach(&mut self) -> bool {
        if self.pending_attaches.is_empty() {
            return false;
        }
        self.attach_selected = Some(self.pending_attaches.len() - 1);
        true
    }

    /// Walk the chips, wrapping at both ends.
    pub fn move_attach_selection(&mut self, delta: isize) -> bool {
        let Some(current) = self.selected_attach() else {
            return false;
        };
        let n = self.pending_attaches.len() as isize;
        self.attach_selected = Some((current as isize + delta).rem_euclid(n) as usize);
        true
    }

    /// Drop the highlighted chip. The selection stays put so repeated
    /// backspaces keep clearing, and returns to the composer when empty.
    pub fn remove_selected_attach(&mut self) -> Option<String> {
        let idx = self.selected_attach()?;
        let removed = self.pending_attaches.remove(idx);
        self.attach_selected = if self.pending_attaches.is_empty() {
            None
        } else {
            Some(idx.min(self.pending_attaches.len() - 1))
        };
        Some(removed.label)
    }

    /// Hand the keyboard back to the composer.
    pub fn clear_attach_selection(&mut self) -> bool {
        self.attach_selected.take().is_some()
    }
}
