use ratatui::Frame;
use ratatui::layout::{Alignment, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode};
use crate::buffer::{grapheme_width, wrap_segments};
use crate::diff::DiffTag;
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
        let a0 = Rect {
            height: p0_h,
            ..text_area
        };
        let m0 = Rect {
            y: text_area.y + p0_h,
            height: 1,
            ..text_area
        };
        let a1 = Rect {
            y: m0.y + 1,
            height: p1_h,
            ..text_area
        };
        let m1 = Rect {
            y: a1.y + p1_h,
            height: 1,
            ..text_area
        };
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
    if matches!(app.mode, Mode::Binder { .. }) {
        draw_binder(frame, app, text_area);
    }
    if matches!(app.mode, Mode::Stats) {
        draw_stats_overlay(frame, app, text_area);
    }
    if matches!(app.mode, Mode::ProjectSearch { .. }) {
        draw_project_search(frame, app, text_area);
    }
    if matches!(app.mode, Mode::Revisions { .. }) {
        draw_revisions(frame, app, text_area);
    }
    if matches!(app.mode, Mode::Diff { .. }) {
        draw_diff(frame, app, text_area);
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
    let fill =
        "─".repeat((area.width as usize).saturating_sub(UnicodeWidthStr::width(label.as_str())));
    let style = if is_active {
        app.theme.status
    } else {
        app.theme.dim
    };
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
        app.theme
            .base
            .add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK),
    ));
    let content_height = lines.len() as u16;

    // Too small for the block banner: fall back to plain centered text.
    if area.width < content_width + 6 || area.height < content_height + 2 {
        let y = area.y + area.height.saturating_sub(1) / 2;
        let fallback_area = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };
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
    frame.render_widget(Paragraph::new(Line::from(hints)).style(app.theme.dim), area);
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

    // Focus dimming (R3.4): in focus mode, everything outside the paragraph
    // being written recedes. Only the focused window dims — the other one has
    // no cursor to be writing at.
    let lit_range = (app.focus.is_some() && app.focus_dim && is_active)
        .then(|| pane.buf.paragraph_line_range(pane.cursor));

    match app.wrap_width_of(pane) {
        Some(width) => {
            let mut doc_line = pane.top_line;
            while lines.len() < height && doc_line < last {
                let (text, mut styles) =
                    line_styles(app, pane, doc_line, query.as_deref(), block_range);
                dim_unless_lit(app, &mut styles, doc_line, lit_range);
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
                let (text, mut styles) =
                    line_styles(app, pane, doc_line, query.as_deref(), block_range);
                dim_unless_lit(app, &mut styles, doc_line, lit_range);
                lines.push(styled_clip(
                    &text,
                    &styles,
                    pane.left_col,
                    area.width as usize,
                ));
            }
        }
    }
    while lines.len() < height {
        lines.push(Line::default());
    }
    frame.render_widget(Paragraph::new(lines).style(app.theme.base), area);
}

/// Push a whole line into the background when focus mode is dimming and the
/// line falls outside the lit paragraph (R3.4).
///
/// Applied *after* `line_styles`, replacing rather than layering: the point is
/// that the surrounding page recedes, so Markdown emphasis and spelling marks
/// go quiet with it.
fn dim_unless_lit(app: &App, styles: &mut [Style], doc_line: usize, lit: Option<(usize, usize)>) {
    let Some((first, last)) = lit else { return };
    if doc_line >= first && doc_line <= last {
        return;
    }
    for style in styles.iter_mut() {
        *style = app.theme.dim;
    }
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

    if crate::normalize::is_note_line(&text) {
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
        lines.push(styled_clip(
            &text,
            &styles,
            app.left_col,
            body.width as usize,
        ));
    }
    frame.render_widget(Paragraph::new(lines).style(app.theme.base), body);
}

fn status_left(app: &App) -> String {
    match &app.mode {
        Mode::ConfirmAbandon => {
            format!(" Abandon changes to {}? (y/N)", app.buf.file_name())
        }
        Mode::ConfirmRecover => format!(
            " Recover unsaved changes to {}? (y/N · Esc decline)",
            app.buf.file_name()
        ),
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
        Mode::Binder { .. } => match &app.status_msg {
            Some(msg) => format!(" {msg}"),
            None => String::from(
                " ↑↓ select · Enter open · ^PV split · ^PM note · ^PE/^PX move · Esc close",
            ),
        },
        Mode::Stats => String::from(" Writing Stats · press any key to close"),
        Mode::ProjectSearch { query, results, .. } => {
            format!(
                " Project search: \"{}\" — {} match(es) · ↑↓ navigate · Enter open · Esc close",
                query,
                results.len()
            )
        }
        Mode::Revisions { entries, .. } => match &app.status_msg {
            Some(msg) => format!(" {msg}"),
            None => format!(
                " {} snapshot(s) · ↑↓ select · Enter diff · Space mark · ^R restore · Esc close",
                entries.len()
            ),
        },
        Mode::Diff { .. } => match &app.status_msg {
            Some(msg) => format!(" {msg}"),
            None => String::from(" ↑↓ scroll · ^R restore the older version · Esc close"),
        },
        // A finished sprint's report outlives the keystroke that would clear an
        // ordinary status message, because a sprint usually ends mid-word
        // (R3.2).
        Mode::Normal => match app.status_msg.as_deref().or_else(|| app.sprint_banner()) {
            Some(msg) => format!(" {msg}"),
            None => {
                let dirty = if app.buf.dirty { " •" } else { "" };
                format!(" {}{}", app.buf.file_name(), dirty)
            }
        },
    }
}

/// The sprint countdown, sized to stay in the corner of the eye (R3.1).
fn sprint_chip(app: &App) -> Option<String> {
    app.sprint
        .as_ref()
        .map(|sprint| sprint.chip(Instant::now(), app.doc_stats.words))
}

/// Focus mode's status row (R3.3): blank unless there is something the writer
/// asked for — a prompt, a message, or a running sprint's countdown.
///
/// The row stays reserved rather than reclaimed for text, so a prompt appearing
/// mid-sentence doesn't reflow the page under the cursor.
fn draw_focus_status(frame: &mut Frame, app: &App, area: Rect) {
    let left = match &app.mode {
        Mode::Normal => match app.status_msg.as_deref().or_else(|| app.sprint_banner()) {
            Some(msg) => format!(" {msg}"),
            None => String::new(),
        },
        _ => status_left(app),
    };
    let right = match sprint_chip(app) {
        Some(chip) => format!("{chip} "),
        None => String::new(),
    };

    let width = area.width as usize;
    let pad = width
        .saturating_sub(UnicodeWidthStr::width(left.as_str()))
        .saturating_sub(UnicodeWidthStr::width(right.as_str()));
    let content = format!("{left}{}{right}", " ".repeat(pad));
    // Styled as the page, not as a status bar: no band of colour across the
    // bottom of an otherwise clean screen.
    frame.render_widget(
        Paragraph::new(Line::from(content)).style(app.theme.base),
        area,
    );
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    if app.focus.is_some() {
        draw_focus_status(frame, app, area);
        return;
    }

    let line_no = app.buf.line_of(app.cursor) + 1;
    let col_no = app.buf.visual_col(app.cursor) + 1;

    let left = status_left(app);

    let pending = match app.prefix {
        Some((p, _)) => match p {
            keymap::Prefix::K => "^K ",
            keymap::Prefix::Q => "^Q ",
            keymap::Prefix::O => "^O ",
            keymap::Prefix::P => "^P ",
        },
        None => "",
    };
    let rec = if app.recording { "● REC  " } else { "" };
    let ins = if app.overtype { "Ovr" } else { "Ins" };

    let words_part = if app.show_word_count {
        format!("  {} words", app.doc_stats.words)
    } else {
        String::new()
    };

    // Selection count (R2.2): show when a block is active.
    let sel_part = if let Some((b, e)) = app.blocks.range() {
        let (sw, sc) = crate::stats::count_slice(&app.buf.rope, b, e);
        format!("  sel: {sw}w/{sc}c")
    } else {
        String::new()
    };

    // Goal progress (R2.3).
    let goal_part = if let Some(ref goal) = app.goal {
        let (current, target) = goal.progress(app.doc_stats.words);
        format!("  [{current}/{target}]")
    } else {
        String::new()
    };

    // Sprint countdown first: it's the one thing a sprinting writer glances at.
    let sprint_part = match sprint_chip(app) {
        Some(chip) => format!("{chip}  "),
        None => String::new(),
    };

    let right = format!(
        "{rec}{pending}{sprint_part}Ln {line_no}  Col {col_no}{words_part}{sel_part}{goal_part}  {ins} "
    );

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
    if matches!(
        app.mode,
        Mode::Palette { .. } | Mode::Outline { .. } | Mode::Revisions { .. } | Mode::Diff { .. }
    ) {
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
    frame.set_cursor_position(Position::new(area.x + col as u16, area.y + row as u16));
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
        lines.push(Line::from(Span::styled(
            " no matching command",
            app.theme.dim,
        )));
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
    let Mode::Outline {
        entries,
        query,
        selected,
    } = &app.mode
    else {
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
        lines.push(Line::from(Span::styled(
            " no headings in this document",
            app.theme.dim,
        )));
    } else if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            " no matching heading",
            app.theme.dim,
        )));
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

/// The binder: project document list with title and word count (^PB, task 1.3).
fn draw_binder(frame: &mut Frame, app: &App, text_area: Rect) {
    let Mode::Binder { entries, selected } = &app.mode else {
        return;
    };

    let width = (text_area.width.saturating_sub(8)).clamp(40, 70);
    // A document with a synopsis takes a second row (R5.3), so the panel is
    // sized from the rows it will actually draw rather than the doc count.
    let rows_per_entry = |entry: &crate::app::BinderEntry| {
        if entry.synopsis.is_empty() { 1 } else { 2 }
    };
    let total_rows: usize = entries.iter().map(rows_per_entry).sum();
    let max_list = (text_area.height as usize).saturating_sub(4).clamp(3, 16);
    let height = (total_rows.clamp(1, max_list) + 2) as u16;
    let area = Rect {
        x: text_area.x + (text_area.width - width) / 2,
        y: text_area.y + 1,
        width,
        height: height.min(text_area.height),
    };

    let visible = (area.height as usize).saturating_sub(2);
    // Scroll by entries, but keep the selected one's rows in view.
    let first = entries
        .iter()
        .enumerate()
        .take(selected + 1)
        .rev()
        .scan(0usize, |used, (i, entry)| {
            *used += rows_per_entry(entry);
            Some((i, *used))
        })
        .take_while(|(_, used)| *used <= visible.max(1))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(*selected);
    let inner = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for (i, entry) in entries.iter().enumerate().skip(first) {
        if lines.len() >= visible {
            break;
        }
        let wc_str = match entry.word_count {
            Some(wc) => format!("{wc} words"),
            None => String::from("—"),
        };
        let missing_marker = if !entry.exists { " [MISSING]" } else { "" };
        // A note is not part of the book (R5.2); saying so here is what explains
        // its absence from a compile.
        let note_marker = if app
            .project
            .as_ref()
            .is_some_and(|project| project.doc_is_note(entry.idx))
        {
            " [note]"
        } else {
            ""
        };
        let pad = inner
            .saturating_sub(
                UnicodeWidthStr::width(entry.title.as_str())
                    + missing_marker.len()
                    + note_marker.len()
                    + wc_str.len()
                    + 3,
            )
            .max(1);
        let row = format!(
            " {}{missing_marker}{note_marker}{}{wc_str}  ",
            entry.title,
            " ".repeat(pad)
        );

        if i == *selected {
            lines.push(Line::from(Span::styled(row, app.theme.block)));
        } else if !entry.exists {
            lines.push(Line::from(Span::styled(row, app.theme.dim)));
        } else {
            lines.push(Line::from(row));
        }

        // The synopsis as a dimmed secondary line, clipped to the panel (R5.3).
        if !entry.synopsis.is_empty() && lines.len() < visible {
            let room = inner.saturating_sub(5);
            let mut text: String = entry.synopsis.chars().take(room).collect();
            if entry.synopsis.chars().count() > room {
                text.push('…');
            }
            lines.push(Line::from(Span::styled(
                format!("   {text}"),
                app.theme.dim,
            )));
        }
    }

    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            " (no documents in project)",
            app.theme.dim,
        )));
    }

    let title = if let Some(ref project) = app.project {
        format!(" Binder: {} ", project.manifest.name)
    } else {
        String::from(" Binder ")
    };

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).style(app.theme.status).block(
            Block::new()
                .borders(Borders::ALL)
                .title(title)
                .style(app.theme.status),
        ),
        area,
    );
}

/// The writing statistics overlay (^OI): daily history and current counts.
fn draw_stats_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(format!(
        " Words: {}  Chars: {}",
        app.doc_stats.words, app.doc_stats.chars
    )));
    lines.push(Line::from(""));
    if let Some(ref goal) = app.goal {
        let (current, target) = goal.progress(app.doc_stats.words);
        let kind = match goal.kind {
            crate::stats::GoalKind::Words => "words",
            crate::stats::GoalKind::Minutes => "min",
        };
        let status = if goal.reached { " ✓" } else { "" };
        lines.push(Line::from(format!(
            " Goal: {current}/{target} {kind}{status}"
        )));
    } else {
        lines.push(Line::from(" Goal: none (^OG to set)"));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(" Today's net words:"));
    lines.push(Line::from(format!("   {}", app.daily_history.today())));
    lines.push(Line::from(""));
    lines.push(Line::from(" Recent days:"));
    for (date, words) in app.daily_history.recent(5) {
        lines.push(Line::from(format!("   {date}: {words:+}")));
    }

    // Recent sprints (R3.2 files them here; R2.5 requires the history be
    // viewable, and this overlay is where a writer looks).
    let sprints = app.daily_history.recent_sprints(3);
    if !sprints.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(" Recent sprints:"));
        for record in sprints {
            let mark = if record.met_target { " ✓" } else { "" };
            lines.push(Line::from(format!(
                "   {}: {} words in {}{mark}",
                record.date,
                record.words,
                crate::sprint::format_duration(std::time::Duration::from_secs(record.seconds)),
            )));
        }
    }

    // Sized to its contents, so the sprint log can't be silently clipped off
    // the bottom on a short terminal.
    let width = 44u16.min(area.width.saturating_sub(4));
    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2 + area.x;
    let y = (area.height.saturating_sub(height)) / 2 + area.y;
    let overlay = Rect::new(x, y, width, height);

    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Writing Stats ");
    let para = Paragraph::new(lines).block(block).style(app.theme.status);
    frame.render_widget(para, overlay);
}

/// Project search results list (^PS, R6.1).
fn draw_project_search(frame: &mut Frame, app: &App, area: Rect) {
    let Mode::ProjectSearch {
        results,
        selected,
        replace_with,
        ..
    } = &app.mode
    else {
        return;
    };

    let width = area.width.saturating_sub(4).min(72);
    let height = area.height.saturating_sub(2).min(20);
    let x = (area.width.saturating_sub(width)) / 2 + area.x;
    let y = (area.height.saturating_sub(height)) / 2 + area.y;
    let overlay = Rect::new(x, y, width, height);

    frame.render_widget(Clear, overlay);

    let visible = (height as usize).saturating_sub(2);
    let first = if *selected >= visible {
        selected - visible + 1
    } else {
        0
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, m) in results.iter().enumerate().skip(first).take(visible) {
        let marker = if i == *selected { "▶ " } else { "  " };
        let ctx: String = m
            .context
            .chars()
            .take((width as usize).saturating_sub(20))
            .collect();
        let entry = format!("{marker}{}: L{} — {}", m.title, m.line, ctx);
        let style = if i == *selected {
            app.theme.highlight
        } else {
            Style::default()
        };
        lines.push(Line::styled(entry, style));
    }

    let title = if replace_with.is_some() {
        " Project Replace (^R replace · ^A all · Esc cancel) "
    } else {
        " Project Search (Enter open · Esc close) "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title);
    let para = Paragraph::new(lines).block(block).style(app.theme.status);
    frame.render_widget(para, overlay);
}

/// The revisions list: this document's snapshots, newest first (^KO, R4.3).
fn draw_revisions(frame: &mut Frame, app: &App, text_area: Rect) {
    let Mode::Revisions {
        entries,
        selected,
        compare,
    } = &app.mode
    else {
        return;
    };

    let width = (text_area.width.saturating_sub(4)).clamp(40, 72);
    let max_list = (text_area.height as usize).saturating_sub(4).clamp(3, 16);
    let height = (entries.len().clamp(1, max_list) + 2) as u16;
    let area = Rect {
        x: text_area.x + (text_area.width.saturating_sub(width)) / 2,
        y: text_area.y + 1,
        width,
        height: height.min(text_area.height),
    };

    let visible = (area.height as usize).saturating_sub(2);
    let first = selected.saturating_sub(visible.saturating_sub(1));
    let inner = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    for (i, entry) in entries.iter().enumerate().skip(first).take(visible) {
        // The mark shows which version Enter will compare against.
        let mark = if *compare == Some(i) { "◆" } else { " " };
        let words = format!("{} words", entry.words);
        let label = entry.display_label();
        let left = format!("{mark} {} {label}", entry.display_time());
        let pad = inner
            .saturating_sub(left.width() + words.width() + 2)
            .max(1);
        let row = format!(" {left}{}{words} ", " ".repeat(pad));

        if i == *selected {
            lines.push(Line::from(Span::styled(row, app.theme.block)));
        } else if entry.auto {
            // The editor's own copies read quieter than the writer's.
            lines.push(Line::from(Span::styled(row, app.theme.dim)));
        } else {
            lines.push(Line::from(row));
        }
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).style(app.theme.status).block(
            Block::new()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(format!(" Revisions: {} ", app.buf.file_name()))
                .style(app.theme.status),
        ),
        area,
    );
}

/// The diff view: added and removed lines between two versions (R4.4).
fn draw_diff(frame: &mut Frame, app: &App, text_area: Rect) {
    let Mode::Diff {
        title,
        lines: diff_lines,
        scroll,
        ..
    } = &app.mode
    else {
        return;
    };

    let area = text_area;
    let visible = (area.height as usize).saturating_sub(2);
    let mut lines: Vec<Line> = Vec::new();

    for line in diff_lines.iter().skip(*scroll).take(visible) {
        let (sign, style) = match line.tag {
            DiffTag::Insert => ("+", app.theme.diff_added),
            DiffTag::Delete => ("−", app.theme.diff_removed),
            DiffTag::Equal => (" ", app.theme.status),
            DiffTag::Gap => (" ", app.theme.dim),
        };
        // Line numbers orient the writer in the draft, not just in the diff.
        let number = match (line.tag, line.new_line, line.old_line) {
            (DiffTag::Gap, _, _) => String::from("    "),
            (DiffTag::Delete, _, Some(n)) => format!("{n:>4}"),
            (_, Some(n), _) => format!("{n:>4}"),
            _ => String::from("    "),
        };
        let body: String = line
            .text
            .chars()
            .take((area.width as usize).saturating_sub(9))
            .collect();
        lines.push(Line::from(Span::styled(
            format!(" {number} {sign} {body}"),
            style,
        )));
    }

    let position = if diff_lines.len() > visible {
        format!(
            " [{}/{}] ",
            (*scroll + visible).min(diff_lines.len()),
            diff_lines.len()
        )
    } else {
        String::new()
    };

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).style(app.theme.status).block(
            Block::new()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(format!(" {title} "))
                .title_bottom(position)
                .style(app.theme.status),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffLine;
    use crate::snapshot::SnapshotEntry;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    /// Render one frame and return the screen as text lines. Exercises the real
    /// draw path, so a width/unicode mistake in an overlay fails here instead of
    /// crashing the editor in front of a writer.
    fn screen(app: &mut App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect()
    }

    fn entry(label: Option<&str>, timestamp: u64, words: usize, auto: bool) -> SnapshotEntry {
        SnapshotEntry {
            file: format!("20240101T000000.000Z-{timestamp}.txt"),
            label: label.map(str::to_owned),
            timestamp,
            words,
            auto,
        }
    }

    #[test]
    fn revisions_overlay_lists_versions_with_time_label_and_words() {
        let mut app = App::new(None).unwrap();
        app.splash = false;
        app.mode = Mode::Revisions {
            entries: vec![
                entry(Some("before the cut"), 1_704_067_500, 1234, false),
                entry(None, 1_704_067_200, 1200, true),
            ],
            selected: 0,
            compare: Some(1),
        };

        let screen = screen(&mut app).join("\n");
        assert!(screen.contains("Revisions"), "{screen}");
        assert!(screen.contains("before the cut"), "{screen}");
        assert!(screen.contains("1234 words"), "{screen}");
        // The automatic version reads as "auto", and the marked one is flagged.
        assert!(screen.contains("auto"), "{screen}");
        assert!(screen.contains('◆'), "{screen}");
        assert!(screen.contains("^R restore"), "{screen}");
    }

    #[test]
    fn diff_overlay_marks_added_and_removed_lines() {
        let mut app = App::new(None).unwrap();
        app.splash = false;
        app.mode = Mode::Diff {
            title: String::from("13:45 before the cut → current draft  +1 −1"),
            lines: crate::diff::lines("one\ntwo\nthree\n", "one\ntwo and a half\nthree\n"),
            scroll: 0,
            restore: None,
        };

        let lines = screen(&mut app);
        let screen = lines.join("\n");
        assert!(screen.contains("before the cut"), "{screen}");
        assert!(screen.contains("+ two and a half"), "{screen}");
        assert!(screen.contains("− two"), "{screen}");
        // Removed lines carry the old line number, added ones the new.
        assert!(screen.contains("   2 − two"), "{screen}");
        assert!(screen.contains("   2 + two and a half"), "{screen}");
    }

    #[test]
    fn diff_overlay_survives_a_narrow_terminal_and_long_lines() {
        let mut app = App::new(None).unwrap();
        app.splash = false;
        app.mode = Mode::Diff {
            title: String::from("a very long comparison title that will not fit at all"),
            lines: vec![DiffLine {
                tag: crate::diff::DiffTag::Insert,
                // Wide and combining characters: naive char-count clipping is
                // exactly what breaks terminal rendering.
                text: "日本語のテキスト ".repeat(20),
                old_line: None,
                new_line: Some(1),
            }],
            scroll: 0,
            restore: None,
        };

        let mut terminal = Terminal::new(TestBackend::new(20, 6)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn scrolled_diff_shows_a_position_indicator() {
        let mut app = App::new(None).unwrap();
        app.splash = false;
        let many = (0..80)
            .map(|i| DiffLine {
                tag: crate::diff::DiffTag::Equal,
                text: format!("line {i}"),
                old_line: Some(i + 1),
                new_line: Some(i + 1),
            })
            .collect();
        app.mode = Mode::Diff {
            title: String::from("old → new"),
            lines: many,
            scroll: 40,
            restore: None,
        };

        let screen = screen(&mut app).join("\n");
        assert!(screen.contains("line 40"), "{screen}");
        assert!(!screen.contains("line 39"), "scrolled past: {screen}");
        assert!(screen.contains("/80"), "{screen}");
    }

    /// Render a frame and return each cell's colours for one row. Colours, not
    /// whole `Style`s: rendering fills in fields (underline colour) that a theme
    /// style leaves unset, so comparing styles wholesale compares noise.
    fn row_colors(app: &mut App, row: u16) -> Vec<(Color, Color)> {
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.width)
            .map(|x| {
                let style = buffer[(x, row)].style();
                (
                    style.fg.unwrap_or(Color::Reset),
                    style.bg.unwrap_or(Color::Reset),
                )
            })
            .collect()
    }

    fn colors_of(style: Style) -> (Color, Color) {
        (
            style.fg.unwrap_or(Color::Reset),
            style.bg.unwrap_or(Color::Reset),
        )
    }

    fn app_with(text: &str) -> App {
        let mut app = App::new(None).unwrap();
        app.splash = false;
        app.buf.insert(0, text);
        app
    }

    /// Put the app in focus mode. The command path itself is covered by the
    /// app-level tests; these tests are about what the frame looks like.
    fn enter_focus(app: &mut App) {
        app.focus = Some(crate::sprint::Focus::enter(app.help_level));
        app.help_level = 0;
    }

    #[test]
    fn focus_mode_leaves_the_status_row_blank_until_it_has_something_to_say() {
        let mut app = app_with("a line of prose\n");
        enter_focus(&mut app);

        let lines = screen(&mut app);
        let status = lines.last().unwrap();
        assert!(status.trim().is_empty(), "focus status row: {status:?}");
        // The chrome the normal status line carries is gone.
        assert!(!lines.join("\n").contains("Ln 1"), "{lines:#?}");

        // A message still reaches the writer.
        app.status_msg = Some(String::from("Saved chapter.md"));
        let status = screen(&mut app).last().unwrap().clone();
        assert!(status.contains("Saved chapter.md"), "{status:?}");
    }

    #[test]
    fn a_running_sprint_shows_its_countdown_in_both_views() {
        let mut app = app_with("prose\n");
        app.sprint = Some(
            crate::sprint::Sprint::parse("25/500", app.doc_stats.words, Instant::now()).unwrap(),
        );

        // Normal view: the chip sits with the rest of the status readouts.
        let status = screen(&mut app).last().unwrap().clone();
        assert!(status.contains('⏱'), "{status:?}");
        assert!(status.contains("0/500"), "{status:?}");
        assert!(
            status.contains("Ln 1"),
            "still the full status line: {status:?}"
        );

        // Focus view: the countdown is the *only* thing on the row.
        enter_focus(&mut app);
        let status = screen(&mut app).last().unwrap().clone();
        assert!(status.contains('⏱'), "{status:?}");
        assert!(!status.contains("Ln 1"), "{status:?}");
    }

    #[test]
    fn focus_dim_dims_only_the_lines_outside_the_current_paragraph() {
        // Cursor starts at 0, so the first paragraph is the lit one.
        let mut app = app_with("first para line\n\nsecond para\n");
        app.cursor = 0;
        enter_focus(&mut app);
        assert!(app.focus_dim, "dimming is on by default (R3.4)");

        let dim = colors_of(app.theme.dim);
        assert_ne!(
            row_colors(&mut app, 0)[0],
            dim,
            "the paragraph being written stays lit"
        );
        assert_eq!(row_colors(&mut app, 2)[0], dim, "other paragraphs recede");

        // Turning it off leaves the page evenly lit (R3.4: configurable off).
        app.focus_dim = false;
        assert_ne!(row_colors(&mut app, 2)[0], dim);
    }

    #[test]
    fn the_stats_overlay_lists_recent_sprints() {
        // R3.2 files sprints in the history; R2.5 requires that history be
        // viewable, and ^OI is where a writer looks for it.
        let mut app = app_with("prose\n");
        app.daily_history.record_sprint(520, 754, true);
        app.daily_history.record_sprint(90, 300, false);
        app.mode = Mode::Stats;

        let screen = screen(&mut app).join("\n");
        assert!(screen.contains("Recent sprints"), "{screen}");
        assert!(screen.contains("520 words in 12:34 ✓"), "{screen}");
        assert!(screen.contains("90 words in 5:00"), "{screen}");
        assert!(
            !screen.contains("90 words in 5:00 ✓"),
            "unmet target: {screen}"
        );
    }

    #[test]
    fn the_binder_shows_synopses_as_secondary_lines_and_marks_notes() {
        let mut app = app_with("");
        app.mode = Mode::Binder {
            entries: vec![
                crate::app::BinderEntry {
                    idx: 0,
                    title: String::from("Chapter One"),
                    word_count: Some(1200),
                    exists: true,
                    synopsis: String::from("Marcus finds the knife."),
                },
                crate::app::BinderEntry {
                    idx: 1,
                    title: String::from("Characters"),
                    word_count: Some(80),
                    exists: true,
                    synopsis: String::new(),
                },
            ],
            selected: 0,
        };

        let screen = screen(&mut app).join("\n");
        assert!(screen.contains("Chapter One"), "{screen}");
        assert!(screen.contains("1200 words"), "{screen}");
        // R5.3: the synopsis as a secondary line under its document.
        assert!(screen.contains("Marcus finds the knife."), "{screen}");
        // A document with no synopsis gets no second row.
        let rows: Vec<&str> = screen
            .lines()
            .filter(|r| r.contains("Characters"))
            .collect();
        assert_eq!(rows.len(), 1, "{screen}");
        assert!(
            screen.contains("^PM note"),
            "the keys are advertised: {screen}"
        );
    }

    #[test]
    fn a_long_synopsis_is_clipped_to_the_panel() {
        let mut app = app_with("");
        app.mode = Mode::Binder {
            entries: vec![crate::app::BinderEntry {
                idx: 0,
                title: String::from("Chapter One"),
                word_count: Some(10),
                exists: true,
                synopsis: "word ".repeat(60),
            }],
            selected: 0,
        };

        // The frame renders and every row stays inside the terminal.
        let rows = screen(&mut app);
        assert!(rows.iter().all(|row| row.chars().count() == 80));
        assert!(rows.join("\n").contains('…'), "clipped with an ellipsis");
    }

    #[test]
    fn recovery_mode_renders_keyboard_confirmation_prompt() {
        let mut app = App::new(None).unwrap();
        app.mode = Mode::ConfirmRecover;

        let prompt = status_left(&app);
        assert!(prompt.contains("Recover unsaved changes"));
        assert!(prompt.contains("y/N"));
        assert!(prompt.contains("Esc decline"));
    }
}
