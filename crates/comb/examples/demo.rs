//! A showcase for `comb`: a selectable menu list and a scrollable log — each its
//! own wheel-scroll zone — plus a spinner gallery, a progress bar, and a
//! right-click context-menu overlay. All over the terminal's own background.
//!
//! Run in a real terminal: `cargo run -p comb --example demo`
//! Keys: ↑/↓ move · enter runs · right-click for a context menu · esc/q quit.
//! Mouse: wheel over the menu or the log scrolls that zone · click to select.

use std::time::{Duration, Instant};

use comb::effects::{bar, shimmer, SPINNERS};
use comb::{
    Border, Color, Event, KeyCode, Line, Menu, MouseButton, MouseKind, MouseMode, Palette, Rect,
    ScrollView, Span, Style, Terminal,
};
use comb::widgets::List;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r, g, b)
}

fn main() -> std::io::Result<()> {
    let mut term = Terminal::new()?;
    term.mouse_mode(MouseMode::Motion)?;
    let start = Instant::now();

    let faint = Style::new().fg(rgb(0x63, 0x63, 0x63));
    let dim = Style::new().fg(rgb(0x9c, 0x9c, 0x9c));

    // Palettes: one for panels over the terminal bg, one for the opaque overlay.
    let pal = Palette {
        panel: Style::new().bg(rgb(0x22, 0x22, 0x22)),
        border: Style::new().fg(rgb(0x6a, 0x6a, 0x6a)),
        normal: Style::new().fg(rgb(0xb8, 0xb8, 0xb8)),
        selected: Style::new().fg(rgb(0x12, 0x12, 0x12)).bg(rgb(0xd8, 0xd8, 0xd8)).bold(),
        track: Style::new().fg(rgb(0x30, 0x30, 0x30)),
        thumb: Style::new().fg(rgb(0x86, 0x86, 0x86)),
    };
    let ctx_pal = Palette {
        normal: Style::new().fg(rgb(0xc4, 0xc4, 0xc4)).bg(rgb(0x2a, 0x2a, 0x2a)),
        panel: Style::new().bg(rgb(0x2a, 0x2a, 0x2a)),
        ..pal
    };

    let mut menu = List::new(
        [
            "New file", "Open…", "Save", "Save as…", "Close", "Undo", "Redo", "Cut", "Copy",
            "Paste", "Find", "Replace", "Settings", "Toggle theme", "Quit",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    );

    let mut log = ScrollView::new(
        (1..=40)
            .map(|i| {
                Line::from(vec![
                    Span::styled(format!(" {i:>2}  "), faint),
                    Span::styled(format!("event #{i:03}  "), dim),
                    Span::styled("processed batch, all good".to_string(), faint),
                ])
            })
            .collect(),
    );

    let mut ctx = Menu::new(
        ["Copy", "Paste", "Rename", "Delete", "Properties"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );
    let mut ctx_at: Option<(u16, u16)> = None;
    let mut status = String::from("ready");
    let mut quit = false;

    while !quit {
        let phase = (start.elapsed().as_millis() / 90) as usize;
        let size = term.size();

        let menu_panel = Rect::new(2, 3, 30, 14);
        let menu_inner = Rect::new(3, 4, 28, 12);
        let log_panel = Rect::new(34, 3, 44, 14);
        let log_inner = Rect::new(35, 4, 42, 12);
        let spin_panel = Rect::new(80, 3, size.width.saturating_sub(82).min(20), 14);

        term.draw(|f| {
            let bright = Style::new().fg(rgb(0xe6, 0xe6, 0xe6)).bold();
            let buf = f.buffer();

            buf.set_line(
                2,
                0,
                &Line::from(shimmer("comb — a native TUI engine", phase)),
                size.width.saturating_sub(4),
            );
            buf.set_str(2, 1, "menus · scroll zones · wheel · overlays · spinners", faint);

            // Menu (selectable, scrollable).
            buf.border_title(menu_panel, Border::Rounded, pal.border, &Line::from(Span::styled("menu", bright)));
            menu.render(buf, menu_inner, &pal);

            // Log (scrollable zone with a scrollbar).
            buf.border_title(log_panel, Border::Rounded, pal.border, &Line::from(Span::styled("log — wheel to scroll", bright)));
            log.render(buf, log_inner, &pal);

            // Spinner gallery.
            if spin_panel.width >= 10 {
                buf.border_title(spin_panel, Border::Rounded, pal.border, &Line::from(Span::styled("spinners", bright)));
                for (i, sp) in SPINNERS.iter().enumerate() {
                    let y = spin_panel.y + 1 + i as u16;
                    if y >= spin_panel.bottom() - 1 {
                        break;
                    }
                    buf.set_str(spin_panel.x + 2, y, sp.frame(phase), bright);
                    buf.set_str(spin_panel.x + 5, y, sp.name, dim);
                }
            }

            // Progress bar.
            let by = menu_panel.bottom() + 1;
            let bw = 42usize;
            let frac = (start.elapsed().as_millis() % 4000) as f32 / 4000.0;
            buf.set_str(2, by, "progress", dim);
            let mut pb = Line::new();
            for s in bar(frac, bw, Style::new().fg(rgb(0xd4, 0xd4, 0xd4)), Style::new().fg(rgb(0x30, 0x30, 0x30))) {
                pb.push(s);
            }
            buf.set_line(2, by + 1, &pb, bw as u16);
            buf.set_str(2 + bw as u16 + 2, by + 1, &format!("{:>3.0}%", frac * 100.0), dim);

            // Status + footer.
            buf.set_str(2, size.height.saturating_sub(2), &format!("status: {status}"), dim);
            buf.set_str(
                2,
                size.height.saturating_sub(1),
                "↑/↓ select · enter run · right-click menu · wheel scrolls the hovered zone · q quits",
                faint,
            );

            // Context menu overlay (drawn last, floats on top).
            if let Some((cx, cy)) = ctx_at {
                ctx.render(f, cx, cy, 100, &ctx_pal);
            }
        })?;

        // Drain input each frame (motion floods), blocking up to 40ms when idle.
        let mut first = true;
        loop {
            let ms = if first { 40 } else { 0 };
            let Some(ev) = term.read_event(Duration::from_millis(ms))? else {
                break;
            };
            first = false;
            match ev {
                Event::Key(k) => match k.code {
                    KeyCode::Char('q') => quit = true,
                    KeyCode::Char('c') if k.mods.ctrl => quit = true,
                    KeyCode::Esc => {
                        if ctx_at.is_some() {
                            ctx_at = None;
                        } else {
                            quit = true;
                        }
                    }
                    KeyCode::Up => {
                        if ctx_at.is_some() {
                            ctx.list.select_up();
                        } else {
                            menu.select_up();
                        }
                    }
                    KeyCode::Down => {
                        if ctx_at.is_some() {
                            ctx.list.select_down();
                        } else {
                            menu.select_down();
                        }
                    }
                    KeyCode::Enter => {
                        if ctx_at.is_some() {
                            status = format!("action: {}", ctx.list.selected_item().unwrap_or(""));
                            ctx_at = None;
                        } else {
                            status = format!("ran: {}", menu.selected_item().unwrap_or(""));
                        }
                    }
                    _ => {}
                },
                Event::Mouse(m) => match m.kind {
                    MouseKind::Down(MouseButton::Left) => {
                        if let Some((cx, cy)) = ctx_at {
                            if let Some(idx) = ctx.index_at(cx, cy, m.col, m.row) {
                                ctx.list.select(idx);
                                status = format!("action: {}", ctx.list.selected_item().unwrap_or(""));
                            }
                            ctx_at = None;
                        } else if let Some(idx) = menu.index_at(menu_inner, m.row) {
                            if menu_inner.contains(m.col, m.row) {
                                menu.select(idx);
                                status = format!("selected: {}", menu.selected_item().unwrap_or(""));
                            }
                        }
                    }
                    MouseKind::Down(MouseButton::Right) => {
                        ctx.list.selected = 0;
                        ctx.list.offset = 0;
                        ctx_at = Some((m.col, m.row));
                    }
                    MouseKind::ScrollUp => {
                        if ctx_at.is_some() {
                            ctx.list.select_up();
                        } else if menu_inner.contains(m.col, m.row) {
                            menu.select_up();
                        } else if log_inner.contains(m.col, m.row) {
                            log.scroll_by(-3);
                        }
                    }
                    MouseKind::ScrollDown => {
                        if ctx_at.is_some() {
                            ctx.list.select_down();
                        } else if menu_inner.contains(m.col, m.row) {
                            menu.select_down();
                        } else if log_inner.contains(m.col, m.row) {
                            log.scroll_by(3);
                        }
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {}
            }
        }
    }
    Ok(())
}
