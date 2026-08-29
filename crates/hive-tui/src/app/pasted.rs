//! Large pasted text blocks: parked behind inline `[ pasted text N ]` tokens
//! in the composer, plus a full-screen viewer.

use super::App;
use comb::{Span, Style};

/// Lead-in of the inline token, e.g. `[ pasted text 3 ]`.
pub const TOKEN_PREFIX: &str = "[ pasted text ";

/// A big paste parked behind an inline token instead of being dumped into the
/// composer.
#[derive(Clone)]
pub struct PastedBlock {
    /// Stable number shown in the token, so removing one paste never
    /// renumbers the tokens still sitting in the composer text.
    pub id: usize,
    pub content: String,
    pub lines: usize,
}

impl PastedBlock {
    pub fn new(id: usize, content: String) -> Self {
        let lines = content.lines().count().max(1);
        Self { id, content, lines }
    }

    /// The inline token standing in for this paste, e.g. `[ pasted text 3 ]`.
    pub fn token(&self) -> String {
        format!("{TOKEN_PREFIX}{} ]", self.id)
    }

    /// Viewer title, e.g. `PASTED TEXT 3 - 12 lines`.
    pub fn label(&self) -> String {
        if self.lines == 1 {
            format!("PASTED TEXT {} - 1 line", self.id)
        } else {
            format!("PASTED TEXT {} - {} lines", self.id, self.lines)
        }
    }
}

/// Byte length of a `[ pasted text N ]` token at the start of `s`.
pub fn token_len(s: &str) -> Option<usize> {
    let rest = s.strip_prefix(TOKEN_PREFIX)?;
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || !rest[digits..].starts_with(" ]") {
        return None;
    }
    Some(TOKEN_PREFIX.len() + digits + 2)
}

/// The N inside a `[ pasted text N ]` token at the start of `s`.
#[cfg(test)]
pub fn token_id(s: &str) -> Option<usize> {
    let len = token_len(s)?;
    s[TOKEN_PREFIX.len()..len - 2].parse().ok()
}

/// Byte offset of the first `[ pasted text N ]` token in `s`.
#[cfg(test)]
pub fn find_token(s: &str) -> Option<usize> {
    s.match_indices(TOKEN_PREFIX)
        .find(|(i, _)| token_len(&s[*i..]).is_some())
        .map(|(i, _)| i)
}

/// Tint `[ pasted text N ]` tokens inside a row of text; other text keeps
/// `base`. Used by the composer and the transcript's user strip.
pub fn token_spans(text: &str, base: Style, token_style: Style) -> Vec<Span> {
    if !text.contains(TOKEN_PREFIX) {
        return vec![Span::styled(text.to_string(), base)];
    }
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some(at) = rest.find(TOKEN_PREFIX) else {
            spans.push(Span::styled(rest.to_string(), base));
            break;
        };
        let (before, tail) = rest.split_at(at);
        if !before.is_empty() {
            spans.push(Span::styled(before.to_string(), base));
        }
        let len = token_len(tail).unwrap_or(tail.len());
        spans.push(Span::styled(tail[..len].to_string(), token_style));
        rest = &tail[len..];
    }
    spans
}

impl App {
    /// `token_spans` with the token the keyboard is on wearing the selection
    /// bar, matching how every other list row highlights.
    pub fn pasted_token_spans(&self, text: &str, base: Style, bg: comb::Color) -> Vec<Span> {
        let theme = &self.theme;
        let token_style = Style::default().fg(theme.dim).bg(bg);
        let mut spans = token_spans(text, base, token_style);
        let Some(idx) = self.selected_pasted() else {
            return spans;
        };
        let token = self.pasted_blocks[idx].token();
        let on = Style::default().fg(theme.sel_fg).bg(theme.sel_bg);
        for span in &mut spans {
            if span.content == token {
                span.style = on;
            }
        }
        spans
    }
}

/// Big enough to be worth a token: more than 5 lines or 400 chars.
pub fn is_large_paste(text: &str) -> bool {
    text.lines().count() > 5 || text.chars().count() > 400
}

/// State of the full-screen pasted-text viewer.
pub struct PastedViewState {
    pub idx: usize,
    pub scroll: usize,
}

impl App {
    pub fn has_pasted_blocks(&self) -> bool {
        !self.pasted_blocks.is_empty()
    }

    /// Token the keyboard is on, if it stepped out of the composer.
    pub fn selected_pasted(&self) -> Option<usize> {
        self.pasted_selected
            .filter(|i| *i < self.pasted_blocks.len())
    }

    pub fn take_pasted_blocks(&mut self) -> Vec<PastedBlock> {
        self.pasted_selected = None;
        std::mem::take(&mut self.pasted_blocks)
    }

    /// Park a paste behind an inline token and return the token text.
    pub fn add_pasted_block(&mut self, content: String) -> String {
        let id = self.next_pasted_id;
        self.next_pasted_id += 1;
        let block = PastedBlock::new(id, content);
        let token = block.token();
        self.pasted_blocks.push(block);
        self.pasted_selected = None;
        token
    }

    /// Drop the highlighted token — the block and its `[ pasted text N ]` text
    /// in the composer. The selection stays put so repeated backspaces keep
    /// clearing, and returns to the composer when empty.
    pub fn remove_selected_pasted(&mut self) -> Option<String> {
        let idx = self.selected_pasted()?;
        let removed = self.pasted_blocks.remove(idx);
        self.pasted_selected = if self.pasted_blocks.is_empty() {
            None
        } else {
            Some(idx.min(self.pasted_blocks.len() - 1))
        };
        // Strip the token from the composer so text and blocks stay in sync.
        let token = removed.token();
        if let Some(start) = self.input.value.find(&token) {
            let end = start + token.len();
            let start_c = self.input.value[..start].chars().count();
            let mut end_c = start_c + token.chars().count();
            if self.input.value[end..].starts_with(' ') {
                end_c += 1;
            }
            self.input.replace_chars(start_c, end_c, "");
        }
        Some(token)
    }

    /// Step from the composer onto the first token. False when there are none,
    /// so the caller can fall back to scrolling.
    pub fn select_first_pasted(&mut self) -> bool {
        if self.pasted_blocks.is_empty() {
            return false;
        }
        self.pasted_selected = Some(0);
        true
    }

    /// Walk the tokens, wrapping at both ends.
    pub fn move_pasted_selection(&mut self, delta: isize) -> bool {
        let Some(current) = self.selected_pasted() else {
            return false;
        };
        let n = self.pasted_blocks.len() as isize;
        self.pasted_selected = Some((current as isize + delta).rem_euclid(n) as usize);
        true
    }

    /// Hand the keyboard back to the composer.
    pub fn clear_pasted_selection(&mut self) -> bool {
        self.pasted_selected.take().is_some()
    }

    pub fn open_pasted_view(&mut self, idx: usize) {
        if idx >= self.pasted_blocks.len() {
            return;
        }
        self.pasted_view = Some(PastedViewState { idx, scroll: 0 });
    }

    /// Open the viewer for the selected token, if any.
    pub fn open_selected_pasted(&mut self) -> bool {
        match self.selected_pasted() {
            Some(idx) => {
                self.open_pasted_view(idx);
                true
            }
            None => false,
        }
    }

    pub fn close_pasted_view(&mut self) {
        self.pasted_view = None;
    }

    pub fn pasted_view_open(&self) -> bool {
        self.pasted_view.is_some()
    }

    /// Scroll the viewer; clamped against the real wrapped height at paint time.
    pub fn pasted_view_scroll(&mut self, delta: isize) -> bool {
        let Some(v) = self.pasted_view.as_mut() else {
            return false;
        };
        let next = if delta < 0 {
            v.scroll.saturating_sub(delta.unsigned_abs())
        } else {
            v.scroll.saturating_add(delta as usize)
        };
        if next == v.scroll {
            return false;
        }
        v.scroll = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trip() {
        assert_eq!(token_len("[ pasted text 12 ]"), Some(18));
        assert_eq!(token_id("[ pasted text 12 ]"), Some(12));
        assert_eq!(token_len("[ pasted text ]"), None);
        assert_eq!(token_len("[ pasted text x ]"), None);
        assert_eq!(token_len("[ pasted text 1"), None);
        assert_eq!(find_token("a [ pasted text 3 ] b"), Some(2));
        assert_eq!(find_token("no tokens here"), None);
    }
}
