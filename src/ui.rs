use ratatui::layout::{Alignment, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode};
use crate::buffer::{grapheme_width, wrap_segments};
use crate::keymap;
use crate::markdown::{self, MdKind};
use crate::outline;
use crate::pane::Pane;
use crate::search::ReplacePhase;
use crate::spellcheck;
use crate::splash;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if app.splash {
        draw_splash(frame, app, area);
        return;
    }
    if area.height < 2 {
        return;
    }
    let mut text_area = Rect {
        height: area.height - 1,
        ..area
    };
    let status_area = Rect {
        y: area.y + area.height - 1,
        height: 1,
        ..area
    };

    // Hint bar at help level 2.
    let hint_area = if app.help_level >= 2 && text_area.height >= 4 {
        text_area.height -= 1;
        Some(Rect {
            y: text_area.y + text_area.height,
            height: 1,
            ..text_area
        })
    } else {
        None
    };

    // Reveal Codes: carve a pane off the bottom of the text area.
    let reveal_area = if app.reveal && text_area.height >= 8 {
        let pane_h = (text_area.height * 2 / 5).clamp(4, 12);
        text_area.height -= pane_h;
        Some(Rect {
            y: text_area.y + text_area.height,
            height: pane_h,
            ..text_area
        })
    } else {
        None
    };

    // Window layout: one pane fills the text area; two stack vertically,
    // each over its own modeline. On a terminal too short to split, only
    // the active pane is shown full-size.
    let split = app.panes.len() == 2 && text_area.height >= 6;
    let (pane_ids, pane_areas, modelines) = if split {
        let h = text_area.height;
        let p0_h = (h - 2) / 2;
        let p1_h = h - 2 - p0_h;
        let a0 = Rect { height: p0_h, ..text_area };
        let m0 = Rect { y: text_area.y + p0_h, height: 1, ..text_area };
        let a1 = Rect { y: m0.y + 1, height: p1_h, ..text_area };
        let m1 = Rect { y: a1.y + p1_h, height: 1, ..text_area };
        (vec![0usize, 1], vec![a0, a1], vec![m0, m1])
    } else {
        (vec![app.active], vec![text_area], Vec::new())
    };

    for (slot, &pid) in pane_ids.iter().enumerate() {
        app.panes[pid].view_rows = pane_areas[slot].height as usize;
        app.panes[pid].view_cols = pane_areas[slot].width as usize;
    }
    app.ensure_visible();

    for (slot, &pid) in pane_ids.iter().enumerate() {
        draw_text(frame, app, pid, pane_areas[slot]);
    }
    for (slot, &pid) in pane_ids.iter().enumerate() {
        if let Some(m) = modelines.get(slot) {
            draw_modeline(frame, app, pid, *m);
        }
    }
    if let Some(ra) = reveal_area {
        draw_reveal(frame, app, ra);
    }
    if let Some(ha) = hint_area {
        draw_hints(frame, app, ha);
    }
    draw_status(frame, app, status_area);
    if app.help_level >= 1 {
        draw_prefix_menu(frame, app, text_area);
    }
    if matches!(app.mode, Mode::Palette { .. }) {
        draw_palette(frame, app, text_area);
    }
    if matches!(app.mode, Mode::Outline { .. }) {
        draw_outline(frame, app, text_area);
    }
    let active_slot = pane_ids.iter().position(|&p| p == app.active).unwrap_or(0);
    place_cursor(frame, app, pane_areas[active_slot]);
}

/// The divider under each window when split: filename, dirty dot, and line
/// number, drawn as a solid bar (bright for the focused window, dim for the
/// other) so it always reads which window has the keyboard.
fn draw_modeline(frame: &mut Frame, app: &App, pane_idx: usize, area: Rect) {
    let pane = &app.panes[pane_idx];
    let is_active = pane_idx == app.active;
    let dirty = if pane.buf.dirty { " •" } else { "" };
    let line_no = pane.buf.line_of(pane.cursor) + 1;
    let marker = if is_active { "▶" } else { "─" };
    let label = format!("{marker} {}{dirty} ─ Ln {line_no} ", pane.buf.file_name());
    let fill = "─".repeat(
        (area.width as usize).saturating_sub(UnicodeWidthStr::width(label.as_str())),
    );
    let style = if is_active { app.theme.status } else { app.theme.dim };
    frame.render_widget(
        Paragraph::new(Line::from(format!("{label}{fill}"))).style(style),
        area,
    );
}

/// The startup splash: a big block-letter banner in a double-bordered box,
/// dismissed by any keypress. Falls back to a single centered line if the
/// terminal is too small for the full banner.
fn draw_splash(frame: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Block::new().style(app.theme.base), area);

    const TAGLINE: &str = "A Writer's Word Processor";
    const BYLINE: &str = "for WordStar & WordPerfect";
    const PROMPT: &str = "Press any key to continue";

    let banner = splash::banner("PERFECTSTAR");
    let banner_width = banner[0].chars().count() as u16;
    let content_width = banner_width
        .max(TAGLINE.len() as u16)
        .max(BYLINE.len() as u16)
        .max(PROMPT.len() as u16);

    let mut lines: Vec<Line> = vec![Line::default()];
    for row in &banner {
        lines.push(Line::styled(row.clone(), app.theme.md_heading));
    }
    lines.push(Line::default());
    lines.push(Line::styled(TAGLINE, app.theme.dim));
    lines.push(Line::styled(BYLINE, app.theme.dim));
    lines.push(Line::default());
    lines.push(Line::styled(
        PROMPT,
        app.theme.base.add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK),
    ));
    let content_height = lines.len() as u16;

    // Too small for the block banner: fall back to plain centered text.
    if area.width < content_width + 6 || area.height < content_height + 2 {
        let y = area.y + area.height.saturating_sub(1) / 2;
        let fallback_area = Rect { x: area.x, y, width: area.width, height: 1 };
        frame.render_widget(
            Paragraph::new(Line::styled("PerfectStar", app.theme.md_heading))
                .alignment(Alignment::Center)
                .style(app.theme.base),
            fallback_area,
        );
        return;
    }

    let box_width = (content_width + 8).min(area.width);
    let box_height = (content_height + 4).min(area.height);
    let box_area = Rect {
        x: area.x + (area.width - box_width) / 2,
        y: area.y + (area.height - box_height) / 2,
        width: box_width,
        height: box_height,
    };

    frame.render_widget(Clear, box_area);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(app.theme.base)
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .style(app.theme.base),
            ),
        box_area,
    );
}

fn draw_hints(frame: &mut Frame, app: &App, area: Rect) {
    let hints = " ^KD save · ^KQ quit · ^QF find · ^KB/^KK mark · ^KC copy · ^KV move · ^KP put · ^U undo · Esc commands";
    frame.render_widget(
        Paragraph::new(Line::from(hints)).style(app.theme.dim),
        area,
    );
}

fn draw_text(frame: &mut Frame, app: &App, pane_idx: usize, area: Rect) {
    let pane = &app.panes[pane_idx];
    let is_active = pane_idx == app.active;
    // Search-match highlighting follows the keyboard: only the focused
    // window shows live matches. Each window paints its own marked block.
    let query = if is_active {
        app.active_query().map(str::to_owned)
    } else {
        None
    };
    let block_range = pane.blocks.visible_range();
    let height = area.height as usize;
    let last = pane.buf.len_lines();
    let mut lines: Vec<Line> = Vec::with_capacity(height);

    match app.wrap_width_of(pane) {
        Some(width) => {
            let mut doc_line = pane.top_line;
            while lines.len() < height && doc_line < last {
                let (text, styles) =
                    line_styles(app, pane, doc_line, query.as_deref(), block_range);
                for (s, e) in wrap_segments(&text, width) {
                    if lines.len() >= height {
                        break;
                    }
                    let (seg_text, seg_styles) = char_slice(&text, &styles, s, e);
                    lines.push(styled_clip(&seg_text, &seg_styles, 0, width));
                }
                doc_line += 1;
            }
        }
        None => {
            for row in 0..height {
                let doc_line = pane.top_line + row;
                if doc_line >= last {
                    break;
                }
                let (text, styles) =
                    line_styles(app, pane, doc_line, query.as_deref(), block_range);
                lines.push(styled_clip(&text, &styles, pane.left_col, area.width as usize));
            }
        }
    }
    while lines.len() < height {
        lines.push(Line::default());
    }
    frame.render_widget(Paragraph::new(lines).style(app.theme.base), area);
}

/// The line's text plus one resolved style per char (markdown, then search
/// matches, then the marked block on top). Note lines are wholly dimmed.
fn line_styles(
    app: &App,
    pane: &Pane,
    doc_line: usize,
    query: Option<&str>,
    block_range: Option<(usize, usize)>,
) -> (String, Vec<Style>) {
    let text = pane.buf.line_text(doc_line).into_owned();
    let n_chars = text.chars().count();

    if text.trim_start().starts_with("..") {
        let styles = vec![app.theme.dim; n_chars];
        return (text, styles);
    }

    let line_start = pane.buf.line_start(doc_line);
    let mut styles: Vec<Style> = vec![Style::default(); n_chars];

    for (s, e, kind) in markdown::scan_line(&text) {
        let style = md_style(app, kind);
        for st in styles.iter_mut().take(e.min(n_chars)).skip(s) {
            *st = style;
        }
    }

    if app.spell_enabled {
        for (s, e) in spellcheck::word_spans(&text) {
            let word: String = text.chars().skip(s).take(e - s).collect();
            if !app.spell.check(&word) {
                for st in styles.iter_mut().take(e.min(n_chars)).skip(s) {
                    *st = st.patch(app.theme.misspelled);
                }
            }
        }
    }

    if let Some(q) = query {
        for (s, e) in match_ranges(&text, q) {
            for st in styles.iter_mut().take(e.min(n_chars)).skip(s) {
                *st = app.theme.highlight;
            }
        }
    }

    if let Some((b, e)) = block_range {
        if e > line_start && b < line_start + n_chars {
            let from = b.saturating_sub(line_start).min(n_chars);
            let to = e.saturating_sub(line_start).min(n_chars);
            for st in styles.iter_mut().take(to).skip(from) {
                *st = app.theme.block;
            }
        }
    }

    (text, styles)
}

/// Slice text + parallel styles by char range.
fn char_slice(text: &str, styles: &[Style], s: usize, e: usize) -> (String, Vec<Style>) {
    let seg: String = text.chars().skip(s).take(e - s).collect();
    let seg_styles = styles[s.min(styles.len())..e.min(styles.len())].to_vec();
    (seg, seg_styles)
}

fn md_style(app: &App, kind: MdKind) -> Style {
    match kind {
        MdKind::Marker => app.theme.md_marker,
        MdKind::Bold => app.theme.md_bold,
        MdKind::Italic => app.theme.md_italic,
        MdKind::Code => app.theme.md_code,
        MdKind::Heading => app.theme.md_heading,
    }
}

/// Walk graphemes, clipping to the window and grouping runs of equal style.
fn styled_clip(text: &str, styles: &[Style], left: usize, width: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_style = Style::default();
    let mut vcol = 0usize;
    let mut char_idx = 0usize;
    let flush = |run: &mut String, style: Style, spans: &mut Vec<Span<'static>>| {
        if !run.is_empty() {
            spans.push(Span::styled(std::mem::take(run), style));
        }
    };
    for g in text.graphemes(true) {
        let w = grapheme_width(g, vcol);
        let g_start = vcol;
        let g_chars = g.chars().count();
        let style = styles.get(char_idx).copied().unwrap_or_default();
        vcol += w;
        char_idx += g_chars;
        if vcol <= left {
            continue;
        }
        if g_start >= left + width {
            break;
        }
        if style != run_style {
            flush(&mut run, run_style, &mut spans);
            run_style = style;
        }
        if g == "\t" {
            let from = g_start.max(left);
            let to = vcol.min(left + width);
            run.push_str(&" ".repeat(to - from));
        } else if g_start < left {
            run.push_str(&" ".repeat(vcol - left));
        } else {
            run.push_str(g);
        }
    }
    flush(&mut run, run_style, &mut spans);
    Line::from(spans)
}

/// Char ranges of `query` matches within `text` (smartcase).
fn match_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let fold = !query.chars().any(|c| c.is_uppercase());
    let hay: Vec<char> = if fold {
        text.chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect()
    } else {
        text.chars().collect()
    };
    let needle: Vec<char> = if fold {
        query
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect()
    } else {
        query.chars().collect()
    };
    let mut out = Vec::new();
    if needle.len() > hay.len() {
        return out;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            out.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// The Reveal Codes pane (^OD): the lines around the cursor with every
/// markup character shown in inverse video, WP 5.1 style.
fn draw_reveal(frame: &mut Frame, app: &App, area: Rect) {
    let title_area = Rect { height: 1, ..area };
    let body = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };

    let title = format!(
        "─ Reveal Codes ─ Ln {} {}",
        app.buf.line_of(app.cursor) + 1,
        "─".repeat(area.width as usize),
    );
    frame.render_widget(
        Paragraph::new(Line::from(title)).style(app.theme.status),
        title_area,
    );

    let rows = body.height as usize;
    let cursor_line = app.buf.line_of(app.cursor);
    let first = cursor_line.saturating_sub(rows / 2);
    let last_line = app.buf.len_lines();
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for row in 0..rows {
        let doc_line = first + row;
        if doc_line >= last_line {
            lines.push(Line::default());
            continue;
        }
        let text = app.buf.line_text(doc_line).into_owned();
        let n_chars = text.chars().count();
        let mut styles: Vec<Style> = vec![Style::default(); n_chars];
        for (s, e, kind) in markdown::scan_line(&text) {
            if kind == MdKind::Marker {
                for st in styles.iter_mut().take(e.min(n_chars)).skip(s) {
                    *st = app.theme.block;
                }
            }
        }
        lines.push(styled_clip(&text, &styles, app.left_col, body.width as usize));
    }
    frame.render_widget(Paragraph::new(lines).style(app.theme.base), body);
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let line_no = app.buf.line_of(app.cursor) + 1;
    let col_no = app.buf.visual_col(app.cursor) + 1;
    let dirty = if app.buf.dirty { " •" } else { "" };

    let left = match &app.mode {
        Mode::ConfirmAbandon => {
            format!(" Abandon changes to {}? (y/N)", app.buf.file_name())
        }
        Mode::Search(s) => {
            format!(" Find: {}▌  (Enter accept · ^L next · Esc cancel)", s.query)
        }
        Mode::Replace(r) => match r.phase {
            ReplacePhase::EnterFind => format!(" Replace: {}▌", r.find),
            ReplacePhase::EnterWith => {
                format!(" Replace: {}  With: {}▌", r.find, r.with)
            }
            ReplacePhase::EnterOptions => format!(
                " Options (g=from top, n=no ask, w=whole words): {}▌",
                r.options
            ),
            ReplacePhase::Confirm(_) => String::from(" Replace? (Y/n/a=all/q=quit)"),
        },
        Mode::Input { label, value, .. } => format!(" {label}: {value}▌"),
        Mode::Palette { .. } => String::from(" ↑↓ select · Enter run · Esc close"),
        Mode::Outline { .. } => String::from(" ↑↓ select · Enter go to heading · Esc close"),
        Mode::Normal => match &app.status_msg {
            Some(msg) => format!(" {msg}"),
            None => format!(" {}{}", app.buf.file_name(), dirty),
        },
    };

    let pending = match app.prefix {
        Some((p, _)) => match p {
            keymap::Prefix::K => "^K ",
            keymap::Prefix::Q => "^Q ",
            keymap::Prefix::O => "^O ",
        },
        None => "",
    };
    let rec = if app.recording { "● REC  " } else { "" };
    let ins = if app.overtype { "Ovr" } else { "Ins" };
    let words = app.buf.word_count();
    let right = format!("{rec}{pending}Ln {line_no}  Col {col_no}  {words} words  {ins} ");

    let width = area.width as usize;
    let left_w = UnicodeWidthStr::width(left.as_str());
    let right_w = UnicodeWidthStr::width(right.as_str());
    let pad = width.saturating_sub(left_w + right_w);
    let content = format!("{left}{}{right}", " ".repeat(pad));

    frame.render_widget(
        Paragraph::new(Line::from(content)).style(app.theme.status),
        area,
    );
}

/// The WordStar delayed menu: once a prefix key has been held pending longer
/// than MENU_DELAY, show what the second key could be.
fn draw_prefix_menu(frame: &mut Frame, app: &App, text_area: Rect) {
    let Some((prefix, since)) = app.prefix else {
        return;
    };
    if since.elapsed() < app.menu_delay {
        return;
    }
    let entries = keymap::menu_entries(prefix);
    if entries.is_empty() {
        return;
    }

    const COLS: usize = 4;
    let rows = entries.len().div_ceil(COLS);
    let col_width = (text_area.width as usize / COLS).max(12);
    let height = (rows + 2) as u16; // borders
    if text_area.height < height {
        return;
    }
    let area = Rect {
        x: text_area.x,
        y: text_area.y + text_area.height - height,
        width: text_area.width,
        height,
    };

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut spans = Vec::new();
        for c in 0..COLS {
            let i = c * rows + r;
            if let Some((key, name)) = entries.get(i) {
                let label = format!(" {}", key.to_ascii_uppercase());
                let desc = format!(" {name}");
                let used = label.len() + desc.width();
                spans.push(Span::styled(label, app.theme.block));
                spans.push(Span::raw(format!(
                    "{desc}{}",
                    " ".repeat(col_width.saturating_sub(used))
                )));
            }
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).style(app.theme.status).block(
            Block::new()
                .borders(Borders::ALL)
                .title(prefix.label())
                .style(app.theme.status),
        ),
        area,
    );
}

fn place_cursor(frame: &mut Frame, app: &App, area: Rect) {
    // While an overlay is open, the terminal cursor stays out of the text.
    if matches!(app.mode, Mode::Palette { .. } | Mode::Outline { .. }) {
        return;
    }
    let line = app.buf.line_of(app.cursor);
    if line < app.top_line {
        return;
    }

    let (row, col) = match app.wrap_width() {
        Some(width) => {
            let mut rows = 0usize;
            for l in app.top_line..line {
                rows += wrap_segments(&app.buf.line_text(l), width).len();
                if rows > area.height as usize {
                    return;
                }
            }
            let (seg_idx, vcol) = app.cursor_segment(width);
            (rows + seg_idx, vcol)
        }
        None => {
            let vcol = app.buf.visual_col(app.cursor);
            if vcol < app.left_col {
                return;
            }
            (line - app.top_line, vcol - app.left_col)
        }
    };
    if row >= area.height as usize || col >= area.width as usize {
        return;
    }
    frame.set_cursor_position(Position::new(
        area.x + col as u16,
        area.y + row as u16,
    ));
}

/// The command palette: a searchable list of every command (Esc / F1).
fn draw_palette(frame: &mut Frame, app: &App, text_area: Rect) {
    let Mode::Palette { query, selected } = &app.mode else {
        return;
    };
    let entries = keymap::filtered_entries(query);

    let width = (text_area.width.saturating_sub(8)).clamp(30, 60);
    let max_list = (text_area.height as usize).saturating_sub(4).clamp(3, 14);
    let height = (entries.len().clamp(1, max_list) + 3) as u16;
    let area = Rect {
        x: text_area.x + (text_area.width - width) / 2,
        y: text_area.y + 1,
        width,
        height: height.min(text_area.height),
    };

    let visible = (area.height as usize).saturating_sub(3);
    let first = selected.saturating_sub(visible.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(format!(" > {query}▌")));
    for (i, (_, name, chord)) in entries.iter().enumerate().skip(first).take(visible) {
        let inner = area.width.saturating_sub(2) as usize;
        let pad = inner.saturating_sub(name.len() + chord.len() + 3);
        let row = format!(" {name}{}{chord}  ", " ".repeat(pad));
        if i == *selected {
            lines.push(Line::from(Span::styled(row, app.theme.block)));
        } else {
            lines.push(Line::from(row));
        }
    }
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(" no matching command", app.theme.dim)));
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).style(app.theme.status).block(
            Block::new()
                .borders(Borders::ALL)
                .title(" Commands ")
                .style(app.theme.status),
        ),
        area,
    );
}

/// The outline: Markdown headings in document order, filtered by title,
/// indented by level. Enter jumps the cursor to the heading.
fn draw_outline(frame: &mut Frame, app: &App, text_area: Rect) {
    let Mode::Outline { entries, query, selected } = &app.mode else {
        return;
    };
    let q = query.to_lowercase();
    let matches: Vec<&outline::Entry> = entries
        .iter()
        .filter(|e| q.is_empty() || e.title.to_lowercase().contains(&q))
        .collect();

    let width = (text_area.width.saturating_sub(8)).clamp(30, 60);
    let max_list = (text_area.height as usize).saturating_sub(4).clamp(3, 14);
    let height = (matches.len().clamp(1, max_list) + 3) as u16;
    let area = Rect {
        x: text_area.x + (text_area.width - width) / 2,
        y: text_area.y + 1,
        width,
        height: height.min(text_area.height),
    };

    let visible = (area.height as usize).saturating_sub(3);
    let first = selected.saturating_sub(visible.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(format!(" > {query}▌")));
    for (i, e) in matches.iter().enumerate().skip(first).take(visible) {
        let indent = "  ".repeat(e.level.saturating_sub(1) as usize);
        let label = format!("{indent}{}", e.title);
        let line_no = format!("{}", e.line + 1);
        let inner = area.width.saturating_sub(2) as usize;
        let pad = inner.saturating_sub(label.len() + line_no.len() + 3).max(1);
        let row = format!(" {label}{}{line_no}  ", " ".repeat(pad));
        if i == *selected {
            lines.push(Line::from(Span::styled(row, app.theme.block)));
        } else {
            lines.push(Line::from(row));
        }
    }
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(" no headings in this document", app.theme.dim)));
    } else if matches.is_empty() {
        lines.push(Line::from(Span::styled(" no matching heading", app.theme.dim)));
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).style(app.theme.status).block(
            Block::new()
                .borders(Borders::ALL)
                .title(" Outline ")
                .style(app.theme.status),
        ),
        area,
    );
}
