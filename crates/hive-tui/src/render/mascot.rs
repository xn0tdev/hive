//! The hive wordmark: an ASCII-art "HIVE". Rendered as one block (left-aligned
//! at a centered x) so the letters never skew.

use comb::{Line, Span, Style};

use crate::theme::Theme;

const ART: [&str; 4] = [
    r" _  _ _____   _____ ",
    r"| || |_ _\ \ / / __|",
    r"| __ || | \ V /| _| ",
    r"|_||_|___| \_/ |___|",
];

/// Column width of the art block (all rows are this wide).
pub const WIDTH: u16 = 20;
pub const HEIGHT: u16 = ART.len() as u16;

pub fn wordmark(theme: &Theme) -> Vec<Line> {
    ART.iter()
        .map(|row| Line::from(Span::styled(row.to_string(), Style::default().fg(theme.dim))))
        .collect()
}
