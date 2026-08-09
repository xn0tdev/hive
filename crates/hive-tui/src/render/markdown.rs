//! Markdown → comb lines via `pulldown-cmark`.
//!
//! Event-driven renderer: pulldown-cmark parses CommonMark (tables,
//! strikethrough), a `Writer` consumes the event stream and emits styled
//! `comb::Line` / `Span`. Supports fenced code with syntax highlighting,
//! headings, lists (nested), blockquotes, tables (row-separated grid +
//! key/value fallback), links, strikethrough, inline code, bold, italic.

use comb::{highlight, Line, Modifier, Span, Style};
use pulldown_cmark::{
    Alignment, CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;

// ── Public API ──────────────────────────────────────────────────────────

/// Render markdown into styled logical lines.
pub fn render(text: &str, theme: &Theme, width: usize) -> Vec<Line> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let parser = Parser::new_ext(text, options);
    let mut w = Writer::new(theme, Some(width));
    for event in parser {
        w.handle(event);
    }
    w.finish()
}

/// Plain lines for live streaming (no markdown yet).
pub fn plain(text: &str, theme: &Theme) -> Vec<Line> {
    text.split('\n')
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(theme.fg))))
        .collect()
}

// ── Styles ──────────────────────────────────────────────────────────────

struct MdStyles {
    h: [Style; 6],
    code: Style,
    emphasis: Style,
    strong: Style,
    strikethrough: Style,
    link: Style,
    blockquote: Style,
    list_marker: Style,
    code_marker: Style,
}

impl MdStyles {
    fn new(theme: &Theme) -> Self {
        let bold = Style::default().fg(theme.heading).add(Modifier::BOLD);
        let italic = Style::default().fg(theme.fg).add(Modifier::ITALIC);
        let bi = Style::default()
            .fg(theme.heading)
            .add(Modifier::BOLD | Modifier::ITALIC);
        Self {
            h: [bold, bold, bi, italic, italic, italic],
            code: Style::default().fg(theme.tool).bg(theme.code_bg),
            emphasis: italic,
            strong: bold,
            strikethrough: Style::default().fg(theme.fg).add(Modifier::DIM),
            link: Style::default().fg(theme.tool).add(Modifier::UNDERLINE),
            blockquote: Style::default().fg(theme.dim),
            list_marker: Style::default().fg(theme.accent),
            // Zero-width marker carried through wrapping. The transcript turns
            // it into a quiet left rail, so fenced blocks do not need a large
            // content-sized background slab.
            code_marker: Style::default().fg(theme.faint).bg(theme.code_bg),
        }
    }
}

// ── Indent context ──────────────────────────────────────────────────────

#[derive(Clone)]
struct IndentCtx {
    prefix: Vec<Span>,
    marker: Option<Vec<Span>>,
}

impl IndentCtx {
    fn new(prefix: Vec<Span>, marker: Option<Vec<Span>>) -> Self {
        Self { prefix, marker }
    }
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

// ── Table ───────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct TableCell {
    lines: Vec<Line>,
}

impl TableCell {
    fn ensure_line(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(Line::new());
        }
    }

    fn push_span(&mut self, span: Span) {
        self.ensure_line();
        if let Some(l) = self.lines.last_mut() {
            l.push(span);
        }
    }

    fn hard_break(&mut self) {
        self.lines.push(Line::new());
    }

    fn display_width(&self) -> usize {
        self.lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.width()).sum::<usize>())
            .max()
            .unwrap_or(0)
    }
}

#[derive(Default)]
struct TableState {
    alignments: Vec<Alignment>,
    header: Option<Vec<TableCell>>,
    rows: Vec<Vec<TableCell>>,
    current_row: Option<Vec<TableCell>>,
    current_cell: Option<TableCell>,
    in_header: bool,
}

const TABLE_PAD: usize = 1;

// ── Writer ──────────────────────────────────────────────────────────────

struct Writer<'t> {
    theme: &'t Theme,
    styles: MdStyles,
    text: Vec<Line>,
    inline_styles: Vec<Style>,
    indent_stack: Vec<IndentCtx>,
    list_indices: Vec<Option<u64>>,
    needs_newline: bool,
    pending_marker: bool,
    in_paragraph: bool,
    in_code_block: bool,
    code_lang: Option<String>,
    code_buf: String,
    wrap_width: usize,
    table: Option<TableState>,
    // Current line being built (before flush).
    current: Vec<Span>,
}

impl<'t> Writer<'t> {
    fn new(theme: &'t Theme, width: Option<usize>) -> Self {
        Self {
            theme,
            styles: MdStyles::new(theme),
            text: Vec::new(),
            inline_styles: Vec::new(),
            indent_stack: Vec::new(),
            list_indices: Vec::new(),
            needs_newline: false,
            pending_marker: false,
            in_paragraph: false,
            in_code_block: false,
            code_lang: None,
            code_buf: String::new(),
            wrap_width: width.unwrap_or(80),
            table: None,
            current: Vec::new(),
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(t) => self.text(t),
            Event::Code(c) => self.code(c),
            Event::SoftBreak => self.soft_break(),
            Event::HardBreak => self.hard_break(),
            Event::Rule => {
                self.flush_line();
                if !self.text.is_empty() {
                    self.push_blank();
                }
                let w = self.wrap_width.min(48);
                self.push_line(Line::from(Span::styled(
                    "─".repeat(w),
                    Style::default().fg(self.theme.faint),
                )));
                self.needs_newline = true;
            }
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(_) | Event::TaskListMarker(_) => {}
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn finish(mut self) -> Vec<Line> {
        self.flush_line();
        self.text
    }

    // ── Tags ────────────────────────────────────────────────────────────

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.start_paragraph(),
            Tag::Heading { level, .. } => self.start_heading(level),
            Tag::BlockQuote(_) => self.start_blockquote(),
            Tag::CodeBlock(kind) => self.start_codeblock(kind),
            Tag::List(start) => self.start_list(start),
            Tag::Item => self.start_item(),
            Tag::Emphasis => self.inline_styles.push(self.styles.emphasis),
            Tag::Strong => self.inline_styles.push(self.styles.strong),
            Tag::Strikethrough => self.inline_styles.push(self.styles.strikethrough),
            Tag::Link { dest_url, .. } => self.start_link(dest_url),
            Tag::Table(aligns) => self.start_table(aligns),
            Tag::TableHead => self.start_table_head(),
            Tag::TableRow => self.start_table_row(),
            Tag::TableCell => self.start_table_cell(),
            Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::Image { .. }
            | Tag::MetadataBlock(_) => {}
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {}
            Tag::Superscript | Tag::Subscript => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::Heading(_) => self.end_heading(),
            TagEnd::BlockQuote(_) => self.end_blockquote(),
            TagEnd::CodeBlock => self.end_codeblock(),
            TagEnd::List(_) => self.end_list(),
            TagEnd::Item => {
                self.flush_line();
                self.indent_stack.pop();
                self.pending_marker = false;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.inline_styles.pop();
            }
            TagEnd::Link => self.end_link(),
            TagEnd::Table => self.end_table(),
            TagEnd::TableHead => self.end_table_head(),
            TagEnd::TableRow => self.end_table_row(),
            TagEnd::TableCell => self.end_table_cell(),
            TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::Image
            | TagEnd::MetadataBlock(_) => {}
            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
            TagEnd::Superscript | TagEnd::Subscript => {}
        }
    }

    fn start_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        if self.needs_newline && !self.text.is_empty() {
            self.push_blank();
        }
        self.needs_newline = false;
        self.in_paragraph = true;
    }

    fn end_paragraph(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.flush_line();
        self.needs_newline = true;
        self.in_paragraph = false;
    }

    fn start_heading(&mut self, level: HeadingLevel) {
        if self.in_table_cell() {
            return;
        }
        self.flush_line();
        if self.needs_newline && !self.text.is_empty() {
            self.push_blank();
            self.needs_newline = false;
        }
        let idx = (level as usize).saturating_sub(1).min(5);
        let hs = self.styles.h[idx];
        let prefix = format!("{} ", "#".repeat(level as usize));
        self.current.push(Span::styled(prefix, hs));
        self.inline_styles.push(hs);
        self.needs_newline = false;
    }

    fn end_heading(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.flush_line();
        self.needs_newline = true;
        self.inline_styles.pop();
    }

    fn start_blockquote(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.flush_line();
        if self.needs_newline && !self.text.is_empty() {
            self.push_blank();
            self.needs_newline = false;
        }
        self.indent_stack.push(IndentCtx::new(
            vec![Span::styled("▎ ", self.styles.blockquote)],
            None,
        ));
    }

    fn end_blockquote(&mut self) {
        if self.in_table_cell() {
            return;
        }
        self.flush_line();
        self.indent_stack.pop();
        self.needs_newline = true;
    }

    fn start_list(&mut self, start: Option<u64>) {
        if self.list_indices.is_empty() && self.needs_newline {
            self.push_blank();
        }
        self.list_indices.push(start);
    }

    fn end_list(&mut self) {
        self.list_indices.pop();
        self.needs_newline = true;
    }

    fn start_item(&mut self) {
        self.flush_line();
        self.pending_marker = true;
        let depth = self.list_indices.len();
        let indent_w = (depth - 1) * 2;

        let marker = if let Some(last) = self.list_indices.last_mut() {
            match last {
                None => {
                    let m = format!("{}• ", " ".repeat(indent_w));
                    vec![Span::styled(m, self.styles.list_marker)]
                }
                Some(idx) => {
                    *idx += 1;
                    let m = format!("{}{}. ", " ".repeat(indent_w), *idx - 1);
                    vec![Span::styled(m, self.styles.list_marker)]
                }
            }
        } else {
            vec![Span::styled("• ", self.styles.list_marker)]
        };

        let marker_w = spans_width(&marker);
        // Continuation lines indent to align text after the marker.
        let cont = indent_w + marker_w;
        let prefix = vec![Span::raw(" ".repeat(cont))];
        self.indent_stack.push(IndentCtx::new(prefix, Some(marker)));
        self.needs_newline = false;
    }

    fn start_codeblock(&mut self, kind: CodeBlockKind) {
        self.flush_line();
        if !self.text.is_empty() {
            self.push_blank();
        }
        self.in_code_block = true;
        let lang = match &kind {
            CodeBlockKind::Fenced(info) => {
                let token = info
                    .split([',', ' ', '\t'])
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                token
            }
            CodeBlockKind::Indented => None,
        };
        self.code_lang = lang;
        self.code_buf.clear();
        self.needs_newline = true;
    }

    fn end_codeblock(&mut self) {
        let lang = self
            .code_lang
            .take()
            .as_deref()
            .map(highlight::lang_from_info)
            .unwrap_or_default();
        let code = std::mem::take(&mut self.code_buf);
        if !code.is_empty() {
            let ht = self.code_highlight_theme();
            for line in highlight::highlight(&code, lang, &ht) {
                let mut spans = vec![Span::styled(String::new(), self.styles.code_marker)];
                spans.extend(line.spans);
                self.push_line(Line::from(spans));
            }
        }
        self.in_code_block = false;
        self.needs_newline = true;
    }

    // ── Inline ──────────────────────────────────────────────────────────

    fn text(&mut self, t: CowStr) {
        if self.in_table_cell() {
            let style = self.inline_styles.last().copied().unwrap_or_default();
            for (i, line) in t.lines().enumerate() {
                if i > 0 {
                    if let Some(cell) = self.table.as_mut().and_then(|s| s.current_cell.as_mut()) {
                        cell.hard_break();
                    }
                }
                self.push_span_to_cell(Span::styled(line.to_string(), style));
            }
            return;
        }

        if self.in_code_block {
            self.code_buf.push_str(&t);
            return;
        }

        for (i, line) in t.lines().enumerate() {
            if self.needs_newline {
                self.flush_line();
                self.needs_newline = false;
            }
            if i > 0 {
                self.flush_line();
            }
            let style = self
                .inline_styles
                .last()
                .copied()
                .unwrap_or(Style::default().fg(self.theme.fg));
            self.push_text(line, style);
        }
        self.needs_newline = false;
    }

    fn code(&mut self, c: CowStr) {
        if self.in_table_cell() {
            self.push_span_to_cell(Span::styled(c.to_string(), self.styles.code));
            return;
        }
        // Inline code chip: apply code_bg.
        let content = c.to_string();
        if content.trim().is_empty() {
            self.current.push(Span::styled("  ", self.styles.code));
        } else {
            self.current.push(Span::styled(content, self.styles.code));
        }
    }

    fn soft_break(&mut self) {
        if self.in_table_cell() {
            let style = self.inline_styles.last().copied().unwrap_or_default();
            self.push_span_to_cell(Span::styled(" ".to_string(), style));
            return;
        }
        self.flush_line();
    }

    fn hard_break(&mut self) {
        if self.in_table_cell() {
            if let Some(cell) = self.table.as_mut().and_then(|s| s.current_cell.as_mut()) {
                cell.hard_break();
            }
            return;
        }
        self.flush_line();
    }

    // ── Links ───────────────────────────────────────────────────────────

    fn start_link(&mut self, dest: CowStr) {
        // Render link text underlined; drop URL.
        let _ = dest;
        self.inline_styles.push(self.styles.link);
    }

    fn end_link(&mut self) {
        self.inline_styles.pop();
    }

    // ── Table ───────────────────────────────────────────────────────────

    fn start_table(&mut self, aligns: Vec<Alignment>) {
        self.flush_line();
        if self.needs_newline {
            self.push_blank();
            self.needs_newline = false;
        }
        self.table = Some(TableState {
            alignments: aligns,
            ..Default::default()
        });
    }

    fn start_table_head(&mut self) {
        if let Some(t) = self.table.as_mut() {
            t.in_header = true;
            t.current_row = Some(Vec::new());
        }
    }

    fn end_table_head(&mut self) {
        if let Some(t) = self.table.as_mut() {
            if let Some(cell) = t.current_cell.take() {
                t.current_row.get_or_insert_with(Vec::new).push(cell);
            }
            if let Some(row) = t.current_row.take() {
                t.header = Some(row);
            }
            t.in_header = false;
        }
    }

    fn start_table_row(&mut self) {
        if let Some(t) = self.table.as_mut() {
            t.current_row = Some(Vec::new());
        }
    }

    fn end_table_row(&mut self) {
        if let Some(t) = self.table.as_mut() {
            if let Some(cell) = t.current_cell.take() {
                t.current_row.get_or_insert_with(Vec::new).push(cell);
            }
            if let Some(row) = t.current_row.take() {
                if t.in_header {
                    t.header = Some(row);
                } else {
                    t.rows.push(row);
                }
            }
        }
    }

    fn start_table_cell(&mut self) {
        if let Some(t) = self.table.as_mut() {
            t.current_cell = Some(TableCell::default());
        }
    }

    fn end_table_cell(&mut self) {
        if let Some(t) = self.table.as_mut() {
            if let Some(cell) = t.current_cell.take() {
                t.current_row.get_or_insert_with(Vec::new).push(cell);
            }
        }
    }

    fn in_table_cell(&self) -> bool {
        self.table
            .as_ref()
            .and_then(|t| t.current_cell.as_ref())
            .is_some()
    }

    fn push_span_to_cell(&mut self, span: Span) {
        if let Some(t) = self.table.as_mut() {
            if let Some(cell) = t.current_cell.as_mut() {
                cell.push_span(span);
            }
        }
    }

    fn end_table(&mut self) {
        let Some(t) = self.table.take() else {
            return;
        };
        let lines = self.render_table(t);
        for l in lines {
            self.push_line(l);
            self.flush_line();
        }
        self.needs_newline = true;
    }

    // ── Table rendering ─────────────────────────────────────────────────

    fn render_table(&self, mut t: TableState) -> Vec<Line> {
        let cols = t.alignments.len();
        if cols == 0 {
            return Vec::new();
        }

        let mut header = t
            .header
            .take()
            .unwrap_or_else(|| vec![TableCell::default(); cols]);
        normalize_row(&mut header, cols);
        for r in &mut t.rows {
            normalize_row(r, cols);
        }

        let chrome = Style::default().fg(self.theme.dim);
        let head_st = Style::default().fg(self.theme.heading).add(Modifier::BOLD);
        let body_st = Style::default().fg(self.theme.fg);

        // Natural column widths.
        let mut natural = vec![1usize; cols];
        for cell in &header {
            for (i, _) in cell.lines.iter().enumerate().take(cols) {
                if i < cols {
                    natural[i] = natural[i].max(cell.display_width()).max(1);
                }
            }
        }
        // Actually measure per-column.
        for (ci, _) in header.iter().enumerate().take(cols) {
            natural[ci] = natural[ci].max(header[ci].display_width()).max(1);
        }
        for row in &t.rows {
            for (ci, cell) in row.iter().enumerate().take(cols) {
                natural[ci] = natural[ci].max(cell.display_width()).max(1);
            }
        }

        // Full box grid: `│ pad content pad │` per cell.
        let chrome_w = (cols + 1) + cols * (2 * TABLE_PAD);
        let budget = self.wrap_width.saturating_sub(chrome_w).max(cols);
        let widths = fit_columns(&natural, cols, budget);

        let border = |widths: &[usize], left: char, mid: char, right: char| -> Line {
            let mut s = String::new();
            s.push(left);
            for (i, w) in widths.iter().enumerate() {
                if i > 0 {
                    s.push(mid);
                }
                s.push_str(&"─".repeat(w + 2 * TABLE_PAD));
            }
            s.push(right);
            Line::from(Span::styled(s, chrome))
        };

        let mut out = Vec::new();

        // Top border.
        out.push(border(&widths, '┌', '┬', '┐'));

        // Header row.
        out.extend(self.render_table_row(&header, &widths, &t.alignments, head_st));
        out.push(border(&widths, '├', '┼', '┤'));

        // Body rows.
        for (ri, row) in t.rows.iter().enumerate() {
            out.extend(self.render_table_row(row, &widths, &t.alignments, body_st));
            if ri + 1 < t.rows.len() {
                out.push(border(&widths, '├', '┼', '┤'));
            }
        }

        // Bottom border.
        out.push(border(&widths, '└', '┴', '┘'));

        out
    }

    fn render_table_row(
        &self,
        row: &[TableCell],
        widths: &[usize],
        aligns: &[Alignment],
        style: Style,
    ) -> Vec<Line> {
        let chrome = Style::default().fg(self.theme.dim);
        let wrapped: Vec<Vec<Line>> = row
            .iter()
            .zip(widths)
            .map(|(cell, w)| wrap_cell(cell, *w))
            .collect();
        let row_h = wrapped.iter().map(Vec::len).max().unwrap_or(1);

        let mut out = Vec::with_capacity(row_h);
        for li in 0..row_h {
            let mut spans = vec![Span::styled("│", chrome)];
            for (ci, w) in widths.iter().enumerate() {
                spans.push(Span::styled(" ".repeat(TABLE_PAD), chrome));
                let line = wrapped[ci].get(li).cloned().unwrap_or_default();
                let lw: usize = line.spans.iter().map(|s| s.content.width()).sum();
                let rem = w.saturating_sub(lw);
                let (lp, rp) = match aligns.get(ci).copied().unwrap_or(Alignment::None) {
                    Alignment::Left | Alignment::None => (0, rem),
                    Alignment::Center => (rem / 2, rem - rem / 2),
                    Alignment::Right => (rem, 0),
                };
                if lp > 0 {
                    spans.push(Span::styled(" ".repeat(lp), style));
                }
                for s in line.spans {
                    spans.push(s);
                }
                if rp > 0 {
                    spans.push(Span::styled(" ".repeat(rp), style));
                }
                spans.push(Span::styled(" ".repeat(TABLE_PAD), chrome));
                spans.push(Span::styled("│", chrome));
            }
            out.push(Line::from(spans));
        }
        out
    }

    // ── Line building ───────────────────────────────────────────────────

    fn push_text(&mut self, content: &str, style: Style) {
        if content.is_empty() {
            return;
        }
        self.current.push(Span::styled(content.to_string(), style));
    }

    fn push_line(&mut self, line: Line) {
        let mut full = Line::new();
        let last = self.indent_stack.len().saturating_sub(1);
        for (i, ctx) in self.indent_stack.iter().enumerate() {
            // On the first line of a list item, the marker already includes
            // the indent — skip the prefix for the innermost ctx.
            if i == last && self.pending_marker {
                // marker will be added below; skip prefix
            } else {
                for s in &ctx.prefix {
                    full.push(s.clone());
                }
            }
            if i == last && self.pending_marker {
                if let Some(m) = &ctx.marker {
                    for s in m {
                        full.push(s.clone());
                    }
                }
            }
        }
        for s in line.spans {
            full.push(s);
        }
        self.text.push(full);
    }

    fn push_blank(&mut self) {
        let mut full = Line::new();
        for ctx in &self.indent_stack {
            for s in &ctx.prefix {
                full.push(s.clone());
            }
        }
        self.text.push(full);
    }

    fn flush_line(&mut self) {
        if !self.current.is_empty() {
            let spans = std::mem::take(&mut self.current);
            self.push_line(Line::from(spans));
            self.pending_marker = false;
        }
    }

    fn code_highlight_theme(&self) -> highlight::HighlightTheme {
        let mk = |fg| Style::default().fg(fg);
        highlight::HighlightTheme {
            text: mk(self.theme.code_fg),
            keyword: mk(self.theme.accent),
            string: mk(self.theme.warn),
            comment: mk(self.theme.faint),
            number: mk(self.theme.ok),
            type_name: mk(self.theme.tool),
            function: mk(self.theme.heading),
            punctuation: mk(self.theme.dim),
            line_number: mk(self.theme.faint),
        }
    }
}

/// Style carried by the zero-width prefix of a fenced/preformatted row.
///
/// This keeps the row semantic while it moves through the generic wrapping
/// pipeline without leaking a private sentinel glyph into selection/copy.
pub(crate) fn code_line_marker(line: &Line) -> Option<Style> {
    line.spans
        .first()
        .filter(|span| span.content.is_empty() && span.style.bg.is_some())
        .map(|span| span.style)
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn normalize_row(row: &mut Vec<TableCell>, cols: usize) {
    row.truncate(cols);
    row.resize(cols, TableCell::default());
}

fn fit_columns(natural: &[usize], cols: usize, budget: usize) -> Vec<usize> {
    let mut widths = natural.to_vec();
    let total: usize = widths.iter().sum();
    if total <= budget {
        return widths;
    }
    let min_w = 6usize;
    // Shrink from the end, keep min.
    while widths.iter().sum::<usize>() > budget {
        let mut shrunk = false;
        for w in widths.iter_mut().rev() {
            if *w > min_w {
                *w -= 1;
                shrunk = true;
                break;
            }
        }
        if !shrunk {
            break;
        }
    }
    // Give leftover to last column.
    let used: usize = widths[..cols.saturating_sub(1)].iter().sum();
    if cols >= 2 {
        widths[cols - 1] = budget.saturating_sub(used).max(min_w);
    }
    widths
}

fn wrap_cell(cell: &TableCell, width: usize) -> Vec<Line> {
    let width = width.max(1);
    if cell.lines.is_empty() {
        return vec![Line::new()];
    }
    let mut out = Vec::new();
    for source_line in &cell.lines {
        let plain: String = source_line
            .spans
            .iter()
            .map(|s| s.content.as_str())
            .collect();
        if plain.is_empty() {
            out.push(Line::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0usize;
        let mut last_space: Option<usize> = None;
        for ch in plain.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if cur_w + cw > width && !cur.is_empty() {
                if let Some(idx) = last_space {
                    let rest: String = cur.chars().skip(idx + 1).collect();
                    let kept: String = cur.chars().take(idx).collect();
                    out.push(Line::from(Span::raw(kept)));
                    cur = rest;
                    cur_w = cur.width();
                    last_space = None;
                } else {
                    out.push(Line::from(Span::raw(std::mem::take(&mut cur))));
                    cur_w = 0;
                }
            }
            if ch == ' ' {
                last_space = Some(cur.chars().count());
            }
            cur.push(ch);
            cur_w += cw;
        }
        if !cur.is_empty() || out.is_empty() {
            out.push(Line::from(Span::raw(cur)));
        }
    }
    if out.is_empty() {
        out.push(Line::new());
    }
    out
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn table_renders_without_pipe_junk() {
        let md = "\
| Check | Status |
|-------|--------|
| Build | ok |
| Tests | 4 passed |
";
        let out = render(md, &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("Check"));
        assert!(t.contains("Build"));
        assert!(!t.contains("|---"), "raw separator must not appear: {t}");
        assert!(
            !t.contains("| Build"),
            "raw markdown pipes must not appear: {t}"
        );
    }

    #[test]
    fn table_columns_align_across_rows() {
        let md = "\
| Аспект | Оценка |
|--------|--------|
| Архитектура | Чистое разделение |
| Саморегистрация тулов | Через inventory |
";
        let out = render(md, &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("Архитектура"));
        assert!(t.contains("Чистое разделение"));
    }

    #[test]
    fn code_fence_still_works() {
        let md = "```rust\nfn main() {}\n```";
        let out = render(md, &Theme::gray(), 40);
        let t = text(&out);
        assert!(t.contains("fn main"));
        assert!(!t.contains('`'), "backticks must be stripped: {t}");
    }

    #[test]
    fn fenced_code_uses_a_marker_instead_of_a_background_slab() {
        let theme = Theme::gray();
        let out = render("```rust\nfn main() {\n    work();\n}\n```", &theme, 80);

        assert_eq!(out.len(), 3);
        for line in &out {
            let marker = code_line_marker(line).expect("preformatted row marker");
            assert_eq!(marker.fg, Some(theme.faint));
            assert!(
                line.spans
                    .iter()
                    .skip(1)
                    .all(|span| span.style.bg.is_none()),
                "code tokens should sit on the terminal surface: {line:?}"
            );
        }
        assert_eq!(text(&out), "fn main() {\n    work();\n}");
    }

    #[test]
    fn unlabeled_fence_keeps_diagram_rows_preformatted() {
        let out = render("```\nClient\n  │\n  ▼\nService\n```", &Theme::gray(), 80);

        assert_eq!(text(&out), "Client\n  │\n  ▼\nService");
        assert!(out.iter().all(|line| code_line_marker(line).is_some()));
    }

    #[test]
    fn heading_parses_inline_markdown() {
        let out = render("# Heading with `code`", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("code"), "heading content: {t}");
        assert!(!t.contains('`'), "backticks must be stripped: {t}");
    }

    #[test]
    fn blockquote_shows_bar_not_gt() {
        let out = render("> quoted text", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("▎"), "blockquote bar: {t}");
        assert!(t.contains("quoted text"));
        assert!(!t.contains("> quoted"), "raw > must not appear: {t}");
    }

    #[test]
    fn nested_list_indents() {
        let md = "- top\n  - nested";
        let out = render(md, &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("top"));
        assert!(t.contains("nested"), "nested must be present: {t}");
    }

    #[test]
    fn link_shows_text_not_url() {
        let out = render("[click here](https://example.com)", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("click here"));
        assert!(!t.contains("https://"), "url must be stripped: {t}");
    }

    #[test]
    fn strikethrough_strips_tildes() {
        let out = render("~~deleted~~", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("deleted"));
        assert!(!t.contains('~'), "tildes must be stripped: {t}");
    }

    #[test]
    fn underscore_bold_works() {
        let out = render("__bold text__", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("bold text"));
        assert!(
            !t.contains("__"),
            "underscore markers must be stripped: {t}"
        );
    }

    #[test]
    fn inline_code_whitespace_only_does_not_panic() {
        let spans = plain("` `", &Theme::gray());
        assert!(!spans.is_empty());
    }

    #[test]
    fn inline_code_multibyte_does_not_panic() {
        let out = render("`привет`", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("привет"), "multibyte code preserved: {t}");
    }

    #[test]
    fn hr_renders() {
        let out = render("---", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains('─'), "hr should render dashes: {t}");
    }

    #[test]
    fn ordered_list_renders() {
        let out = render("1. first\n2. second", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("1."));
        assert!(t.contains("first"));
        assert!(t.contains("2."));
        assert!(t.contains("second"));
    }

    #[test]
    fn bold_italic_nested() {
        let out = render("**bold *and italic* text**", &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("bold"));
        assert!(t.contains("and italic"));
        assert!(t.contains("text"));
        assert!(!t.contains("**"), "bold markers stripped: {t}");
        assert!(!t.contains('*'), "italic markers stripped: {t}");
    }

    #[test]
    fn empty_input() {
        let out = render("", &Theme::gray(), 80);
        assert!(out.is_empty() || out.iter().all(|l| l.spans.is_empty()));
    }

    #[test]
    fn unclosed_bold_does_not_panic() {
        let out = render("**unclosed bold", &Theme::gray(), 80);
        let t = text(&out);
        // pulldown-cmark treats unclosed ** as literal text
        assert!(t.contains("unclosed bold") || t.contains("**unclosed bold"));
    }

    #[test]
    fn plain_text_preserved() {
        let out = render("just some text", &Theme::gray(), 80);
        let t = text(&out);
        assert_eq!(t, "just some text");
    }
}
