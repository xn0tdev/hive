//! Reusable visual effects. Each takes an animation `phase` (a monotonically
//! rising frame counter) so callers stay in control of timing.

use crate::core::style::{Color, Modifier, Style};
use crate::core::text::Span;

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

/// A named spinner: a cyclic set of single-width frames. Advance with `phase`.
pub struct Spinner {
    pub name: &'static str,
    pub frames: &'static [&'static str],
}

impl Spinner {
    pub fn frame(&self, phase: usize) -> &'static str {
        if self.frames.is_empty() {
            " "
        } else {
            self.frames[phase % self.frames.len()]
        }
    }
}

/// A gallery of ready-made spinners — every frame is one cell wide, no emoji, so
/// they line up perfectly in the grid.
pub const SPINNERS: &[Spinner] = &[
    Spinner { name: "braille", frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"] },
    Spinner { name: "dots", frames: &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"] },
    Spinner { name: "line", frames: &["|", "/", "-", "\\"] },
    Spinner { name: "arc", frames: &["◜", "◠", "◝", "◞", "◡", "◟"] },
    Spinner { name: "circle", frames: &["◐", "◓", "◑", "◒"] },
    Spinner { name: "triangle", frames: &["◢", "◣", "◤", "◥"] },
    Spinner { name: "quadrant", frames: &["▖", "▘", "▝", "▗"] },
    Spinner { name: "bar", frames: &["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█", "▉", "▊", "▌", "▍", "▎"] },
    Spinner { name: "star", frames: &["✶", "✸", "✹", "✺", "✹", "✷"] },
    Spinner { name: "bounce", frames: &["⠁", "⠂", "⠄", "⠂"] },
];

/// Render a fractional progress bar of `width` cells using eighth-block glyphs
/// for a smooth sub-cell edge. Returns styled spans (filled run + empty run).
pub fn bar(fraction: f32, width: usize, filled: Style, empty: Style) -> Vec<Span> {
    let frac = fraction.clamp(0.0, 1.0);
    let total = width as f32 * frac;
    let full = (total.floor() as usize).min(width);
    let eighths = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

    let mut s = "█".repeat(full);
    let mut used = full;
    if used < width {
        let idx = ((total - full as f32) * 8.0).round() as usize;
        if idx > 0 {
            s.push_str(eighths[idx.min(7)]);
            used += 1;
        }
    }

    let mut out = Vec::new();
    if !s.is_empty() {
        out.push(Span::styled(s, filled));
    }
    if used < width {
        out.push(Span::styled(" ".repeat(width - used), empty));
    }
    out
}

const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A tiny inline sparkline: `values` in `[0, 1]`, resampled to `width` cells.
pub fn sparkline(values: &[f32], width: usize, style: Style) -> Vec<Span> {
    if width == 0 {
        return Vec::new();
    }
    if values.is_empty() {
        return vec![Span::styled(" ".repeat(width), style)];
    }
    let n = values.len();
    let mut s = String::with_capacity(width);
    for i in 0..width {
        let idx = i * n / width;
        let v = values[idx.min(n - 1)].clamp(0.0, 1.0);
        let ch = SPARK[(v * 7.0).round() as usize];
        s.push(ch);
    }
    vec![Span::styled(s, style)]
}
