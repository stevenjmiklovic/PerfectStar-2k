use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;

use crate::block::adjust_pos;
use crate::buffer::wrap_segments;
use crate::config::Config;
use crate::export::docx::DocxExporter;
use crate::export::epub::EpubExporter;
use crate::export::html::{HtmlExporter, PlainTextExporter};
use crate::export::{CompiledDoc, Exporter};
use crate::history::{Edit, EditGroup, EditKind};
use crate::keymap::{self, Cmd, Prefix};
use crate::killring::{KillRing, PutCycle};
use crate::normalize;
use crate::outline;
use crate::pane::Pane;
use crate::project::Project;
use crate::recovery::{self, Journal};
use crate::rtf;
use crate::search::{ReplacePhase, ReplaceState, SearchState};
use crate::spellcheck;
use crate::stats::{DailyHistory, DocStats, GoalKind, SessionGoal};
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
    ExportDocx,
    ExportEpub,
    ExportHtml,
    ExportProjectDocx,
    ExportProjectEpub,
    ExportProjectHtml,
    WrapMargin,
    /// ^OK with one window: filename to open in the second window.
    OpenSplit,
    /// A replacement destination offered after a normal save fails.
    SaveAs,
    /// ^PN: path for new project manifest (.pstarproj file) to create.
    ProjectNew,
    /// ^PP: project manifest path (.pstarproj file) to open.
    ProjectOpen,
    /// ^PA: file path to add to the project.
    ProjectAddDoc,
    /// Set a session word goal (e.g. "500" for 500 words).
    SetGoal,
}

/// What the keyboard is currently driving.
pub enum Mode {
    Normal,
    /// Quit with unsaved changes? (y/N)
    ConfirmAbandon,
    /// A newer crash journal is available for the active document.
    ConfirmRecover,
    Search(SearchState),
    Replace(ReplaceState),
    Input {
        label: String,
        value: String,
        action: InputAction,
    },
    /// The command palette / searchable help (Esc or F1).
    Palette {
        query: String,
        selected: usize,
    },
    /// The outline: jump between Markdown headings (^QO).
    Outline {
        entries: Vec<outline::Entry>,
        query: String,
        selected: usize,
    },
    /// The binder: project document list (^PB).
    Binder {
        entries: Vec<BinderEntry>,
        selected: usize,
    },
    /// Daily writing history overlay (R2.5, task 4.4).
    Stats,
}

/// An entry in the binder panel, with display information for a project document.
#[derive(Debug, Clone)]
pub struct BinderEntry {
    /// Index into the project's manifest.docs.
    pub idx: usize,
    /// Display title.
    pub title: String,
    /// Word count (None if not yet computed or file missing).
    pub word_count: Option<usize>,
    /// Whether the file exists on disk.
    pub exists: bool,
}

struct PendingRecovery {
    pane: usize,
    text: String,
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
    /// Idle autosave; zero disables manuscript autosave (recovery remains on).
    autosave: Duration,
    /// Maximum timestamped rolling backups retained per saved document.
    backup_depth: usize,
    /// Metadata root for crash journals and the distinct `backups/` tree.
    backup_root: Option<PathBuf>,
    /// One plain-text crash journal per pane, kept index-aligned with `panes`.
    recovery_journals: Vec<Journal>,
    /// Recovery text is loaded before prompting so a disappearing record can
    /// never strand the only copy between detection and confirmation.
    pending_recovery: Option<PendingRecovery>,
    /// Keyboard macro (^QM record, ^QJ play).
    macro_keys: Vec<KeyEvent>,
    pub recording: bool,
    playing: bool,
    quit: bool,
    /// The loaded project (book-scale manuscript). None when editing a bare
    /// file; Some when a .pstarproj manifest has been opened (R1, task 1.2).
    pub project: Option<Project>,
    /// Cached prose statistics for the active document (R2, task 4.1).
    pub doc_stats: DocStats,
    /// Whether the always-on word count is shown in the status line.
    pub show_word_count: bool,
    /// Session writing goal (R2.3).
    pub goal: Option<SessionGoal>,
    /// Goal-reached notification was already shown this goal.
    goal_notified: bool,
    /// Per-day words-written history for the active document.
    pub daily_history: DailyHistory,
    /// Word count at start of this editing session (for daily delta).
    session_start_words: usize,
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
        let recovery_journal = Journal::new(pane.buf.path.as_deref());
        let config = Config::load();
        let initial_stats = DocStats::from_rope(&pane.buf.rope);
        let daily_history = DailyHistory::load(pane.buf.path.as_deref());
        let start_words = initial_stats.words;
        let mut app = App {
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
            backup_depth: config.backup_depth,
            backup_root: crate::paths::recovery(),
            recovery_journals: vec![recovery_journal],
            pending_recovery: None,
            macro_keys: Vec::new(),
            recording: false,
            playing: false,
            quit: false,
            project: None, // R1.8: backward compat — no project at bare-file launch
            doc_stats: initial_stats,
            show_word_count: true,
            goal: None,
            goal_notified: false,
            daily_history,
            session_start_words: start_words,
        };
        app.offer_recovery_for_active();
        Ok(app)
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
        self.clear_clean_recovery();
        // Persist daily words-written delta before exiting (R2.5).
        let delta = self.doc_stats.words as i64 - self.session_start_words as i64;
        self.daily_history.record_delta(delta);
        let _ = self.daily_history.save();
        for pane in &self.panes {
            pane.save_session();
        }
        Ok(())
    }

    /// The existing idle callback is also the recovery throttle: each edit is
    /// journaled at most once, before a manuscript autosave is attempted.
    fn maybe_autosave(&mut self) {
        let deadline = self.autosave;
        let backup_depth = self.backup_depth;
        let backup_root = self.backup_root.clone();
        let mut autosaved = false;
        let mut warnings = Vec::new();
        let (panes, journals) = (&mut self.panes, &mut self.recovery_journals);

        for (pane, journal) in panes.iter_mut().zip(journals.iter_mut()) {
            if !pane.buf.dirty {
                continue;
            }

            if let Err(error) = journal.write_if_changed(&pane.buf.rope, pane.last_edit) {
                warnings.push(format!("Recovery write failed: {error}"));
            }

            if !deadline.is_zero()
                && pane.buf.path.is_some()
                && pane.last_edit.elapsed() >= deadline
            {
                match pane.buf.save() {
                    Ok(()) => {
                        autosaved = true;
                        if let Err(error) = write_backup_after_save(
                            backup_root.as_deref(),
                            pane.buf.path.as_deref(),
                            backup_depth,
                        ) {
                            warnings.push(format!("Rolling backup failed: {error}"));
                        }
                        if let Err(error) = journal.clear() {
                            warnings.push(format!("Recovery cleanup failed: {error}"));
                        }
                    }
                    Err(error) => {
                        // The buffer remains dirty and its successfully-written
                        // journal remains available for crash recovery.
                        warnings.push(format!("Autosave failed: {error}"));
                    }
                }
            }
        }

        if !warnings.is_empty() {
            self.status_msg = Some(warnings.join("; "));
        } else if autosaved {
            self.status_msg = Some(String::from("Autosaved"));
        }

        // Debounced full recount to correct any drift (R2.7).
        if self.doc_stats.needs_recount() {
            self.doc_stats
                .full_recount(&self.panes[self.active].buf.rope);
        }

        // Check session goal progress (R2.4).
        if let Some(ref mut goal) = self.goal {
            if !goal.reached && goal.is_met(self.doc_stats.words) {
                goal.reached = true;
                if !self.goal_notified {
                    self.goal_notified = true;
                    let (current, target) = goal.progress(self.doc_stats.words);
                    self.status_msg = Some(format!(
                        "Goal reached! {current}/{target} {}",
                        match goal.kind {
                            GoalKind::Words => "words",
                            GoalKind::Minutes => "minutes",
                        }
                    ));
                }
            }
        }
    }

    fn clear_clean_recovery(&mut self) {
        for (pane, journal) in self.panes.iter().zip(self.recovery_journals.iter_mut()) {
            if !pane.buf.dirty {
                let _ = journal.clear();
            }
        }
    }

    fn offer_recovery_for_active(&mut self) {
        let pane = self.active;
        match self.recovery_journals[pane].recoverable_text() {
            Ok(Some(text)) => {
                self.pending_recovery = Some(PendingRecovery { pane, text });
                self.mode = Mode::ConfirmRecover;
                // Recovery is more urgent than the decorative startup splash:
                // the first key must answer the prompt rather than be discarded.
                self.splash = false;
            }
            Ok(None) => {}
            Err(error) => {
                // Preserve unreadable data for manual recovery instead of
                // deleting or overwriting the only possible copy.
                self.status_msg = Some(format!("Recovery record unreadable: {error}"));
            }
        }
    }

    fn answer_recovery_prompt(&mut self, restore: bool) {
        let Some(pending) = self.pending_recovery.take() else {
            self.mode = Mode::Normal;
            self.status_msg = Some(String::from("Recovery record is no longer available"));
            return;
        };
        self.active = pending.pane;
        self.mode = Mode::Normal;

        if restore {
            let old_len = self.buf.len_chars();
            let cursor_after = self.cursor.min(pending.text.chars().count());
            self.apply_edit(0, old_len, &pending.text, EditKind::Other, cursor_after);
            self.history.break_group();
            self.status_msg = Some(String::from("Recovered unsaved changes; save to keep them"));
        } else {
            self.status_msg = Some(match self.recovery_journals[pending.pane].clear() {
                Ok(()) => String::from("Recovery declined"),
                Err(error) => format!("Recovery declined; cleanup failed: {error}"),
            });
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
            Mode::ConfirmRecover => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => self.answer_recovery_prompt(true),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter | KeyCode::Esc => {
                    self.answer_recovery_prompt(false)
                }
                _ => {}
            },
            Mode::Search(_) => self.handle_search_key(key),
            Mode::Replace(_) => self.handle_replace_key(key),
            Mode::Input { .. } => self.handle_input_key(key),
            Mode::Palette { .. } => self.handle_palette_key(key),
            Mode::Outline { .. } => self.handle_outline_key(key),
            Mode::Binder { .. } => self.handle_binder_key(key),
            Mode::Stats => self.mode = Mode::Normal,
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
                    'p' => self.prefix = Some((Prefix::P, Instant::now())),
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
        let Mode::Outline {
            entries,
            query,
            selected,
        } = &mut self.mode
        else {
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

    /// The binder: project document list with navigation (^PB, task 1.3).
    fn handle_binder_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Mode::Binder { entries, selected } = &mut self.mode else {
            return;
        };
        let num_entries = entries.len();
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::Down => *selected = (*selected + 1).min(num_entries.saturating_sub(1)),
            KeyCode::Char('e') if ctrl => *selected = selected.saturating_sub(1),
            KeyCode::Char('x') if ctrl => {
                *selected = (*selected + 1).min(num_entries.saturating_sub(1))
            }
            KeyCode::Enter => {
                // Open the selected document in the active pane.
                // R1.3: session restore is automatic via Pane::open.
                if let Some(entry) = entries.get(*selected) {
                    if !entry.exists {
                        self.status_msg = Some(format!("File missing: {}", entry.title));
                        return;
                    }
                    // Get the document path from the project.
                    if let Some(ref project) = self.project {
                        if let Some(doc) = project.manifest.docs.get(entry.idx) {
                            let path = doc.path.clone();
                            self.mode = Mode::Normal;
                            // Open the document in the active pane.
                            // This replaces the current pane's buffer and restores its session.
                            match crate::pane::Pane::open(Some(path)) {
                                Ok(pane) => {
                                    let journal = Journal::new(pane.buf.path.as_deref());
                                    self.panes[self.active] = pane;
                                    self.recovery_journals[self.active] = journal;
                                    self.status_msg = Some(format!("Opened: {}", doc.title));
                                }
                                Err(e) => {
                                    self.status_msg = Some(format!("Failed to open: {e}"));
                                }
                            }
                        }
                    }
                }
            }
            // Handle ^P prefix commands while in binder mode.
            KeyCode::Char('p') if ctrl => {
                self.prefix = Some((Prefix::P, Instant::now()));
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
                    Prefix::O | Prefix::P => {
                        self.status_msg = Some(String::from("Unknown command"))
                    }
                }
            }
            Some(c) => match keymap::lookup_prefixed(prefix, c) {
                Some(cmd) => self.execute(cmd),
                None => {
                    self.status_msg = Some(format!("Unknown command {}{c}", prefix_caret(prefix)))
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
                        label: String::from("Write block to file"),
                        value: String::new(),
                        action: InputAction::WriteBlock,
                    };
                } else {
                    self.status_msg = Some(String::from("No block marked"));
                }
            }
            Cmd::BlockRead => {
                self.mode = Mode::Input {
                    label: String::from("Read file"),
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
                        label: String::from("Open in second window"),
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
                    label: String::from("Export to file (notes stripped)"),
                    value: String::new(),
                    action: InputAction::ExportClean,
                };
            }
            Cmd::ExportManuscript => {
                self.mode = Mode::Input {
                    label: String::from("Export manuscript RTF to file"),
                    value: String::new(),
                    action: InputAction::ExportManuscript,
                };
            }
            Cmd::ExportDocx => {
                self.mode = Mode::Input {
                    label: String::from("Export DOCX to file"),
                    value: String::new(),
                    action: InputAction::ExportDocx,
                };
            }
            Cmd::ExportEpub => {
                self.mode = Mode::Input {
                    label: String::from("Export EPUB to file"),
                    value: String::new(),
                    action: InputAction::ExportEpub,
                };
            }
            Cmd::ExportHtml => {
                self.mode = Mode::Input {
                    label: String::from("Export HTML to file"),
                    value: String::new(),
                    action: InputAction::ExportHtml,
                };
            }
            Cmd::ExportProjectDocx => {
                if self.project.is_some() {
                    self.mode = Mode::Input {
                        label: String::from("Export project DOCX to file"),
                        value: String::new(),
                        action: InputAction::ExportProjectDocx,
                    };
                } else {
                    self.status_msg = Some(String::from("No project loaded (^PP to open)"));
                }
            }
            Cmd::ExportProjectEpub => {
                if self.project.is_some() {
                    self.mode = Mode::Input {
                        label: String::from("Export project EPUB to file"),
                        value: String::new(),
                        action: InputAction::ExportProjectEpub,
                    };
                } else {
                    self.status_msg = Some(String::from("No project loaded (^PP to open)"));
                }
            }
            Cmd::ExportProjectHtml => {
                if self.project.is_some() {
                    self.mode = Mode::Input {
                        label: String::from("Export project HTML to file"),
                        value: String::new(),
                        action: InputAction::ExportProjectHtml,
                    };
                } else {
                    self.status_msg = Some(String::from("No project loaded (^PP to open)"));
                }
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
                    label: String::from("Wrap margin in columns (0 = window width)"),
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
            Cmd::ProjectNew => {
                self.mode = Mode::Input {
                    label: String::from("New project path (.pstarproj)"),
                    value: String::new(),
                    action: InputAction::ProjectNew,
                };
            }
            Cmd::ProjectOpen => {
                self.mode = Mode::Input {
                    label: String::from("Open project manifest (.pstarproj)"),
                    value: String::new(),
                    action: InputAction::ProjectOpen,
                };
            }
            Cmd::BinderToggle => {
                // Toggle binder panel (R1.2, task 1.3).
                // If already in binder mode, exit to normal. Otherwise, open the binder
                // if a project is loaded.
                if matches!(self.mode, Mode::Binder { .. }) {
                    self.mode = Mode::Normal;
                } else if let Some(ref project) = self.project {
                    // Build the binder entries.
                    let entries: Vec<BinderEntry> = project
                        .manifest
                        .docs
                        .iter()
                        .enumerate()
                        .map(|(idx, doc)| BinderEntry {
                            idx,
                            title: doc.title.clone(),
                            word_count: project.doc_word_count(idx),
                            exists: project.doc_exists(idx),
                        })
                        .collect();
                    self.mode = Mode::Binder {
                        entries,
                        selected: 0,
                    };
                } else {
                    self.status_msg = Some(String::from("No project loaded (^PP to open)"));
                }
            }
            Cmd::BinderMoveUp => self.binder_move_up(),
            Cmd::BinderMoveDown => self.binder_move_down(),
            Cmd::ProjectAddDoc => {
                if self.project.is_some() {
                    self.mode = Mode::Input {
                        label: String::from("Add document to project (file path)"),
                        value: String::new(),
                        action: InputAction::ProjectAddDoc,
                    };
                } else {
                    self.status_msg = Some(String::from("No project loaded (^PP to open)"));
                }
            }
            Cmd::ProjectRemoveDoc => self.project_remove_doc(),
            Cmd::WordCount => {
                self.show_word_count = !self.show_word_count;
                self.status_msg = Some(String::from(if self.show_word_count {
                    "Word count on"
                } else {
                    "Word count off"
                }));
            }
            Cmd::SetGoal => {
                self.mode = Mode::Input {
                    label: String::from("Session goal (words, e.g. 500)"),
                    value: String::new(),
                    action: InputAction::SetGoal,
                };
            }
            Cmd::StatsOverlay => {
                if matches!(self.mode, Mode::Stats) {
                    self.mode = Mode::Normal;
                } else {
                    self.mode = Mode::Stats;
                }
            }
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
        let cursor = self.cursor;
        if self.jump_stack.last() != Some(&cursor) {
            self.jump_stack.push(cursor);
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
            let cursor = self.cursor;
            if p != cursor && p <= self.buf.len_chars() {
                // Rotate: the position we're leaving goes to the bottom, so
                // repeated ^QP cycles rather than exhausts.
                self.jump_stack.insert(0, cursor);
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
            if normalize::is_note_line(&text) {
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

        // Incremental stats update over the affected lines.
        let from_line = self.buf.line_of(at);
        let end_pos = at + insert.chars().count();
        let to_line = self
            .buf
            .line_of(end_pos.min(self.buf.len_chars().saturating_sub(1)))
            + 1;
        // Split borrow: access the rope through the pane vec index to avoid
        // conflicting borrows of self (Deref goes through panes[active]).
        let active = self.active;
        self.doc_stats
            .invalidate_lines(&self.panes[active].buf.rope, from_line, to_line);

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
            self.apply_edit(
                self.cursor,
                end - self.cursor,
                "",
                EditKind::Other,
                self.cursor,
            );
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
                    InputAction::ExportDocx => self.export_with(&DocxExporter, &path, "DOCX"),
                    InputAction::ExportEpub => self.export_with(&EpubExporter, &path, "EPUB"),
                    InputAction::ExportHtml => self.export_with(&HtmlExporter, &path, "HTML"),
                    InputAction::ExportProjectDocx => {
                        self.export_project(&DocxExporter, &path, "DOCX")
                    }
                    InputAction::ExportProjectEpub => {
                        self.export_project(&EpubExporter, &path, "EPUB")
                    }
                    InputAction::ExportProjectHtml => {
                        self.export_project(&HtmlExporter, &path, "HTML")
                    }
                    InputAction::SaveAs => self.save_as(&path),
                    InputAction::OpenSplit => self.open_split(&path),
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
                    InputAction::ProjectOpen => self.open_project(&path),
                    InputAction::ProjectNew => self.new_project(&path),
                    InputAction::ProjectAddDoc => self.project_add_doc(&path),
                    InputAction::SetGoal => match path.parse::<usize>() {
                        Ok(n) if n > 0 => {
                            self.goal =
                                Some(SessionGoal::new(GoalKind::Words, n, self.doc_stats.words));
                            self.goal_notified = false;
                            self.status_msg = Some(format!("Goal set: {n} words this session"));
                        }
                        _ => {
                            self.status_msg =
                                Some(String::from("Enter a positive number of words"));
                        }
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

    /// ^KE: write a copy with note lines stripped. Markdown and punctuation
    /// remain unchanged for compatibility with the original command.
    fn export_clean(&mut self, path: &str) {
        let doc = CompiledDoc::from_buffer(&self.buf);
        self.finish_export(&PlainTextExporter, &doc, path, "Plain text", 0);
    }

    /// ^KM remains a current-document standard-manuscript export.
    fn export_manuscript(&mut self, path: &str) {
        let doc = CompiledDoc::from_buffer(&self.buf);
        let exporter = rtf::RtfExporter {
            font: self.manuscript_font,
        };
        self.finish_export(&exporter, &doc, path, "Manuscript", 0);
    }

    /// New professional formats export the loaded project when present,
    /// otherwise the active document. Project compilation supplies binder
    /// order and the manifest's selected separators.
    fn export_with<E: Exporter>(&mut self, exporter: &E, path: &str, format: &str) {
        let doc = CompiledDoc::from_buffer(&self.buf);
        self.finish_export(exporter, &doc, path, format, 0);
    }

    /// Project-scoped export: compiles all included docs in binder order.
    /// Refuses if any open buffer matching an included doc is dirty (unsaved
    /// edits would be silently lost), or if any included doc is unreadable
    /// (the output would be an incomplete book).
    fn export_project<E: Exporter>(&mut self, exporter: &E, path: &str, format: &str) {
        let Some(ref project) = self.project else {
            self.status_msg = Some(String::from("No project loaded (^PP to open)"));
            return;
        };

        // Check for dirty open buffers that are part of the compile set.
        for pane in &self.panes {
            if !pane.buf.dirty {
                continue;
            }
            if let Some(ref pane_path) = pane.buf.path {
                for entry in &project.manifest.docs {
                    if entry.include_in_compile && entry.path == *pane_path {
                        self.status_msg = Some(format!(
                            "{format} export aborted: \"{}\" has unsaved changes — save first (^KS)",
                            entry.title
                        ));
                        return;
                    }
                }
            }
        }

        let compiled = project.compile();

        // Block export if any included doc was unreadable (R7.8 — never
        // silently replace a good export with an incomplete book).
        if !compiled.skipped.is_empty() {
            let names: Vec<&str> = compiled.skipped.iter().map(|s| s.title.as_str()).collect();
            self.status_msg = Some(format!(
                "{format} export aborted: cannot read included document(s): {}",
                names.join(", ")
            ));
            return;
        }

        let doc = CompiledDoc::from_compiled(&compiled.text);
        self.finish_export(exporter, &doc, path, format, 0);
    }

    fn finish_export<E: Exporter>(
        &mut self,
        exporter: &E,
        doc: &CompiledDoc,
        path: &str,
        format: &str,
        _skipped: usize,
    ) {
        match exporter.export(doc, Path::new(path)) {
            Ok(()) => {
                self.status_msg = Some(format!("{format} exported to {path}"));
            }
            Err(err) => self.status_msg = Some(format!("{format} export failed: {err}")),
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
                    let Mode::Replace(state) = &mut self.mode else {
                        return;
                    };
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
                    let Mode::Replace(state) = &mut self.mode else {
                        return;
                    };
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
            Ok(()) => self.finish_save(),
            Err(error) => self.prompt_alternate_save(error),
        }
    }

    fn save_as(&mut self, path: &str) {
        match self.buf.save_as(PathBuf::from(path)) {
            Ok(()) => self.finish_save(),
            Err(error) => self.prompt_alternate_save(error),
        }
    }

    fn finish_save(&mut self) {
        let file_name = self.buf.file_name();
        let source = self.buf.path.clone();
        let mut warnings = Vec::new();
        if let Err(error) = write_backup_after_save(
            self.backup_root.as_deref(),
            source.as_deref(),
            self.backup_depth,
        ) {
            warnings.push(format!("rolling backup failed: {error}"));
        }
        if let Err(error) = self.recovery_journals[self.active].clear() {
            warnings.push(format!("recovery cleanup failed: {error}"));
        }
        // Save As may have changed the buffer path, so future dirty edits need
        // a journal keyed to the newly adopted destination.
        self.recovery_journals[self.active] = Journal::new(source.as_deref());
        self.save_session();
        self.status_msg = Some(if warnings.is_empty() {
            format!("Saved {file_name}")
        } else {
            format!("Saved {file_name}; {}", warnings.join("; "))
        });
    }

    fn prompt_alternate_save(&mut self, error: io::Error) {
        let message = format!("Save failed: {error}");
        self.status_msg = Some(message.clone());
        self.mode = Mode::Input {
            label: format!("{message}. Alternate save path"),
            value: String::new(),
            action: InputAction::SaveAs,
        };
    }

    // --- windows ---------------------------------------------------------------

    /// ^KQ/^KX: with two windows, close just the active one; with one, quit.
    fn close_or_quit(&mut self) {
        if self.panes.len() > 1 {
            self.panes[self.active].save_session();
            let pane = self.panes.remove(self.active);
            let mut journal = self.recovery_journals.remove(self.active);
            if !pane.buf.dirty {
                let _ = journal.clear();
            }
            self.active = 0;
            self.status_msg = Some(String::from("Window closed"));
        } else {
            if !self.buf.dirty {
                let _ = self.recovery_journals[self.active].clear();
            }
            self.quit = true;
        }
    }

    /// ^OK with one window: open `path` in a second window below and focus it.
    fn open_split(&mut self, path: &str) {
        if self.panes.len() >= 2 {
            return;
        }
        match Pane::open(Some(PathBuf::from(path))) {
            Ok(pane) => {
                let journal = Journal::new(pane.buf.path.as_deref());
                self.panes.push(pane);
                self.recovery_journals.push(journal);
                self.active = self.panes.len() - 1;
            }
            Err(e) => self.status_msg = Some(format!("Open failed: {e}")),
        }
    }

    /// ^KA (WordStar): copy the block marked in the *other* window to the
    /// cursor here. Each window keeps its own marked block — this is the
    /// bridge between them. The copy becomes this window's marked block,
    /// mirroring ^KC's behavior within one document.
    fn copy_from_other(&mut self) {
        if self.panes.len() < 2 {
            self.status_msg = Some(String::from("No second window (^OK opens one)"));
            return;
        }
        let other = 1 - self.active;
        let Some((b, e)) = self.panes[other].blocks.range() else {
            self.status_msg = Some(String::from("No block marked in the other window"));
            return;
        };
        let text: String = self.panes[other].buf.rope.slice(b..e).to_string();
        let len = e - b;
        let at = self.cursor;
        self.blocks.remember();
        self.apply_edit(at, 0, &text, EditKind::Other, at);
        self.blocks.begin = Some(at);
        self.blocks.end = Some(at + len);
        self.blocks.hidden = false;
        self.status_msg = Some(String::from("Block copied from other window"));
    }

    // --- project management ---------------------------------------------------

    /// Open a project from a .pstarproj manifest file (R1, task 1.2).
    /// Stores the loaded project in `self.project`, or shows an error if the
    /// load fails (file not found, TOML parse error). Keeps `self.project` as
    /// `None` on failure, so the editor stays usable (C3: never lose work).
    fn open_project(&mut self, path: &str) {
        use std::path::Path;

        let manifest_path = Path::new(path);

        // Load the project using Project::load from project.rs
        match Project::load(manifest_path) {
            Ok(proj) => {
                let name = proj.manifest.name.clone();
                let count = proj.manifest.docs.len();
                self.project = Some(proj);
                self.status_msg = Some(format!(
                    "Project \"{}\" opened ({} document{})",
                    name,
                    count,
                    if count == 1 { "" } else { "s" }
                ));
            }
            Err(e) => {
                // R1.8: backward compatibility — keep project as None on error
                self.project = None;
                self.status_msg = Some(format!("Failed to open project: {}", e));
            }
        }
    }

    /// Create a new project manifest at the given path (^PN).
    ///
    /// The user supplies a path ending in `.pstarproj`. The project name is
    /// derived from the file stem (e.g. `MyNovel.pstarproj` → name "MyNovel").
    /// The manifest is written atomically (R11.5) and immediately loaded so the
    /// binder and other project commands become available.
    fn new_project(&mut self, path: &str) {
        use crate::paths;
        use crate::project::{ProjectManifest, Separator};
        use std::path::Path;

        let manifest_path = Path::new(path);

        // Ensure the path has the .pstarproj extension.
        let has_ext = manifest_path
            .extension()
            .map(|e| e == "pstarproj")
            .unwrap_or(false);
        let manifest_path = if has_ext {
            manifest_path.to_path_buf()
        } else {
            manifest_path.with_extension("pstarproj")
        };

        // Don't overwrite an existing manifest — use ^PP to open it instead.
        if manifest_path.exists() {
            self.status_msg = Some(String::from(
                "File already exists (use ^PP to open an existing project)",
            ));
            return;
        }

        // Ensure the parent directory exists.
        if let Some(parent) = manifest_path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    self.status_msg = Some(format!("Cannot create directory: {e}"));
                    return;
                }
            }
        }

        // Derive the project name from the file stem.
        let name = manifest_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        // Build an empty manifest.
        let manifest = ProjectManifest {
            name: name.clone(),
            docs: Vec::new(),
            separator: Separator::default(),
        };

        // Serialize and write atomically.
        let data = match toml::to_string_pretty(&manifest) {
            Ok(s) => s,
            Err(e) => {
                self.status_msg = Some(format!("Failed to serialize manifest: {e}"));
                return;
            }
        };
        if let Err(e) = paths::write_atomic(&manifest_path, data.as_bytes()) {
            self.status_msg = Some(format!("Failed to write manifest: {e}"));
            return;
        }

        // Load the freshly-created project so it's immediately active.
        match Project::load(&manifest_path) {
            Ok(proj) => {
                self.project = Some(proj);
                self.status_msg = Some(format!(
                    "Project \"{name}\" created (use ^PA to add documents)"
                ));
            }
            Err(e) => {
                self.status_msg = Some(format!("Created manifest but failed to load: {e}"));
            }
        }
    }

    /// Move the selected document up in the binder (^PE, R1.4, task 1.4).
    /// Only works when in binder mode. Updates the manifest atomically.
    fn binder_move_up(&mut self) {
        // Check if we're in binder mode.
        let Mode::Binder { selected, .. } = &self.mode else {
            self.status_msg = Some(String::from("Not in binder mode"));
            return;
        };
        let selected_idx = *selected;

        // Can't move the first document up.
        if selected_idx == 0 {
            self.status_msg = Some(String::from("Already at top"));
            return;
        }

        // Perform the reorder on the project.
        if let Some(ref mut project) = self.project {
            let from = selected_idx;
            let to = selected_idx - 1;
            project.reorder_doc(from, to);

            // Save the manifest atomically (R1.4, R11.5).
            match project.save() {
                Ok(()) => {
                    self.status_msg = Some(String::from("Document moved up"));
                    // Refresh the binder to show the new order, with selection following the moved doc.
                    self.refresh_binder_with_selection(to);
                }
                Err(e) => {
                    // On save error, keep the old state in memory and show error.
                    // Reload the project to revert the in-memory change.
                    let manifest_path = project.manifest_path.clone();
                    match Project::load(&manifest_path) {
                        Ok(proj) => self.project = Some(proj),
                        Err(_) => self.project = None,
                    }
                    self.status_msg = Some(format!("Failed to save manifest: {e}"));
                }
            }
        } else {
            self.status_msg = Some(String::from("No project loaded"));
        }
    }

    /// Move the selected document down in the binder (^PX, R1.4, task 1.4).
    /// Only works when in binder mode. Updates the manifest atomically.
    fn binder_move_down(&mut self) {
        // Check if we're in binder mode.
        let Mode::Binder { selected, .. } = &self.mode else {
            self.status_msg = Some(String::from("Not in binder mode"));
            return;
        };
        let selected_idx = *selected;

        // Get the project and check if we can move down.
        let Some(ref mut project) = self.project else {
            self.status_msg = Some(String::from("No project loaded"));
            return;
        };

        let num_docs = project.manifest.docs.len();
        if selected_idx + 1 >= num_docs {
            self.status_msg = Some(String::from("Already at bottom"));
            return;
        }

        // Perform the reorder.
        let from = selected_idx;
        let to = selected_idx + 1;
        project.reorder_doc(from, to);

        // Save the manifest atomically (R1.4, R11.5).
        match project.save() {
            Ok(()) => {
                self.status_msg = Some(String::from("Document moved down"));
                // Refresh the binder to show the new order, with selection following the moved doc.
                self.refresh_binder_with_selection(to);
            }
            Err(e) => {
                // On save error, revert the in-memory change by reloading.
                let manifest_path = project.manifest_path.clone();
                match Project::load(&manifest_path) {
                    Ok(proj) => self.project = Some(proj),
                    Err(_) => self.project = None,
                }
                self.status_msg = Some(format!("Failed to save manifest: {e}"));
            }
        }
    }

    /// Refresh the binder panel with updated entries, maintaining a specific selection.
    fn refresh_binder_with_selection(&mut self, new_selected: usize) {
        if let Some(ref project) = self.project {
            let entries: Vec<BinderEntry> = project
                .manifest
                .docs
                .iter()
                .enumerate()
                .map(|(idx, doc)| BinderEntry {
                    idx,
                    title: doc.title.clone(),
                    word_count: project.doc_word_count(idx),
                    exists: project.doc_exists(idx),
                })
                .collect();
            let selected = new_selected.min(entries.len().saturating_sub(1));
            self.mode = Mode::Binder { entries, selected };
        }
    }

    /// Add a document to the project (^PA, R1.5, task 1.4).
    /// Prompts for a file path, generates a title, adds to the manifest, and saves atomically.
    fn project_add_doc(&mut self, path: &str) {
        use std::path::Path;

        let file_path = Path::new(path);

        // Check if the file exists (R1.5).
        if !file_path.exists() {
            self.status_msg = Some(format!("File not found: {path}"));
            return;
        }

        // Generate a title from the file stem or first heading.
        // For simplicity, use the file stem; a future enhancement could scan for headings.
        let title = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled")
            .to_string();

        // Check if we need to refresh binder before mutating project.
        let in_binder = matches!(self.mode, Mode::Binder { .. });

        // Add to the project.
        if let Some(ref mut project) = self.project {
            project.add_doc(file_path.to_path_buf(), title.clone());

            // Save the manifest atomically (R1.4, R11.5).
            let save_result = project.save();
            let new_idx = project.manifest.docs.len() - 1;
            let manifest_path_on_error = project.manifest_path.clone();

            match save_result {
                Ok(()) => {
                    self.status_msg = Some(format!("Added: {title}"));
                }
                Err(e) => {
                    // On save error, revert the in-memory change by reloading.
                    match Project::load(&manifest_path_on_error) {
                        Ok(proj) => self.project = Some(proj),
                        Err(_) => self.project = None,
                    }
                    self.status_msg = Some(format!("Failed to save manifest: {e}"));
                    return;
                }
            }

            // If in binder mode, refresh to show the new document.
            if in_binder {
                self.refresh_binder_with_selection(new_idx);
            }
        } else {
            self.status_msg = Some(String::from("No project loaded"));
        }
    }

    /// Remove a document from the project (^PR, R1.5, task 1.4).
    /// When in binder mode, removes the selected document. The file on disk is NEVER deleted (R1.5).
    fn project_remove_doc(&mut self) {
        // Check if we're in binder mode.
        let Mode::Binder { selected, .. } = &self.mode else {
            self.status_msg = Some(String::from("Use this command in binder mode (^PB)"));
            return;
        };
        let selected_idx = *selected;

        // Remove from the project.
        if let Some(ref mut project) = self.project {
            if let Some(removed) = project.remove_doc(selected_idx) {
                // Save the manifest atomically (R1.4, R11.5).
                match project.save() {
                    Ok(()) => {
                        // R1.5: clear message that the file is kept on disk.
                        self.status_msg = Some(format!(
                            "\"{}\" removed from project (file kept on disk)",
                            removed.title
                        ));
                        // Refresh the binder, adjusting selection if needed.
                        let new_selected = if project.manifest.docs.is_empty() {
                            0
                        } else {
                            selected_idx.min(project.manifest.docs.len().saturating_sub(1))
                        };
                        self.refresh_binder_with_selection(new_selected);
                    }
                    Err(e) => {
                        // On save error, revert by reloading the project.
                        let manifest_path = project.manifest_path.clone();
                        match Project::load(&manifest_path) {
                            Ok(proj) => self.project = Some(proj),
                            Err(_) => self.project = None,
                        }
                        self.status_msg = Some(format!("Failed to save manifest: {e}"));
                    }
                }
            } else {
                self.status_msg = Some(String::from("Invalid document index"));
            }
        } else {
            self.status_msg = Some(String::from("No project loaded"));
        }
    }
}

fn write_backup_after_save(
    root: Option<&Path>,
    source: Option<&Path>,
    depth: usize,
) -> io::Result<()> {
    if depth == 0 {
        return Ok(());
    }
    let root = root.ok_or_else(|| io::Error::other("metadata directory is unavailable"))?;
    let source = source.ok_or_else(|| io::Error::other("saved document has no path"))?;
    recovery::write_rolling_backup(root, source, depth).map(|_| ())
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

fn is_prefix_key(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(c)
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(c.to_ascii_lowercase(), 'k' | 'q' | 'o' | 'p'))
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
            | Cmd::CopyFromOther
    )
}

fn prefix_caret(prefix: Prefix) -> &'static str {
    match prefix {
        Prefix::K => "^K",
        Prefix::Q => "^Q",
        Prefix::O => "^O",
        Prefix::P => "^P",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{DocEntry, ProjectManifest};
    use ropey::Rope;

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pstar-app-{tag}-{}-{id}", std::process::id()))
    }

    #[test]
    fn save_failure_preserves_every_copy_then_alternate_save_succeeds() {
        let dir = scratch_dir("save-failure");
        std::fs::create_dir_all(&dir).unwrap();
        let failed_path = dir.join("chapter.md");
        let alternate_path = dir.join("recovered.md");
        let disk_text = "previous good manuscript";
        std::fs::write(&failed_path, disk_text).unwrap();

        let mut app = App::new(Some(failed_path.clone())).unwrap();
        let end = app.buf.len_chars();
        app.buf.insert(end, " with unsaved changes");
        let expected_text = app.buf.rope.to_string();
        let recovery_root = dir.join("recovery");
        app.backup_depth = 2;
        app.backup_root = Some(recovery_root.clone());
        app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&failed_path), "unused");
        app.maybe_autosave();
        let recovery_path = app.recovery_journals[0].path().unwrap().to_path_buf();
        assert_eq!(
            std::fs::read_to_string(&recovery_path).unwrap(),
            expected_text
        );

        // A directory at the atomic helper's sibling temp path deterministically
        // simulates a write failure while a prior good manuscript exists.
        let mut temporary = failed_path.clone().into_os_string();
        temporary.push(".tmp~");
        std::fs::create_dir(PathBuf::from(temporary)).unwrap();

        // Exercise the actual command arm, not just Buffer::save directly.
        app.execute(Cmd::Save);

        assert_eq!(app.buf.rope.to_string(), expected_text);
        assert_eq!(app.buf.path.as_deref(), Some(failed_path.as_path()));
        assert!(app.buf.dirty);
        assert_eq!(std::fs::read_to_string(&failed_path).unwrap(), disk_text);
        assert_eq!(
            std::fs::read_to_string(failed_path.with_extension("md.bak")).unwrap(),
            disk_text
        );
        assert_eq!(
            std::fs::read_to_string(&recovery_path).unwrap(),
            expected_text
        );
        assert!(
            app.status_msg
                .as_deref()
                .unwrap()
                .starts_with("Save failed:")
        );
        match &app.mode {
            Mode::Input {
                label,
                action: InputAction::SaveAs,
                ..
            } => {
                assert!(label.contains("Save failed:"));
                assert!(label.contains("Alternate save path"));
            }
            _ => panic!("save failure did not open the alternate-path prompt"),
        }

        if let Mode::Input { value, .. } = &mut app.mode {
            *value = alternate_path.to_string_lossy().into_owned();
        }
        app.handle_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.buf.path.as_deref(), Some(alternate_path.as_path()));
        assert!(!app.buf.dirty);
        assert!(
            !recovery_path.exists(),
            "successful Save As left the old recovery journal"
        );
        assert_eq!(std::fs::read_to_string(&failed_path).unwrap(), disk_text);
        assert_eq!(
            std::fs::read_to_string(&alternate_path).unwrap(),
            expected_text
        );
        assert_eq!(
            app.status_msg.as_deref(),
            Some(format!("Saved {}", app.buf.file_name()).as_str())
        );
        let save_as_backups = recovery_root
            .join("backups")
            .join(crate::paths::path_key(&alternate_path));
        let backup = std::fs::read_dir(save_as_backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(std::fs::read_to_string(backup).unwrap(), expected_text);

        // finish_save persists the pane session under the normal metadata root;
        // remove this test's path-keyed artifact as well as its manuscript.
        if let Some(sessions) = crate::paths::sessions() {
            let session =
                sessions.join(format!("{}.json", crate::paths::path_key(&alternate_path)));
            let _ = std::fs::remove_file(session);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn successful_save_paths_rotate_to_exact_backup_depth() {
        let dir = scratch_dir("rolling-save-seams");
        let source = dir.join("chapter.md");
        let recovery_root = dir.join("recovery");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, "saved").unwrap();

        let mut app = App::new(Some(source.clone())).unwrap();
        app.backup_depth = 2;
        app.backup_root = Some(recovery_root.clone());

        app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
        let first_end = app.buf.len_chars();
        app.buf.insert(first_end, " manually");
        app.save();

        app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
        let second_end = app.buf.len_chars();
        app.buf.insert(second_end, " twice");
        app.save();

        app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
        app.autosave = Duration::from_nanos(1);
        let autosave_end = app.buf.len_chars();
        app.buf.insert(autosave_end, " and automatically");
        app.maybe_autosave();

        let backup_dir = recovery_root
            .join("backups")
            .join(crate::paths::path_key(&source));
        let mut backups = std::fs::read_dir(backup_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        backups.sort();
        assert_eq!(backups.len(), app.backup_depth);
        assert_eq!(
            std::fs::read_to_string(&backups[0]).unwrap(),
            "saved manually twice"
        );
        assert_eq!(
            std::fs::read_to_string(&backups[1]).unwrap(),
            "saved manually twice and automatically"
        );
        assert_eq!(
            std::fs::read_to_string(&source).unwrap(),
            "saved manually twice and automatically"
        );
        assert_eq!(app.status_msg.as_deref(), Some("Autosaved"));
        assert!(!app.buf.dirty);

        if let Some(sessions) = crate::paths::sessions() {
            let session = sessions.join(format!("{}.json", crate::paths::path_key(&source)));
            let _ = std::fs::remove_file(session);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rolling_backup_failure_warns_without_turning_save_into_failure() {
        let dir = scratch_dir("rolling-warning");
        let source = dir.join("chapter.md");
        let blocked_root = dir.join("not-a-directory");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, "old").unwrap();
        std::fs::write(&blocked_root, "blocks directory creation").unwrap();

        let mut app = App::new(Some(source.clone())).unwrap();
        app.backup_depth = 1;
        app.backup_root = Some(blocked_root);
        let old_len = app.buf.len_chars();
        app.buf.delete(0..old_len);
        app.buf.insert(0, "new saved text");
        app.save();

        assert!(!app.buf.dirty);
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "new saved text");
        let message = app.status_msg.as_deref().unwrap();
        assert!(message.starts_with("Saved chapter.md;"));
        assert!(message.contains("rolling backup failed:"));

        if let Some(sessions) = crate::paths::sessions() {
            let session = sessions.join(format!("{}.json", crate::paths::path_key(&source)));
            let _ = std::fs::remove_file(session);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn idle_tick_journals_unnamed_buffer_when_autosave_is_disabled() {
        let dir = scratch_dir("recovery-unnamed");
        let mut app = App::new(None).unwrap();
        app.autosave = Duration::ZERO;
        app.recovery_journals[0] = Journal::in_root(&dir, None, "untitled-idle");
        app.buf.insert(0, "new unsaved manuscript");

        app.maybe_autosave();

        let journal_path = app.recovery_journals[0].path().unwrap();
        assert!(app.buf.dirty);
        assert_eq!(
            std::fs::read_to_string(journal_path).unwrap(),
            "new unsaved manuscript"
        );
        assert!(
            journal_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("untitled-idle-")
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clean_exit_clears_journal_but_dirty_abandon_retains_it() {
        let clean_dir = scratch_dir("recovery-clean-exit");
        let mut clean_app = App::new(None).unwrap();
        clean_app.autosave = Duration::ZERO;
        clean_app.recovery_journals[0] = Journal::in_root(&clean_dir, None, "untitled-clean");
        clean_app.buf.insert(0, "saved work");
        clean_app.maybe_autosave();
        let clean_path = clean_app.recovery_journals[0].path().unwrap().to_path_buf();
        clean_app.buf.dirty = false;

        clean_app.close_or_quit();

        assert!(clean_app.quit);
        assert!(!clean_path.exists());

        let dirty_dir = scratch_dir("recovery-dirty-exit");
        let mut dirty_app = App::new(None).unwrap();
        dirty_app.autosave = Duration::ZERO;
        dirty_app.recovery_journals[0] = Journal::in_root(&dirty_dir, None, "untitled-dirty");
        dirty_app.buf.insert(0, "abandoned but recoverable");
        dirty_app.maybe_autosave();
        let dirty_path = dirty_app.recovery_journals[0].path().unwrap().to_path_buf();

        // This is called only after the user confirms abandoning changes.
        dirty_app.close_or_quit();

        assert!(dirty_app.quit);
        assert!(dirty_path.exists());
        assert_eq!(
            std::fs::read_to_string(&dirty_path).unwrap(),
            "abandoned but recoverable"
        );

        let _ = std::fs::remove_dir_all(clean_dir);
        let _ = std::fs::remove_dir_all(dirty_dir);
    }

    #[test]
    fn startup_recovery_restores_as_one_undoable_dirty_edit() {
        let dir = scratch_dir("recovery-restore");
        let source = dir.join("chapter.md");
        let recovery_root = dir.join("recovery");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, "saved manuscript").unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let mut app = App::new(Some(source.clone())).unwrap();
        app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
        app.recovery_journals[0]
            .write_if_changed(&Rope::from_str("recovered manuscript"), Instant::now())
            .unwrap();
        let recovery_path = app.recovery_journals[0].path().unwrap().to_path_buf();
        app.offer_recovery_for_active();

        assert!(matches!(app.mode, Mode::ConfirmRecover));
        assert!(
            !app.splash,
            "the splash must not consume the recovery answer"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::ConfirmRecover));
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.buf.rope.to_string(), "recovered manuscript");
        assert!(app.buf.dirty);
        assert_eq!(
            std::fs::read_to_string(&source).unwrap(),
            "saved manuscript"
        );
        assert!(
            recovery_path.exists(),
            "restore must retain the journal until save"
        );

        app.execute(Cmd::Undo);
        assert_eq!(app.buf.rope.to_string(), "saved manuscript");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn simulated_crash_restart_restores_then_save_clears_journal() {
        let dir = scratch_dir("recovery-restart");
        let source = dir.join("chapter.md");
        let recovery_root = dir.join("recovery");
        let disk_text = "saved manuscript";
        let recovered_text = "saved manuscript plus unsaved work";
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, disk_text).unwrap();
        std::thread::sleep(Duration::from_millis(20));

        // First process: edit and reach the idle recovery tick, then disappear
        // without save/clean-exit cleanup.
        let mut crashed = App::new(Some(source.clone())).unwrap();
        crashed.autosave = Duration::ZERO;
        crashed.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
        let end = crashed.buf.len_chars();
        crashed.buf.insert(end, " plus unsaved work");
        crashed.maybe_autosave();
        let recovery_path = crashed.recovery_journals[0].path().unwrap().to_path_buf();
        assert_eq!(
            std::fs::read_to_string(&recovery_path).unwrap(),
            recovered_text
        );
        assert_eq!(std::fs::read_to_string(&source).unwrap(), disk_text);
        drop(crashed);

        // Second process: reopen from disk, discover the surviving journal,
        // accept it, and keep the on-disk manuscript unchanged until save.
        let mut restarted = App::new(Some(source.clone())).unwrap();
        restarted.backup_depth = 0;
        restarted.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
        restarted.offer_recovery_for_active();
        assert!(matches!(restarted.mode, Mode::ConfirmRecover));
        restarted.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

        assert_eq!(restarted.buf.rope.to_string(), recovered_text);
        assert!(restarted.buf.dirty);
        assert_eq!(std::fs::read_to_string(&source).unwrap(), disk_text);
        assert!(recovery_path.exists());

        restarted.save();

        assert!(!restarted.buf.dirty);
        assert_eq!(std::fs::read_to_string(&source).unwrap(), recovered_text);
        assert!(
            !recovery_path.exists(),
            "committing restored text must clear its crash journal"
        );

        if let Some(sessions) = crate::paths::sessions() {
            let session = sessions.join(format!("{}.json", crate::paths::path_key(&source)));
            let _ = std::fs::remove_file(session);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn declining_startup_recovery_keeps_disk_text_and_clears_record() {
        let dir = scratch_dir("recovery-decline");
        let source = dir.join("chapter.md");
        let recovery_root = dir.join("recovery");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, "saved manuscript").unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let mut app = App::new(Some(source.clone())).unwrap();
        app.recovery_journals[0] = Journal::in_root(&recovery_root, Some(&source), "unused");
        app.recovery_journals[0]
            .write_if_changed(&Rope::from_str("declined manuscript"), Instant::now())
            .unwrap();
        let recovery_path = app.recovery_journals[0].path().unwrap().to_path_buf();
        app.offer_recovery_for_active();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.buf.rope.to_string(), "saved manuscript");
        assert!(!app.buf.dirty);
        assert!(!recovery_path.exists());
        assert_eq!(app.status_msg.as_deref(), Some("Recovery declined"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn app_project_initializes_as_none() {
        // R1.8: backward compat — app starts with no project when opening
        // a bare file. Task 1.2.
        let app = App::new(None).expect("App creation should succeed");
        assert!(app.project.is_none(), "Project should be None at startup");
    }

    #[test]
    fn project_field_holds_loaded_project() {
        // Task 1.2: the project field exists and can store a Project.
        // We don't test the full load flow here (that's in project.rs tests),
        // just that the type can hold one.
        let mut app = App::new(None).expect("App creation should succeed");
        assert!(app.project.is_none());

        // Setting a project (would be done by open_project in practice).
        // Just verify the field can hold it.
        let _can_be_set: Option<Project> = app.project.take();
        app.project = None; // Put it back
        assert!(app.project.is_none());
    }

    /// Helper to create a test project with sample documents.
    fn setup_test_project() -> (std::path::PathBuf, Project) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("pstar-app-test-1.4-{}", id));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect(&format!("Failed to create test dir: {:?}", dir));

        // Create test files.
        let doc1 = dir.join("doc1.md");
        let doc2 = dir.join("doc2.md");
        let doc3 = dir.join("doc3.md");
        std::fs::write(&doc1, "First document content").expect("Failed to write doc1");
        std::fs::write(&doc2, "Second document content").expect("Failed to write doc2");
        std::fs::write(&doc3, "Third document content").expect("Failed to write doc3");

        // Create a project manifest.
        let manifest_path = dir.join("test.pstarproj");
        let project = Project {
            manifest_path: manifest_path.clone(),
            manifest: crate::project::ProjectManifest {
                name: "Test Project".to_string(),
                docs: vec![
                    crate::project::DocEntry {
                        path: doc1,
                        title: "Doc 1".to_string(),
                        include_in_compile: true,
                    },
                    crate::project::DocEntry {
                        path: doc2,
                        title: "Doc 2".to_string(),
                        include_in_compile: true,
                    },
                    crate::project::DocEntry {
                        path: doc3,
                        title: "Doc 3".to_string(),
                        include_in_compile: true,
                    },
                ],
                separator: crate::project::Separator::PageBreak,
            },
        };
        project.save().expect(&format!(
            "Failed to save project manifest to {:?}",
            manifest_path
        ));

        (dir, project)
    }

    #[test]
    fn binder_move_up_at_top_is_noop() {
        // Task 1.4: moving the first document up should be a no-op.
        let (_dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        app.project = Some(project);

        // Enter binder mode with first doc selected.
        app.mode = Mode::Binder {
            entries: vec![
                BinderEntry {
                    idx: 0,
                    title: "Doc 1".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 1,
                    title: "Doc 2".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
            ],
            selected: 0,
        };

        // Try to move up from position 0.
        app.binder_move_up();

        // Should show "Already at top" message.
        assert!(app.status_msg.as_ref().unwrap().contains("top"));

        // Order should be unchanged.
        let project = app.project.as_ref().unwrap();
        assert_eq!(project.manifest.docs[0].title, "Doc 1");
        assert_eq!(project.manifest.docs[1].title, "Doc 2");
    }

    #[test]
    fn binder_move_down_at_bottom_is_noop() {
        // Task 1.4: moving the last document down should be a no-op.
        let (_dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        app.project = Some(project);

        // Enter binder mode with last doc selected.
        app.mode = Mode::Binder {
            entries: vec![
                BinderEntry {
                    idx: 0,
                    title: "Doc 1".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 1,
                    title: "Doc 2".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 2,
                    title: "Doc 3".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
            ],
            selected: 2,
        };

        // Try to move down from last position.
        app.binder_move_down();

        // Should show "Already at bottom" message.
        assert!(app.status_msg.as_ref().unwrap().contains("bottom"));

        // Order should be unchanged.
        let project = app.project.as_ref().unwrap();
        assert_eq!(project.manifest.docs[2].title, "Doc 3");
    }

    #[test]
    fn binder_move_up_reorders_and_saves() {
        // Task 1.4: moving a document up should reorder and persist atomically (R1.4).
        let (_dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        app.project = Some(project);

        // Enter binder mode with second doc selected.
        app.mode = Mode::Binder {
            entries: vec![
                BinderEntry {
                    idx: 0,
                    title: "Doc 1".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 1,
                    title: "Doc 2".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 2,
                    title: "Doc 3".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
            ],
            selected: 1,
        };

        // Move second doc up (should become first).
        app.binder_move_up();

        // Check the new order in memory.
        let project = app.project.as_ref().unwrap();
        assert_eq!(project.manifest.docs[0].title, "Doc 2");
        assert_eq!(project.manifest.docs[1].title, "Doc 1");
        assert_eq!(project.manifest.docs[2].title, "Doc 3");

        // Verify it was saved (reload and check).
        let manifest_path = project.manifest_path.clone();
        let reloaded = Project::load(&manifest_path).unwrap();
        assert_eq!(reloaded.manifest.docs[0].title, "Doc 2");
        assert_eq!(reloaded.manifest.docs[1].title, "Doc 1");
    }

    #[test]
    fn binder_move_down_reorders_and_saves() {
        // Task 1.4: moving a document down should reorder and persist atomically (R1.4).
        let (_dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        app.project = Some(project);

        // Enter binder mode with first doc selected.
        app.mode = Mode::Binder {
            entries: vec![
                BinderEntry {
                    idx: 0,
                    title: "Doc 1".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 1,
                    title: "Doc 2".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 2,
                    title: "Doc 3".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
            ],
            selected: 0,
        };

        // Move first doc down (should become second).
        app.binder_move_down();

        // Check the new order in memory.
        let project = app.project.as_ref().unwrap();
        assert_eq!(project.manifest.docs[0].title, "Doc 2");
        assert_eq!(project.manifest.docs[1].title, "Doc 1");
        assert_eq!(project.manifest.docs[2].title, "Doc 3");

        // Verify it was saved.
        let manifest_path = project.manifest_path.clone();
        let reloaded = Project::load(&manifest_path).unwrap();
        assert_eq!(reloaded.manifest.docs[0].title, "Doc 2");
        assert_eq!(reloaded.manifest.docs[1].title, "Doc 1");
    }

    #[test]
    fn project_add_doc_adds_and_saves() {
        // Task 1.4: adding a document should append to manifest and save atomically (R1.5).
        let (dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        app.project = Some(project);

        // Create a new file to add.
        let new_doc = dir.join("doc4.md");
        std::fs::write(&new_doc, "New document").unwrap();

        // Add the document.
        app.project_add_doc(new_doc.to_str().unwrap());

        // Verify it was added.
        let project = app.project.as_ref().unwrap();
        assert_eq!(project.manifest.docs.len(), 4);
        assert_eq!(project.manifest.docs[3].title, "doc4");

        // Verify it was saved.
        let manifest_path = project.manifest_path.clone();
        let reloaded = Project::load(&manifest_path).unwrap();
        assert_eq!(reloaded.manifest.docs.len(), 4);
        assert_eq!(reloaded.manifest.docs[3].title, "doc4");
    }

    #[test]
    fn project_add_doc_missing_file_shows_error() {
        // Task 1.4: adding a missing file should show an error (R1.5).
        let (_dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        app.project = Some(project);

        // Try to add a non-existent file.
        app.project_add_doc("/nonexistent/file.md");

        // Should show "File not found" error.
        assert!(app.status_msg.as_ref().unwrap().contains("not found"));

        // Project should be unchanged.
        let project = app.project.as_ref().unwrap();
        assert_eq!(project.manifest.docs.len(), 3);
    }

    #[test]
    fn project_remove_doc_removes_and_saves() {
        // Task 1.4: removing a document should delete from manifest but keep file (R1.5).
        let (_dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        let doc2_path = project.manifest.docs[1].path.clone();
        app.project = Some(project);

        // Enter binder mode with second doc selected.
        app.mode = Mode::Binder {
            entries: vec![
                BinderEntry {
                    idx: 0,
                    title: "Doc 1".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 1,
                    title: "Doc 2".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 2,
                    title: "Doc 3".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
            ],
            selected: 1,
        };

        // Remove the selected document.
        app.project_remove_doc();

        // Verify it was removed from the manifest.
        let project = app.project.as_ref().unwrap();
        assert_eq!(project.manifest.docs.len(), 2);
        assert_eq!(project.manifest.docs[0].title, "Doc 1");
        assert_eq!(project.manifest.docs[1].title, "Doc 3");

        // R1.5: verify the file still exists on disk.
        assert!(doc2_path.exists(), "File should not be deleted from disk");

        // Verify the change was saved.
        let manifest_path = project.manifest_path.clone();
        let reloaded = Project::load(&manifest_path).unwrap();
        assert_eq!(reloaded.manifest.docs.len(), 2);
    }

    #[test]
    fn project_remove_doc_shows_clear_message() {
        // Task 1.4: removing should show clear message that file is kept (R1.5).
        let (_dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        app.project = Some(project);

        // Enter binder mode with first doc selected.
        app.mode = Mode::Binder {
            entries: vec![BinderEntry {
                idx: 0,
                title: "Doc 1".to_string(),
                word_count: Some(3),
                exists: true,
            }],
            selected: 0,
        };

        // Remove the document.
        app.project_remove_doc();

        // Verify the status message mentions "file kept on disk".
        let msg = app.status_msg.as_ref().unwrap();
        assert!(
            msg.contains("kept on disk"),
            "Message should clarify file is kept: {}",
            msg
        );
    }

    #[test]
    fn project_remove_last_doc_adjusts_selection() {
        // Task 1.4: removing the last document should adjust selection to avoid out-of-bounds.
        let (_dir, project) = setup_test_project();
        let mut app = App::new(None).unwrap();
        app.project = Some(project);

        // Enter binder mode with last doc selected.
        app.mode = Mode::Binder {
            entries: vec![
                BinderEntry {
                    idx: 0,
                    title: "Doc 1".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 1,
                    title: "Doc 2".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 2,
                    title: "Doc 3".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
            ],
            selected: 2,
        };

        // Remove the last document.
        app.project_remove_doc();

        // Verify selection was adjusted.
        if let Mode::Binder { selected, entries } = &app.mode {
            assert_eq!(
                *selected, 1,
                "Selection should be adjusted to last valid index"
            );
            assert_eq!(entries.len(), 2);
        } else {
            panic!("Should still be in binder mode");
        }
    }

    #[test]
    fn missing_file_resilience_in_binder() {
        // Task 1.5: missing files should be flagged in binder but not prevent opening the project (R1.7).
        let dir = std::env::temp_dir().join("pstar-missing-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create a project with three docs, but only two files exist.
        let doc1 = dir.join("exists1.md");
        let doc2 = dir.join("missing.md"); // This file will NOT be created
        let doc3 = dir.join("exists2.md");

        std::fs::write(&doc1, "File 1 content").unwrap();
        // doc2 intentionally not created
        std::fs::write(&doc3, "File 3 content").unwrap();

        let manifest_path = dir.join("test.pstarproj");
        let project = Project {
            manifest_path: manifest_path.clone(),
            manifest: ProjectManifest {
                name: "Test Project".to_string(),
                docs: vec![
                    DocEntry {
                        path: doc1.clone(),
                        title: "Existing Doc 1".to_string(),
                        include_in_compile: true,
                    },
                    DocEntry {
                        path: doc2.clone(),
                        title: "Missing Doc".to_string(),
                        include_in_compile: true,
                    },
                    DocEntry {
                        path: doc3.clone(),
                        title: "Existing Doc 3".to_string(),
                        include_in_compile: true,
                    },
                ],
                separator: crate::project::Separator::PageBreak,
            },
        };
        project.save().unwrap();

        // Load the project - should succeed despite missing file (R1.7).
        let loaded_project = Project::load(&manifest_path).unwrap();
        assert_eq!(loaded_project.manifest.docs.len(), 3);

        // Verify doc_exists correctly identifies missing file.
        assert!(loaded_project.doc_exists(0), "Doc 1 should exist");
        assert!(!loaded_project.doc_exists(1), "Doc 2 should be missing");
        assert!(loaded_project.doc_exists(2), "Doc 3 should exist");

        // Verify word count returns None for missing file.
        assert!(
            loaded_project.doc_word_count(0).is_some(),
            "Word count should work for existing files"
        );
        assert!(
            loaded_project.doc_word_count(1).is_none(),
            "Word count should return None for missing file"
        );
        assert!(
            loaded_project.doc_word_count(2).is_some(),
            "Word count should work for existing files"
        );

        // Create an App with this project and open the binder.
        let mut app = App::new(None).unwrap();
        app.project = Some(loaded_project);

        // Trigger binder toggle to build entries.
        app.execute(Cmd::BinderToggle);

        // Verify binder entries reflect the existence status.
        if let Mode::Binder { entries, .. } = &app.mode {
            assert_eq!(entries.len(), 3);
            assert!(entries[0].exists, "Entry 0 should exist");
            assert!(!entries[1].exists, "Entry 1 should be marked as missing");
            assert!(entries[2].exists, "Entry 2 should exist");

            assert!(entries[0].word_count.is_some());
            assert!(entries[1].word_count.is_none());
            assert!(entries[2].word_count.is_some());
        } else {
            panic!("Should be in binder mode");
        }

        // Simulate trying to open the missing file from the binder.
        app.mode = Mode::Binder {
            entries: vec![
                BinderEntry {
                    idx: 0,
                    title: "Existing Doc 1".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 1,
                    title: "Missing Doc".to_string(),
                    word_count: None,
                    exists: false,
                },
                BinderEntry {
                    idx: 2,
                    title: "Existing Doc 3".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
            ],
            selected: 1, // Select the missing file
        };

        // Try to open it by simulating Enter key.
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        app.handle_binder_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        // Should show error message and stay in binder mode.
        assert!(
            app.status_msg.as_ref().unwrap().contains("missing"),
            "Should show missing file error"
        );
        assert!(
            matches!(app.mode, Mode::Binder { .. }),
            "Should stay in binder mode"
        );

        // Verify we can open existing files.
        app.mode = Mode::Binder {
            entries: vec![
                BinderEntry {
                    idx: 0,
                    title: "Existing Doc 1".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
                BinderEntry {
                    idx: 1,
                    title: "Missing Doc".to_string(),
                    word_count: None,
                    exists: false,
                },
                BinderEntry {
                    idx: 2,
                    title: "Existing Doc 3".to_string(),
                    word_count: Some(3),
                    exists: true,
                },
            ],
            selected: 0, // Select existing file
        };

        app.handle_binder_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        // Should successfully open and exit binder mode.
        assert!(
            matches!(app.mode, Mode::Normal),
            "Should exit binder after opening existing file"
        );
        assert!(
            app.status_msg.as_ref().unwrap().contains("Opened"),
            "Should show success message"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
