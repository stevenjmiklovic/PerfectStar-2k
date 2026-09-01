use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ropey::Rope;

use crate::block::adjust_pos;
use crate::buffer::wrap_segments;
use crate::config::Config;
use crate::diff::{self, DiffLine};
use crate::export::docx::DocxExporter;
use crate::export::epub::EpubExporter;
use crate::export::html::{HtmlExporter, PlainTextExporter};
use crate::export::{self, CompiledDoc, Exporter};
use crate::history::{Edit, EditGroup, EditKind};
use crate::keymap::{self, Cmd, Prefix};
use crate::killring::{KillRing, PutCycle};
use crate::meta;
use crate::normalize;
use crate::outline;
use crate::pane::Pane;
use crate::project::{DocRole, Project};
use crate::projsearch;
use crate::recovery::{self, Journal};
use crate::rtf;
use crate::search::{ReplacePhase, ReplaceState, SearchState};
use crate::snapshot::{SnapshotEntry, SnapshotStore};
use crate::spellcheck;
use crate::sprint::{Focus, Report, Sprint};
use crate::stats::{DailyHistory, DocStats, GoalKind, SessionGoal};
use crate::style::{Readability, StyleEngine};
use crate::theme::Theme;
use crate::ui;

const JUMP_STACK_MAX: usize = 32;

/// The most characters an editorial comment body may hold (R9.1). Longer bodies
/// are trimmed to this on create/edit rather than rejected, since the writer's
/// words up to the limit are still worth keeping.
const ANNOTATION_MAX_CHARS: usize = 2000;

/// How long a finished sprint's report stays on screen. Long enough to read
/// after a sentence, short enough that it never becomes chrome.
const SPRINT_BANNER: Duration = Duration::from_secs(10);

/// How long the goal-reached notification stays on screen (R2.4: at least 5
/// seconds). Like the sprint banner, it survives keystrokes so the letter the
/// writer types when the goal lands does not immediately erase it.
const GOAL_BANNER: Duration = Duration::from_secs(5);

/// How long the word/character counts stay on screen when revealed on demand
/// (R2.1: at least 3 seconds). The writer glances at the number and returns to
/// typing without the count becoming permanent chrome.
const COUNT_FLASH: Duration = Duration::from_secs(3);

/// Result of a shared list-navigation key check. Panels call `list_nav_key`
/// before their own match arms so Up/Down/^E/^X/Esc are handled uniformly.
enum ListNav {
    /// Move selection up by 1 (saturating).
    Up,
    /// Move selection down by 1 (clamped to last).
    Down,
    /// Close the panel (Esc).
    Dismiss,
    /// Not a navigation key — let the panel handle it.
    Other,
}

/// Map a key event to a list-navigation action shared by all overlay panels.
fn list_nav_key(key: &KeyEvent) -> ListNav {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => ListNav::Dismiss,
        KeyCode::Up => ListNav::Up,
        KeyCode::Down => ListNav::Down,
        KeyCode::Char('e') if ctrl => ListNav::Up,
        KeyCode::Char('x') if ctrl => ListNav::Down,
        _ => ListNav::Other,
    }
}

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
    /// Project-wide search query (^PS).
    ProjectSearch,
    /// Project-wide replace: "find|replace" format (^PW).
    ProjectReplace,
    /// Sprint terms: `minutes[/words]` (^OP, R3.1).
    SprintSpec,
    /// A document's one-line synopsis (^PI, R5.1). Like a snapshot label, an
    /// empty answer is meaningful: it clears the synopsis.
    Synopsis,
    /// An editorial comment's text (^PC, R9.1). Empty deletes the comment.
    AnnotationText,
    /// Optional label for a snapshot taken with ^KN (R4.1). Unlike every other
    /// prompt, an empty answer is meaningful: take the snapshot unlabelled.
    SnapshotLabel,
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
    /// Project-wide search results (R6, task 5.1).
    ProjectSearch {
        query: String,
        results: Vec<projsearch::Match>,
        selected: usize,
        /// Whether to enter replace mode after search.
        replace_with: Option<String>,
    },
    /// This document's snapshots, newest first (R4.3, task 7.3).
    Revisions {
        entries: Vec<SnapshotEntry>,
        selected: usize,
        /// A version marked with Space, to compare against instead of the
        /// current draft (R4.4: "two snapshots, or a snapshot and current").
        compare: Option<usize>,
    },
    /// Every editorial comment in the document (R9.4).
    Annotations {
        entries: Vec<meta::Annotation>,
        selected: usize,
    },
    /// The unified export format picker (R7.2): choose a target, then a path.
    /// Only available formats appear; unbundled ones are omitted (R7.8).
    ExportMenu {
        entries: Vec<export::Target>,
        selected: usize,
    },
    /// A rendered comparison between two versions (R4.4, task 7.4).
    Diff {
        /// What is being compared, e.g. "13:45 before the cut → current draft".
        title: String,
        lines: Vec<DiffLine>,
        /// First visible diff line.
        scroll: usize,
        /// The older version on display, offered for restore with ^R (R4.5).
        restore: Option<SnapshotEntry>,
    },
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
    /// The document's one-line synopsis from its sidecar, shown as the binder's
    /// secondary line (R5.3). Read when the binder opens rather than per frame.
    pub synopsis: String,
}

/// A snapshot of the on-demand style figures (R8.4, R8.5).
#[derive(Debug, Clone)]
pub struct StyleReport {
    /// Whether these figures describe the marked block rather than the document.
    pub selection: bool,
    pub readability: Readability,
    pub overused: Vec<(String, usize)>,
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
    /// The style checker, configured from `config.toml` (R8, ADR-015).
    pub style: StyleEngine,
    pub style_enabled: bool,
    /// Readability and overused-word figures, computed when the stats overlay
    /// opens rather than per frame — on a book-length document this is real work
    /// and must not land in the draw path (R8.6).
    pub style_report: Option<StyleReport>,
    /// Keep the cursor's line pinned at a fixed row (view_rows / 2) and
    /// scroll the document under it, instead of only scrolling at the edges.
    pub typewriter: bool,
    /// Body font for `^KM` manuscript RTF export.
    pub manuscript_font: rtf::ManuscriptFont,
    /// Idle autosave; zero disables manuscript autosave (recovery remains on).
    autosave: Duration,
    /// Maximum timestamped rolling backups retained per saved document.
    backup_depth: usize,
    /// Automatic snapshots retained per document; 0 disables taking new ones
    /// (R4.2). Manual snapshots are never pruned.
    snapshot_keep: usize,
    /// Idle cadence for automatic snapshots; zero leaves them to saves alone.
    autosnapshot: Duration,
    /// When the idle cadence last fired; starts at launch so the first
    /// automatic snapshot is a full interval in, not on the first idle tick.
    /// Deliberately app-wide rather than per-pane: it's a cadence, and one clock
    /// keeps two dirty windows from drifting into alternating snapshots.
    last_autosnapshot: Instant,
    /// Metadata root for crash journals and the distinct `backups/` tree.
    backup_root: Option<PathBuf>,
    /// Root of the per-document snapshot tree; `None` disables snapshots.
    snapshot_root: Option<PathBuf>,
    /// Root of the per-document sidecar tree (synopsis, notes); `None` disables
    /// sidecar metadata (R5).
    pub meta_root: Option<PathBuf>,
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
    /// Cached whole-project prose word count (R2.1). Refreshed when the project
    /// loads, when the active document is switched, and on save — not per
    /// keystroke, so a 40-file project never pays disk I/O in the typing path
    /// (C6). `None` when no project is open.
    pub project_words: Option<usize>,
    /// When set, the counts are shown "on demand" until this instant elapses,
    /// even if the always-on display is off (R2.1: at least 3 seconds).
    count_flash: Option<Instant>,
    /// Whether the binder shows each document's synopsis as a secondary line
    /// (R5.3). Toggled by ^PY; on by default so a synopsis the writer set is
    /// visible without having to ask for it.
    pub show_synopsis: bool,
    /// Session writing goal (R2.3).
    pub goal: Option<SessionGoal>,
    /// Goal-reached notification was already shown this goal.
    goal_notified: bool,
    /// The goal-reached notification and when it landed (R2.4). Kept out of
    /// `status_msg` for the same reason as the sprint banner: it must survive
    /// the keystroke that reaches the goal, staying up for [`GOAL_BANNER`].
    goal_banner: Option<(String, Instant)>,
    /// Per-day words-written history for the active document.
    pub daily_history: DailyHistory,
    /// Word count at start of this editing session (for daily delta).
    session_start_words: usize,
    /// The running writing sprint, if any (R3.1).
    pub sprint: Option<Sprint>,
    /// The last sprint's result and when it landed (R3.2).
    ///
    /// Kept separately from `status_msg` because that is cleared by the next
    /// keystroke — and a sprint typically ends *mid-word*, so the report would
    /// be erased by the letter the writer was already typing. This survives
    /// typing for [`SPRINT_BANNER`] instead, without blocking anything.
    sprint_banner: Option<(String, Instant)>,
    /// Focus mode state, holding the help level to restore (R3.3).
    pub focus: Option<Focus>,
    /// Whether focus mode dims everything outside the current paragraph (R3.4).
    pub focus_dim: bool,
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
        // As with `snapshot_root`: a test run must not write into the writer's
        // real stats directory. An unrooted history keeps working in memory and
        // saves nowhere, and the tests that check persistence build their own.
        let (daily_history, goal) = if cfg!(test) {
            (DailyHistory::default(), None)
        } else {
            (
                DailyHistory::load(pane.buf.path.as_deref()),
                // Resume an active session goal if the writer left one running
                // when they last quit (R2.5).
                SessionGoal::load_for(pane.buf.path.as_deref()),
            )
        };
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
            style: StyleEngine::new(config.style_checks()),
            style_enabled: config.style,
            style_report: None,
            typewriter: config.typewriter,
            manuscript_font: config.manuscript_font(),
            autosave: Duration::from_secs(config.autosave_secs),
            backup_depth: config.backup_depth,
            snapshot_keep: config.snapshot_keep,
            autosnapshot: Duration::from_secs(config.autosnapshot_secs),
            last_autosnapshot: Instant::now(),
            backup_root: crate::paths::recovery(),
            // A snapshot is a full copy of the document, so tests must never
            // default into the writer's real metadata tree; the ones that
            // exercise snapshots point this at a temporary directory.
            snapshot_root: if cfg!(test) {
                None
            } else {
                crate::paths::snapshots()
            },
            // Same reasoning as `snapshot_root`: tests point this at a
            // temporary directory rather than the writer's real sidecars.
            meta_root: if cfg!(test) {
                None
            } else {
                crate::paths::meta()
            },
            recovery_journals: vec![recovery_journal],
            pending_recovery: None,
            macro_keys: Vec::new(),
            recording: false,
            playing: false,
            quit: false,
            project: None, // R1.8: backward compat — no project at bare-file launch
            doc_stats: initial_stats,
            show_word_count: true,
            project_words: None,
            count_flash: None,
            show_synopsis: true,
            // A goal restored from disk (R2.5) whose target was already reached
            // should not re-fire its notification on reopen.
            goal_notified: goal.as_ref().is_some_and(|g| g.reached),
            goal,
            goal_banner: None,
            daily_history,
            session_start_words: start_words,
            sprint: None,
            sprint_banner: None,
            focus: None,
            focus_dim: config.focus_dim,
        };
        app.load_annotations(0);
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
            // Checked every turn, not only on idle ticks, so a sprint ends when
            // it ends even if the writer is typing straight through it.
            self.tick_sprint();
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
        let _ = self.flush_annotations();
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
        let snapshot_keep = self.snapshot_keep;
        let snapshot_root = self.snapshot_root.clone();
        // The idle snapshot cadence starts counting from the first idle tick,
        // so a session that never idles never owes one.
        let cadence_due =
            !self.autosnapshot.is_zero() && self.last_autosnapshot.elapsed() >= self.autosnapshot;
        let mut cadence_fired = false;
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

            // An interval snapshot of work in progress (R4.2). Taken before the
            // autosave below so the version on record is the one being edited,
            // and never at the cost of the save if it fails.
            if cadence_due {
                cadence_fired = true;
                if let Err(error) = auto_snapshot(
                    snapshot_root.as_deref(),
                    pane.buf.path.as_deref(),
                    &pane.buf.rope,
                    snapshot_keep,
                ) {
                    warnings.push(format!("Snapshot failed: {error}"));
                }
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
                        // R4.2: a snapshot per saved version. Skipped when the
                        // cadence just took one of identical text.
                        if !cadence_due
                            && let Err(error) = auto_snapshot(
                                snapshot_root.as_deref(),
                                pane.buf.path.as_deref(),
                                &pane.buf.rope,
                                snapshot_keep,
                            )
                        {
                            warnings.push(format!("Snapshot failed: {error}"));
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

        if cadence_fired {
            self.last_autosnapshot = Instant::now();
        }

        if !warnings.is_empty() {
            self.status_msg = Some(warnings.join("; "));
        } else if autosaved {
            self.status_msg = Some(String::from("Autosaved"));
        }

        // Annotation anchors that moved while typing (R9.5) are written back on
        // the same idle tick as the recovery journal.
        if let Some(warning) = self.flush_annotations() {
            self.status_msg = Some(warning);
        }

        // Debounced full recount to correct any drift (R2.7).
        if self.doc_stats.needs_recount() {
            self.doc_stats
                .full_recount(&self.panes[self.active].buf.rope);
        }

        // Check session goal progress (R2.4).
        // Gather the reached-goal facts under the goal's borrow, then release it
        // before touching the rest of `self` (banner, persistence).
        let reached_now = if let Some(goal) = self.goal.as_mut() {
            if !goal.reached && goal.is_met(self.doc_stats.words) {
                goal.reached = true;
                let (current, target) = goal.progress(self.doc_stats.words);
                let unit = match goal.kind {
                    GoalKind::Words => "words",
                    GoalKind::Minutes => "minutes",
                };
                Some(format!("Goal reached! {current}/{target} {unit}"))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(message) = reached_now
            && !self.goal_notified
        {
            self.goal_notified = true;
            // A goal that survives a restart (R2.5) must not re-notify, so
            // persist that it has been reached. As with the daily history,
            // tests never touch the writer's real stats dir.
            if !cfg!(test) {
                let path = self.panes[self.active].buf.path.clone();
                if let Some(goal) = self.goal.as_ref() {
                    goal.save_for(path.as_deref());
                }
            }
            // Non-blocking, no dismissal, visible for at least GOAL_BANNER
            // (R2.4) — carried in its own banner so the keystroke that reached
            // the goal cannot erase it.
            self.goal_banner = Some((message, Instant::now()));
        }
    }

    /// Set a session goal from the ^OG prompt (R2.3). A bare number, or a
    /// number with a `w`/`words` suffix, is a word goal; a `m`/`min`/`minutes`
    /// suffix makes it a time goal. Both are bounds-checked before taking.
    fn set_goal(&mut self, spec: &str) {
        let spec = spec.trim();
        let lower = spec.to_ascii_lowercase();
        let (kind, digits) = if let Some(rest) = lower
            .strip_suffix("minutes")
            .or_else(|| lower.strip_suffix("min"))
            .or_else(|| lower.strip_suffix('m'))
        {
            (GoalKind::Minutes, rest.trim())
        } else if let Some(rest) = lower
            .strip_suffix("words")
            .or_else(|| lower.strip_suffix('w'))
        {
            (GoalKind::Words, rest.trim())
        } else {
            (GoalKind::Words, lower.trim())
        };

        let parsed = digits.parse::<usize>().ok();
        let bounded = match (kind, parsed) {
            (GoalKind::Words, Some(n)) => SessionGoal::validate_words(n).map(|n| (kind, n)),
            (GoalKind::Minutes, Some(n)) => SessionGoal::validate_minutes(n).map(|n| (kind, n)),
            (GoalKind::Minutes, None) => Err(String::from("Enter a number of minutes, e.g. 30m")),
            (GoalKind::Words, None) => Err(String::from("Enter a number of words, e.g. 500")),
        };

        match bounded {
            Ok((kind, target)) => {
                let goal = SessionGoal::new(kind, target, self.doc_stats.words);
                // Persist immediately so the target/baseline/start survive a
                // restart (R2.5); tests stay out of the real stats dir.
                if !cfg!(test) {
                    let path = self.panes[self.active].buf.path.clone();
                    goal.save_for(path.as_deref());
                }
                self.goal = Some(goal);
                self.goal_notified = false;
                self.goal_banner = None;
                self.status_msg = Some(match kind {
                    GoalKind::Words => format!("Goal set: {target} words this session"),
                    GoalKind::Minutes => format!("Goal set: {target} minutes this session"),
                });
            }
            Err(message) => self.status_msg = Some(message),
        }
    }

    /// The goal-reached notification, while it is still fresh (R2.4).
    pub fn goal_banner(&self) -> Option<&str> {
        self.goal_banner
            .as_ref()
            .filter(|(_, shown)| shown.elapsed() < GOAL_BANNER)
            .map(|(message, _)| message.as_str())
    }

    /// End a sprint whose clock ran out or whose word target was met: report it
    /// and file it in the daily history (R3.2).
    ///
    /// Nothing here touches the buffer (R3.5) — a sprint is a clock and a
    /// counter, and its only side effect on disk is one line of history.
    fn tick_sprint(&mut self) {
        self.tick_sprint_at(Instant::now());
    }

    /// The clock-injected form, so tests can run a sprint out without waiting.
    fn tick_sprint_at(&mut self, now: Instant) {
        let Some(sprint) = self.sprint.clone() else {
            return;
        };
        let words = self.doc_stats.words;
        if !sprint.is_finished(now, words) {
            return;
        }

        let report = sprint.report(now, words);
        self.sprint = None;
        self.file_sprint(&report, now, "Sprint done");
    }

    /// File a completed sprint: append it to the daily history and raise the
    /// non-modal summary that outlives the keystroke that ended it.
    ///
    /// Both a sprint that reaches its target and one the writer calls off end
    /// here (R3.3): a cancelled sprint is a sprint that ended, so its words and
    /// elapsed time are still recorded (R3.4). `verb` is the only thing that
    /// differs between the two — "Sprint done" versus "Sprint stopped".
    fn file_sprint(&mut self, report: &Report, now: Instant, verb: &str) {
        self.daily_history
            .record_sprint(report.words, report.elapsed.as_secs(), report.met_target);
        let mark = if report.met_target { " ✓" } else { "" };
        let message = match self.daily_history.save() {
            Ok(()) => format!("{verb} — {}{mark}", report.summary()),
            Err(error) => format!(
                "{verb} — {}{mark} (history not saved: {error})",
                report.summary()
            ),
        };
        self.status_msg = Some(message.clone());
        self.sprint_banner = Some((message, now));
    }

    /// The finished-sprint report, while it is still fresh (R3.2).
    pub fn sprint_banner(&self) -> Option<&str> {
        self.sprint_banner
            .as_ref()
            .filter(|(_, shown)| shown.elapsed() < SPRINT_BANNER)
            .map(|(message, _)| message.as_str())
    }

    /// Whether the word/character counts should render this frame (R2.1): either
    /// the always-on display is enabled, or an on-demand reveal is still within
    /// its at-least-3-second window.
    pub fn counts_visible(&self) -> bool {
        self.show_word_count
            || self
                .count_flash
                .is_some_and(|shown| shown.elapsed() < COUNT_FLASH)
    }

    /// Recompute the cached whole-project prose word count (R2.1). Called at the
    /// points where the set of documents or their on-disk contents may have
    /// changed — project load, document switch, and save — never in the
    /// keystroke path, so a large project never costs disk I/O while typing (C6).
    pub fn refresh_project_words(&mut self) {
        self.project_words = self.project.as_ref().map(|p| p.total_prose_words());
    }

    /// ^OF: strip the screen to the prose, or put the chrome back (R3.3).
    ///
    /// Purely presentational (R3.5). The one piece of editor state it changes is
    /// the help level, remembered on the way in so a writer who runs with menus
    /// gets them back on the way out.
    fn toggle_focus(&mut self) {
        match self.focus.take() {
            Some(focus) => {
                self.help_level = focus.prior_help_level();
                self.status_msg = Some(String::from("Focus mode off"));
            }
            None => {
                self.focus = Some(Focus::enter(self.help_level));
                self.help_level = 0;
                self.status_msg = Some(String::from("Focus mode — ^OF to exit"));
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
        // R11.7: if the state directory is inaccessible the journal has no path
        // and every write is a silent no-op. Warn once at startup so the writer
        // knows crash recovery is off for this session rather than assuming
        // their unsaved work is protected.
        if !self.recovery_journals[pane].is_available() {
            self.status_msg = Some(String::from("Crash recovery unavailable for this session"));
            return;
        }
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
            Mode::ProjectSearch { .. } => self.handle_project_search_key(key),
            Mode::Revisions { .. } => self.handle_revisions_key(key),
            Mode::Annotations { .. } => self.handle_annotations_key(key),
            Mode::ExportMenu { .. } => self.handle_export_menu_key(key),
            Mode::Diff { .. } => self.handle_diff_key(key),
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
        let last = entries.len().saturating_sub(1);
        match list_nav_key(&key) {
            ListNav::Dismiss => {
                self.mode = Mode::Normal;
                return;
            }
            ListNav::Up => {
                *selected = selected.saturating_sub(1);
                return;
            }
            ListNav::Down => {
                *selected = (*selected + 1).min(last);
                return;
            }
            ListNav::Other => {}
        }
        match key.code {
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
        let last = matches.len().saturating_sub(1);
        match list_nav_key(&key) {
            ListNav::Dismiss => {
                self.mode = Mode::Normal;
                return;
            }
            ListNav::Up => {
                *selected = selected.saturating_sub(1);
                return;
            }
            ListNav::Down => {
                *selected = (*selected + 1).min(last);
                return;
            }
            ListNav::Other => {}
        }
        match key.code {
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
        let last = entries.len().saturating_sub(1);
        match list_nav_key(&key) {
            ListNav::Dismiss => {
                self.mode = Mode::Normal;
                return;
            }
            ListNav::Up => {
                *selected = selected.saturating_sub(1);
                return;
            }
            ListNav::Down => {
                *selected = (*selected + 1).min(last);
                return;
            }
            ListNav::Other => {}
        }
        match key.code {
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
                            let title = doc.title.clone();
                            self.mode = Mode::Normal;
                            // R1.3: save the currently open document before
                            // opening the selected one, mirroring the project-
                            // search jump path. Session restore on the newly
                            // opened doc is handled by Pane::open.
                            if self.panes[self.active].buf.dirty {
                                self.save();
                            }
                            match self.switch_active_pane(path) {
                                Ok(()) => {
                                    self.status_msg = Some(format!("Opened: {title}"));
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
                self.project_prompt(
                    "Export project DOCX to file",
                    InputAction::ExportProjectDocx,
                );
            }
            Cmd::ExportProjectEpub => {
                self.project_prompt(
                    "Export project EPUB to file",
                    InputAction::ExportProjectEpub,
                );
            }
            Cmd::ExportProjectHtml => {
                self.project_prompt(
                    "Export project HTML to file",
                    InputAction::ExportProjectHtml,
                );
            }
            Cmd::ExportMenu => self.open_export_menu(),
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
                } else if self.project.is_some() {
                    // Opening the binder is the moment the writer reviews the
                    // whole project, and doc add/remove/reorder all reload the
                    // project — so re-sum the whole-project total here (R2.1).
                    self.refresh_project_words();
                    self.mode = Mode::Binder {
                        entries: self.binder_entries(),
                        selected: 0,
                    };
                } else {
                    self.status_msg = Some(String::from("No project loaded (^PP to open)"));
                }
            }
            Cmd::BinderMoveUp => self.binder_move_up(),
            Cmd::BinderMoveDown => self.binder_move_down(),
            Cmd::ProjectAddDoc => {
                self.project_prompt(
                    "Add document to project (file path)",
                    InputAction::ProjectAddDoc,
                );
            }
            Cmd::ProjectRemoveDoc => self.project_remove_doc(),
            Cmd::WordCount => {
                // R2.1: ^OC toggles the counts between always-visible and
                // on-demand. Turning the always-on display off still flashes the
                // counts for at least COUNT_FLASH so the writer sees them once as
                // they dismiss the permanent readout.
                self.show_word_count = !self.show_word_count;
                if self.show_word_count {
                    self.count_flash = None;
                    self.status_msg = Some(String::from("Word count on"));
                } else {
                    self.count_flash = Some(Instant::now());
                    self.status_msg = Some(String::from("Word count off (shown briefly)"));
                }
            }
            Cmd::SetGoal => {
                self.mode = Mode::Input {
                    label: String::from("Session goal: words (500) or time (30m)"),
                    value: String::new(),
                    action: InputAction::SetGoal,
                };
            }
            Cmd::StatsOverlay => {
                if matches!(self.mode, Mode::Stats) {
                    self.mode = Mode::Normal;
                    self.style_report = None;
                } else {
                    self.style_report = Some(self.style_report());
                    self.mode = Mode::Stats;
                }
            }
            Cmd::ProjectFind => {
                self.project_prompt("Project search", InputAction::ProjectSearch);
            }
            Cmd::ProjectReplace => {
                self.project_prompt(
                    "Project replace (find|replace)",
                    InputAction::ProjectReplace,
                );
            }
            Cmd::Snapshot => {
                if self.buf.path.is_some() {
                    self.mode = Mode::Input {
                        label: String::from("Snapshot label (Enter for none)"),
                        value: String::new(),
                        action: InputAction::SnapshotLabel,
                    };
                } else {
                    // Snapshots are keyed by the document's path; an unsaved
                    // buffer would file them under a name nothing can find again.
                    self.status_msg = Some(String::from(
                        "Save the document first, then ^KN snapshots it",
                    ));
                }
            }
            Cmd::RevisionsList => self.open_revisions(),
            Cmd::SprintStart => match self.sprint.take() {
                // The same chord stops a running sprint: a countdown you can't
                // call off isn't a writing tool. Cancelling still *ends* the
                // sprint, so its words and elapsed time are recorded up to the
                // cancellation point and appended to the session history
                // (R3.3, R3.4) — the same non-modal summary a finished sprint
                // raises, only labelled "stopped".
                Some(sprint) => {
                    let now = Instant::now();
                    let report = sprint.report(now, self.doc_stats.words);
                    self.file_sprint(&report, now, "Sprint stopped");
                }
                None => {
                    // A new sprint replaces the last one's report.
                    self.sprint_banner = None;
                    self.mode = Mode::Input {
                        label: String::from("Sprint: minutes[/words], e.g. 25 or 25/500"),
                        value: String::new(),
                        action: InputAction::SprintSpec,
                    };
                }
            },
            Cmd::FocusMode => self.toggle_focus(),
            Cmd::EditSynopsis => self.prompt_synopsis(),
            Cmd::ToggleSynopsis => self.toggle_synopsis(),
            Cmd::OpenNotes => self.open_notes(),
            Cmd::ToggleDocRole => self.toggle_doc_role(),
            Cmd::BinderOpenSplit => self.binder_open_split(),
            Cmd::Annotate => self.prompt_annotation(),
            Cmd::AnnotationList => self.open_annotation_list(),
            Cmd::NextAnnotation => self.jump_annotation(true),
            Cmd::PrevAnnotation => self.jump_annotation(false),
            Cmd::ToggleStyle => {
                self.style_enabled = !self.style_enabled;
                self.status_msg = Some(String::from(if self.style_enabled {
                    "Style checking on"
                } else {
                    "Style checking off"
                }));
            }
            Cmd::NextStyleIssue => self.next_style_issue(),
        }
        if !matches!(cmd, Cmd::Undo) && !is_edit_cmd(cmd) {
            self.history.break_group();
        }
    }

    /// Open a project-gated input prompt: if a project is loaded, enter
    /// `Mode::Input` with the given label and action; otherwise show the
    /// standard "no project" hint.
    fn project_prompt(&mut self, label: &str, action: InputAction) {
        if self.project.is_some() {
            self.mode = Mode::Input {
                label: String::from(label),
                value: String::new(),
                action,
            };
        } else {
            self.status_msg = Some(String::from("No project loaded (^PP to open)"));
        }
    }

    /// Replace the active pane with a freshly opened file, setting up its
    /// recovery journal and recomputing document statistics. Returns `Err`
    /// with a human-readable message on failure.
    fn switch_active_pane(&mut self, path: PathBuf) -> Result<(), String> {
        match Pane::open(Some(path)) {
            Ok(pane) => {
                let journal = Journal::new(pane.buf.path.as_deref());
                self.panes[self.active] = pane;
                self.recovery_journals[self.active] = journal;
                self.doc_stats =
                    crate::stats::DocStats::from_rope(&self.panes[self.active].buf.rope);
                let active = self.active;
                self.load_annotations(active);
                // The active document changed; the whole-project total may too
                // (R2.1). Recompute once, off the keystroke path.
                self.refresh_project_words();
                Ok(())
            }
            Err(e) => Err(format!("{e}")),
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
        // Editorial comments travel with the text they're about (R9.5), through
        // the same adjustment as the marks above; a comment whose text is gone
        // is orphaned, never dropped (R9.6).
        if !self.annotations.is_empty() {
            for annotation in self.annotations.iter_mut() {
                annotation.adjust(at, del_chars, ins);
            }
            self.annotations_dirty = true;
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
                // An empty answer cancels every prompt that needs a path, but an
                // unlabelled snapshot and a cleared synopsis are both real
                // answers.
                if path.is_empty() && !allows_empty_answer(action) {
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
                    InputAction::SetGoal => self.set_goal(&path),
                    InputAction::SnapshotLabel => {
                        let label = (!path.is_empty()).then_some(path.as_str());
                        self.take_snapshot(label);
                    }
                    InputAction::Synopsis => self.set_synopsis(&path),
                    InputAction::AnnotationText => self.set_annotation(&path),
                    InputAction::SprintSpec => {
                        let now = Instant::now();
                        match Sprint::parse(&path, self.doc_stats.words, now) {
                            Ok(sprint) => {
                                self.status_msg = Some(format!(
                                    "Sprint started — {}",
                                    sprint.chip(now, self.doc_stats.words)
                                ));
                                self.sprint = Some(sprint);
                            }
                            Err(message) => self.status_msg = Some(message),
                        }
                    }
                    InputAction::ProjectSearch => self.run_project_search(&path, None),
                    InputAction::ProjectReplace => {
                        // Format: "find|replace"
                        if let Some((find, replace)) = path.split_once('|') {
                            let find = find.to_string();
                            let replace = replace.to_string();
                            self.run_project_search(&find, Some(replace));
                        } else {
                            self.status_msg =
                                Some(String::from("Format: search|replacement (separate with |)"));
                        }
                    }
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

    /// ^KF: open the unified format-selection step (R7.2). The list is built
    /// from the single availability source of truth, so a format whose
    /// dependency is unbundled is omitted and its name reported (R7.8) rather
    /// than offered and left to fail. Toggles closed if already open.
    fn open_export_menu(&mut self) {
        if matches!(self.mode, Mode::ExportMenu { .. }) {
            self.mode = Mode::Normal;
            return;
        }
        let entries = export::available_targets();
        if entries.is_empty() {
            // No format can be produced by this build. Name what's missing
            // rather than presenting an empty picker (R7.8).
            let missing: Vec<String> = export::unavailable_targets()
                .into_iter()
                .map(|(t, dep)| format!("{} (needs {dep})", t.label()))
                .collect();
            self.status_msg = Some(format!("No export formats available: {}", missing.join(", ")));
            return;
        }
        // If any format was omitted for a missing dependency, say so up front so
        // the writer knows why it isn't in the list (R7.8).
        let omitted = export::unavailable_targets();
        if !omitted.is_empty() {
            let names: Vec<String> = omitted
                .into_iter()
                .map(|(t, dep)| format!("{} (needs {dep})", t.label()))
                .collect();
            self.status_msg = Some(format!("Unavailable: {}", names.join(", ")));
        }
        self.mode = Mode::ExportMenu {
            entries,
            selected: 0,
        };
    }

    fn handle_export_menu_key(&mut self, key: KeyEvent) {
        let (count, selected) = match &self.mode {
            Mode::ExportMenu { entries, selected } => (entries.len(), *selected),
            _ => return,
        };
        let last = count.saturating_sub(1);
        match list_nav_key(&key) {
            ListNav::Dismiss => {
                self.mode = Mode::Normal;
                return;
            }
            ListNav::Up => {
                if let Mode::ExportMenu { selected, .. } = &mut self.mode {
                    *selected = selected.saturating_sub(1);
                }
                return;
            }
            ListNav::Down => {
                if let Mode::ExportMenu { selected, .. } = &mut self.mode {
                    *selected = (*selected + 1).min(last);
                }
                return;
            }
            ListNav::Other => {}
        }
        if key.code == KeyCode::Enter {
            let target = match &self.mode {
                Mode::ExportMenu { entries, .. } => entries.get(selected).copied(),
                _ => None,
            };
            if let Some(target) = target {
                self.route_export_target(target);
            }
        }
    }

    /// Send a chosen format into the existing output-path prompt with the
    /// matching `InputAction`, so the format-selection step reuses
    /// `export_with` / `export_project` / `finish_export` unchanged (R7.2).
    /// A project, when loaded, exports the compiled book for the rich formats
    /// exactly as the direct project chords (^PD/^PF/^PH) do; plain-text and
    /// manuscript RTF stay single-document (matching ^KE/^KM).
    fn route_export_target(&mut self, target: export::Target) {
        let has_project = self.project.is_some();
        let (label, action) = match target {
            export::Target::PlainText => (
                "Export to file (notes stripped)",
                InputAction::ExportClean,
            ),
            export::Target::Rtf => (
                "Export manuscript RTF to file",
                InputAction::ExportManuscript,
            ),
            export::Target::Docx if has_project => {
                ("Export project DOCX to file", InputAction::ExportProjectDocx)
            }
            export::Target::Docx => ("Export DOCX to file", InputAction::ExportDocx),
            export::Target::Epub if has_project => {
                ("Export project EPUB to file", InputAction::ExportProjectEpub)
            }
            export::Target::Epub => ("Export EPUB to file", InputAction::ExportEpub),
            export::Target::Html if has_project => {
                ("Export project HTML to file", InputAction::ExportProjectHtml)
            }
            export::Target::Html => ("Export HTML to file", InputAction::ExportHtml),
        };
        self.mode = Mode::Input {
            label: String::from(label),
            value: String::new(),
            action,
        };
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
        // R4.2: every saved version gets an automatic snapshot. A snapshot
        // failure is reported but never turns a good save into a bad one.
        let active = self.active;
        if let Err(error) = auto_snapshot(
            self.snapshot_root.as_deref(),
            source.as_deref(),
            &self.panes[active].buf.rope,
            self.snapshot_keep,
        ) {
            warnings.push(format!("snapshot failed: {error}"));
        }
        if let Err(error) = self.recovery_journals[self.active].clear() {
            warnings.push(format!("recovery cleanup failed: {error}"));
        }
        // Anchors that moved with this document's text belong on disk with it.
        if let Some(warning) = self.flush_annotations() {
            warnings.push(warning);
        }
        // Save As may have changed the buffer path, so future dirty edits need
        // a journal keyed to the newly adopted destination.
        self.recovery_journals[self.active] = Journal::new(source.as_deref());
        self.save_session();
        // The active document's on-disk contents just changed, so the cached
        // whole-project total is stale (R2.1). Refresh it here, off the
        // keystroke path.
        self.refresh_project_words();
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
                let active = self.active;
                self.load_annotations(active);
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
                // A project is now open; compute its whole-project total (R2.1).
                self.refresh_project_words();
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
                self.refresh_project_words();
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
        if self.project.is_some() {
            let entries = self.binder_entries();
            let selected = new_selected.min(entries.len().saturating_sub(1));
            self.mode = Mode::Binder { entries, selected };
        }
    }

    /// The binder's rows: title, word count, existence, and each document's
    /// synopsis from its sidecar (R1.2, R5.3).
    fn binder_entries(&self) -> Vec<BinderEntry> {
        let Some(ref project) = self.project else {
            return Vec::new();
        };
        project
            .manifest
            .docs
            .iter()
            .enumerate()
            .map(|(idx, doc)| BinderEntry {
                idx,
                // R1.2: the binder title is the document's first Markdown
                // heading, falling back to the manifest title (file stem).
                title: project.doc_title(idx),
                word_count: project.doc_word_count(idx),
                exists: project.doc_exists(idx),
                // R5.3: the synopsis is a toggleable secondary line. When hidden
                // it reads as empty, which is exactly what the binder renderer
                // already treats as "no second row" — one switch, no new UI code.
                synopsis: match &self.meta_root {
                    Some(root) if self.show_synopsis => meta::synopsis(root, &doc.path),
                    _ => String::new(),
                },
            })
            .collect()
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

    // --- project-wide search (R6) -------------------------------------------

    fn run_project_search(&mut self, query: &str, replace_with: Option<String>) {
        let Some(ref project) = self.project else {
            self.status_msg = Some(String::from("No project loaded"));
            return;
        };
        let docs: Vec<(usize, String, std::path::PathBuf)> = project
            .manifest
            .docs
            .iter()
            .enumerate()
            .map(|(i, d)| (i, d.title.clone(), d.path.clone()))
            .collect();

        let active_path = self.panes[self.active].buf.path.clone();
        let results = projsearch::search_project(
            &docs,
            query,
            false,
            active_path.as_deref(),
            Some(&self.panes[self.active].buf.rope),
        );

        if results.is_empty() {
            self.status_msg = Some(format!("No matches for \"{query}\" in project"));
            return;
        }
        let count = results.len();
        self.mode = Mode::ProjectSearch {
            query: query.to_string(),
            results,
            selected: 0,
            replace_with,
        };
        self.status_msg = Some(format!("{count} match(es) found"));
    }

    fn handle_project_search_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let (num_results, selected, replace_with) = {
            let Mode::ProjectSearch {
                results,
                selected,
                replace_with,
                ..
            } = &self.mode
            else {
                return;
            };
            (results.len(), *selected, replace_with.clone())
        };
        let last = num_results.saturating_sub(1);
        match list_nav_key(&key) {
            ListNav::Dismiss => {
                self.mode = Mode::Normal;
                return;
            }
            ListNav::Up => {
                if let Mode::ProjectSearch { selected, .. } = &mut self.mode {
                    *selected = selected.saturating_sub(1);
                }
                return;
            }
            ListNav::Down => {
                if let Mode::ProjectSearch { selected, .. } = &mut self.mode {
                    *selected = (*selected + 1).min(last);
                }
                return;
            }
            ListNav::Other => {}
        }
        let replacing = replace_with.is_some();
        match key.code {
            // Y/N/A/Q per-match confirm during replace, consistent with the
            // in-document ^QA replace ergonomics (R6.3).
            KeyCode::Char('y') | KeyCode::Char('Y') if replacing => {
                self.project_search_replace_at(selected);
            }
            KeyCode::Char('n') | KeyCode::Char('N') if replacing => {
                // Skip this match; leave it in the list and advance.
                if let Mode::ProjectSearch { selected, .. } = &mut self.mode {
                    *selected = (*selected + 1).min(last);
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') if replacing => {
                // Replace all remaining (R6.3).
                self.project_search_replace_all();
            }
            KeyCode::Char('q') | KeyCode::Char('Q') if replacing => {
                self.mode = Mode::Normal;
                self.status_msg = Some(String::from("Project replace cancelled"));
            }
            // Legacy ^R accelerator for replace-current, retained for muscle memory.
            KeyCode::Char('r') if ctrl && replacing => {
                self.project_search_replace_at(selected);
            }
            KeyCode::Enter => {
                // Jump to the selected result (R6.2).
                self.project_search_jump(selected);
            }
            _ => {}
        }
    }

    fn project_search_jump(&mut self, idx: usize) {
        let (path, char_pos) = {
            let Mode::ProjectSearch { results, .. } = &self.mode else {
                return;
            };
            let Some(m) = results.get(idx) else { return };
            (m.path.clone(), m.char_pos)
        };
        self.mode = Mode::Normal;

        // If the file is already in the active pane, just jump.
        if self.panes[self.active].buf.path.as_deref() == Some(path.as_path()) {
            self.long_jump(char_pos);
            return;
        }

        // Open the doc (R6.2).
        match self.switch_active_pane(path) {
            Ok(()) => {
                self.long_jump(char_pos);
            }
            Err(e) => {
                self.status_msg = Some(format!("Failed to open: {e}"));
            }
        }
    }

    fn project_search_replace_at(&mut self, idx: usize) {
        let (path, char_pos, query_len, replacement) = {
            let Mode::ProjectSearch {
                results,
                query,
                replace_with,
                ..
            } = &self.mode
            else {
                return;
            };
            let Some(m) = results.get(idx) else { return };
            let Some(rep) = replace_with.as_ref() else {
                return;
            };
            (
                m.path.clone(),
                m.char_pos,
                query.chars().count(),
                rep.clone(),
            )
        };

        // Ensure the file is open in the active pane. If the current pane is
        // dirty and we must switch to a different file, persist it first via
        // the atomic-save path (R6.4) so no in-buffer work is silently lost
        // when the pane is replaced.
        if self.panes[self.active].buf.path.as_deref() != Some(path.as_path()) {
            if self.panes[self.active].buf.dirty {
                self.save();
            }
            if let Err(e) = self.switch_active_pane(path.clone()) {
                self.status_msg = Some(format!("Failed to open for replace: {e}"));
                return;
            }
        }

        // Apply the replacement as an undoable edit in this document's own log
        // (R6.4). The buffer is left dirty and is NOT persisted here: the user
        // reviews and explicitly saves (R6.7 — no silent unreviewable writes).
        self.set_cursor(char_pos);
        self.apply_edit(
            char_pos,
            query_len,
            &replacement,
            EditKind::Other,
            char_pos + replacement.chars().count(),
        );

        // Advance to next result (remove current from the list). Char positions
        // after the current match in the same file shift by the length delta.
        let delta = replacement.chars().count() as isize - query_len as isize;
        if let Mode::ProjectSearch {
            results, selected, ..
        } = &mut self.mode
        {
            if idx < results.len() {
                results.remove(idx);
            }
            if delta != 0 {
                for m in results.iter_mut() {
                    if m.path == path && m.char_pos > char_pos {
                        m.char_pos = (m.char_pos as isize + delta).max(0) as usize;
                    }
                }
            }
            if results.is_empty() {
                self.mode = Mode::Normal;
                self.status_msg = Some(String::from(
                    "All replacements applied (unsaved — ^KS/^KD to save)",
                ));
            } else {
                *selected = idx.min(results.len().saturating_sub(1));
            }
        }
    }

    fn project_search_replace_all(&mut self) {
        // Replace from the end to preserve char positions within each file.
        let entries = {
            let Mode::ProjectSearch {
                results,
                query,
                replace_with,
                ..
            } = &self.mode
            else {
                return;
            };
            let Some(rep) = replace_with.as_ref() else {
                return;
            };
            // Group by path, process in reverse char-pos order so offsets stay valid.
            let mut entries: Vec<(std::path::PathBuf, usize, usize, String)> = results
                .iter()
                .map(|m| {
                    (
                        m.path.clone(),
                        m.char_pos,
                        query.chars().count(),
                        rep.clone(),
                    )
                })
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
            entries
        };

        let mut count = 0usize;
        let mut last_path: Option<std::path::PathBuf> = None;

        for (path, char_pos, query_len, replacement) in &entries {
            // Open file if not already loaded.
            if self.panes[self.active].buf.path.as_deref() != Some(path.as_path()) {
                // Save previous file if it was modified.
                if self.panes[self.active].buf.dirty {
                    self.save();
                }
                if self.switch_active_pane(path.clone()).is_err() {
                    continue;
                }
            }
            last_path = Some(path.clone());
            self.set_cursor(*char_pos);
            self.apply_edit(
                *char_pos,
                *query_len,
                replacement,
                EditKind::Other,
                char_pos + replacement.chars().count(),
            );
            count += 1;
        }

        // Save the last modified file.
        if self.panes[self.active].buf.dirty {
            self.save();
        }
        let _ = last_path; // suppress unused warning
        self.mode = Mode::Normal;
        self.status_msg = Some(format!("{count} replacement(s) made"));
    }

    // --- style & readability (R8) ---------------------------------------------

    /// ^QI: jump to the next style issue, as ^QN does for spelling (R8.3).
    fn next_style_issue(&mut self) {
        if !self.style_enabled {
            self.status_msg = Some(String::from("Style checking is off (^OY to enable)"));
            return;
        }
        let cursor = self.cursor;
        let lines = self.buf.len_lines();
        let mut from_line = self.buf.line_of(cursor);
        // Walk from the cursor's line to the end, then wrap once, so the command
        // always finds the *next* issue rather than re-reporting this one.
        for step in 0..=lines {
            let line_no = (from_line + step) % lines.max(1);
            let text = self.buf.line_text(line_no).into_owned();
            let start = self.buf.line_start(line_no);
            for issue in self.style.issues_in_line(&text) {
                let pos = start + issue.start;
                if step == 0 && pos <= cursor {
                    continue;
                }
                self.long_jump(pos.min(self.buf.len_chars()));
                self.status_msg = Some(format!("Style: {}", issue.kind.label()));
                return;
            }
            if step == lines {
                break;
            }
            from_line = from_line.max(0);
        }
        self.status_msg = Some(String::from("No style issues found"));
    }

    /// The style issue under the cursor, for the status line (R8.2).
    pub fn style_issue_at_cursor(&self) -> Option<&'static str> {
        if !self.style_enabled {
            return None;
        }
        let line_no = self.buf.line_of(self.cursor);
        let offset = self.cursor - self.buf.line_start(line_no);
        let text = self.buf.line_text(line_no);
        self.style
            .issues_in_line(&text)
            .into_iter()
            .find(|issue| offset >= issue.start && offset < issue.end)
            .map(|issue| issue.kind.label())
    }

    /// Readability and overused words for the whole document, or for the marked
    /// block when there is one (R8.4, R8.5).
    fn style_report(&self) -> StyleReport {
        let text = match self.blocks.range() {
            Some((begin, end)) => self.buf.rope.slice(begin..end).to_string(),
            None => self.buf.rope.to_string(),
        };
        StyleReport {
            selection: self.blocks.range().is_some(),
            readability: crate::style::readability(&text),
            overused: crate::style::word_frequency(&text, 5),
        }
    }

    // --- editorial annotations (R9) -------------------------------------------

    /// Load a pane's annotations from its sidecar.
    ///
    /// Called wherever a pane adopts a document, so a window can never be left
    /// showing the comment set of the file that used to be in it.
    fn load_annotations(&mut self, index: usize) {
        let loaded = match (&self.meta_root, self.panes[index].buf.path.as_deref()) {
            (Some(root), Some(doc)) => meta::annotations(root, doc),
            _ => Vec::new(),
        };
        self.panes[index].annotations = loaded;
        self.panes[index].annotations_dirty = false;
    }

    /// Write back any pane whose annotations have moved, returning the first
    /// failure as a warning string.
    fn flush_annotations(&mut self) -> Option<String> {
        let root = self.meta_root.clone()?;
        let mut warning = None;
        for pane in self.panes.iter_mut() {
            if !pane.annotations_dirty {
                continue;
            }
            let Some(doc) = pane.buf.path.clone() else {
                continue;
            };
            match meta::set_annotations(&root, &doc, &pane.annotations) {
                Ok(()) => pane.annotations_dirty = false,
                Err(error) => {
                    warning.get_or_insert(format!("Annotations not saved: {error}"));
                }
            }
        }
        warning
    }

    /// The text of the comment the cursor is inside, for the status line (R9.2).
    pub fn annotation_under_cursor(&self) -> Option<&str> {
        self.annotation_at_cursor()
            .and_then(|i| self.annotations.get(i))
            .map(|annotation| annotation.text.as_str())
    }

    /// The annotation the cursor is inside, if any.
    fn annotation_at_cursor(&self) -> Option<usize> {
        let cursor = self.cursor;
        self.annotations
            .iter()
            .position(|annotation| annotation.covers(cursor))
    }

    /// ^PC: comment on the marked block, or on the cursor position (R9.1).
    ///
    /// With the cursor already inside a comment, this edits that one instead —
    /// the same chord reads as "the comment here".
    fn prompt_annotation(&mut self) {
        if self.meta_target().is_none() {
            return;
        }
        let existing = self
            .annotation_at_cursor()
            .and_then(|i| self.annotations.get(i))
            .map(|annotation| annotation.text.clone());
        let label = match (&existing, self.blocks.range()) {
            (Some(_), _) => String::from("Comment (empty to delete)"),
            (None, Some(_)) => String::from("Comment on the marked block"),
            (None, None) => String::from("Comment here"),
        };
        self.mode = Mode::Input {
            label,
            value: existing.unwrap_or_default(),
            action: InputAction::AnnotationText,
        };
    }

    fn set_annotation(&mut self, text: &str) {
        let Some((root, doc)) = self.meta_target() else {
            return;
        };
        // R9.1: a body is up to 2000 characters. Trim surrounding whitespace so
        // an all-spaces answer reads as empty (and deletes/discards below), and
        // cap the length by chars — not bytes — so the limit is the same for
        // any script the writer types.
        let trimmed = text.trim();
        let over_limit = trimmed.chars().count() > ANNOTATION_MAX_CHARS;
        let text: String = trimmed.chars().take(ANNOTATION_MAX_CHARS).collect();
        let existing = self.annotation_at_cursor();

        let message = match (existing, text.is_empty()) {
            // Emptying an existing comment deletes it — the one deliberate way
            // to remove one, since nothing else ever discards a comment (R9.6).
            (Some(i), true) => {
                self.annotations.remove(i);
                String::from("Comment deleted")
            }
            (Some(i), false) => {
                self.annotations[i].text = text;
                self.annotations[i].orphaned = false;
                String::from(if over_limit {
                    "Comment updated (trimmed to 2000 chars)"
                } else {
                    "Comment updated"
                })
            }
            (None, true) => String::from("Comment discarded"),
            (None, false) => {
                // A marked block anchors the comment to that span; otherwise it
                // attaches to the cursor position.
                let (anchor, len) = match self.blocks.range() {
                    Some((begin, end)) => (begin, end - begin),
                    None => (self.cursor, 0),
                };
                self.annotations
                    .push(meta::Annotation::new(anchor, len, text));
                self.annotations
                    .sort_by_key(|annotation| (annotation.anchor, annotation.len));
                String::from(if over_limit {
                    "Comment added (trimmed to 2000 chars)"
                } else {
                    "Comment added"
                })
            }
        };
        let active = self.active;
        self.panes[active].annotations_dirty = true;
        let _ = (root, doc); // the write goes through flush_annotations
        self.status_msg = Some(match self.flush_annotations() {
            Some(warning) => format!("{message}, but {warning}"),
            None => message,
        });
    }

    /// ^PG / ^PU: jump to the next or previous comment (R9.4).
    ///
    /// Orphaned comments are skipped here — they have no text to jump to — and
    /// are reachable from the list instead.
    fn jump_annotation(&mut self, forward: bool) {
        if self.annotations.is_empty() {
            self.status_msg = Some(String::from(
                "No comments in this document (^PC to add one)",
            ));
            return;
        }
        let cursor = self.cursor;
        let mut live: Vec<usize> = self
            .annotations
            .iter()
            .filter(|annotation| !annotation.orphaned)
            .map(|annotation| annotation.anchor)
            .collect();
        live.sort_unstable();
        let target = if forward {
            live.iter().find(|&&anchor| anchor > cursor).copied()
        } else {
            live.iter().rev().find(|&&anchor| anchor < cursor).copied()
        };
        match target {
            Some(anchor) => self.long_jump(anchor.min(self.buf.len_chars())),
            None if live.is_empty() => {
                self.status_msg =
                    Some(String::from("Every comment here is orphaned (^PL to list)"));
            }
            None => {
                self.status_msg = Some(String::from(if forward {
                    "No further comments"
                } else {
                    "No earlier comments"
                }));
            }
        }
    }

    /// ^PL: every comment in the document, orphans included (R9.4).
    fn open_annotation_list(&mut self) {
        if matches!(self.mode, Mode::Annotations { .. }) {
            self.mode = Mode::Normal;
            return;
        }
        if self.annotations.is_empty() {
            self.status_msg = Some(String::from(
                "No comments in this document (^PC to add one)",
            ));
            return;
        }
        let mut entries = self.annotations.clone();
        entries.sort_by_key(|annotation| (annotation.orphaned, annotation.anchor));
        self.mode = Mode::Annotations {
            entries,
            selected: 0,
        };
    }

    fn handle_annotations_key(&mut self, key: KeyEvent) {
        let (count, selected) = match &self.mode {
            Mode::Annotations { entries, selected } => (entries.len(), *selected),
            _ => return,
        };
        let last = count.saturating_sub(1);
        match list_nav_key(&key) {
            ListNav::Dismiss => {
                self.mode = Mode::Normal;
                return;
            }
            ListNav::Up => {
                if let Mode::Annotations { selected, .. } = &mut self.mode {
                    *selected = selected.saturating_sub(1);
                }
                return;
            }
            ListNav::Down => {
                if let Mode::Annotations { selected, .. } = &mut self.mode {
                    *selected = (*selected + 1).min(last);
                }
                return;
            }
            ListNav::Other => {}
        }
        if key.code == KeyCode::Enter {
            let anchor = match &self.mode {
                Mode::Annotations { entries, .. } => entries.get(selected).map(|a| a.anchor),
                _ => None,
            };
            if let Some(anchor) = anchor {
                self.mode = Mode::Normal;
                self.long_jump(anchor.min(self.buf.len_chars()));
            }
        }
    }

    // --- notes & research sidecar (R5) ---------------------------------------

    /// The sidecar root and the active document's path, or `None` with a status
    /// message saying why this document can't carry metadata.
    fn meta_target(&mut self) -> Option<(PathBuf, PathBuf)> {
        let Some(doc) = self.buf.path.clone() else {
            self.status_msg = Some(String::from(
                "Save the document first — notes are keyed to its path",
            ));
            return None;
        };
        let Some(root) = self.meta_root.clone() else {
            self.status_msg = Some(String::from("No metadata directory available for notes"));
            return None;
        };
        Some((root, doc))
    }

    /// ^PY: show or hide the binder's synopsis secondary lines (R5.3).
    fn toggle_synopsis(&mut self) {
        self.show_synopsis = !self.show_synopsis;
        self.status_msg = Some(String::from(if self.show_synopsis {
            "Binder synopses shown"
        } else {
            "Binder synopses hidden"
        }));
        // Rebuild the panel so the change is visible immediately if it's open.
        if let Mode::Binder { selected, .. } = self.mode {
            self.refresh_binder_with_selection(selected);
        }
    }

    /// ^PI: edit this document's synopsis, pre-filled with the current one so it
    /// reads as editing rather than retyping (R5.1).
    fn prompt_synopsis(&mut self) {
        let Some((root, doc)) = self.meta_target() else {
            return;
        };
        self.mode = Mode::Input {
            // Pre-filled, so Enter accepts what's shown; emptying the line and
            // pressing Enter is how a synopsis is cleared.
            label: String::from("Synopsis"),
            value: meta::synopsis(&root, &doc),
            action: InputAction::Synopsis,
        };
    }

    fn set_synopsis(&mut self, text: &str) {
        let Some((root, doc)) = self.meta_target() else {
            return;
        };
        // Committed straight to disk rather than waiting for an idle tick: it's
        // one short line, and a synopsis the writer typed should survive a crash
        // in the next second as surely as one they typed an hour ago (R5.6).
        self.status_msg = Some(match meta::set_synopsis(&root, &doc, text) {
            Ok(()) if text.trim().is_empty() => String::from("Synopsis cleared"),
            Ok(()) => String::from("Synopsis saved"),
            Err(error) => format!("Synopsis not saved: {error}"),
        });
        // The binder's secondary line is built from these, so refresh it if open.
        if let Mode::Binder { selected, .. } = self.mode {
            self.refresh_binder_with_selection(selected);
        }
    }

    /// ^PT: open this document's freeform notes beside it (R5.1, R5.4).
    ///
    /// The notes are an ordinary Markdown document, so they arrive with undo,
    /// search, spellcheck, autosave, and the crash journal already working.
    fn open_notes(&mut self) {
        let Some((root, doc)) = self.meta_target() else {
            return;
        };
        // The directory has to exist before the first save of a brand-new notes
        // file, and opening is the moment the writer commits to having notes.
        if let Err(error) = std::fs::create_dir_all(&root) {
            self.status_msg = Some(format!("Cannot create notes directory: {error}"));
            return;
        }
        let notes = meta::notes_path(&root, &doc);
        self.open_beside(notes);
    }

    /// Open `path` in the other window, splitting if there is only one.
    ///
    /// Never discards unsaved work: if the window being reused is dirty, the
    /// command refuses and says so rather than replacing the buffer (C3).
    fn open_beside(&mut self, path: PathBuf) {
        match Pane::open(Some(path)) {
            Ok(pane) => {
                let journal = Journal::new(pane.buf.path.as_deref());
                let name = pane.buf.file_name();
                if self.panes.len() < 2 {
                    self.panes.push(pane);
                    self.recovery_journals.push(journal);
                    self.active = self.panes.len() - 1;
                } else {
                    let other = 1 - self.active;
                    if self.panes[other].buf.dirty {
                        self.status_msg = Some(String::from(
                            "The other window has unsaved changes (^KS to save it first)",
                        ));
                        return;
                    }
                    self.panes[other].save_session();
                    self.panes[other] = pane;
                    self.recovery_journals[other] = journal;
                    self.active = other;
                }
                let active = self.active;
                self.doc_stats = DocStats::from_rope(&self.panes[active].buf.rope);
                self.load_annotations(active);
                self.mode = Mode::Normal;
                self.status_msg = Some(format!("Opened {name} in the other window"));
            }
            Err(error) => self.status_msg = Some(format!("Cannot open: {error}")),
        }
    }

    /// ^PM: mark the selected binder document as a note, or back (R5.2).
    fn toggle_doc_role(&mut self) {
        let Mode::Binder { selected, .. } = self.mode else {
            self.status_msg = Some(String::from("Open the binder first (^PB)"));
            return;
        };
        let Some(project) = self.project.as_mut() else {
            self.status_msg = Some(String::from("No project loaded (^PP to open)"));
            return;
        };
        let Some((role, title)) = project
            .toggle_role(selected)
            .zip(project.manifest.docs.get(selected).map(|d| d.title.clone()))
        else {
            return;
        };
        let saved = project.save();
        self.status_msg = Some(match saved {
            Ok(()) => match role {
                DocRole::Note => format!("\"{title}\" is a note — excluded from compile"),
                DocRole::Manuscript => format!("\"{title}\" is part of the manuscript again"),
            },
            Err(error) => format!("Role changed but the manifest was not saved: {error}"),
        });
        self.refresh_binder_with_selection(selected);
    }

    /// ^PV: open the selected binder document beside the manuscript (R5.4).
    fn binder_open_split(&mut self) {
        let Mode::Binder {
            ref entries,
            selected,
        } = self.mode
        else {
            self.status_msg = Some(String::from("Open the binder first (^PB)"));
            return;
        };
        let Some(entry) = entries.get(selected) else {
            return;
        };
        if !entry.exists {
            self.status_msg = Some(format!("File missing: {}", entry.title));
            return;
        }
        let idx = entry.idx;
        let Some(path) = self
            .project
            .as_ref()
            .and_then(|project| project.manifest.docs.get(idx))
            .map(|doc| doc.path.clone())
        else {
            return;
        };
        self.open_beside(path);
    }

    // --- snapshots & revisions (R4) ------------------------------------------

    /// The snapshot store for the active document, or `None` with a status
    /// message explaining why this document can't have snapshots.
    fn active_snapshot_store(&mut self) -> Option<SnapshotStore> {
        let Some(path) = self.buf.path.clone() else {
            self.status_msg = Some(String::from(
                "Save the document first — snapshots are keyed to its path",
            ));
            return None;
        };
        let Some(root) = self.snapshot_root.clone() else {
            self.status_msg = Some(String::from(
                "No metadata directory available for snapshots",
            ));
            return None;
        };
        Some(SnapshotStore::for_file_in(&root, &path))
    }

    /// ^KN: snapshot the current text, with an optional label (R4.1).
    fn take_snapshot(&mut self, label: Option<&str>) {
        let Some(mut store) = self.active_snapshot_store() else {
            return;
        };
        let active = self.active;
        // The rope is only borrowed: a failed snapshot leaves the working
        // buffer exactly as it was, and says so (R4.6).
        match store.capture(&self.panes[active].buf.rope, label) {
            Ok(entry) => {
                let named = match entry.label.as_deref() {
                    Some(label) => format!(" \"{label}\""),
                    None => String::new(),
                };
                self.status_msg = Some(format!(
                    "Snapshot{named} saved — {} words (^KO to list)",
                    entry.words
                ));
            }
            Err(error) => self.status_msg = Some(format!("Snapshot failed: {error}")),
        }
    }

    /// ^KO: this document's snapshots, newest first (R4.3).
    fn open_revisions(&mut self) {
        if matches!(self.mode, Mode::Revisions { .. }) {
            self.mode = Mode::Normal;
            return;
        }
        let Some(store) = self.active_snapshot_store() else {
            return;
        };
        // The store keeps them oldest-first; a writer looking for "the version
        // before lunch" starts from the newest.
        let entries: Vec<SnapshotEntry> = store.entries().iter().rev().cloned().collect();
        if entries.is_empty() {
            self.status_msg = Some(String::from(
                "No snapshots of this document yet (^KN takes one)",
            ));
            return;
        }
        self.mode = Mode::Revisions {
            entries,
            selected: 0,
            compare: None,
        };
    }

    fn handle_revisions_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let (count, selected) = match &self.mode {
            Mode::Revisions {
                entries, selected, ..
            } => (entries.len(), *selected),
            _ => return,
        };
        let last = count.saturating_sub(1);
        match list_nav_key(&key) {
            ListNav::Dismiss => {
                self.mode = Mode::Normal;
                return;
            }
            ListNav::Up => {
                if let Mode::Revisions { selected, .. } = &mut self.mode {
                    *selected = selected.saturating_sub(1);
                }
                return;
            }
            ListNav::Down => {
                if let Mode::Revisions { selected, .. } = &mut self.mode {
                    *selected = (*selected + 1).min(last);
                }
                return;
            }
            ListNav::Other => {}
        }
        let new_selected = match key.code {
            KeyCode::Enter => {
                self.revisions_diff(selected);
                return;
            }
            KeyCode::Char('r') if ctrl => {
                self.restore_selected_revision(selected);
                return;
            }
            KeyCode::Char(' ') => {
                self.revisions_mark(selected);
                return;
            }
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return,
        };
        if let Mode::Revisions { selected, .. } = &mut self.mode {
            *selected = new_selected.min(last);
        }
    }

    /// Space: mark (or unmark) a version to compare against instead of the
    /// current draft (R4.4).
    fn revisions_mark(&mut self, selected: usize) {
        let Mode::Revisions {
            entries,
            compare,
            selected: cursor,
        } = &mut self.mode
        else {
            return;
        };
        *compare = if *compare == Some(selected) {
            None
        } else {
            Some(selected)
        };
        let marked = compare.is_some();
        // Marking is a two-step gesture; move on so Enter lands on the *other*
        // version rather than diffing the marked one against itself.
        if marked && *cursor + 1 < entries.len() {
            *cursor += 1;
        }
        self.status_msg = Some(String::from(if marked {
            "Marked for comparison — Enter on another version to diff them"
        } else {
            "Comparison mark cleared"
        }));
    }

    /// Enter: diff the selected version against the marked one, or against the
    /// current draft when nothing is marked (R4.4).
    fn revisions_diff(&mut self, selected: usize) {
        let (entries, compare) = match &self.mode {
            Mode::Revisions {
                entries, compare, ..
            } => (entries.clone(), *compare),
            _ => return,
        };
        let Some(chosen) = entries.get(selected).cloned() else {
            return;
        };
        let Some(store) = self.active_snapshot_store() else {
            return;
        };

        // Whichever version was marked, the diff reads chronologically:
        // older on the left, newer on the right.
        let marked = compare
            .and_then(|i| entries.get(i).cloned())
            .filter(|marked| marked.file != chosen.file);
        let (older, newer) = match marked {
            Some(marked) => {
                if (marked.timestamp, &marked.file) <= (chosen.timestamp, &chosen.file) {
                    (marked, Some(chosen))
                } else {
                    (chosen, Some(marked))
                }
            }
            None => (chosen, None),
        };

        let old_text = match store.read_text(&older) {
            Ok(text) => text,
            Err(error) => {
                self.status_msg = Some(format!("Cannot read snapshot: {error}"));
                return;
            }
        };
        let (new_text, new_title) = match &newer {
            Some(entry) => match store.read_text(entry) {
                Ok(text) => (text, revision_title(entry)),
                Err(error) => {
                    self.status_msg = Some(format!("Cannot read snapshot: {error}"));
                    return;
                }
            },
            None => (
                self.panes[self.active].buf.rope.to_string(),
                String::from("current draft"),
            ),
        };

        let lines = diff::lines(&old_text, &new_text);
        if lines.is_empty() {
            // Stay in the list; there is nothing to show and closing it would
            // read as a failure.
            self.status_msg = Some(format!(
                "{} and {new_title} are identical",
                revision_title(&older)
            ));
            return;
        }
        let summary = diff::summarize(&lines);
        self.mode = Mode::Diff {
            title: format!(
                "{} → {new_title}  +{} −{}",
                revision_title(&older),
                summary.added,
                summary.removed
            ),
            lines,
            scroll: 0,
            restore: Some(older),
        };
    }

    fn handle_diff_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.view_rows.max(1);
        let (count, scroll, restore) = match &self.mode {
            Mode::Diff {
                lines,
                scroll,
                restore,
                ..
            } => (lines.len(), *scroll, restore.clone()),
            _ => return,
        };
        let last = count.saturating_sub(1);
        let new_scroll = match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Char('r') if ctrl => {
                match restore {
                    Some(entry) => self.restore_snapshot(&entry),
                    None => self.status_msg = Some(String::from("Nothing to restore here")),
                }
                return;
            }
            KeyCode::Up => scroll.saturating_sub(1),
            KeyCode::Down => scroll + 1,
            KeyCode::Char('e') if ctrl => scroll.saturating_sub(1),
            KeyCode::Char('x') if ctrl => scroll + 1,
            KeyCode::PageUp => scroll.saturating_sub(page),
            KeyCode::PageDown => scroll + page,
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return,
        };
        if let Mode::Diff { scroll, .. } = &mut self.mode {
            *scroll = new_scroll.min(last);
        }
    }

    fn restore_selected_revision(&mut self, selected: usize) {
        let entry = match &self.mode {
            Mode::Revisions { entries, .. } => entries.get(selected).cloned(),
            _ => None,
        };
        if let Some(entry) = entry {
            self.restore_snapshot(&entry);
        }
    }

    /// Replace the buffer with a snapshot's text as one undoable edit (R4.5).
    ///
    /// Restoration goes through the ordinary edit path rather than swapping the
    /// rope, so ^U takes the whole restore back in a single step and the version
    /// it replaced is never lost (ADR-003). A read failure leaves the buffer
    /// untouched.
    fn restore_snapshot(&mut self, entry: &SnapshotEntry) {
        let Some(store) = self.active_snapshot_store() else {
            return;
        };
        let text = match store.read_text(entry) {
            Ok(text) => text,
            Err(error) => {
                self.status_msg = Some(format!("Restore failed: {error}"));
                return;
            }
        };

        // Bracketed by group breaks so the restore neither absorbs the edits
        // before it nor gets absorbed by the ones after: exactly one undo step.
        self.history.break_group();
        let old_len = self.buf.len_chars();
        let cursor_after = self.cursor.min(text.chars().count());
        self.apply_edit(0, old_len, &text, EditKind::Other, cursor_after);
        self.history.break_group();

        let active = self.active;
        self.doc_stats = DocStats::from_rope(&self.panes[active].buf.rope);
        self.mode = Mode::Normal;
        self.status_msg = Some(format!(
            "Restored {} — ^U to undo, ^KS to keep",
            revision_title(entry)
        ));
    }
}

/// How a version is named in the revisions list, the diff title, and status
/// messages: its timestamp, plus the writer's label when it has one.
fn revision_title(entry: &SnapshotEntry) -> String {
    let label = entry.display_label();
    if label.is_empty() {
        entry.display_time()
    } else {
        format!("{} {label}", entry.display_time())
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

/// Take an automatic snapshot of a document and apply retention (R4.2).
///
/// A free function so the autosave sweep can call it while holding a mutable
/// borrow of the pane list. Quietly does nothing when automatic snapshots are
/// off (`snapshot_keep = 0`), when the buffer has no path to key snapshots by,
/// or when the platform offers no metadata directory — none of those are
/// failures the writer needs to hear about mid-sentence.
fn auto_snapshot(
    root: Option<&Path>,
    source: Option<&Path>,
    rope: &Rope,
    keep: usize,
) -> io::Result<()> {
    if keep == 0 {
        return Ok(());
    }
    let (Some(root), Some(source)) = (root, source) else {
        return Ok(());
    };
    let mut store = SnapshotStore::for_file_in(root, source);
    store.capture_auto(rope)?;
    store.prune_auto(keep).map(|_| ())
}

/// Prompts where an empty answer means something other than "cancel".
fn allows_empty_answer(action: InputAction) -> bool {
    matches!(
        action,
        InputAction::SnapshotLabel | InputAction::Synopsis | InputAction::AnnotationText
    )
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
#[path = "app_tests.rs"]
mod tests;
