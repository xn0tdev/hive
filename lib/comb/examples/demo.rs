//! A showcase for `comb`: draggable windows, Code/Diff editor, scroll zones,
//! spinners, toasts, and a text input — over the terminal's own background.
//!
//! Run: `cargo run -p comb --example demo`
//! Keys: ↑/↓ · enter · ctrl+tab tabs · b code bg · d Code/Diff · q quit.
//! Mouse: drag title bars to move · drag edges to resize · wheel scrolls.

use std::time::{Duration, Instant};

use comb::effects::{shimmer, sparkline, SPINNERS};
use comb::highlight::Lang;
use comb::{
    CodeBlock, Color, DiffView, Event, KeyCode, Line, Menu, MouseButton, MouseKind, MouseMode,
    Palette, Rect, ScrollView, ScrollbarStyle, Span, Style, Tabs, Terminal, TextInput, Toasts,
    Window,
};
use comb::widgets::List;

const SAMPLE_RUST: &str = r#"use comb::{CodeBlock, Lang};

pub fn run(items: &[Item]) -> Result<()> {
    // drag the title bar to move this window
    // drag edges / corner to resize
    let mut block = CodeBlock::new(source, Lang::Rust);
    for item in items {
        process(item)?;
    }
    block.render(buf, area, None);
    Ok(())
}

fn process(item: &Item) -> Result<()> {
    let name = item.name();
    println!("ok: {name}");
    Ok(())
}"#;

const SAMPLE_RUST_OLD: &str = r#"use comb::{CodeBlock, Lang};

pub fn run(items: &[Item]) -> Result<()> {
    let mut block = CodeBlock::new(source, Lang::Rust);
    for item in items {
        process(item)?;
    }
    block.render(buf, area, Some(panel));
    Ok(())
}

fn process(item: &Item) -> Result<()> {
    println!("ok");
    Ok(())
}"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum WinId {
    Menu,
    Log,
    Spin,
    Editor,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditorMode {
    Code,
    Diff,
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r, g, b)
}

fn bring_front(order: &mut Vec<WinId>, id: WinId) {
    order.retain(|w| *w != id);
    order.push(id);
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
        accent: Style::new().fg(rgb(0xe6, 0xe6, 0xe6)),
        selected: Style::new()
            .fg(rgb(0x12, 0x12, 0x12))
            .bg(rgb(0xd8, 0xd8, 0xd8))
            .bold(),
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
    log.scrollbar.style = ScrollbarStyle {
        track_glyph: '░',
        thumb_glyph: '▐',
        track: Style::new().fg(rgb(0x40, 0x40, 0x40)).bg(rgb(0x22, 0x22, 0x22)),
        thumb: Style::new().fg(rgb(0x72, 0x72, 0x72)).bg(rgb(0x22, 0x22, 0x22)),
        thumb_active: Style::new()
            .fg(rgb(0xcc, 0xcc, 0xcc))
            .bg(rgb(0x22, 0x22, 0x22))
            .bold(),
        ..Default::default()
    };
    let mut ctx = Menu::new(
        ["Copy", "Paste", "Delete"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );

    let mut code = CodeBlock::new(SAMPLE_RUST, Lang::Rust);
    code.scrollbar.style = ScrollbarStyle {
        track_glyph: '▕',
        thumb_glyph: '█',
        width: 1,
        track: Style::new().fg(rgb(0x35, 0x35, 0x35)).bg(rgb(0x22, 0x22, 0x22)),
        thumb: Style::new().fg(rgb(0x88, 0x88, 0x88)).bg(rgb(0x22, 0x22, 0x22)),
        thumb_active: Style::new().fg(rgb(0xee, 0xee, 0xee)).bg(rgb(0x22, 0x22, 0x22)),
    };
    let mut diff = DiffView::new(SAMPLE_RUST_OLD, SAMPLE_RUST);
    diff.scrollbar.style = code.scrollbar.style;

    let mut mode_tabs = Tabs::new(
        ["Code", "Diff"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );
    let mut editor_mode = EditorMode::Code;

    let mut win_menu = Window::new("menu", 2, 3, 28, 10);
    let mut win_log = Window::new("log — drag ▐", 32, 3, 40, 10);
    let mut win_spin = Window::new("spin", 74, 3, 18, 10);
    let mut win_editor = Window::new("editor", 2, 14, 72, 18);
    win_menu.fill = pal.panel;
    win_log.fill = pal.panel;
    win_spin.fill = pal.panel;
    win_editor.fill = pal.panel;
    win_editor.min_w = 28;
    win_editor.min_h = 8;

    let mut zorder = vec![WinId::Spin, WinId::Menu, WinId::Log, WinId::Editor];
    let mut focused = WinId::Editor;

    let mut input = TextInput::with_placeholder("type a toast message…");
    let mut toasts = Toasts::default();

    let mut ctx_at: Option<(u16, u16)> = None;
    let mut input_focus = false;
    let mut status = String::from("ready");
    let mut quit = false;
    let mut code_fill = false;

    while !quit {
        toasts.tick();
        let phase = (start.elapsed().as_millis() / 90) as usize;
        let size = term.size();

        let footer_rows = 2u16;
        let input_h = 3u16;
        let input_y = size.height.saturating_sub(footer_rows + input_h);
        let input_area = Rect::new(2, input_y, size.width.saturating_sub(4).min(52), input_h);
        let workspace = Rect::new(0, 2, size.width, input_y.saturating_sub(2));

        for w in [
            &mut win_menu,
            &mut win_log,
            &mut win_spin,
            &mut win_editor,
        ] {
            w.clamp_to(workspace);
        }

        win_editor.title = match editor_mode {
            EditorMode::Code => "editor · Code".into(),
            EditorMode::Diff => "editor · Diff".into(),
        };
        mode_tabs.selected = match editor_mode {
            EditorMode::Code => 0,
            EditorMode::Diff => 1,
        };

        term.draw(|f| {
            let bright = Style::new().fg(rgb(0xe6, 0xe6, 0xe6)).bold();
            let buf = f.buffer();

            buf.set_line(
                2,
                0,
                &Line::from(shimmer("comb — drag windows · Code / Diff", phase)),
                size.width.saturating_sub(4),
            );
            buf.set_str(
                2,
                1,
                "title-bar move · edge resize · Code/Diff toggle · b fill",
                faint,
            );

            for id in &zorder {
                let focused_win = *id == focused;
                match id {
                    WinId::Menu => {
                        win_menu.render_chrome(buf, &pal, focused_win);
                        menu.render(buf, win_menu.inner(), &pal);
                    }
                    WinId::Log => {
                        win_log.render_chrome(buf, &pal, focused_win);
                        log.render(buf, win_log.inner(), &pal);
                    }
                    WinId::Spin => {
                        win_spin.render_chrome(buf, &pal, focused_win);
                        let inner = win_spin.inner();
                        let on = |st: Style| st.patch(pal.panel);
                        for (i, sp) in SPINNERS.iter().take(8).enumerate() {
                            let y = inner.y + i as u16;
                            if y >= inner.bottom() {
                                break;
                            }
                            buf.set_str(inner.x, y, sp.frame(phase), on(bright));
                            buf.set_str(inner.x + 2, y, sp.name, on(dim));
                        }
                    }
                    WinId::Editor => {
                        win_editor.render_chrome(buf, &pal, focused_win);
                        let inner = win_editor.inner();
                        if inner.height < 3 {
                            continue;
                        }
                        let mode_bar = Rect::new(inner.x, inner.y, inner.width, 2);
                        mode_tabs.render(buf, mode_bar, &pal);
                        let body = Rect::new(
                            inner.x,
                            inner.y + 2,
                            inner.width,
                            inner.height.saturating_sub(2),
                        );
                        // Always opaque inside a stacked window; `b` toggles a
                        // slightly lifted panel vs the window chrome fill.
                        let bg = Some(if code_fill {
                            Style::new().bg(rgb(0x28, 0x28, 0x28))
                        } else {
                            win_editor.fill
                        });
                        match editor_mode {
                            EditorMode::Code => code.render(buf, body, bg),
                            EditorMode::Diff => diff.render(buf, body, bg),
                        }
                    }
                }
            }

            let spark_x = input_area.right().saturating_add(2);
            if spark_x + 20 < size.width {
                let t = start.elapsed().as_secs_f32();
                let values: Vec<f32> = (0..16)
                    .map(|i| {
                        let x = i as f32 / 4.0 + t;
                        (x.sin() * 0.4 + 0.5).clamp(0.05, 0.95)
                    })
                    .collect();
                let mut line = Line::new();
                for s in sparkline(&values, 16, Style::new().fg(rgb(0xc8, 0xc8, 0xc8))) {
                    line.push(s);
                }
                buf.set_line(spark_x, input_y + 1, &line, 16);
            }

            let input_cursor = input.render(buf, input_area, &pal);

            let mode = match editor_mode {
                EditorMode::Code => "Code",
                EditorMode::Diff => "Diff",
            };
            let fill = if code_fill { "fill" } else { "plain" };
            buf.set_str(
                2,
                size.height.saturating_sub(2),
                &format!(
                    "status: {status} · {mode} · bg:{fill} · editor {}×{}",
                    win_editor.width, win_editor.height
                ),
                dim,
            );
            let hint = if input_focus {
                "input focused · enter toast · tab unfocus"
            } else {
                "drag title · resize edges · d Code/Diff · b fill · q quit"
            };
            buf.set_str(2, size.height.saturating_sub(1), hint, faint);

            if let Some((cx, cy)) = ctx_at {
                ctx.render(f, cx, cy, 100, &ctx_pal);
            }
            toasts.render(f, 150, &pal);
            if input_focus {
                if let Some((x, y)) = input_cursor {
                    f.set_cursor(x, y);
                }
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
                    KeyCode::Char('b') => {
                        code_fill = !code_fill;
                        status = if code_fill {
                            "code bg: panel fill".into()
                        } else {
                            "code bg: terminal (plain)".into()
                        };
                    }
                    KeyCode::Char('d') => {
                        editor_mode = match editor_mode {
                            EditorMode::Code => EditorMode::Diff,
                            EditorMode::Diff => EditorMode::Code,
                        };
                        status = format!("mode: {editor_mode:?}");
                        bring_front(&mut zorder, WinId::Editor);
                        focused = WinId::Editor;
                    }
                    KeyCode::Tab if k.mods.ctrl && k.mods.shift => {
                        // cycle focus backward
                        if let Some(pos) = zorder.iter().position(|w| *w == focused) {
                            let prev = if pos == 0 { zorder.len() - 1 } else { pos - 1 };
                            focused = zorder[prev];
                            bring_front(&mut zorder, focused);
                        }
                    }
                    KeyCode::Tab if k.mods.ctrl => {
                        if let Some(pos) = zorder.iter().position(|w| *w == focused) {
                            let next = (pos + 1) % zorder.len();
                            focused = zorder[next];
                            bring_front(&mut zorder, focused);
                        }
                    }
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
                            toasts.show(msg, Duration::from_secs(2));
                            ctx_at = None;
                        } else {
                            let msg = menu.selected_item().unwrap_or("").to_string();
                            status = format!("ran: {msg}");
                            toasts.show(format!("ran {msg}"), Duration::from_secs(2));
                        }
                    }
                    _ => {}
                },
                Event::Mouse(m) => {
                    // Active window drag/resize first.
                    let dragging = win_menu.is_dragging()
                        || win_log.is_dragging()
                        || win_spin.is_dragging()
                        || win_editor.is_dragging();
                    if dragging {
                        for id in zorder.iter().rev() {
                            let win = match id {
                                WinId::Menu => &mut win_menu,
                                WinId::Log => &mut win_log,
                                WinId::Spin => &mut win_spin,
                                WinId::Editor => &mut win_editor,
                            };
                            if win.is_dragging() && win.handle_mouse(m.kind, m.col, m.row, workspace)
                            {
                                break;
                            }
                        }
                        if matches!(m.kind, MouseKind::Up) {
                            win_menu.end_drag();
                            win_log.end_drag();
                            win_spin.end_drag();
                            win_editor.end_drag();
                            code.end_drag();
                            diff.end_drag();
                            log.scrollbar.end_drag();
                            menu.scrollbar.end_drag();
                        }
                        continue;
                    }

                    if let MouseKind::Down(MouseButton::Left) = m.kind {
                        // Hit topmost window under cursor.
                        let mut hit: Option<WinId> = None;
                        for id in zorder.iter().rev() {
                            let win = match id {
                                WinId::Menu => &win_menu,
                                WinId::Log => &win_log,
                                WinId::Spin => &win_spin,
                                WinId::Editor => &win_editor,
                            };
                            if win.contains(m.col, m.row) {
                                hit = Some(*id);
                                break;
                            }
                        }
                        if let Some(id) = hit {
                            focused = id;
                            bring_front(&mut zorder, id);
                            let win = match id {
                                WinId::Menu => &mut win_menu,
                                WinId::Log => &mut win_log,
                                WinId::Spin => &mut win_spin,
                                WinId::Editor => &mut win_editor,
                            };
                            if win.handle_mouse(m.kind, m.col, m.row, workspace) {
                                continue;
                            }
                            // Editor mode tabs.
                            if id == WinId::Editor {
                                let inner = win_editor.inner();
                                let mode_bar = Rect::new(inner.x, inner.y, inner.width, 2);
                                if let Some(idx) = mode_tabs.index_at(mode_bar, m.col, m.row) {
                                    editor_mode = if idx == 0 {
                                        EditorMode::Code
                                    } else {
                                        EditorMode::Diff
                                    };
                                    status = format!("mode: {editor_mode:?}");
                                    continue;
                                }
                            }
                        } else if input_area.contains(m.col, m.row) {
                            input_focus = true;
                            continue;
                        }
                    }

                    // Content mouse (scroll / select) on focused or hit window.
                    let editor_inner = win_editor.inner();
                    let mode_bar_h = 2u16;
                    let editor_body = Rect::new(
                        editor_inner.x,
                        editor_inner.y + mode_bar_h,
                        editor_inner.width,
                        editor_inner.height.saturating_sub(mode_bar_h),
                    );

                    if log.handle_mouse(win_log.inner(), m.kind, m.col, m.row)
                        || menu.handle_mouse(win_menu.inner(), m.kind, m.col, m.row)
                        || match editor_mode {
                            EditorMode::Code => {
                                code.handle_mouse(editor_body, m.kind, m.col, m.row)
                            }
                            EditorMode::Diff => {
                                diff.handle_mouse(editor_body, m.kind, m.col, m.row)
                            }
                        }
                    {
                        continue;
                    }

                    match m.kind {
                        MouseKind::Down(MouseButton::Left) => {
                            if let Some((cx, cy)) = ctx_at {
                                if let Some(idx) = ctx.index_at(cx, cy, m.col, m.row) {
                                    ctx.list.select(idx);
                                    let msg = ctx.list.selected_item().unwrap_or("").to_string();
                                    status = format!("action: {msg}");
                                    toasts.show(msg, Duration::from_secs(2));
                                }
                                ctx_at = None;
                            } else if win_menu.inner().contains(m.col, m.row) {
                                if let Some(idx) = menu.index_at(win_menu.inner(), m.row) {
                                    menu.select(idx);
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
                            } else if win_menu.inner().contains(m.col, m.row) {
                                menu.select_up();
                            }
                        }
                        MouseKind::ScrollDown => {
                            if ctx_at.is_some() {
                                ctx.list.select_down();
                            } else if win_menu.inner().contains(m.col, m.row) {
                                menu.select_down();
                            }
                        }
                        MouseKind::Up => {
                            win_menu.end_drag();
                            win_log.end_drag();
                            win_spin.end_drag();
                            win_editor.end_drag();
                            code.end_drag();
                            diff.end_drag();
                            log.scrollbar.end_drag();
                            menu.scrollbar.end_drag();
                        }
                        _ => {}
                    }
                }
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
        KeyCode::Char('c') if k.mods.ctrl => InputResult::Quit,
        KeyCode::Esc | KeyCode::Tab => InputResult::Unfocus,
        KeyCode::Enter => {
            let msg = input.take();
            let msg = msg.trim().to_string();
            if !msg.is_empty() {
                toasts.show(msg.clone(), Duration::from_secs(3));
                *status = format!("toast: {msg}");
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
    Quit,
    Unfocus,
    Stay,
}
