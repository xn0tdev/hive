//! Block-letter "HIVE" wordmark, with an optional click "bonk" shimmer that
//! ripples out from the hit cell.

use std::time::Instant;

use comb::{Color, Line, Modifier, Rect, Span, Style};

use crate::theme::Theme;

/// How long a bonk ripple runs.
pub const BONK_MS: u128 = 1100;

/// A click-triggered ripple on the logo.
#[derive(Clone, Debug)]
pub struct LogoBonk {
    /// Local cell coords inside the art rect (origin of the ripple).
    pub ox: u16,
    pub oy: u16,
    pub started: Instant,
}

impl LogoBonk {
    pub fn fresh(ox: u16, oy: u16) -> Self {
        LogoBonk {
            ox,
            oy,
            started: Instant::now(),
        }
    }

    pub fn alive(&self) -> bool {
        self.started.elapsed().as_millis() < BONK_MS
    }
}

// Equal-weight block letters (H=5, I=3, V=5, E=5) with single-cell gaps.
pub(crate) const ART: [&str; 5] = [
    "█   █ ███ █   █ █████",
    "█   █  █  █   █ █    ",
    "█████  █  █   █ ████ ",
    "█   █  █   █ █  █    ",
    "█   █ ███   █   █████",
];

pub const WIDTH: u16 = 21;
pub const HEIGHT: u16 = ART.len() as u16;

/// Styled lines for the wordmark. When `bonk` is active, a dark shimmer ring
/// expands from the click cell and paints the blocks behind it.
pub fn lines(theme: &Theme, bonk: Option<&LogoBonk>) -> Vec<Line> {
    debug_assert!(
        ART.iter().all(|r| r.chars().count() == WIDTH as usize),
        "wordmark rows must share WIDTH"
    );

    let elapsed = bonk
        .map(|b| b.started.elapsed().as_secs_f32())
        .unwrap_or(0.0);
    let dur = BONK_MS as f32 / 1000.0;
    // Ripple grows across the logo over the whole bonk window.
    let radius = elapsed * (WIDTH as f32 + HEIGHT as f32 + 4.0) / dur;
    let fading = (1.0 - elapsed / dur).clamp(0.0, 1.0);

    ART.iter()
        .enumerate()
        .map(|(row, text)| {
            let spans: Vec<Span> = text
                .chars()
                .enumerate()
                .map(|(col, ch)| {
                    if ch == ' ' {
                        return Span::raw(" ");
                    }
                    let color = match bonk.filter(|b| b.alive()) {
                        Some(b) => cell_bonk_color(
                            col as u16, row as u16, b, radius, fading, elapsed, theme,
                        ),
                        None => theme.fg,
                    };
                    Span::styled(
                        ch.to_string(),
                        Style::default().fg(color).add(Modifier::BOLD),
                    )
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

fn cell_bonk_color(
    x: u16,
    y: u16,
    bonk: &LogoBonk,
    radius: f32,
    fading: f32,
    elapsed: f32,
    theme: &Theme,
) -> Color {
    let dx = x as f32 - bonk.ox as f32;
    let dy = y as f32 - bonk.oy as f32;
    let dist = (dx * dx + dy * dy).sqrt();

    let Color::Rgb(br, bg, bb) = theme.fg else {
        return theme.fg;
    };
    let Color::Rgb(dr, dg, db) = theme.dim else {
        return theme.dim;
    };
    let Color::Rgb(fr, fg, fb) = theme.faint else {
        return theme.faint;
    };
    // Even darker floor than theme.faint for the painted trail.
    let (er, eg, eb) = (
        (fr as f32 * 0.55) as u8,
        (fg as f32 * 0.55) as u8,
        (fb as f32 * 0.55) as u8,
    );

    // Wide primary shimmer band on the expanding front.
    let band = 3.4;
    let rim = (1.0 - (dist - radius).abs() / band).clamp(0.0, 1.0);
    // Extra traveling shimmer inside the band (phase along the ring).
    let angle = dy.atan2(dx);
    let sparkle = (0.5 + 0.5 * (angle * 3.0 + elapsed * 14.0).sin()).powf(2.2);
    let shimmer = rim * (0.55 + 0.45 * sparkle);

    // Secondary echo ring a bit behind the front.
    let echo_r = (radius - 2.6).max(0.0);
    let echo = (1.0 - (dist - echo_r).abs() / 2.0).clamp(0.0, 1.0) * 0.55;

    // Dark wash for everything the wave has already covered.
    let inside = if dist <= radius {
        (1.0 - dist / radius.max(0.01)).clamp(0.0, 1.0) * 0.85
    } else {
        0.0
    };

    let peak = (shimmer.max(echo * 0.7)) * fading;
    let wash = inside * fading;

    // Idle fg → dim on shimmer peaks → deep faint in the trail.
    let r = lerp(br, dr, peak * 0.9);
    let g = lerp(bg, dg, peak * 0.9);
    let b = lerp(bb, db, peak * 0.9);
    let r = lerp(r, er, wash);
    let g = lerp(g, eg, wash);
    let b = lerp(b, eb, wash);
    // Tiny lift on the sparkle so the shimmer reads, still darker than base fg.
    let lift = shimmer * fading * 0.25;
    let r = lerp(r, dr, lift);
    let g = lerp(g, dg, lift);
    let b = lerp(b, db, lift);
    Color::Rgb(r, g, b)
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

/// True when `(col, row)` (screen coords) lands on a solid block cell of the logo.
pub fn hit_cell(logo: Rect, col: u16, row: u16) -> Option<(u16, u16)> {
    if !logo.contains(col, row) {
        return None;
    }
    let lx = col - logo.x;
    let ly = row - logo.y;
    let ch = ART
        .get(ly as usize)
        .and_then(|line| line.chars().nth(lx as usize))?;
    if ch == '█' {
        Some((lx, ly))
    } else {
        // Clicks in letter gaps still count — juicier hit target.
        Some((lx, ly))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn wordmark_has_block_glyphs() {
        let theme = Theme::from_name("gray");
        let lines = lines(&theme, None);
        assert_eq!(lines.len(), HEIGHT as usize);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_str()))
            .collect();
        assert!(text.contains('█'));
        assert!(!text.contains("(o.o)"));
    }
}
