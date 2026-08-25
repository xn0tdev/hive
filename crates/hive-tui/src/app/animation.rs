//! Animation, toasts, scrolling, and transient UI state.

use super::*;

impl App {
    pub fn spinner_char(&self) -> &'static str {
        spinner::frame(self.spinner)
    }

    /// Advance clocks and retire one-shot visual state. Returns true when a
    /// final repaint is needed (for example, to erase an expired toast).
    pub fn tick(&mut self) -> bool {
        self.spinner = (self.anim_start.elapsed().as_millis() / 100) as usize;
        let bonk_expired = self.logo_bonk.as_ref().is_some_and(|b| !b.alive());
        if bonk_expired {
            self.logo_bonk = None;
        }
        let flash_expired = self
            .flash_msg
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed().as_millis() >= FLASH_MS.max(TOAST_MS));
        if flash_expired {
            self.flash_msg = None;
        }
        let error_expired = self
            .error_flash
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed().as_millis() >= FLASH_MS.max(TOAST_MS));
        if error_expired {
            self.error_flash = None;
        }
        let paste_expired = self
            .image_pasted_at
            .is_some_and(|at| at.elapsed().as_millis() >= TOAST_MS);
        if paste_expired {
            self.image_pasted_at = None;
        }
        let tasks_retired = self
            .todos_completed_at
            .is_some_and(|at| at.elapsed().as_millis() >= TASKS_DONE_VISIBLE_MS);
        if tasks_retired {
            self.clear_todos();
        }
        let input_blurred = self.maybe_idle_blur_input();
        bonk_expired
            || flash_expired
            || error_expired
            || paste_expired
            || tasks_retired
            || input_blurred
    }

    /// Cadence required by genuinely moving visuals. Deadline-only state such
    /// as toasts is deliberately excluded so it does not create a 10 FPS loop.
    pub fn animation_interval(&self) -> Option<std::time::Duration> {
        let fast = self.logo_bonk.is_some()
            || self.running
            || self.recap_overlay.as_ref().is_some_and(|o| {
                self.recap_card(o.recap_id)
                    .is_some_and(|c| c.recap.is_generating() && !o.reveal_stream)
            })
            || self.blocks.iter().any(|b| match b {
                Block::Tool(c) => c.status == ToolStatus::Running,
                Block::Subagent(c) => c.status == SubagentStatus::Running,
                Block::Plan(c) => c.status == PlanStatus::Writing,
                Block::Terminal(c) => {
                    !self.in_terminal_view()
                        && matches!(c.process, hive_core::TerminalProcessState::Running)
                }
                Block::Reasoning(th) => th.elapsed_ms.is_none(),
                Block::Assistant {
                    streaming: true, ..
                } => true,
                Block::Compacted(c) => c.before.is_none(),
                _ => false,
            });
        if fast {
            Some(std::time::Duration::from_millis(100))
        } else if self.goal.as_ref().is_some_and(|g| !g.paused) {
            Some(std::time::Duration::from_secs(1))
        } else {
            None
        }
    }

    pub fn animation_frame(&self, interval: std::time::Duration) -> u128 {
        let step = interval.as_millis().max(1);
        self.anim_start.elapsed().as_millis() / step
    }

    /// Nearest one-shot state transition that must wake the loop. The idle
    /// input poll may wake sooner, but no redraw happens until the deadline.
    pub fn next_visual_deadline(&self) -> Option<std::time::Duration> {
        fn remaining(at: std::time::Instant, after_ms: u128) -> std::time::Duration {
            let left = after_ms.saturating_sub(at.elapsed().as_millis());
            std::time::Duration::from_millis(left.min(u128::from(u64::MAX)) as u64)
        }
        let mut next: Option<std::time::Duration> = None;
        let mut include = |duration: std::time::Duration| {
            next = Some(next.map_or(duration, |current| current.min(duration)));
        };
        if let Some((_, at)) = &self.flash_msg {
            include(remaining(*at, FLASH_MS.max(TOAST_MS)));
        }
        if let Some((_, at)) = &self.error_flash {
            include(remaining(*at, FLASH_MS.max(TOAST_MS)));
        }
        if let Some(at) = self.image_pasted_at {
            include(remaining(at, TOAST_MS));
        }
        if let Some(at) = self.todos_completed_at {
            include(remaining(at, TASKS_DONE_VISIBLE_MS));
        }
        if let Some(dispatch) = &self.pending_dispatch {
            include(remaining(dispatch.submitted_at, DISPATCH_GRACE_MS));
        }
        if self.input_focused {
            if let Some(at) = self.input_last_activity {
                include(remaining(at, INPUT_IDLE_BLUR_MS));
            }
        }
        next
    }

    /// Compatibility helper used by render tests.
    #[cfg(test)]
    pub fn needs_animation(&self) -> bool {
        self.animation_interval().is_some()
            || self.flash_text().is_some()
            || self.error_toast().is_some()
            || self.just_pasted_image()
            || self.pending_dispatch.is_some()
            || self.todos_completed_at.is_some()
    }

    /// Start a logo bonk ripple at a screen cell (landing only).
    pub fn bonk_logo_at(&mut self, col: u16, row: u16) -> bool {
        let Some(logo) = self.logo_hit else {
            return false;
        };
        let Some((lx, ly)) = crate::render::wordmark::hit_cell(logo, col, row) else {
            return false;
        };
        self.logo_bonk = Some(LogoBonk::fresh(lx, ly));
        crate::sound::play_bonk();
        true
    }

    /// True when nothing has happened yet (only the welcome block) and no turn
    /// is running — the landing screen with a centered input.
    pub fn is_empty_chat(&self) -> bool {
        !self.running && !self.blocks.iter().any(|b| !matches!(b, Block::Welcome))
    }

    // --- ephemeral footer messages & double-ctrl+c ---

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.flash_msg = Some((msg.into(), std::time::Instant::now()));
    }

    pub fn flash_error(&mut self, msg: impl Into<String>) {
        self.error_flash = Some((msg.into(), std::time::Instant::now()));
    }

    /// The Info toast text, if it hasn't expired yet.
    pub fn flash_text(&self) -> Option<&str> {
        live_toast(&self.flash_msg)
    }

    /// The Error toast text, if it hasn't expired yet.
    pub fn error_toast(&self) -> Option<&str> {
        live_toast(&self.error_flash)
    }

    /// First press arms; a second within the window confirms the quit.
    pub fn arm_or_confirm_quit(&mut self) -> bool {
        if let Some(at) = self.ctrl_c_armed {
            if at.elapsed().as_millis() < FLASH_MS {
                return true;
            }
        }
        self.ctrl_c_armed = Some(std::time::Instant::now());
        self.flash("Ctrl+C again to quit");
        false
    }

    pub fn disarm_quit(&mut self) {
        self.ctrl_c_armed = None;
    }
}

fn live_toast(slot: &Option<(String, std::time::Instant)>) -> Option<&str> {
    match slot {
        Some((msg, at)) if at.elapsed().as_millis() < TOAST_MS => Some(msg.as_str()),
        _ => None,
    }
}
