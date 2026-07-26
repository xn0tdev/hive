//! A small "back to the newest message" chip.
//!
//! Lives on the blank row between the transcript and the composer, right-
//! aligned like the mode chip below it, and only exists while the transcript
//! is scrolled up. Hover lightens it so it reads as clickable.

use comb::{Buffer, Line, Rect, Span, Style};

use crate::app::App;

/// Chip label. Kept to an arrow so it stays out of the way of the transcript.
const LABEL: &str = " ↓ ";

/// Cells the chip occupies.
pub fn width() -> u16 {
    LABEL.chars().count() as u16
}

/// Draw the chip right-aligned in `row` and record its hit target. Clears the
/// hit target when there's nothing to scroll back to.
pub fn draw(buf: &mut Buffer, row: Rect, app: &mut App) {
    let w = width();
    if !app.can_scroll_down() || row.height == 0 || row.width < w {
        app.scroll_bottom_hit = None;
        return;
    }

    let theme = &app.theme;
    let (fg, bg) = if app.hover_scroll_bottom {
        (theme.fg, theme.strip_hover)
    } else {
        (theme.dim, theme.strip)
    };

    let rect = Rect::new(row.x + row.width - w, row.y, w, 1);
    let line = Line::from(Span::styled(LABEL, Style::default().fg(fg).bg(bg)));
    buf.set_line(rect.x, rect.y, &line, w);
    app.scroll_bottom_hit = Some(rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::TuiInit;
    use comb::Size;

    fn app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: Vec::new(),
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
            ui: Default::default(),
            context_window: 128_000,
            cost_input: 0.0,
            cost_output: 0.0,
        })
    }

    fn row() -> Rect {
        Rect::new(0, 4, 40, 1)
    }

    #[test]
    fn absent_when_already_at_the_bottom() {
        let mut a = app();
        let mut buf = Buffer::blank(Size::new(40, 6));
        draw(&mut buf, row(), &mut a);
        assert!(a.scroll_bottom_hit.is_none());
        assert!(!buf.text().contains('↓'));
    }

    #[test]
    fn appears_right_aligned_once_scrolled_up() {
        let mut a = app();
        a.set_transcript_max_scroll(20);
        a.scroll_up(5);

        let mut buf = Buffer::blank(Size::new(40, 6));
        draw(&mut buf, row(), &mut a);

        let hit = a.scroll_bottom_hit.expect("chip");
        assert_eq!(hit.width, width());
        assert_eq!(hit.x + hit.width, 40, "flush with the right edge");
        assert_eq!(hit.y, 4);
        assert!(buf.text().contains('↓'));
    }

    #[test]
    fn hover_changes_the_background() {
        let mut a = app();
        a.set_transcript_max_scroll(20);
        a.scroll_up(5);
        let mut buf = Buffer::blank(Size::new(40, 6));

        draw(&mut buf, row(), &mut a);
        let idle = buf.get(38, 4).expect("cell").style.bg;

        assert!(a.set_hover_scroll_bottom(true));
        draw(&mut buf, row(), &mut a);
        let hovered = buf.get(38, 4).expect("cell").style.bg;

        assert_ne!(idle, hovered);
    }

    #[test]
    fn a_narrow_row_is_left_alone() {
        let mut a = app();
        a.set_transcript_max_scroll(20);
        a.scroll_up(5);
        let mut buf = Buffer::blank(Size::new(40, 6));
        draw(&mut buf, Rect::new(0, 4, 2, 1), &mut a);
        assert!(a.scroll_bottom_hit.is_none());
    }
}
