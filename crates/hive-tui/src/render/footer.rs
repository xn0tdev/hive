//! Footer content: `model · tokens` and the working directory. Exposed as
//! reusable lines so both the bottom bar and the centered landing can use them.
//! BUILD/PLAN chip sits on the model row (right), under the input strip.
//! While a turn is running, a compact 5-cube ping-pong wave sits before the model.

use comb::{Buffer, Color, Frame, Line, Rect, Span, Style};

use crate::render::input_box;

/// How many cubes in the working indicator.
const WORKING_BLOCKS: usize = 5;
/// Peak travel in block-units per spinner tick (~100ms).
const WORKING_SPEED: f32 = 0.30;
/// Soft falloff half-width (block-units) around the traveling peak.
const WORKING_SOFT: f32 = 1.35;
/// Lit cube (larger); inactive uses [`WORKING_GLYPH_DIM`].
const WORKING_GLYPH_LIT: char = '■';
/// Unlit / mostly-unlit cube (smaller).
const WORKING_GLYPH_DIM: char = '▪';
/// Brightness at/above which a cube switches to the large lit glyph.
const WORKING_GLYPH_THRESHOLD: f32 = 0.45;

/// Soft ping-pong cubes + `model · tokens · attachments` as a single line.
/// Wave travels while a turn is running; idle shows model only.
pub fn model_line(app: &crate::app::App) -> Line {
    let theme = &app.theme;
    let mut spans = Vec::new();

    if app.running {
        spans.extend(working_spans(app.spinner, theme));
        spans.push(Span::styled("  ", Style::default()));
    }

    spans.push(Span::styled(
        app.model_display.clone(),
        Style::default().fg(theme.dim),
    ));
    if app.usage.total_tokens > 0 {
        spans.push(Span::styled(" · ", Style::default().fg(theme.faint)));
        spans.push(Span::styled(
            format_tokens(app.usage.total_tokens),
            Style::default().fg(theme.faint),
        ));
    }
    if !app.pending_images.is_empty() {
        spans.push(Span::styled(" · ", Style::default().fg(theme.faint)));
        spans.push(Span::styled(
            format!("{} image(s) attached", app.pending_images.len()),
            Style::default().fg(theme.warn),
        ));
    }
    Line::from(spans)
}

pub fn cwd_line(app: &crate::app::App) -> Line {
    Line::from(Span::styled(
        tilde(&app.cwd),
        Style::default().fg(app.theme.faint),
    ))
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &crate::app::App) {
    if area.height < 2 {
        return;
    }
    buf.paint(area, Style::default());
    buf.set_line(area.x, area.y, &model_line(app), area.width);
    buf.set_line(area.x, area.y + 1, &cwd_line(app), area.width);
}

/// Model + cwd under the input; BUILD/PLAN on the model row (right).
/// Toasts are drawn separately (centered), so they never replace the chip.
pub fn draw_with_mode(f: &mut Frame, area: Rect, app: &crate::app::App) {
    if area.height < 2 {
        return;
    }
    f.buffer().paint(area, Style::default());
    f.buffer()
        .set_line(area.x, area.y, &model_line(app), area.width);
    input_box::draw_mode_chip(f, Rect::new(area.x, area.y, area.width, 1), app);
    f.buffer()
        .set_line(area.x, area.y + 1, &cwd_line(app), area.width);
}

/// Soft ping-pong cubes: lit peak travels L→R then R→L and loops (no label).
/// Cubes are adjacent (no gap characters between glyphs).
fn working_spans(tick: usize, theme: &crate::theme::Theme) -> Vec<Span> {
    (0..WORKING_BLOCKS)
        .map(|i| {
            let t = block_brightness(tick, i);
            let glyph = if t >= WORKING_GLYPH_THRESHOLD {
                WORKING_GLYPH_LIT
            } else {
                WORKING_GLYPH_DIM
            };
            Span::styled(
                glyph.to_string(),
                Style::default().fg(lerp_color(theme.faint, theme.accent, t)),
            )
        })
        .collect()
}

/// Brightness of cube `index` at spinner `tick` (0.0 = unlit … 1.0 = peak).
///
/// The active peak travels left→right across the row, then right→left, looping.
/// Brightness falls off softly with distance from the peak.
fn block_brightness(tick: usize, index: usize) -> f32 {
    let max_i = (WORKING_BLOCKS - 1) as f32;
    // One full bounce: 0 → max → 0.
    let cycle = 2.0 * max_i;
    let phase = (tick as f32 * WORKING_SPEED) % cycle;
    let peak = if phase <= max_i {
        phase
    } else {
        cycle - phase
    };

    let dist = (peak - index as f32).abs();
    (1.0 - dist / WORKING_SOFT).clamp(0.0, 1.0)
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => Color::Rgb(
            (ar as f32 + (br as f32 - ar as f32) * t).round() as u8,
            (ag as f32 + (bg as f32 - ag as f32) * t).round() as u8,
            (ab as f32 + (bb as f32 - ab as f32) * t).round() as u8,
        ),
        _ => {
            if t > 0.5 {
                b
            } else {
                a
            }
        }
    }
}

fn format_tokens(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k tokens", n as f64 / 1000.0)
    } else {
        format!("{n} tokens")
    }
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::TuiInit;
    use hive_core::event::AgentEvent;

    fn app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "Kimi 2.6".into(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
        })
    }

    fn line_text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_str())
            .collect()
    }

    fn working_glyph_count(text: &str) -> usize {
        text.chars()
            .filter(|&c| c == WORKING_GLYPH_LIT || c == WORKING_GLYPH_DIM)
            .count()
    }

    fn peak_at(tick: usize) -> f32 {
        let max_i = (WORKING_BLOCKS - 1) as f32;
        let cycle = 2.0 * max_i;
        let phase = (tick as f32 * WORKING_SPEED) % cycle;
        if phase <= max_i {
            phase
        } else {
            cycle - phase
        }
    }

    /// Spinner tick whose traveling peak is closest to `target`.
    fn tick_for_peak(target: f32) -> usize {
        (0..200)
            .min_by(|a, b| {
                (peak_at(*a) - target)
                    .abs()
                    .total_cmp(&(peak_at(*b) - target).abs())
            })
            .unwrap()
    }

    #[test]
    fn block_brightness_ping_pong_peak() {
        // Peak near first cube: cube 0 bright, later cubes dim.
        let t0 = tick_for_peak(0.0);
        assert!(block_brightness(t0, 0) > 0.9);
        assert!(block_brightness(t0, 1) < WORKING_GLYPH_THRESHOLD);
        assert!(block_brightness(t0, 4) < 0.05);

        // Peak mid-row going right: center bright, neighbors soft, ends dimmer.
        let t_mid = tick_for_peak(2.0);
        assert!((peak_at(t_mid) - 2.0).abs() < 0.2, "peak={}", peak_at(t_mid));
        assert!(block_brightness(t_mid, 2) > 0.9);
        assert!(block_brightness(t_mid, 1) > 0.15);
        assert!(block_brightness(t_mid, 3) > 0.15);
        assert!(block_brightness(t_mid, 0) < WORKING_GLYPH_THRESHOLD);
        assert!(block_brightness(t_mid, 4) < WORKING_GLYPH_THRESHOLD);

        // Peak at the right end.
        let t_right = tick_for_peak(4.0);
        assert!(block_brightness(t_right, 4) > 0.9);
        assert!(block_brightness(t_right, 0) < 0.05);

        // After the right end, peak travels back left (same soft profile).
        let t_back = tick_for_peak(2.0);
        // Prefer a tick on the return leg (phase > max_i).
        let t_back = (0..200)
            .find(|&t| {
                let max_i = (WORKING_BLOCKS - 1) as f32;
                let phase = (t as f32 * WORKING_SPEED) % (2.0 * max_i);
                phase > max_i && (peak_at(t) - 2.0).abs() < 0.25
            })
            .unwrap_or(t_back);
        assert!(block_brightness(t_back, 2) > 0.85);
        assert!(block_brightness(t_back, 0) < WORKING_GLYPH_THRESHOLD);
        assert!(block_brightness(t_back, 4) < WORKING_GLYPH_THRESHOLD);

        // Soft edge: peak between cubes lights both partially.
        let t_between = tick_for_peak(1.5);
        let b1 = block_brightness(t_between, 1);
        let b2 = block_brightness(t_between, 2);
        assert!(b1 > 0.4 && b1 < 0.95, "soft between for cube 1: {b1}");
        assert!(b2 > 0.4 && b2 < 0.95, "soft between for cube 2: {b2}");
        assert!((b1 - b2).abs() < 0.15, "symmetric soft edge: {b1} vs {b2}");
    }

    #[test]
    fn working_spans_switch_glyph_by_brightness() {
        let theme = crate::theme::Theme::gray();
        // Peak on first cube: first large, rest small.
        let early = working_spans(tick_for_peak(0.0), &theme);
        assert_eq!(early.len(), WORKING_BLOCKS);
        assert_eq!(early[0].content.as_str(), WORKING_GLYPH_LIT.to_string());
        for span in &early[1..] {
            assert_eq!(span.content.as_str(), WORKING_GLYPH_DIM.to_string());
        }

        // Peak on last cube: last large, rest small.
        let right = working_spans(tick_for_peak(4.0), &theme);
        for span in &right[..WORKING_BLOCKS - 1] {
            assert_eq!(span.content.as_str(), WORKING_GLYPH_DIM.to_string());
        }
        assert_eq!(
            right[WORKING_BLOCKS - 1].content.as_str(),
            WORKING_GLYPH_LIT.to_string()
        );
    }

    #[test]
    fn working_cubes_have_no_gap_characters() {
        let theme = crate::theme::Theme::gray();
        let text: String = working_spans(tick_for_peak(2.0), &theme)
            .iter()
            .map(|s| s.content.as_str())
            .collect();
        assert_eq!(text.chars().count(), WORKING_BLOCKS, "{text}");
        assert!(!text.contains(' '), "unexpected space in cubes: {text:?}");
        assert!(!text.contains('\u{3000}'), "unexpected ideographic space: {text:?}");
    }

    #[test]
    fn model_line_hides_working_when_idle() {
        let a = app();
        let t = line_text(&model_line(&a));
        assert!(t.contains("Kimi 2.6"), "{t}");
        assert!(!t.contains("Working"), "{t}");
        assert_eq!(working_glyph_count(&t), 0, "{t}");
    }

    #[test]
    fn model_line_shows_dots_when_running() {
        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.spinner = 4;
        let t = line_text(&model_line(&a));
        assert!(!t.contains("Working"), "{t}");
        assert!(t.contains("Kimi 2.6"), "{t}");
        assert_eq!(working_glyph_count(&t), WORKING_BLOCKS, "{t}");
        // Cubes sit before the model name, packed with no gap chars between them.
        let start = t
            .char_indices()
            .find(|(_, c)| *c == WORKING_GLYPH_LIT || *c == WORKING_GLYPH_DIM)
            .map(|(i, _)| i)
            .unwrap();
        assert!(start < t.find("Kimi 2.6").unwrap(), "{t}");
        let cubes: String = t[start..]
            .chars()
            .take(WORKING_BLOCKS)
            .collect();
        assert!(
            cubes
                .chars()
                .all(|c| c == WORKING_GLYPH_LIT || c == WORKING_GLYPH_DIM),
            "cubes should be adjacent: {t}"
        );
        assert_eq!(cubes.chars().count(), WORKING_BLOCKS, "{t}");
    }

    #[test]
    fn footer_draw_shows_working_near_model() {
        use comb::{render, Size};

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::AssistantTextDelta("hi".into()));
        a.spinner = 2;

        let buf = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        let text = buf.text();
        assert!(!text.contains("Working"), "{text}");
        assert!(text.contains("Kimi 2.6"), "{text}");
        assert_eq!(working_glyph_count(&text), WORKING_BLOCKS, "{text}");
    }
}
