//! A showcase for `comb`: a shimmer title, a gallery of spinners, framed panels
//! in every border style, a mouse-draggable card, a hover-highlighted button,
//! and a progress bar — all over the terminal's own background (no fill).
//!
//! Run in a real terminal: `cargo run -p comb --example demo`
//! Keys: space cycles border style · q / esc / ctrl-c quit.
//! Mouse: drag the card · click the button · hover to highlight.

use std::time::{Duration, Instant};

use comb::effects::{bar, shimmer, SPINNERS};
use comb::{
    Border, Color, Event, KeyCode, Line, MouseButton, MouseKind, MouseMode, Rect, Span, Style,
    Terminal,
};

fn main() -> std::io::Result<()> {
    let mut term = Terminal::new()?;
    term.mouse_mode(MouseMode::Motion)?; // hover + drag
    let start = Instant::now();

    let borders = [Border::Rounded, Border::Plain, Border::Double, Border::Thick];
    let names = ["rounded", "plain", "double", "thick"];
    let mut bidx = 0usize;

    let mut card = Rect::new(42, 8, 30, 8);
    let mut drag: Option<(u16, u16)> = None; // cursor offset within the card
    let mut hover: Option<(u16, u16)> = None;
    let mut clicks = 0u32;
    let mut quit = false;

    while !quit {
        let phase = (start.elapsed().as_millis() / 90) as usize;
        let size = term.size();
        let border = borders[bidx];
        let name = names[bidx];

        let btn = Rect::new(2, size.height.saturating_sub(4), 24, 3);
        let btn_hot = hover.is_some_and(|(x, y)| btn.contains(x, y));
        let card_hot = drag.is_some() || hover.is_some_and(|(x, y)| card.contains(x, y));

        term.draw(|f| {
            let dim = Style::new().fg(Color::rgb(0x63, 0x63, 0x63));
            let label = Style::new().fg(Color::rgb(0x9c, 0x9c, 0x9c));
            let bright = Style::new().fg(Color::rgb(0xe6, 0xe6, 0xe6)).bold();
            let root = f.area();
            let buf = f.buffer();

            // Title + subtitle — no background fill, terminal bg shows through.
            buf.set_line(
                2,
                0,
                &Line::from(shimmer("comb — a native TUI engine", phase)),
                root.width.saturating_sub(4),
            );
            buf.set_str(2, 1, "borders · spinners · drag · hover · progress", dim);

            // Spinner gallery in a framed panel.
            let gal = Rect::new(2, 3, 32, SPINNERS.len() as u16 + 2);
            buf.border_title(
                gal,
                border,
                label,
                &Line::from(Span::styled("spinners", bright)),
            );
            for (i, sp) in SPINNERS.iter().enumerate() {
                let y = gal.y + 1 + i as u16;
                buf.set_str(gal.x + 2, y, sp.frame(phase), bright);
                buf.set_str(gal.x + 5, y, sp.name, label);
            }

            // Progress bar under the gallery (loops every 4s).
            let by = gal.bottom() + 1;
            let bw = 30usize;
            let frac = (start.elapsed().as_millis() % 4000) as f32 / 4000.0;
            buf.set_str(2, by, "progress", label);
            let mut pb = Line::new();
            for s in bar(
                frac,
                bw,
                Style::new().fg(Color::rgb(0xd4, 0xd4, 0xd4)),
                Style::new().fg(Color::rgb(0x33, 0x33, 0x33)),
            ) {
                pb.push(s);
            }
            buf.set_line(2, by + 1, &pb, bw as u16);
            buf.set_str(2 + bw as u16 + 2, by + 1, &format!("{:>3.0}%", frac * 100.0), dim);

            // Draggable card — brightens on hover/drag.
            let cd = card.intersection(root);
            let edge = if card_hot {
                Style::new().fg(Color::rgb(0xe6, 0xe6, 0xe6))
            } else {
                Style::new().fg(Color::rgb(0x6a, 0x6a, 0x6a))
            };
            let fill = Style::new().bg(Color::rgb(0x1e, 0x1e, 0x1e));
            buf.paint(cd, fill);
            buf.border_title(
                cd,
                border,
                edge,
                &Line::from(Span::styled(
                    "drag me",
                    Style::new().fg(Color::rgb(0xe6, 0xe6, 0xe6)).bg(Color::rgb(0x1e, 0x1e, 0x1e)).bold(),
                )),
            );
            let body = Style::new().fg(Color::rgb(0xb0, 0xb0, 0xb0)).bg(Color::rgb(0x1e, 0x1e, 0x1e));
            buf.set_str(card.x + 2, card.y + 2, "drag with the mouse", body);
            buf.set_str(card.x + 2, card.y + 3, "hover to brighten the frame", body);
            buf.set_str(card.x + 2, card.y + 5, &format!("clicks: {clicks}"), body);

            // Hover-highlighted button that cycles the border style.
            let (bfg, bbg) = if btn_hot {
                (Color::rgb(0x14, 0x14, 0x14), Color::rgb(0xd8, 0xd8, 0xd8))
            } else {
                (Color::rgb(0xd4, 0xd4, 0xd4), Color::rgb(0x2e, 0x2e, 0x2e))
            };
            buf.block(btn, Border::Rounded, Style::new().fg(bbg), Some(Style::new().bg(bbg)));
            buf.set_str(
                btn.x + 2,
                btn.y + 1,
                &format!("border: {name}"),
                Style::new().fg(bfg).bg(bbg).bold(),
            );

            buf.set_str(
                2,
                size.height.saturating_sub(1),
                "click button · drag card · space cycles · q quits",
                dim,
            );
        })?;

        // Drain all pending input each frame (hover floods events), blocking up
        // to 40ms when idle so the animation keeps a steady cadence.
        let mut first = true;
        loop {
            let ms = if first { 40 } else { 0 };
            let Some(ev) = term.read_event(Duration::from_millis(ms))? else {
                break;
            };
            first = false;
            match ev {
                Event::Key(k) => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => quit = true,
                    KeyCode::Char('c') if k.mods.ctrl => quit = true,
                    KeyCode::Char(' ') => bidx = (bidx + 1) % borders.len(),
                    _ => {}
                },
                Event::Mouse(m) => match m.kind {
                    MouseKind::Moved => hover = Some((m.col, m.row)),
                    MouseKind::Down(MouseButton::Left) => {
                        if btn.contains(m.col, m.row) {
                            bidx = (bidx + 1) % borders.len();
                            clicks += 1;
                        } else if card.contains(m.col, m.row) {
                            drag = Some((m.col - card.x, m.row - card.y));
                        }
                    }
                    MouseKind::Drag => {
                        if let Some((dx, dy)) = drag {
                            card.x = m.col.saturating_sub(dx).min(size.width.saturating_sub(card.width));
                            card.y = m.row.saturating_sub(dy).min(size.height.saturating_sub(card.height));
                        }
                        hover = Some((m.col, m.row));
                    }
                    MouseKind::Up => drag = None,
                    _ => {}
                },
                Event::Resize(_, _) => {}
            }
        }
    }
    Ok(())
}
