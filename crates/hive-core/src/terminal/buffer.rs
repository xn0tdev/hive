use std::collections::VecDeque;

use super::{OUTPUT_JOURNAL_CAP, SCROLLBACK_ROWS};

pub(crate) struct TerminalBuffer {
    parser: vt100::Parser,
    revision: u64,
    journal: VecDeque<OutputChunk>,
    journal_bytes: usize,
    journal_cap: usize,
    dropped_through_revision: u64,
    rows: u16,
    cols: u16,
}

struct OutputChunk {
    revision: u64,
    bytes: Vec<u8>,
}

pub(crate) struct PrintableDelta {
    pub text: String,
    pub truncated: bool,
}

impl TerminalBuffer {
    pub(crate) fn new(rows: u16, cols: u16) -> Self {
        Self::with_journal_cap(rows, cols, OUTPUT_JOURNAL_CAP)
    }

    pub(crate) fn with_journal_cap(rows: u16, cols: u16, journal_cap: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        Self {
            parser: vt100::Parser::new(rows, cols, SCROLLBACK_ROWS),
            revision: 0,
            journal: VecDeque::new(),
            journal_bytes: 0,
            journal_cap: journal_cap.max(1),
            dropped_through_revision: 0,
            rows,
            cols,
        }
    }

    pub(crate) fn process(&mut self, bytes: &[u8]) -> u64 {
        self.parser.process(bytes);
        self.revision = self.revision.saturating_add(1);
        self.journal.push_back(OutputChunk {
            revision: self.revision,
            bytes: bytes.to_vec(),
        });
        self.journal_bytes = self.journal_bytes.saturating_add(bytes.len());
        self.enforce_journal_cap();
        self.revision
    }

    pub(crate) fn touch(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }

    pub(crate) fn resize(&mut self, rows: u16, cols: u16) -> u64 {
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self.parser.screen_mut().set_size(self.rows, self.cols);
        self.touch()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    #[cfg(test)]
    pub(crate) fn size(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    pub(crate) fn screen(&self) -> String {
        self.parser.screen().contents()
    }

    pub(crate) fn screen_snapshot(&self) -> vt100::Screen {
        self.parser.screen().clone()
    }

    pub(crate) fn application_cursor(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    pub(crate) fn output_since(&self, revision: u64) -> PrintableDelta {
        let raw = self
            .journal
            .iter()
            .filter(|chunk| chunk.revision > revision)
            .flat_map(|chunk| chunk.bytes.iter().copied())
            .collect::<Vec<_>>();
        let stripped = strip_ansi_escapes::strip(raw);
        let normalized = String::from_utf8_lossy(&stripped).replace("\r\n", "\n");
        let text = normalized
            .chars()
            .filter(|ch| *ch == '\n' || !ch.is_control())
            .collect();

        PrintableDelta {
            text,
            truncated: revision < self.dropped_through_revision,
        }
    }

    fn enforce_journal_cap(&mut self) {
        while self.journal_bytes > self.journal_cap && self.journal.len() > 1 {
            if let Some(chunk) = self.journal.pop_front() {
                self.journal_bytes = self.journal_bytes.saturating_sub(chunk.bytes.len());
                self.dropped_through_revision = self.dropped_through_revision.max(chunk.revision);
            }
        }

        let Some(chunk) = self.journal.front_mut() else {
            return;
        };
        if self.journal_bytes <= self.journal_cap {
            return;
        }

        let excess = self.journal_bytes - self.journal_cap;
        let drain_len = excess.min(chunk.bytes.len());
        chunk.bytes.drain(..drain_len);
        self.journal_bytes = self.journal_bytes.saturating_sub(drain_len);
        self.dropped_through_revision = self.dropped_through_revision.max(chunk.revision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_tracks_screen_revision_and_printable_delta() {
        let mut buffer = TerminalBuffer::new(4, 20);
        assert_eq!(buffer.revision(), 0);
        buffer.process(b"hello \x1b[31mred\x1b[0m\r\n");
        let revision = buffer.revision();
        assert_eq!(revision, 1);
        assert!(buffer.screen().contains("hello red"));

        let delta = buffer.output_since(0);
        assert!(delta.text.contains("hello red"));
        assert!(!delta.text.contains("\x1b["));
        assert!(!delta.truncated);
    }

    #[test]
    fn buffer_caps_journal_and_resizes_screen() {
        let mut buffer = TerminalBuffer::with_journal_cap(2, 8, 8);
        buffer.process(b"12345678");
        let old_revision = buffer.revision();
        buffer.process(b"abcdefgh");
        let delta = buffer.output_since(0);
        assert!(delta.truncated);
        assert!(delta.text.ends_with("abcdefgh"));
        buffer.resize(6, 40);
        assert_eq!(buffer.size(), (6, 40));
        assert!(buffer.revision() > old_revision);
    }

    #[test]
    fn touch_advances_revision_without_printable_output() {
        let mut buffer = TerminalBuffer::new(2, 8);
        assert_eq!(buffer.touch(), 1);
        let delta = buffer.output_since(0);
        assert_eq!(delta.text, "");
        assert!(!delta.truncated);
    }
}
