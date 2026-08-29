//! Shared flat overlay chrome for Hive.
//!
//! Geometry and scrim behavior live in `comb::Modal`; this module only applies
//! the Hive theme and its quiet title-left / hint-right header.

use comb::{Buffer, Line, Modal, ModalLayout, Modifier, Rect, Span, Style};

use crate::theme::Theme;

const MIN_HEADER_GAP: usize = 2;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Panel<'a> {
    title: &'a str,
    hint: &'a str,
    height: u16,
    width_numerator: u16,
    width_denominator: u16,
    min_width: u16,
    max_width: u16,
    pad_x: u16,
    pad_y: u16,
    v_margin: u16,
}

impl<'a> Panel<'a> {
    pub(crate) fn new(title: &'a str, hint: &'a str, height: u16) -> Self {
        Self {
            title,
            hint,
            height,
            width_numerator: 2,
            width_denominator: 3,
            min_width: 0,
            max_width: u16::MAX,
            pad_x: 2,
            pad_y: 1,
            v_margin: 0,
        }
    }

    pub(crate) fn width_ratio(mut self, numerator: u16, denominator: u16) -> Self {
        self.width_numerator = numerator;
        self.width_denominator = denominator;
        self
    }

    pub(crate) fn width_bounds(mut self, min: u16, max: u16) -> Self {
        self.min_width = min;
        self.max_width = max;
        self
    }

    pub(crate) fn padding(mut self, horizontal: u16, vertical: u16) -> Self {
        self.pad_x = horizontal;
        self.pad_y = vertical;
        self
    }

    /// Keep this many rows free above and below the panel when possible.
    pub(crate) fn vertical_margin(mut self, rows: u16) -> Self {
        self.v_margin = rows;
        self
    }

    pub(crate) fn layout(self, area: Rect) -> ModalLayout {
        self.modal(Style::default()).layout(area)
    }

    pub(crate) fn render(self, buf: &mut Buffer, area: Rect, theme: &Theme) -> ModalLayout {
        let panel_style = Style::default().bg(theme.strip);
        let layout = self.modal(panel_style).render(buf, area);
        if !self.title.is_empty() || !self.hint.is_empty() {
            draw_header(buf, layout.content, self.title, self.hint, theme);
        }
        layout
    }

    fn modal(self, fill: Style) -> Modal {
        Modal::new(self.height)
            .width_ratio(self.width_numerator, self.width_denominator)
            .width_bounds(self.min_width, self.max_width)
            .vertical_margin(self.v_margin)
            .padding(self.pad_x, self.pad_y)
            .fill(fill)
    }
}

fn draw_header(buf: &mut Buffer, area: Rect, title: &str, hint: &str, theme: &Theme) {
    if area.is_empty() {
        return;
    }
    let width = usize::from(area.width);
    let title_width = title.chars().count();
    let hint = if title_width + MIN_HEADER_GAP + hint.chars().count() <= width {
        hint
    } else if title_width + MIN_HEADER_GAP + 3 <= width {
        "esc"
    } else {
        ""
    };
    let gap = width.saturating_sub(title_width + hint.chars().count());
    let base = Style::default().bg(theme.strip);
    let line = Line::from(vec![
        Span::styled(title, Style::default().fg(theme.fg).add(Modifier::BOLD)),
        Span::styled(" ".repeat(gap), base),
        Span::styled(hint, Style::default().fg(theme.faint)),
    ]);
    buf.set_line_on(area.x, area.y, &line, area.width, base);
}

#[cfg(test)]
mod tests {
    use super::*;
    use comb::Size;

    #[test]
    fn long_hint_falls_back_without_colliding_with_title() {
        let theme = Theme::gray();
        let mut buf = Buffer::blank(Size::new(16, 3));
        draw_header(
            &mut buf,
            Rect::new(0, 1, 16, 1),
            "Settings",
            "enter toggle · esc back",
            &theme,
        );

        let row: String = (0..16).map(|x| buf.get(x, 1).unwrap().ch).collect();
        assert_eq!(row, "Settings     esc");
        assert!(!row.contains("Settingsesc"));
    }
}
