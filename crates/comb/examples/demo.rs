//! A tiny showcase for `comb`: an animated shimmer title, a base scene, and two
//! overlapping overlay surfaces to demonstrate z-ordered layering.
//!
//! Run in a real terminal: `cargo run -p comb --example demo`
//! Keys: arrows move the top popup · space toggles it · q / esc / ctrl-c quit.

use std::time::{Duration, Instant};

use comb::effects::shimmer;
use comb::{Color, Event, KeyCode, MouseKind, Rect, Style, Terminal};

fn main() -> std::io::Result<()> {
    let mut term = Terminal::new()?;
    let start = Instant::now();

    let mut show_popup = true;
    let mut px: u16 = 8;
    let mut py: u16 = 5;

    loop {
        let phase = (start.elapsed().as_millis() / 90) as usize;
        let size = term.size();

        term.draw(|f| {
            let bg = Style::new().bg(Color::rgb(0x14, 0x14, 0x14));
            let fg = Style::new().fg(Color::rgb(0xd4, 0xd4, 0xd4));
            let faint = Style::new().fg(Color::rgb(0x63, 0x63, 0x63));

            // Base scene.
            let root = f.buffer();
            root.paint(Rect::new(0, 0, size.width, size.height), bg);
            let title = shimmer("comb — a native TUI engine", phase);
            root.set_line(2, 1, &comb::Line::from(title), size.width.saturating_sub(4));
            root.set_str(2, 3, "surfaces · layers · effects, no ratatui", faint);
            for i in 0..6u16 {
                root.set_str(
                    2,
                    5 + i,
                    &format!("row {i}: the base layer sits underneath the popups"),
                    fg,
                );
            }
            root.set_str(2, size.height.saturating_sub(1), "arrows move · space toggles · q quits", faint);

            if show_popup {
                // Layer A (z=10): an opaque panel.
                let a = Rect::new(px, py, 34, 7).intersection(f.area());
                let panel = f.layer(a, 10);
                let pbg = Style::new().bg(Color::rgb(0x26, 0x26, 0x26));
                panel.paint(Rect::new(0, 0, a.width, a.height), pbg);
                panel.set_str(2, 1, "Layer A — opaque surface", Style::new().fg(Color::rgb(0xe6, 0xe6, 0xe6)).bg(Color::rgb(0x26, 0x26, 0x26)).bold());
                panel.set_str(2, 3, "It hides the base beneath it.", Style::new().fg(Color::rgb(0xb0, 0xb0, 0xb0)).bg(Color::rgb(0x26, 0x26, 0x26)));

                // Layer B (z=20): overlaps A, higher z wins.
                let b = Rect::new(px + 18, py + 3, 30, 5).intersection(f.area());
                let top = f.layer(b, 20);
                let tbg = Style::new().bg(Color::rgb(0x3a, 0x3a, 0x3a));
                top.paint(Rect::new(0, 0, b.width, b.height), tbg);
                top.set_str(2, 1, "Layer B — on top (z=20)", Style::new().fg(Color::rgb(0xf2, 0xf2, 0xf2)).bg(Color::rgb(0x3a, 0x3a, 0x3a)).bold());
                top.set_str(2, 3, "Composited above Layer A.", Style::new().fg(Color::rgb(0xd0, 0xd0, 0xd0)).bg(Color::rgb(0x3a, 0x3a, 0x3a)));
            }
        })?;

        if let Some(ev) = term.read_event(Duration::from_millis(50))? {
            match ev {
                Event::Key(k) => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c') if k.mods.ctrl => break,
                    KeyCode::Char(' ') => show_popup = !show_popup,
                    KeyCode::Left => px = px.saturating_sub(1),
                    KeyCode::Right => px = (px + 1).min(size.width.saturating_sub(2)),
                    KeyCode::Up => py = py.saturating_sub(1),
                    KeyCode::Down => py = (py + 1).min(size.height.saturating_sub(2)),
                    _ => {}
                },
                Event::Mouse(m) => {
                    if let MouseKind::Down(_) = m.kind {
                        show_popup = !show_popup;
                    }
                }
                Event::Resize(_, _) => {}
            }
        }
    }
    Ok(())
}
