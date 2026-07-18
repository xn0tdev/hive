//! Reserved compact bee character art (not used on About / landing — those use
//! the HIVE wordmark in [`crate::render::wordmark`]).

#![allow(dead_code)]

use comb::{Line, Modifier, Span, Style};

use crate::theme::Theme;

const BEE: [&str; 5] = [
    r#"    \./    "#,
    r#"   (o.o)   "#,
    r#"   {(")}   "#,
    r#"    ^ ^    "#,
    r#"   ~hive~  "#,
];

pub const WIDTH: u16 = 11;
pub const HEIGHT: u16 = BEE.len() as u16;

/// Styled lines for the reserved bee character (unused for now).
pub fn lines(theme: &Theme) -> Vec<Line> {
    debug_assert!(
        BEE.iter().all(|r| r.chars().count() == WIDTH as usize),
        "bee rows must share WIDTH"
    );
    let body = Style::default().fg(theme.fg).add(Modifier::BOLD);
    let soft = Style::default().fg(theme.dim);
    let brand = Style::default().fg(theme.accent).add(Modifier::BOLD);
    BEE.iter()
        .enumerate()
        .map(|(i, text)| {
            if i == BEE.len() - 1 {
                // ~hive~ — accent the word, dim the tildes.
                let spans: Vec<Span> = text
                    .chars()
                    .map(|ch| match ch {
                        '~' => Span::styled("~", soft),
                        ' ' => Span::raw(" "),
                        _ => Span::styled(ch.to_string(), brand),
                    })
                    .collect();
                return Line::from(spans);
            }
            let style = if i == 0 { soft } else { body };
            let spans: Vec<Span> = text
                .chars()
                .map(|ch| {
                    if ch == ' ' {
                        Span::raw(" ")
                    } else {
                        Span::styled(ch.to_string(), style)
                    }
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}
