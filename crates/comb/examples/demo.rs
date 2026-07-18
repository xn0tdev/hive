//! A showcase for `comb`: menus, scroll zones, overlays, spinners, tabs, toasts,
//! a text input, and a sparkline — all over the terminal's own background.
//!
//! Run: `cargo run -p comb --example demo`
//! Keys: ↑/↓ · enter · tab focuses input · ctrl+tab cycles tabs · q quits.
//! Mouse: wheel scrolls hovered zone · click tabs · right-click context menu.

use std::time::{Duration, Instant};

use comb::effects::{bar, shimmer, sparkline, SPINNERS};
use comb::{
    Border, Color, Event, KeyCode, Line, Menu, MouseButton, MouseKind, MouseMode, Palette,
    Rect, ScrollView, Span, Style, Tabs, Terminal, TextInput, Toasts,
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
        ["New", "Open", "Save", "Copy", "Paste", "Find", "Settings", "Quit"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );
    let mut log = ScrollView::new(
        (1..=40)
            .map(|i| {
                Line::from(vec![
                    Span::styled(format!(" {i:>2}  "), faint),
                    Span::styled(format!("#{i:03}  "), dim),
                    Span::styled("ok".to_string(), faint),
                ])
            })
            .collect(),
    );
    let mut ctx = Menu::new(
        ["Copy", "Paste", "Delete"].iter().map(|s| s.to_string()).collect(),
    );
    let mut tabs = Tabs::new(
        ["widgets", "metrics", "about"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );
    let mut input = TextInput::with_placeholder("type a toast message…");
    let mut toasts = Toasts::default();

    let mut ctx_at: Option<(u16, u16)> = None;
    let mut input_focus = false;
    let mut status = String::from("ready");
    let mut quit = false;

    while !quit {
        toasts.tick();
        let phase = (start.elapsed().as_millis() / 90) as usize;
        let size = term.size();
        let caret_on = phase % 16 < 10;

        let menu_panel = Rect::new(2, 3, 28, 12);
        let menu_inner = Rect::new(3, 4, 26, 10);
        let log_panel = Rect::new(32, 3, 40, 12);
        let log_inner = Rect::new(33, 4, 38, 10);
        let spin_w = size.width.saturating_sub(76).min(18);
        let spin_panel = Rect::new(74, 3, spin_w, 12);

        let content_w = size.width.saturating_sub(4).min(70);
        let footer_rows = 2u16;
        let input_h = 3u16;
        let tab_body_h = 5u16;
        let tab_bar_h = 2u16;
        let bottom = size.height.saturating_sub(footer_rows);
        let input_y = bottom.saturating_sub(input_h);
        let tab_body_y = input_y.saturating_sub(tab_body_h);
        let tab_bar_y = tab_body_y.saturating_sub(tab_bar_h);

        let tab_bar = Rect::new(2, tab_bar_y, content_w, tab_bar_h);
        let tab_body = Rect::new(2, tab_body_y, content_w, tab_body_h);
        let input_area = Rect::new(2, input_y, content_w.min(52), input_h);

        term.draw(|f| {
            let bright = Style::new().fg(rgb(0xe6, 0xe6, 0xe6)).bold();
            let buf = f.buffer();

            buf.set_line(
                2,
                0,
                &Line::from(shimmer("comb — a native TUI engine", phase)),
                size.width.saturating_sub(4),
            );
            buf.set_str(2, 1, "menus · scroll · tabs · toast · input · sparkline", faint);

            buf.border_title(menu_panel, Border::Rounded, pal.border, &Line::from(Span::styled("menu", bright)));
            menu.render(buf, menu_inner, &pal);

            buf.border_title(log_panel, Border::Rounded, pal.border, &Line::from(Span::styled("log", bright)));
            log.render(buf, log_inner, &pal);

            if spin_panel.width >= 8 {
                buf.border_title(spin_panel, Border::Rounded, pal.border, &Line::from(Span::styled("spin", bright)));
                for (i, sp) in SPINNERS.iter().take(8).enumerate() {
                    let y = spin_panel.y + 1 + i as u16;
                    if y >= spin_panel.bottom() - 1 {
                        break;
                    }
                    buf.set_str(spin_panel.x + 1, y, sp.frame(phase), bright);
                    buf.set_str(spin_panel.x + 3, y, sp.name, dim);
                }
            }

            tabs.render(buf, tab_bar, &pal);
            buf.border(tab_body, Border::Rounded, pal.border);
            match tabs.selected {
                0 => {
                    let lines = [
                        " List — selectable menu rows + scrollbar",
                        " ScrollView — wheel-scroll zone",
                        " Menu — right-click overlay layer",
                        " Tabs / Toast / TextInput — this demo",
                    ];
                    for (i, l) in lines.iter().enumerate() {
                        buf.set_str(tab_body.x + 2, tab_body.y + 1 + i as u16, l, dim);
                    }
                }
                1 => {
                    let t = start.elapsed().as_secs_f32();
                    let values: Vec<f32> = (0..48)
                        .map(|i| {
                            let x = i as f32 / 8.0 + t;
                            (x.sin() * 0.4 + 0.5).clamp(0.05, 0.95)
                        })
                        .collect();
                    buf.set_str(tab_body.x + 2, tab_body.y + 1, "throughput (live)", dim);
                    let w = tab_body.width.saturating_sub(4) as usize;
                    let mut line = Line::new();
                    for s in sparkline(&values, w, Style::new().fg(rgb(0xc8, 0xc8, 0xc8))) {
                        line.push(s);
                    }
                    buf.set_line(tab_body.x + 2, tab_body.y + 3, &line, w as u16);
                }
                _ => {
                    buf.set_str(tab_body.x + 2, tab_body.y + 1, "comb — surfaces, layers, diffed ANSI", dim);
                    buf.set_str(tab_body.x + 2, tab_body.y + 2, "core · draw · term · widgets", faint);
                }
            }

            let bw = 36usize;
            let frac = (start.elapsed().as_millis() % 4000) as f32 / 4000.0;
            let mut pb = Line::new();
            for s in bar(frac, bw, Style::new().fg(rgb(0xd4, 0xd4, 0xd4)), Style::new().fg(rgb(0x30, 0x30, 0x30))) {
                pb.push(s);
            }
            buf.set_line(
                tab_body.x + 2,
                tab_body.y + tab_body.height.saturating_sub(2),
                &pb,
                bw as u16,
            );

            let caret = input.render(buf, input_area, &pal, input_focus && caret_on);

            buf.set_str(2, size.height.saturating_sub(2), &format!("status: {status}"), dim);
            let hint = if input_focus {
                "input focused · enter sends toast · tab unfocus · ctrl+tab next tab"
            } else {
                "↑/↓ menu · wheel scrolls zone · right-click menu · tab → input · q quit"
            };
            buf.set_str(2, size.height.saturating_sub(1), hint, faint);

            if let Some((cx, cy)) = ctx_at {
                ctx.render(f, cx, cy, 100, &ctx_pal);
            }
            toasts.render(f, 150, &pal);
            if let Some((cx, cy)) = caret {
                f.set_cursor(cx, cy);
            }
        })?;

        let mut first = true;
        loop {
            let ms = if first { 40 } else { 0 };
            let Some(ev) = term.read_event(Duration::from_millis(ms))? else {
                break;
            };
            first = false;

            if input_focus {
                match handle_input_key(&mut input, &mut toasts, &mut status, &ev) {
                    InputResult::Quit => {
                        quit = true;
                        break;
                    }
                    InputResult::Unfocus => input_focus = false,
                    InputResult::Stay => {}
                }
                continue;
            }

            match ev {
                Event::Key(k) => match k.code {
                    KeyCode::Char('q') => quit = true,
                    KeyCode::Char('c') if k.mods.ctrl => quit = true,
                    KeyCode::Tab if k.mods.ctrl && k.mods.shift => tabs.select_prev(),
                    KeyCode::Tab if k.mods.ctrl => tabs.select_next(),
                    KeyCode::Tab => input_focus = true,
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
                            let msg = ctx.list.selected_item().unwrap_or("").to_string();
                            status = format!("action: {msg}");
                            toasts.show(format!("{msg}"), Duration::from_secs(2));
                            ctx_at = None;
                        } else {
                            let msg = menu.selected_item().unwrap_or("").to_string();
                            status = format!("ran: {msg}");
                            toasts.show(format!("ran {msg}"), Duration::from_secs(2));
                        }
                    }
                    _ => {}
                },
                Event::Mouse(m) => match m.kind {
                    MouseKind::Down(MouseButton::Left) => {
                        if let Some(idx) = tabs.index_at(tab_bar, m.col, m.row) {
                            tabs.select(idx);
                        } else if let Some((cx, cy)) = ctx_at {
                            if let Some(idx) = ctx.index_at(cx, cy, m.col, m.row) {
                                ctx.list.select(idx);
                                let msg = ctx.list.selected_item().unwrap_or("").to_string();
                                status = format!("action: {msg}");
                                toasts.show(msg.clone(), Duration::from_secs(2));
                            }
                            ctx_at = None;
                        } else if menu_inner.contains(m.col, m.row) {
                            if let Some(idx) = menu.index_at(menu_inner, m.row) {
                                menu.select(idx);
                            }
                        } else if input_area.contains(m.col, m.row) {
                            input_focus = true;
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

fn handle_input_key(
    input: &mut TextInput,
    toasts: &mut Toasts,
    status: &mut String,
    ev: &Event,
) -> InputResult {
    let Event::Key(k) = ev else {
        return InputResult::Stay;
    };
    match k.code {
        KeyCode::Char('q') if k.mods.ctrl => InputResult::Quit,
        KeyCode::Esc | KeyCode::Tab => InputResult::Unfocus,
        KeyCode::Enter => {
            let text = input.take();
            if !text.trim().is_empty() {
                *status = format!("toast: {text}");
                toasts.show(text, Duration::from_secs(2));
            }
            InputResult::Stay
        }
        KeyCode::Char(c) if !k.mods.ctrl => {
            input.insert(c);
            InputResult::Stay
        }
        KeyCode::Backspace => {
            input.backspace();
            InputResult::Stay
        }
        KeyCode::Left => {
            input.left();
            InputResult::Stay
        }
        KeyCode::Right => {
            input.right();
            InputResult::Stay
        }
        _ => InputResult::Stay,
    }
}

enum InputResult {
    Stay,
    Unfocus,
    Quit,
}
