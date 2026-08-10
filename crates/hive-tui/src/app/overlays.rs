//! Palette, context, settings, goal, and about overlay state.

use super::*;

impl App {
    pub fn open_palette(&mut self) {
        self.about_open = false;
        self.palette = Some(PaletteState::commands());
    }

    pub fn open_model_picker(
        &mut self,
        input_tx: &tokio::sync::mpsc::UnboundedSender<crate::InputCommand>,
    ) {
        self.about_open = false;
        self.palette = Some(PaletteState::models());
        // Keep a ready catalog visible while it refreshes. Reopening /models
        // should feel instant instead of flashing back to a loading row.
        if self.models_catalog != ModelsCatalogState::Ready {
            self.models_catalog = ModelsCatalogState::Loading;
        }
        let _ = input_tx.send(crate::InputCommand::FetchModels);
    }

    pub fn open_connect_picker(&mut self) {
        self.about_open = false;
        self.close_settings();
        let mut pal = PaletteState::connect();
        pal.clamp_selection(&self.model_choices, &self.connections, &self.saved_sessions);
        self.palette = Some(pal);
    }

    pub fn close_palette(&mut self) {
        self.palette = None;
    }

    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    // -- Context menu (click on user prompt / tool card) --

    pub fn context_menu_open(&self) -> bool {
        self.context_menu.is_some()
    }

    pub fn close_context_menu(&mut self) {
        self.context_menu = None;
        self.context_menu_win = None;
        self.context_menu_hits.clear();
    }

    /// Point the selection at the item under the cursor. False when the cursor
    /// isn't on one, so hover doesn't force a redraw.
    pub fn context_menu_hover(&mut self, col: u16, row: u16) -> bool {
        let Some(idx) = self
            .context_menu_hits
            .iter()
            .find(|(rect, _)| rect.contains(col, row))
            .map(|(_, i)| *i)
        else {
            return false;
        };
        match self.context_menu.as_mut() {
            Some(menu) if menu.selected != idx => {
                menu.selected = idx;
                true
            }
            _ => false,
        }
    }

    /// True when the cursor is on one of the menu's action rows.
    pub fn context_menu_on_item(&self, col: u16, row: u16) -> bool {
        self.context_menu_hits
            .iter()
            .any(|(rect, _)| rect.contains(col, row))
    }

    /// True when `(col, row)` is inside the open menu panel.
    pub fn context_menu_contains(&self, col: u16, row: u16) -> bool {
        self.context_menu_win
            .is_some_and(|win| win.contains(col, row))
    }

    pub fn context_menu_up(&mut self) {
        if let Some(m) = &mut self.context_menu {
            if m.selected > 0 {
                m.selected -= 1;
            } else {
                m.selected = m.items.len().saturating_sub(1);
            }
        }
    }

    pub fn context_menu_down(&mut self) {
        if let Some(m) = &mut self.context_menu {
            let last = m.items.len().saturating_sub(1);
            if m.selected < last {
                m.selected += 1;
            } else {
                m.selected = 0;
            }
        }
    }

    pub(super) fn open_context_menu(&mut self, items: Vec<ContextMenuItem>) {
        self.context_menu = Some(ContextMenu { items, selected: 0 });
    }

    /// Build and open a context menu for a user prompt block.
    pub fn open_prompt_menu(&mut self, block_idx: usize) {
        let text = match self.blocks.get(block_idx) {
            Some(Block::User(text)) => text.clone(),
            _ => return,
        };
        let items = vec![ContextMenuItem {
            label: "Copy".into(),
            action: ContextAction::Copy(text),
        }];
        self.open_context_menu(items);
    }

    /// Build and open a context menu for a tool card block.
    pub fn open_tool_menu(&mut self, block_idx: usize) {
        let (id, output, status, details_open, snapshot) = match self.blocks.get(block_idx) {
            Some(Block::Tool(card)) => (
                card.id.clone(),
                card.output.clone(),
                card.status,
                card.details_open,
                card.snapshot.clone(),
            ),
            _ => return,
        };
        let mut items = Vec::new();
        if status == ToolStatus::Ok && !output.trim().is_empty() {
            items.push(ContextMenuItem {
                label: if details_open {
                    "Hide details".into()
                } else {
                    "Show details".into()
                },
                action: ContextAction::ToggleToolDetails { id },
            });
        }
        if let Some(snap) = snapshot.filter(|_| self.ui.tool_revert).as_ref() {
            items.push(ContextMenuItem {
                label: format!("Revert {}", snap.path),
                action: ContextAction::RevertFile {
                    path: snap.path.clone(),
                    content: snap.content.clone(),
                },
            });
        }
        if !output.trim().is_empty() {
            items.push(ContextMenuItem {
                label: "Copy output".into(),
                action: ContextAction::Copy(output),
            });
        }
        if items.is_empty() {
            return;
        }
        self.open_context_menu(items);
    }

    pub fn toggle_tool_details(&mut self, id: &str) {
        if let Some(card) = self.blocks.iter_mut().rev().find_map(|block| match block {
            Block::Tool(card) if card.id == id => Some(card),
            _ => None,
        }) {
            card.details_open = !card.details_open;
            self.invalidate_transcript();
        }
    }

    /// Take the selected action from the context menu, closing it.
    pub fn take_context_action(&mut self) -> Option<ContextAction> {
        let m = self.context_menu.as_ref()?;
        let item = m.items.get(m.selected)?;
        Some(item.action.clone())
    }

    pub fn open_about(&mut self) {
        self.close_palette();
        self.close_settings();
        self.about_open = true;
    }

    pub fn close_about(&mut self) {
        self.about_open = false;
    }

    pub fn about_open(&self) -> bool {
        self.about_open
    }
}
