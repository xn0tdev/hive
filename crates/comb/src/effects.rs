//! Reusable visual effects. Each takes an animation `phase` (a monotonically
//! rising frame counter) so callers stay in control of timing.

use crate::style::{Color, Modifier, Style};
use crate::text::Span;

/// A grayscale "shimmer": a bright highlight sweeps left-to-right across `text`,
/// so a word glows while something is in flight. `phase` is the frame counter.
pub fn shimmer(text: &str, phase: usize) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as i32;
    if n == 0 {
        return Vec::new();
    }
    // The highlight head sweeps from just before the word to just past it.
    let head = (phase as i32 % (n + 6)) - 3;
    let (lo, hi) = (0x70i32, 0xf2i32);
    chars
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let dist = (i as i32 - head).unsigned_abs() as f32;
            let t = (1.0 - dist / 3.0).max(0.0);
            let v = (lo as f32 + (hi - lo) as f32 * t) as u8;
            Span::styled(
                c.to_string(),
                Style::new().fg(Color::rgb(v, v, v)).add(Modifier::BOLD),
            )
        })
        .collect()
}

/// A pulsing intensity in `[lo, hi]` following a triangle wave of `period`
/// frames — good for a breathing cursor or a soft attention cue.
pub fn pulse(phase: usize, period: usize, lo: u8, hi: u8) -> Color {
    let period = period.max(2);
    let p = phase % period;
    let half = period / 2;
    let t = if p < half {
        p as f32 / half as f32
    } else {
        1.0 - (p - half) as f32 / (period - half) as f32
    };
    let v = lo as f32 + (hi as f32 - lo as f32) * t;
    let v = v as u8;
    Color::rgb(v, v, v)
}
