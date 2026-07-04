use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::block::adjust_pos;
use crate::buffer::wrap_segments;
use crate::config::Config;
use crate::history::{Edit, EditGroup, EditKind};
use crate::keymap::{self, Cmd, Prefix};
use crate::killring::{KillRing, PutCycle};
use crate::outline;
use crate::pane::Pane;
use crate::rtf;
use crate::search::{ReplacePhase, ReplaceState, SearchState};
use crate::spellcheck;
use crate::theme::Theme;
use crate::ui;

const JUMP_STACK_MAX: usize = 32;

/// What a text-input prompt is collecting (^KW, ^KR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    WriteBlock,
    ReadFile,
    ExportClean,
    ExportManuscript,
    WrapMargin,
    /// ^OK with one window: filename to open in the second window.
    OpenSplit,
}

/// What the keyboard is currently driving.
pub enum Mode {
    Normal,
    /// Quit with unsaved changes? (y/N)
    ConfirmAbandon,
    Search(SearchState),
    Replace(ReplaceState),
    Input {
        label: &'static str,
        value: String,
        action: InputAction,
    },
    /// The command palette / searchable help (Esc or F1).
    Palette { query: String, selected: usize },
    /// The outline: jump between Markdown headings (^QO).
    Outline {
        entries: Vec<outline::Entry>,
        query: String,
        selected: usize,
    },
}

pub struct App {
    /// The editing windows (one or two, stacked). Per-document state —
    /// buffer, cursor, undo, blocks, bookmarks — lives in each `Pane`.
    pub panes: Vec<Pane>,
    /// Which pane has the keyboard.
    pub active: usize,
    pub theme: Theme,
    /// Showing the startup splash; any keypress dismisses it and is
    /// otherwise discarded (never typed into the document).
    pub splash: bool,
    pub status_msg: Option<String>,
    pub mode: Mode,
    pub prefix: Option<(Prefix, Instant)>,
    /// Shared across panes on purpose: cut in one window, put in the other.
    pub kill: KillRing,
    put_cycle: Option<PutCycle>,
    /// Last accepted search, for ^L — global so a search made in one window
    /// can be repeated in the other.
    pub last_search: Option<String>,
    /// Reveal Codes split pane (^OD).
    pub reveal: bool,
    pub menu_delay: Duration,
    /// 0 = clean screen, 1 = delayed menus, 2 = menus + hint bar.
    pub help_level: u8,
    pub overtype: bool,
    pub wrap: bool,
    pub wrap_margin: usize,
    pub spell: spellcheck::Spellchecker,
    pub spell_enabled: bool,
    /// Keep the cursor's line pinned at a fixed row (view_rows / 2) and
    /// scroll the document under it, instead of only scrolling at the edges.
    pub typewriter: bool,
    /// Body font for `^KM` manuscript RTF export.
    pub manuscript_font: rtf::ManuscriptFont,
    /// Idle autosave; zero disables.
    autosave: Duration,
    /// Keyboard macro (^QM record, ^QJ play).
    macro_keys: Vec<KeyEvent>,
    pub recording: bool,
    playing: bool,
    quit: bool,
}

/// `App` dereferences to the active pane, so the whole editing engine —
/// movement, editing, undo, marks — reads and writes `self.cursor`,
/// `self.buf`, etc. and always operates on the focused window. Deliberate
/// use of deref-to-owned-state: the alternative is threading a pane index
/// through two hundred call sites for the same result.
impl std::ops::Deref for App {
    type Target = Pane;
    fn deref(&self) -> &Pane {
        &self.panes[self.active]
    }
}

impl std::ops::DerefMut for App {
    fn deref_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active]
    }
}

impl App {
    pub fn new(path: Option<PathBuf>) -> io::Result<Self> {
        let pane = Pane::open(path)?;
        let config = Config::load();
        Ok(App {
            panes: vec![pane],
            active: 0,
            theme: config.theme(),
            splash: true,
            status_msg: None,
            mode: Mode::Normal,
            prefix: None,
            kill: KillRing::new(),
            put_cycle: None,
            last_search: None,
            reveal: false,
            menu_delay: Duration::from_millis(config.menu_delay_ms),
            help_level: config.help_level.min(2),
            overtype: false,
            wrap: config.wrap,
            wrap_margin: config.wrap_margin,
            spell: spellcheck::Spellchecker::load(),
            spell_enabled: config.spellcheck,
            typewriter: config.typewriter,
            manuscript_font: config.manuscript_font(),
            autosave: Duration::from_secs(config.autosave_secs),
            macro_keys: Vec::new(),
            recording: false,
            playing: false,
            quit: false,
        })
    }

    /// Effective wrap width for the active pane, if soft wrap is on.
    pub fn wrap_width(&self) -> Option<usize> {
        self.wrap_width_of(self)
    }

    /// Effective wrap width for a given pane, if soft wrap is on.
    pub fn wrap_width_of(&self, pane: &Pane) -> Option<usize> {
        if !self.wrap {
            return None;
        }
        let w = if self.wrap_margin == 0 {
            pane.view_cols
        } else {
            self.wrap_margin.min(pane.view_cols)
        };
        Some(w.max(1))
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.quit {
            terminal.draw(|f| ui::draw(f, self))?;
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => self.handle_key(k),
                    _ => {}
                }
            } else {
                self.maybe_autosave();
            }
        }
        for pane in &self.panes {
            pane.save_session();
        }
        Ok(())
    }

    fn maybe_autosave(&mut self) {
        if self.autosave.is_zero() {
            return;
        }
        let deadline = self.autosave;
        let mut saved = false;
        for pane in &mut self.panes {
            if pane.buf.dirty && pane.buf.path.is_some() && pane.last_edit.elapsed() >= deadline {
                saved |= pane.buf.save().is_ok();
            }
        }
        if saved {
            self.status_msg = Some(String::from("Autosaved"));
        }
    }

    /// The query whose matches should be highlighted in the text area.
    pub fn active_query(&self) -> Option<&str> {
        match &self.mode {
            Mode::Search(s) if !s.query.is_empty() => Some(&s.query),
            Mode::Replace(r) if matches!(r.phase, ReplacePhase::Confirm(_)) => Some(&r.find),
            _ => None,
        }
    }

    // --- key dispatch -----------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) {
        if self.splash {
            self.splash = false;
            return;
        }
        self.status_msg = None;

        if self.recording && !self.playing {
            self.macro_keys.push(key);
        }

        if let Some((prefix, _)) = self.prefix.take() {
            self.handle_prefixed(prefix, key);
            return;
        }

        // Any keystroke that isn't (potentially) part of a ^KP chord ends a
        // put-cycle.
        if !is_prefix_key(&key) {
            self.put_cycle = None;
        }

        match &mut self.mode {
            Mode::ConfirmAbandon => {
                let yes = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
                self.mode = Mode::Normal;
                if yes {
                    self.close_or_quit();
                }
            }
            Mode::Search(_) => self.handle_search_key(key),
            Mode::Replace(_) => self.handle_replace_key(key),
            Mode::Input { .. } => self.handle_input_key(key),
            Mode::Palette { .. } => self.handle_palette_key(key),
            Mode::Outline { .. } => self.handle_outline_key(key),
            Mode::Normal => self.handle_normal_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char(c) if ctrl => {
                let c = c.to_ascii_lowercase();
                match c {
                    'k' => self.prefix = Some((Prefix::K, Instant::now())),
                    'q' => self.prefix = Some((Prefix::Q, Instant::now())),
                    'o' => self.prefix = Some((Prefix::O, Instant::now())),
                    _ => {
                        if let Some(cmd) = keymap::lookup_bare(c) {
                            self.execute(cmd);
                        }
                    }
                }
            }
            KeyCode::Up => self.execute(Cmd::Up),
            KeyCode::Down => self.execute(Cmd::Down),
            KeyCode::Left => self.execute(Cmd::Left),
            KeyCode::Right => self.execute(Cmd::Right),
            KeyCode::PageUp => self.execute(Cmd::PageUp),
            KeyCode::PageDown => self.execute(Cmd::PageDown),
            KeyCode::Home => self.execute(Cmd::LineStart),
            KeyCode::End => self.execute(Cmd::LineEnd),
            KeyCode::Insert => self.execute(Cmd::ToggleInsert),
            KeyCode::F(1) => self.execute(Cmd::Palette),
            KeyCode::Esc => self.execute(Cmd::Palette),
            KeyCode::Backspace => self.execute(Cmd::DeleteLeft),
            KeyCode::Delete => self.execute(Cmd::DeleteRight),
            KeyCode::Enter => self.insert_text("\n", EditKind::Other),
            KeyCode::Tab => self.insert_text("\t", EditKind::InsertChar),
            KeyCode::Char(c) => self.insert_text(&c.to_string(), EditKind::InsertChar),
            _ => {}
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Mode::Palette { query, selected } = &mut self.mode else {
            return;
        };
        let entries = keymap::filtered_entries(query);
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Down => *selected = (*selected + 1).min(entries.len().saturating_sub(1)),
            KeyCode::Char('e') if ctrl => *selected = selected.saturating_sub(1),
            KeyCode::Char('x') if ctrl => {
                *selected = (*selected + 1).min(entries.len().saturating_sub(1))
            }
            KeyCode::Backspace => {
                query.pop();
                *selected = 0;
            }
            KeyCode::Enter => {
                let cmd = entries.get(*selected).map(|e| e.0);
                self.mode = Mode::Normal;
                if let Some(cmd) = cmd {
                    self.execute(cmd);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                query.push(c);
                *selected = 0;
            }
            _ => {}
        }
    }

    /// The outline: jump between Markdown headings, filtered by title.
    fn handle_outline_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Mode::Outline { entries, query, selected } = &mut self.mode else {
            return;
        };
        let matches: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                query.is_empty() || e.title.to_lowercase().contains(&query.to_lowercase())
            })
            .map(|(i, _)| i)
            .collect();
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Down => *selected = (*selected + 1).min(matches.len().saturating_sub(1)),
            KeyCode::Char('e') if ctrl => *selected = selected.saturating_sub(1),
            KeyCode::Char('x') if ctrl => {
                *selected = (*selected + 1).min(matches.len().saturating_sub(1))
            }
            KeyCode::Backspace => {
                query.pop();
                *selected = 0;
            }
            KeyCode::Enter => {
                let target = matches.get(*selected).map(|&i| entries[i].char_pos);
                self.mode = Mode::Normal;
                if let Some(pos) = target {
                    self.long_jump(pos);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                query.push(c);
                *selected = 0;
            }
            _ => {}
        }
    }

    fn handle_prefixed(&mut self, prefix: Prefix, key: KeyEvent) {
        let code = match key.code {
            KeyCode::Char(c) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        match code {
            Some(d) if d.is_ascii_digit() => {
                let slot = d as usize - '0' as usize;
                match prefix {
                    Prefix::K => self.set_bookmark(slot),
                    Prefix::Q => self.jump_bookmark(slot),
                    Prefix::O => self.status_msg = Some(String::from("Unknown command")),
                }
            }
            Some(c) => match keymap::lookup_prefixed(prefix, c) {
                Some(cmd) => self.execute(cmd),
                None => {
                    self.status_msg =
                        Some(format!("Unknown command {}{c}", prefix_caret(prefix)))
                }
            },
            None if key.code == KeyCode::Esc => {}
            None => self.status_msg = Some(String::from("Unknown command")),
        }
    }

    fn execute(&mut self, cmd: Cmd) {
        if cmd != Cmd::Undo {
            self.history.break_chain();
        }
        if cmd != Cmd::Put {
            self.put_cycle = None;
        }
        match cmd {
            Cmd::Up => self.move_vertical(-1),
            Cmd::Down => self.move_vertical(1),
            Cmd::Left => self.set_cursor(self.buf.prev_grapheme(self.cursor)),
            Cmd::Right => self.set_cursor(self.buf.next_grapheme(self.cursor)),
            Cmd::WordLeft => self.set_cursor(self.buf.word_left(self.cursor)),
            Cmd::WordRight => self.set_cursor(self.buf.word_right(self.cursor)),
            Cmd::ScrollUp => self.scroll(-1),
            Cmd::ScrollDown => self.scroll(1),
            Cmd::PageUp => self.page(-1),
            Cmd::PageDown => self.page(1),
            Cmd::LineStart => self.set_cursor(self.buf.line_start(self.buf.line_of(self.cursor))),
            Cmd::LineEnd => self.set_cursor(self.buf.line_end(self.buf.line_of(self.cursor))),
            Cmd::ScreenTop => self.move_to_screen_row(0),
            Cmd::ScreenBottom => self.move_to_screen_row(self.view_rows.saturating_sub(1)),
            Cmd::DocStart => self.long_jump(0),
            Cmd::DocEnd => self.long_jump(self.buf.len_chars()),
            Cmd::SentenceBack => self.set_cursor(self.buf.sentence_back(self.cursor)),
            Cmd::SentenceFwd => self.set_cursor(self.buf.sentence_fwd(self.cursor)),
            Cmd::ParaBack => self.set_cursor(self.buf.para_back(self.cursor)),
            Cmd::ParaFwd => self.set_cursor(self.buf.para_fwd(self.cursor)),
            Cmd::PrevPosition => self.jump_back(),
            Cmd::Outline => {
                let entries = outline::scan(&self.buf);
                if entries.is_empty() {
                    self.status_msg = Some(String::from("No headings in this document"));
                } else {
                    self.mode = Mode::Outline {
                        entries,
                        query: String::new(),
                        selected: 0,
                    };
                }
            }
            Cmd::NextMisspelling => {
                if !self.spell_enabled {
                    self.status_msg = Some(String::from("Spellcheck is off (^OS to enable)"));
                } else if let Some(pos) = self.find_next_misspelling() {
                    self.long_jump(pos);
                } else {
                    self.status_msg = Some(String::from("No misspelled words found"));
                }
            }

            Cmd::DeleteRight => self.delete_right(),
            Cmd::DeleteLeft => self.delete_left(),
            Cmd::DeleteWordRight => {
                self.delete_range(self.cursor, self.buf.word_right(self.cursor), true)
            }
            Cmd::DeleteLine => self.delete_line(),
            Cmd::DeleteToLineEnd => self.delete_range(
                self.cursor,
                self.buf.line_end(self.buf.line_of(self.cursor)),
                true,
            ),
            Cmd::InsertBlankLine => self.insert_blank_line(),
            Cmd::TransposeWords => self.transpose_words(),
            Cmd::TransposeChars => self.transpose_chars(),
            Cmd::Undo => self.undo(),

            Cmd::FindIncremental => {
                self.history.break_group();
                self.mode = Mode::Search(SearchState::new(self.cursor));
            }
            Cmd::FindReplace => {
                self.history.break_group();
                self.mode = Mode::Replace(ReplaceState::new());
            }
            Cmd::FindNext => self.find_next(),

            Cmd::BlockBegin => {
                self.blocks.begin = Some(self.cursor);
                self.blocks.hidden = false;
                self.status_msg = Some(String::from("Block begin set"));
            }
            Cmd::BlockEnd => {
                self.blocks.end = Some(self.cursor);
                self.blocks.hidden = false;
                self.status_msg = Some(String::from("Block end set"));
            }
            Cmd::BlockCopy => self.block_copy(),
            Cmd::BlockMove => self.block_move(),
            Cmd::BlockDelete => self.block_delete(),
            Cmd::BlockWrite => {
                if self.blocks.range().is_some() {
                    self.mode = Mode::Input {
                        label: "Write block to file",
                        value: String::new(),
                        action: InputAction::WriteBlock,
                    };
                } else {
                    self.status_msg = Some(String::from("No block marked"));
                }
            }
            Cmd::BlockRead => {
                self.mode = Mode::Input {
                    label: "Read file",
                    value: String::new(),
                    action: InputAction::ReadFile,
                };
            }
            Cmd::BlockHide => {
                if self.blocks.range().is_some() {
                    self.blocks.hidden = !self.blocks.hidden;
                } else {
                    self.status_msg = Some(String::from("No block marked"));
                }
            }
            Cmd::BlockPrev => {
                if !self.blocks.toggle_previous() {
                    self.status_msg = Some(String::from("No previous block"));
                }
            }
            Cmd::Put => self.put(),
            Cmd::JumpBlockBegin => match self.blocks.begin {
                Some(p) => self.long_jump(p.min(self.buf.len_chars())),
                None => self.status_msg = Some(String::from("No block begin")),
            },
            Cmd::JumpBlockEnd => match self.blocks.end {
                Some(p) => self.long_jump(p.min(self.buf.len_chars())),
                None => self.status_msg = Some(String::from("No block end")),
            },
            Cmd::JumpBlockSource => match self.blocks.source {
                Some(p) => self.long_jump(p.min(self.buf.len_chars())),
                None => self.status_msg = Some(String::from("Block has not been moved")),
            },

            Cmd::Save => self.save(),
            Cmd::SaveExit => {
                self.save();
                if !self.buf.dirty {
                    self.close_or_quit();
                }
            }
            Cmd::Quit => {
                if self.buf.dirty {
                    self.mode = Mode::ConfirmAbandon;
                } else {
                    self.close_or_quit();
                }
            }
            Cmd::OtherWindow => {
                if self.panes.len() == 1 {
                    self.mode = Mode::Input {
                        label: "Open in second window",
                        value: String::new(),
                        action: InputAction::OpenSplit,
                    };
                } else {
                    self.active = 1 - self.active;
                }
            }
            Cmd::CopyFromOther => self.copy_from_other(),
            Cmd::CycleTheme => self.theme = self.theme.next(),
            Cmd::RevealCodes => self.reveal = !self.reveal,
            Cmd::ExportClean => {
                self.mode = Mode::Input {
                    label: "Export to file (notes stripped)",
                    value: String::new(),
                    action: InputAction::ExportClean,
                };
            }
            Cmd::ExportManuscript => {
                self.mode = Mode::Input {
                    label: "Export manuscript RTF to file",
                    value: String::new(),
                    action: InputAction::ExportManuscript,
                };
            }
            Cmd::ToggleWrap => {
                self.wrap = !self.wrap;
                self.left_col = 0;
                self.status_msg = Some(String::from(if self.wrap {
                    "Word wrap on"
                } else {
                    "Word wrap off"
                }));
            }
            Cmd::SetWrapMargin => {
                self.mode = Mode::Input {
                    label: "Wrap margin in columns (0 = window width)",
                    value: String::new(),
                    action: InputAction::WrapMargin,
                };
            }
            Cmd::ToggleInsert => {
                self.overtype = !self.overtype;
            }
            Cmd::CycleHelpLevel => {
                self.help_level = (self.help_level + 1) % 3;
                self.status_msg = Some(format!(
                    "Help level {} — {}",
                    self.help_level,
                    match self.help_level {
                        0 => "clean screen",
                        1 => "delayed menus",
                        _ => "menus + hint bar",
                    }
                ));
            }
            Cmd::Palette => {
                self.mode = Mode::Palette {
                    query: String::new(),
                    selected: 0,
                };
            }
            Cmd::ToggleSpellcheck => {
                self.spell_enabled = !self.spell_enabled;
                self.status_msg = Some(String::from(if self.spell_enabled {
                    "Spellcheck on"
                } else {
                    "Spellcheck off"
                }));
            }
            Cmd::AddToDictionary => match self.word_at_cursor() {
                Some(w) => {
                    self.spell.learn(&w);
                    self.status_msg = Some(format!("Added \"{w}\" to personal dictionary"));
                }
                None => self.status_msg = Some(String::from("No word at cursor")),
            },
            Cmd::ToggleTypewriter => {
                self.typewriter = !self.typewriter;
                self.status_msg = Some(String::from(if self.typewriter {
                    "Typewriter scrolling on"
                } else {
                    "Typewriter scrolling off"
                }));
            }
            Cmd::MacroRecord => {
                if self.recording {
                    // Drop the ^Q,M chord that ended the recording.
                    let n = self.macro_keys.len();
                    self.macro_keys.truncate(n.saturating_sub(2));
                    self.recording = false;
                    self.status_msg =
                        Some(format!("Macro recorded ({} keys)", self.macro_keys.len()));
                } else {
                    self.macro_keys.clear();
                    self.recording = true;
                }
            }
            Cmd::MacroPlay => self.play_macro(),
        }
        if !matches!(cmd, Cmd::Undo) && !is_edit_cmd(cmd) {
            self.history.break_group();
        }
    }

    // --- movement -----------------------------------------------------------

    fn goal(&mut self) -> usize {
        match self.goal_col {
            Some(g) => g,
            None => {
                let g = self.buf.visual_col(self.cursor);
                self.goal_col = Some(g);
                g
            }
        }
    }

    /// Move to `pos`, resetting the sticky column.
    fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos.min(self.buf.len_chars());
        self.goal_col = None;
        self.ensure_visible();
    }

    /// Remember the current position on the jump ring.
    fn push_jump(&mut self) {
        if self.jump_stack.last() != Some(&self.cursor) {
            self.jump_stack.push(self.cursor);
            if self.jump_stack.len() > JUMP_STACK_MAX {
                self.jump_stack.remove(0);
            }
        }
    }

    /// A jump the writer will want to return from (^QP).
    fn long_jump(&mut self, pos: usize) {
        self.push_jump();
        self.set_cursor(pos);
    }

    /// ^QP: go back to where you were; repeated presses walk the ring.
    fn jump_back(&mut self) {
        while let Some(p) = self.jump_stack.pop() {
            if p != self.cursor && p <= self.buf.len_chars() {
                // Rotate: the position we're leaving goes to the bottom, so
                // repeated ^QP cycles rather than exhausts.
                self.jump_stack.insert(0, self.cursor);
                self.set_cursor(p);
                return;
            }
        }
        self.status_msg = Some(String::from("No previous position"));
    }

    fn move_vertical(&mut self, delta: isize) {
        let line = self.buf.line_of(self.cursor);
        let goal = self.goal();
        let last = self.buf.len_lines().saturating_sub(1);
        let target = line.saturating_add_signed(delta).min(last);
        self.cursor = self.buf.char_at_visual_col(target, goal);
        self.ensure_visible();
    }

    fn move_to_screen_row(&mut self, row: usize) {
        let goal = self.goal();
        let last = self.buf.len_lines().saturating_sub(1);
        let target = (self.top_line + row).min(last);
        self.cursor = self.buf.char_at_visual_col(target, goal);
        self.ensure_visible();
    }

    /// Scroll the view by one line; the cursor is dragged along only if it
    /// would leave the screen (WordStar ^W/^Z behavior).
    fn scroll(&mut self, delta: isize) {
        let last = self.buf.len_lines().saturating_sub(1);
        self.top_line = self.top_line.saturating_add_signed(delta).min(last);
        let line = self.buf.line_of(self.cursor);
        let goal = self.goal();
        let bottom = self.top_line + self.view_rows.saturating_sub(1);
        if line < self.top_line {
            self.cursor = self.buf.char_at_visual_col(self.top_line, goal);
        } else if line > bottom {
            self.cursor = self.buf.char_at_visual_col(bottom.min(last), goal);
        }
    }

    fn page(&mut self, dir: isize) {
        let step = self.view_rows.saturating_sub(1).max(1);
        let last = self.buf.len_lines().saturating_sub(1);
        let line = self.buf.line_of(self.cursor);
        let goal = self.goal();
        let target = line.saturating_add_signed(dir * step as isize).min(last);
        self.top_line = self
            .top_line
            .saturating_add_signed(dir * step as isize)
            .min(last);
        self.cursor = self.buf.char_at_visual_col(target, goal);
        self.ensure_visible();
    }

    /// Keep the cursor inside the viewport, adjusting scroll as needed.
    pub fn ensure_visible(&mut self) {
        if let Some(width) = self.wrap_width() {
            self.ensure_visible_wrapped(width);
            return;
        }
        let line = self.buf.line_of(self.cursor);
        if self.typewriter {
            // Pin the cursor's line at a fixed row; the document scrolls
            // under it instead of the cursor moving to the edge.
            self.top_line = line.saturating_sub(self.view_rows / 2);
        } else if line < self.top_line {
            self.top_line = line;
        } else {
            let bottom = self.top_line + self.view_rows.saturating_sub(1);
            if line > bottom {
                self.top_line = line - self.view_rows.saturating_sub(1);
            }
        }
        let vcol = self.buf.visual_col(self.cursor);
        if vcol < self.left_col {
            self.left_col = vcol;
        }
        let width = self.view_cols.max(1);
        if vcol >= self.left_col + width {
            self.left_col = vcol + 1 - width;
        }
    }

    fn ensure_visible_wrapped(&mut self, width: usize) {
        self.left_col = 0;
        let cline = self.buf.line_of(self.cursor);

        if self.typewriter {
            // Walk back from the cursor's own wrapped segment, accumulating
            // rows, until we've gone back far enough to center it.
            let target_row = self.view_rows / 2;
            let (seg_idx, _) = self.cursor_segment(width);
            let mut line = cline;
            let mut accumulated = seg_idx;
            while accumulated < target_row && line > 0 {
                line -= 1;
                accumulated += wrap_segments(&self.buf.line_text(line), width).len().max(1);
            }
            self.top_line = line;
            return;
        }

        if cline < self.top_line {
            self.top_line = cline;
            return;
        }
        // Every line occupies at least one row, so this is a safe floor.
        if cline >= self.top_line + self.view_rows {
            self.top_line = cline + 1 - self.view_rows;
        }
        // Scroll down until the cursor's wrapped row fits.
        while self.top_line < cline {
            let mut rows = 0usize;
            for line in self.top_line..cline {
                rows += wrap_segments(&self.buf.line_text(line), width).len();
                if rows >= self.view_rows {
                    break;
                }
            }
            let (seg_idx, _) = self.cursor_segment(width);
            if rows + seg_idx + 1 <= self.view_rows {
                break;
            }
            self.top_line += 1;
        }
    }

    /// The cursor's wrapped-segment index within its line and its visual
    /// column within that segment.
    pub fn cursor_segment(&self, width: usize) -> (usize, usize) {
        let cline = self.buf.line_of(self.cursor);
        let text = self.buf.line_text(cline);
        let off = self.cursor - self.buf.line_start(cline);
        let segs = wrap_segments(&text, width);
        let idx = segs
            .iter()
            .position(|&(s, e)| off >= s && off < e)
            .unwrap_or(segs.len().saturating_sub(1));
        let (s, _) = segs[idx.min(segs.len() - 1)];
        let vcol = crate::buffer::segment_vcol(&text, s, off);
        (idx, vcol)
    }

    fn play_macro(&mut self) {
        if self.recording {
            self.status_msg = Some(String::from("Can't play while recording"));
            return;
        }
        if self.playing {
            return; // no recursive playback
        }
        if self.macro_keys.is_empty() {
            self.status_msg = Some(String::from("No macro recorded"));
            return;
        }
        self.playing = true;
        for key in self.macro_keys.clone() {
            if self.quit {
                break;
            }
            self.handle_key(key);
        }
        self.playing = false;
    }

    // --- bookmarks ------------------------------------------------------------

    fn set_bookmark(&mut self, slot: usize) {
        self.bookmarks[slot] = Some(self.cursor);
        self.status_msg = Some(format!("Bookmark {slot} set"));
    }

    fn jump_bookmark(&mut self, slot: usize) {
        match self.bookmarks[slot] {
            Some(p) => self.long_jump(p.min(self.buf.len_chars())),
            None => self.status_msg = Some(format!("Bookmark {slot} not set")),
        }
    }

    // --- spellcheck -------------------------------------------------------------

    /// The word span touching the cursor — either it's inside one, or it
    /// sits right after one, as it does right after typing a word.
    fn word_at_cursor(&self) -> Option<String> {
        let line = self.buf.line_of(self.cursor);
        let line_start = self.buf.line_start(line);
        let text = self.buf.line_text(line);
        let offset = self.cursor - line_start;
        spellcheck::word_spans(&text)
            .into_iter()
            .find(|&(s, e)| s != e && offset >= s && offset <= e)
            .map(|(s, e)| text.chars().skip(s).take(e - s).collect())
    }

    /// ^QN: the char position of the next misspelled word strictly after the
    /// cursor, wrapping around the whole document if nothing is found first.
    /// Note (`..`) lines are skipped, matching what's rendered.
    fn find_next_misspelling(&self) -> Option<usize> {
        let total_lines = self.buf.len_lines();
        if total_lines == 0 {
            return None;
        }
        let start_line = self.buf.line_of(self.cursor);
        for offset in 0..=total_lines {
            let line = (start_line + offset) % total_lines;
            let text = self.buf.line_text(line);
            if text.trim_start().starts_with("..") {
                continue;
            }
            let line_start = self.buf.line_start(line);
            for (s, e) in spellcheck::word_spans(&text) {
                let char_pos = line_start + s;
                if offset == 0 && char_pos <= self.cursor {
                    continue;
                }
                let word: String = text.chars().skip(s).take(e - s).collect();
                if !self.spell.check(&word) {
                    return Some(char_pos);
                }
            }
        }
        None
    }

    // --- editing (all mutations go through here, feeding history) -----------

    /// Mutate the rope and keep every mark in step. Returns the deleted text.
    fn apply_raw(&mut self, at: usize, del_chars: usize, insert: &str) -> String {
        let deleted = if del_chars > 0 {
            self.buf.delete(at..at + del_chars)
        } else {
            String::new()
        };
        if !insert.is_empty() {
            self.buf.insert(at, insert);
        }
        self.buf.dirty = true;
        let ins = insert.chars().count();
        self.blocks.adjust(at, del_chars, ins);
        for b in self.bookmarks.iter_mut() {
            if let Some(p) = *b {
                *b = Some(adjust_pos(p, at, del_chars, ins));
            }
        }
        for p in self.jump_stack.iter_mut() {
            *p = adjust_pos(*p, at, del_chars, ins);
        }
        self.last_edit = Instant::now();
        deleted
    }

    /// Replace `del_chars` chars at `at` with `insert`, record it, and move
    /// the cursor to `cursor_after`. Returns the deleted text.
    fn apply_edit(
        &mut self,
        at: usize,
        del_chars: usize,
        insert: &str,
        kind: EditKind,
        cursor_after: usize,
    ) -> String {
        let cursor_before = self.cursor;
        let deleted = self.apply_raw(at, del_chars, insert);
        self.history.record(
            Edit {
                at,
                deleted: deleted.clone(),
                inserted: insert.to_string(),
            },
            kind,
            cursor_before,
            cursor_after,
        );
        self.cursor = cursor_after;
        self.goal_col = None;
        self.ensure_visible();
        deleted
    }

    fn insert_text(&mut self, text: &str, kind: EditKind) {
        // Overtype: a typed character replaces the one under the cursor,
        // except at line end (never eats the newline).
        let del = if self.overtype && kind == EditKind::InsertChar && text != "\n" {
            let end = self.buf.next_grapheme(self.cursor);
            let line_end = self.buf.line_end(self.buf.line_of(self.cursor));
            if end > self.cursor && self.cursor < line_end {
                end - self.cursor
            } else {
                0
            }
        } else {
            0
        };
        self.apply_edit(
            self.cursor,
            del,
            text,
            if del > 0 { EditKind::Other } else { kind },
            self.cursor + text.chars().count(),
        );
    }

    /// ^N: open a new line at the cursor without moving (WordStar behavior).
    fn insert_blank_line(&mut self) {
        self.apply_edit(self.cursor, 0, "\n", EditKind::Other, self.cursor);
    }

    fn delete_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.buf.prev_grapheme(self.cursor);
        self.apply_edit(start, self.cursor - start, "", EditKind::DeleteLeft, start);
    }

    fn delete_right(&mut self) {
        let end = self.buf.next_grapheme(self.cursor);
        if end > self.cursor {
            self.apply_edit(self.cursor, end - self.cursor, "", EditKind::Other, self.cursor);
        }
    }

    /// Delete a range; with `kill`, the text joins the kill ring.
    fn delete_range(&mut self, from: usize, to: usize, kill: bool) {
        if to > from {
            let deleted = self.apply_edit(from, to - from, "", EditKind::Other, from);
            if kill {
                self.kill.push(deleted);
            }
        }
    }

    /// ^Y: delete the whole line including its terminator.
    fn delete_line(&mut self) {
        let line = self.buf.line_of(self.cursor);
        let start = self.buf.line_start(line);
        let end = if line + 1 < self.buf.len_lines() {
            self.buf.line_start(line + 1)
        } else {
            self.buf.len_chars()
        };
        self.delete_range(start, end, true);
    }

    /// ^QG: swap the graphemes on either side of the cursor and advance
    /// (Emacs C-t semantics).
    fn transpose_chars(&mut self) {
        let len = self.buf.len_chars();
        if self.cursor == 0 || self.cursor >= len {
            return;
        }
        let a_start = self.buf.prev_grapheme(self.cursor);
        let b_end = self.buf.next_grapheme(self.cursor);
        let a: String = self.buf.rope.slice(a_start..self.cursor).to_string();
        let b: String = self.buf.rope.slice(self.cursor..b_end).to_string();
        if a.contains('\n') || b.contains('\n') {
            return;
        }
        let swapped = format!("{b}{a}");
        self.apply_edit(a_start, b_end - a_start, &swapped, EditKind::Other, b_end);
    }

    /// ^QT: swap the word before the cursor with the word after it, leaving
    /// the cursor after the pair (Emacs M-t semantics).
    fn transpose_words(&mut self) {
        let mut a_end = self.cursor;
        while a_end > 0 && !is_word(self.buf.rope.char(a_end - 1)) {
            a_end -= 1;
        }
        let a_start = self.buf.word_left(a_end);
        if a_end == 0 || a_start == a_end {
            return;
        }
        let len = self.buf.len_chars();
        let mut b_start = self.cursor;
        while b_start < len && !is_word(self.buf.rope.char(b_start)) {
            b_start += 1;
        }
        let mut b_end = b_start;
        while b_end < len && is_word(self.buf.rope.char(b_end)) {
            b_end += 1;
        }
        if b_start >= b_end || b_start < a_end {
            return;
        }
        let a: String = self.buf.rope.slice(a_start..a_end).to_string();
        let mid: String = self.buf.rope.slice(a_end..b_start).to_string();
        let b: String = self.buf.rope.slice(b_start..b_end).to_string();
        let swapped = format!("{b}{mid}{a}");
        self.apply_edit(a_start, b_end - a_start, &swapped, EditKind::Other, b_end);
    }

    fn undo(&mut self) {
        let Some(group) = self.history.next_undo() else {
            self.status_msg = Some(String::from("Nothing to undo"));
            return;
        };
        let mut inv_edits = Vec::with_capacity(group.edits.len());
        for e in group.edits.iter().rev() {
            let ins_len = e.inserted.chars().count();
            self.apply_raw(e.at, ins_len, &e.deleted);
            inv_edits.push(e.inverse());
        }
        let inverse = EditGroup {
            edits: inv_edits,
            cursor_before: group.cursor_after,
            cursor_after: group.cursor_before,
        };
        self.cursor = group.cursor_before.min(self.buf.len_chars());
        self.goal_col = None;
        self.history.confirm_undo(inverse);
        self.ensure_visible();
    }

    // --- blocks & kill ring ---------------------------------------------------

    fn block_copy(&mut self) {
        let Some((b, e)) = self.blocks.range() else {
            self.status_msg = Some(String::from("No block marked"));
            return;
        };
        let text: String = self.buf.rope.slice(b..e).to_string();
        let len = e - b;
        let at = self.cursor;
        self.kill.push(text.clone());
        self.blocks.remember();
        self.apply_edit(at, 0, &text, EditKind::Other, at);
        // The marks moved with the edit; re-point them at the fresh copy and
        // remember where it came from.
        self.blocks.begin = Some(at);
        self.blocks.end = Some(at + len);
        self.blocks.source = Some(adjust_pos(b, at, 0, len));
        self.blocks.hidden = false;
    }

    fn block_move(&mut self) {
        let Some((b, e)) = self.blocks.range() else {
            self.status_msg = Some(String::from("No block marked"));
            return;
        };
        let at = self.cursor;
        if at >= b && at <= e {
            self.status_msg = Some(String::from("Cursor is inside the block"));
            return;
        }
        let text: String = self.buf.rope.slice(b..e).to_string();
        let len = e - b;
        let cursor_before = self.cursor;
        // Two edits, one undo group: delete the block, insert it at the
        // destination (in post-delete coordinates).
        let dest = if at > e { at - len } else { at };
        self.apply_raw(b, len, "");
        self.apply_raw(dest, 0, &text);
        self.history.record_group(
            vec![
                Edit {
                    at: b,
                    deleted: text.clone(),
                    inserted: String::new(),
                },
                Edit {
                    at: dest,
                    deleted: String::new(),
                    inserted: text,
                },
            ],
            cursor_before,
            dest,
        );
        self.blocks.remember();
        self.blocks.begin = Some(dest);
        self.blocks.end = Some(dest + len);
        self.blocks.source = Some(b.min(self.buf.len_chars()));
        self.blocks.hidden = false;
        self.set_cursor(dest);
    }

    fn block_delete(&mut self) {
        let Some((b, e)) = self.blocks.range() else {
            self.status_msg = Some(String::from("No block marked"));
            return;
        };
        self.delete_range(b, e, true);
        self.blocks.begin = None;
        self.blocks.end = None;
    }

    /// ^KP: put the newest clipping; pressed again immediately, swap it for
    /// the next older one (Emacs yank-pop).
    fn put(&mut self) {
        match self.put_cycle {
            Some(PutCycle { at, chars, index }) => {
                if self.kill.len() < 2 {
                    self.status_msg = Some(String::from("No older clippings"));
                    return;
                }
                let next = (index + 1) % self.kill.len();
                let text = self.kill.get(next).cloned().unwrap_or_default();
                let n = text.chars().count();
                self.apply_edit(at, chars, &text, EditKind::Other, at + n);
                self.put_cycle = Some(PutCycle {
                    at,
                    chars: n,
                    index: next,
                });
                self.status_msg = Some(format!("Clipping {}/{}", next + 1, self.kill.len()));
            }
            None => {
                let Some(text) = self.kill.top() else {
                    self.status_msg = Some(String::from("Nothing to put"));
                    return;
                };
                let at = self.cursor;
                let n = text.chars().count();
                self.apply_edit(at, 0, &text, EditKind::Other, at + n);
                self.put_cycle = Some(PutCycle {
                    at,
                    chars: n,
                    index: 0,
                });
            }
        }
    }

    // --- input prompts (^KW, ^KR) ----------------------------------------------

    fn handle_input_key(&mut self, key: KeyEvent) {
        let Mode::Input { value, action, .. } = &mut self.mode else {
            return;
        };
        let action = *action;
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => value.push(c),
            KeyCode::Backspace => {
                value.pop();
            }
            KeyCode::Enter => {
                let path = value.trim().to_string();
                self.mode = Mode::Normal;
                if path.is_empty() {
                    return;
                }
                match action {
                    InputAction::WriteBlock => self.write_block(&path),
                    InputAction::ReadFile => self.read_file(&path),
                    InputAction::ExportClean => self.export_clean(&path),
                    InputAction::ExportManuscript => self.export_manuscript(&path),
                    InputAction::WrapMargin => match path.parse::<usize>() {
                        Ok(n) => {
                            self.wrap_margin = n;
                            self.wrap = true;
                            self.left_col = 0;
                            self.status_msg = Some(if n == 0 {
                                String::from("Wrapping at window width")
                            } else {
                                format!("Wrapping at column {n}")
                            });
                        }
                        Err(_) => self.status_msg = Some(String::from("Not a number")),
                    },
                }
            }
            _ => {}
        }
    }

    fn write_block(&mut self, path: &str) {
        let Some((b, e)) = self.blocks.range() else {
            return;
        };
        let text: String = self.buf.rope.slice(b..e).to_string();
        match std::fs::write(path, &text) {
            Ok(()) => self.status_msg = Some(format!("Block written to {path}")),
            Err(err) => self.status_msg = Some(format!("Write failed: {err}")),
        }
    }

    /// ^KE: write a copy of the document with `..` note lines stripped —
    /// the notes are for the writer, never the reader.
    fn export_clean(&mut self, path: &str) {
        let mut out = String::with_capacity(self.buf.rope.len_bytes());
        for line in 0..self.buf.len_lines() {
            let start = self.buf.line_start(line);
            let end = if line + 1 < self.buf.len_lines() {
                self.buf.line_start(line + 1)
            } else {
                self.buf.len_chars()
            };
            if start == end {
                continue;
            }
            let text: String = self.buf.rope.slice(start..end).to_string();
            if text.trim_start().starts_with("..") {
                continue;
            }
            out.push_str(&text);
        }
        match std::fs::write(path, &out) {
            Ok(()) => self.status_msg = Some(format!("Exported to {path}")),
            Err(err) => self.status_msg = Some(format!("Export failed: {err}")),
        }
    }

    /// ^KM: write a standard-manuscript-format RTF copy — double-spaced,
    /// first-line indented, chapter breaks on `#` headings, Markdown emphasis
    /// rendered as real bold/italic. `..` note lines are skipped, like ^KE.
    fn export_manuscript(&mut self, path: &str) {
        let rtf = rtf::render(&self.buf, self.manuscript_font);
        match std::fs::write(path, &rtf) {
            Ok(()) => self.status_msg = Some(format!("Manuscript exported to {path}")),
            Err(err) => self.status_msg = Some(format!("Export failed: {err}")),
        }
    }

    fn read_file(&mut self, path: &str) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let n = text.chars().count();
                let at = self.cursor;
                self.apply_edit(at, 0, &text, EditKind::Other, at + n);
                self.status_msg = Some(format!("Read {path}"));
            }
            Err(err) => self.status_msg = Some(format!("Read failed: {err}")),
        }
    }

    // --- search -------------------------------------------------------------

    fn handle_search_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Mode::Search(state) = &mut self.mode else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                let origin = state.origin;
                self.mode = Mode::Normal;
                self.set_cursor(origin);
            }
            KeyCode::Enter => {
                if !state.query.is_empty() {
                    self.last_search = Some(state.query.clone());
                }
                let origin = state.origin;
                let landed = state.current;
                self.mode = Mode::Normal;
                if landed.is_some() && origin != self.cursor {
                    self.jump_stack.push(origin);
                    if self.jump_stack.len() > JUMP_STACK_MAX {
                        self.jump_stack.remove(0);
                    }
                }
            }
            KeyCode::Backspace => {
                state.query.pop();
                self.search_update(false);
            }
            KeyCode::Char(c) if ctrl && (c == 'l' || c == 'f') => self.search_update(true),
            KeyCode::Char(c) if !ctrl => {
                state.query.push(c);
                self.search_update(false);
            }
            _ => {}
        }
    }

    /// Re-run the incremental search. With `next`, look past the current match.
    fn search_update(&mut self, next: bool) {
        let Mode::Search(state) = &mut self.mode else {
            return;
        };
        if state.query.is_empty() {
            let origin = state.origin;
            state.current = None;
            self.cursor = origin;
            self.ensure_visible();
            return;
        }
        let from = match (next, state.current) {
            (true, Some(cur)) => cur + 1,
            _ => state.current.unwrap_or(state.origin).min(state.origin),
        };
        let query = state.query.clone();
        let found = self
            .buf
            .find(&query, from.min(self.buf.len_chars()), false)
            .or_else(|| {
                if next {
                    self.buf.find(&query, 0, false)
                } else {
                    None
                }
            });
        match found {
            Some(at) => {
                let wrapped = next && at < from;
                if let Mode::Search(state) = &mut self.mode {
                    state.wrapped = wrapped;
                    state.current = Some(at);
                }
                self.cursor = at;
                self.goal_col = None;
                self.ensure_visible();
                if wrapped {
                    self.status_msg = Some(String::from("Wrapped to top"));
                }
            }
            None => {
                if let Mode::Search(state) = &mut self.mode {
                    state.current = None;
                }
                self.status_msg = Some(format!("Not found: {query}"));
            }
        }
    }

    /// ^L: jump to the next occurrence of the last search.
    fn find_next(&mut self) {
        let Some(query) = self.last_search.clone() else {
            self.status_msg = Some(String::from("No previous search"));
            return;
        };
        let from = self.cursor + 1;
        match self
            .buf
            .find(&query, from.min(self.buf.len_chars()), false)
            .or_else(|| self.buf.find(&query, 0, false))
        {
            Some(at) => {
                if at < from {
                    self.status_msg = Some(String::from("Wrapped to top"));
                }
                self.long_jump(at);
            }
            None => self.status_msg = Some(format!("Not found: {query}")),
        }
    }

    // --- replace ------------------------------------------------------------

    fn handle_replace_key(&mut self, key: KeyEvent) {
        let Mode::Replace(state) = &mut self.mode else {
            return;
        };
        if key.code == KeyCode::Esc {
            let count = state.count;
            let started = matches!(state.phase, ReplacePhase::Confirm(_));
            self.mode = Mode::Normal;
            if started {
                self.status_msg = Some(format!("Replaced {count} occurrence(s)"));
            }
            return;
        }
        match state.phase {
            ReplacePhase::EnterFind | ReplacePhase::EnterWith | ReplacePhase::EnterOptions => {
                let field = match state.phase {
                    ReplacePhase::EnterFind => &mut state.find,
                    ReplacePhase::EnterWith => &mut state.with,
                    _ => &mut state.options,
                };
                match key.code {
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        field.push(c)
                    }
                    KeyCode::Backspace => {
                        field.pop();
                    }
                    KeyCode::Enter => match state.phase {
                        ReplacePhase::EnterFind => {
                            if state.find.is_empty() {
                                self.mode = Mode::Normal;
                            } else {
                                state.phase = ReplacePhase::EnterWith;
                            }
                        }
                        ReplacePhase::EnterWith => state.phase = ReplacePhase::EnterOptions,
                        _ => {
                            self.last_search = Some(state.find.clone());
                            let start = if state.from_top() { 0 } else { self.cursor };
                            self.push_jump();
                            self.replace_advance(start);
                        }
                    },
                    _ => {}
                }
            }
            ReplacePhase::Confirm(at) => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.replace_at(at);
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.replace_advance(at + 1);
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    let Mode::Replace(state) = &mut self.mode else { return };
                    state.options.push('n');
                    self.replace_at(at);
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    let count = state.count;
                    self.mode = Mode::Normal;
                    self.status_msg = Some(format!("Replaced {count} occurrence(s)"));
                }
                _ => {}
            },
        }
    }

    /// Find the next match at or after `from`; ask about it, or in no-ask
    /// mode keep replacing until the matches run out.
    fn replace_advance(&mut self, mut from: usize) {
        loop {
            let Mode::Replace(state) = &mut self.mode else {
                return;
            };
            let find = state.find.clone();
            let whole = state.whole_word();
            let no_ask = state.no_ask();
            match self.buf.find(&find, from.min(self.buf.len_chars()), whole) {
                Some(at) if no_ask => {
                    from = self.do_replace(at);
                }
                Some(at) => {
                    let Mode::Replace(state) = &mut self.mode else { return };
                    state.phase = ReplacePhase::Confirm(at);
                    self.cursor = at;
                    self.goal_col = None;
                    self.ensure_visible();
                    return;
                }
                None => {
                    let count = match &self.mode {
                        Mode::Replace(state) => state.count,
                        _ => 0,
                    };
                    self.mode = Mode::Normal;
                    self.status_msg = Some(format!("Replaced {count} occurrence(s)"));
                    self.ensure_visible();
                    return;
                }
            }
        }
    }

    /// Replace the match at `at`; returns the position to continue from.
    fn do_replace(&mut self, at: usize) -> usize {
        let Mode::Replace(state) = &mut self.mode else {
            return at + 1;
        };
        let find_len = state.find.chars().count();
        let with = state.with.clone();
        state.count += 1;
        let after = at + with.chars().count();
        self.apply_edit(at, find_len, &with, EditKind::Other, after);
        after.max(at + 1)
    }

    /// Replace the match at `at` interactively and step to the next one.
    fn replace_at(&mut self, at: usize) {
        let from = self.do_replace(at);
        self.replace_advance(from);
    }

    // --- file ops -------------------------------------------------------------

    fn save(&mut self) {
        match self.buf.save() {
            Ok(()) => {
                self.status_msg = Some(format!("Saved {}", self.buf.file_name()));
                self.save_session();
            }
            Err(e) => self.status_msg = Some(format!("Save failed: {e}")),
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

fn is_prefix_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(c)
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(c.to_ascii_lowercase(), 'k' | 'q' | 'o'))
}

fn is_edit_cmd(cmd: Cmd) -> bool {
    matches!(
        cmd,
        Cmd::DeleteRight
            | Cmd::DeleteLeft
            | Cmd::DeleteWordRight
            | Cmd::DeleteLine
            | Cmd::DeleteToLineEnd
            | Cmd::InsertBlankLine
            | Cmd::TransposeWords
            | Cmd::TransposeChars
            | Cmd::BlockCopy
            | Cmd::BlockMove
            | Cmd::BlockDelete
            | Cmd::Put
    )
}

fn prefix_caret(prefix: Prefix) -> &'static str {
    match prefix {
        Prefix::K => "^K",
        Prefix::Q => "^Q",
        Prefix::O => "^O",
    }
}
