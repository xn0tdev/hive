//! Footer content: `model · context` and the working directory. Exposed as
//! reusable lines so both the bottom bar and the centered landing can use them.
//! MAKE/PLAN chip sits on the model row (right), under the input strip.
//! While a turn is running, a compact 5-cube ping-pong wave sits just left of
//! the mode chip (or flush-right when there is no chip) — not next to the model.

use comb::{Buffer, Color, Frame, Line, Rect, Span, Style};
use hive_core::AgentMode;

use crate::render::input_bars;

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
/// Gap between the cubes and the mode chip (columns).
const WORKING_CHIP_GAP: u16 = 1;

/// `model · context · $cost` as a single line (no activity chrome).
/// Session spend lives in the project sidebar.
pub fn model_line(app: &crate::app::App) -> Line {
    let theme = &app.theme;
    let mut spans = vec![
        Span::styled(app.model_display.clone(), Style::default().fg(theme.dim)),
        Span::styled(" · ", Style::default().fg(theme.faint)),
        Span::styled(
            format_context(app.context_tokens, app.context_window),
            Style::default().fg(theme.faint),
        ),
    ];
    let cost = session_cost(app);
    if !cost.is_empty() {
        spans.push(Span::styled(" · ", Style::default().fg(theme.faint)));
        spans.push(Span::styled(cost, Style::default().fg(theme.faint)));
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
    draw_working(buf, Rect::new(area.x, area.y, area.width, 1), app, 0);
    buf.set_line(area.x, area.y + 1, &cwd_line(app), area.width);
}

/// Model + cwd under the input; MAKE/PLAN on the model row (right).
/// Working cubes sit just left of the chip. Toasts are drawn separately
/// (centered), so they never replace the chip.
pub fn draw_with_mode(f: &mut Frame, area: Rect, app: &crate::app::App) {
    if area.height < 2 {
        return;
    }
    f.buffer().paint(area, Style::default());
    f.buffer()
        .set_line(area.x, area.y, &model_line(app), area.width);
    let chip_w = mode_chip_width(app);
    draw_working(
        f.buffer(),
        Rect::new(area.x, area.y, area.width, 1),
        app,
        chip_w,
    );
    input_bars::draw_mode_chip(f, Rect::new(area.x, area.y, area.width, 1), app);
    f.buffer()
        .set_line(area.x, area.y + 1, &cwd_line(app), area.width);
}

/// Soft ping-pong cubes on the model row, right-aligned (left of an optional chip).
fn draw_working(buf: &mut Buffer, area: Rect, app: &crate::app::App, chip_w: u16) {
    if !app.running || area.width == 0 || area.height == 0 {
        return;
    }
    let line = Line::from(working_spans(app.spinner, &app.theme));
    let w = line.width() as u16;
    let reserve = chip_w.saturating_add(if chip_w > 0 { WORKING_CHIP_GAP } else { 0 });
    if area.width < w.saturating_add(reserve) {
        return;
    }
    let x = area.x + area.width.saturating_sub(w + reserve);
    buf.set_line(x, area.y, &line, w);
}

fn mode_chip_width(app: &crate::app::App) -> u16 {
    let label = match app.agent_mode {
        AgentMode::Plan => " PLAN ",
        AgentMode::Make => " MAKE ",
        AgentMode::Multitask => " MULTITASK ",
    };
    label.chars().count() as u16
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
    let peak = if phase <= max_i { phase } else { cycle - phase };

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

/// `used / window` for the context meter in the footer.
fn format_context(used: u64, window: u64) -> String {
    let window = window.max(1);
    format!("{} / {}", short_tokens(used), short_tokens(window))
}

/// Session cost in USD based on cumulative token usage and model pricing.
/// Returns empty string when pricing is unknown (0 for both rates).
fn session_cost(app: &crate::app::App) -> String {
    if app.cost_input == 0.0 && app.cost_output == 0.0 {
        return String::new();
    }
    let input_cost = app.usage.prompt_tokens as f64 * app.cost_input / 1_000_000.0;
    let output_cost = app.usage.completion_tokens as f64 * app.cost_output / 1_000_000.0;
    let total = input_cost + output_cost;
    if total < 0.01 {
        format!("${:.4}", total)
    } else {
        format!("${:.2}", total)
    }
}

fn short_tokens(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

fn tilde(path: &str) -> String {
    let home = std::env::var("HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|s| !s.is_empty()));
    match home {
        Some(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
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

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_str()).collect()
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
        assert!(
            (peak_at(t_mid) - 2.0).abs() < 0.2,
            "peak={}",
            peak_at(t_mid)
        );
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
        assert!(
            !text.contains('\u{3000}'),
            "unexpected ideographic space: {text:?}"
        );
    }

    #[test]
    fn model_line_stays_clean_when_idle_or_running() {
        let mut a = app();
        let idle = line_text(&model_line(&a));
        assert_eq!(idle, "Kimi 2.6 · 0 / 128.0k", "{idle}");
        assert_eq!(working_glyph_count(&idle), 0, "{idle}");

        a.apply(AgentEvent::TurnStarted);
        a.spinner = 4;
        let running = line_text(&model_line(&a));
        assert_eq!(running, "Kimi 2.6 · 0 / 128.0k", "{running}");
        assert!(!running.contains("Working"), "{running}");
        assert_eq!(working_glyph_count(&running), 0, "{running}");
    }

    #[test]
    fn footer_draw_shows_working_near_mode_chip() {
        use comb::{render, Size};

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::AssistantTextDelta("hi".into()));
        a.spinner = 2;

        let buf = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        let text = buf.text();
        assert!(!text.contains("Working"), "{text}");
        assert!(text.contains("Kimi 2.6"), "{text}");
        assert!(text.contains("MAKE"), "{text}");
        assert_eq!(working_glyph_count(&text), WORKING_BLOCKS, "{text}");

        // Cubes sit on the model row between the model name and the chip.
        let model_row = text
            .lines()
            .find(|l| l.contains("Kimi 2.6") && l.contains("MAKE"))
            .unwrap_or("");
        let model_i = model_row.find("Kimi 2.6").unwrap();
        let cube_i = model_row
            .char_indices()
            .find(|(_, c)| *c == WORKING_GLYPH_LIT || *c == WORKING_GLYPH_DIM)
            .map(|(i, _)| i)
            .expect("cubes on model row");
        let make_i = model_row.find("MAKE").unwrap();
        assert!(model_i < cube_i, "model left of cubes: {model_row:?}");
        assert!(cube_i < make_i, "cubes left of chip: {model_row:?}");
        let cubes: String = model_row[cube_i..].chars().take(WORKING_BLOCKS).collect();
        assert!(
            cubes
                .chars()
                .all(|c| c == WORKING_GLYPH_LIT || c == WORKING_GLYPH_DIM),
            "cubes should be adjacent: {model_row:?}"
        );
    }

    #[test]
    fn footer_hides_working_when_idle() {
        use comb::{render, Size};

        let mut a = app();
        a.apply(AgentEvent::TurnStarted);
        a.apply(AgentEvent::AssistantTextDelta("hi".into()));
        a.apply(AgentEvent::TurnFinished);

        let buf = render(Size::new(80, 24), |f| crate::render::draw(f, &mut a));
        let text = buf.text();
        assert!(text.contains("Kimi 2.6"), "{text}");
        assert_eq!(working_glyph_count(&text), 0, "{text}");
    }
}
