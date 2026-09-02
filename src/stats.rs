//! Writing statistics: incremental word/char counting, session goals, and
//! per-day words-written history.
//!
//! The counting is **prose-aware**: `..` note lines and Markdown markers are
//! excluded so the count matches what the exporter produces (R2.6, design §4.2).
//! An authoritative count is computed on load; subsequent edits update it over
//! the changed line-range only. A debounced full recount on idle corrects drift.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use ropey::Rope;
use serde::{Deserialize, Serialize};

use crate::normalize;

/// Count prose words in a single line (excluding notes and Markdown markers).
pub fn prose_words_in_line(line: &str) -> usize {
    if normalize::is_note_line(line) {
        return 0;
    }
    let stripped = normalize::strip_markers(line);
    word_count_str(&stripped)
}

/// Count words in a plain string (runs of alphanumeric characters).
fn word_count_str(s: &str) -> usize {
    let mut count = 0usize;
    let mut in_word = false;
    for c in s.chars() {
        let w = c.is_alphanumeric();
        if w && !in_word {
            count += 1;
        }
        in_word = w;
    }
    count
}

/// Count prose characters in a single line (excluding notes and markers).
fn prose_chars_in_line(line: &str) -> usize {
    if normalize::is_note_line(line) {
        return 0;
    }
    let stripped = normalize::strip_markers(line);
    stripped.chars().filter(|c| !c.is_control()).count()
}

/// Cached document statistics maintained incrementally.
#[derive(Debug, Clone)]
pub struct DocStats {
    /// Authoritative prose word count (excludes notes and markers).
    pub words: usize,
    /// Authoritative prose character count.
    pub chars: usize,
    /// Per-line word counts for incremental delta computation.
    line_words: Vec<usize>,
    /// Per-line char counts.
    line_chars: Vec<usize>,
    /// Monotonic generation counter — bumped on every invalidation.
    generation: u64,
    /// The generation at which the last full recount ran.
    last_full_gen: u64,
}

impl DocStats {
    /// Full-document count from a rope. Called on load and during idle recount.
    pub fn from_rope(rope: &Rope) -> Self {
        let n = rope.len_lines();
        let mut words = 0usize;
        let mut chars = 0usize;
        let mut line_words = Vec::with_capacity(n);
        let mut line_chars = Vec::with_capacity(n);
        for i in 0..n {
            let text = rope.line(i).to_string();
            let lw = prose_words_in_line(&text);
            let lc = prose_chars_in_line(&text);
            words += lw;
            chars += lc;
            line_words.push(lw);
            line_chars.push(lc);
        }
        DocStats {
            words,
            chars,
            line_words,
            line_chars,
            generation: 0,
            last_full_gen: 0,
        }
    }

    /// Notify that lines in `[from_line, to_line)` have changed. Recomputes
    /// just those lines and adjusts the totals.
    pub fn invalidate_lines(&mut self, rope: &Rope, from_line: usize, to_line: usize) {
        self.generation += 1;
        let n = rope.len_lines();

        // When an edit adds or removes lines, every subsequent line can shift,
        // so the old per-line cache no longer maps to the rope. Rebuild it
        // authoritatively rather than applying deltas against stale indexes.
        if n != self.line_words.len() {
            let generation = self.generation;
            let last_full_gen = self.last_full_gen;
            *self = Self::from_rope(rope);
            self.generation = generation;
            self.last_full_gen = last_full_gen;
            return;
        }

        let end = to_line.min(n);
        for i in from_line..end {
            let text = rope.line(i).to_string();
            let new_w = prose_words_in_line(&text);
            let new_c = prose_chars_in_line(&text);
            let old_w = self.line_words.get(i).copied().unwrap_or(0);
            let old_c = self.line_chars.get(i).copied().unwrap_or(0);
            self.words = self.words.wrapping_add(new_w).wrapping_sub(old_w);
            self.chars = self.chars.wrapping_add(new_c).wrapping_sub(old_c);
            self.line_words[i] = new_w;
            self.line_chars[i] = new_c;
        }
    }

    /// Full recount to correct any drift. Returns whether it actually changed.
    pub fn full_recount(&mut self, rope: &Rope) -> bool {
        if self.generation == self.last_full_gen {
            return false;
        }
        let fresh = Self::from_rope(rope);
        let changed = fresh.words != self.words || fresh.chars != self.chars;
        *self = DocStats {
            last_full_gen: fresh.generation,
            ..fresh
        };
        changed
    }

    /// Whether a full recount is due (there have been edits since the last one).
    pub fn needs_recount(&self) -> bool {
        self.generation != self.last_full_gen
    }
}

/// Count words in a rope slice (for selection/block counts on demand).
pub fn count_slice(rope: &Rope, from: usize, to: usize) -> (usize, usize) {
    let from_line = rope.char_to_line(from);
    let to_line = rope.char_to_line(to.min(rope.len_chars().saturating_sub(1)));
    let mut words = 0usize;
    let mut chars = 0usize;
    for i in from_line..=to_line {
        let line_start = rope.line_to_char(i);
        let line_text: String = rope.line(i).to_string();
        // Clip to the selection range within this line.
        let sel_start = from.saturating_sub(line_start);
        let sel_end = (to - line_start).min(line_text.chars().count());
        if sel_start >= sel_end {
            continue;
        }
        let slice: String = line_text
            .chars()
            .skip(sel_start)
            .take(sel_end - sel_start)
            .collect();
        if normalize::is_note_line(&line_text) {
            continue;
        }
        words += word_count_str(&slice);
        chars += slice.chars().filter(|c| !c.is_control()).count();
    }
    (words, chars)
}

// --- Session goals -----------------------------------------------------------

/// What kind of session target the writer set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalKind {
    Words,
    Minutes,
}

/// Session-goal bounds (R2.3). A word goal is 1..=1,000,000 words; a time goal
/// is 1..=480 minutes. Enforced at the point the goal is set so a fat-fingered
/// entry cannot create an unreachable or nonsensical target.
pub const MAX_WORD_GOAL: usize = 1_000_000;
pub const MAX_MINUTE_GOAL: usize = 480;

/// Now, as whole seconds since the Unix epoch. The goal's start time is stored
/// on this clock (not a monotonic `Instant`) so that time-goal progress resumes
/// correctly after the process restarts (R2.5).
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A running session goal. `start_unix` anchors time-goal progress to the wall
/// clock so it survives a restart; `started` is kept for any in-process elapsed
/// use but is not authoritative across restarts.
#[derive(Debug, Clone)]
pub struct SessionGoal {
    pub kind: GoalKind,
    pub target: usize,
    pub start_words: usize,
    /// Wall-clock start (Unix seconds) — the persisted, restart-safe anchor
    /// for time-goal progress (survives a process restart, R2.5).
    pub start_unix: u64,
    pub reached: bool,
}

/// The persisted shape of a session goal, stored beside the daily history in
/// `stats/` so reopening the same document resumes progress (R2.5). Only the
/// wall-clock fields survive; the monotonic `Instant` is rebuilt on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GoalFile {
    kind: GoalKind,
    target: usize,
    baseline: usize,
    start_unix: u64,
    #[serde(default)]
    reached: bool,
}

impl SessionGoal {
    /// Validate a word goal against its bounds (R2.3), returning the target or
    /// an explanation of why it was rejected.
    pub fn validate_words(target: usize) -> Result<usize, String> {
        match target {
            0 => Err(String::from("Enter a positive number of words")),
            n if n > MAX_WORD_GOAL => Err(format!("Word goal must be at most {MAX_WORD_GOAL}")),
            n => Ok(n),
        }
    }

    /// Validate a minute goal against its bounds (R2.3).
    pub fn validate_minutes(target: usize) -> Result<usize, String> {
        match target {
            0 => Err(String::from("Enter a positive number of minutes")),
            n if n > MAX_MINUTE_GOAL => Err(format!(
                "Time goal must be at most {MAX_MINUTE_GOAL} minutes"
            )),
            n => Ok(n),
        }
    }

    pub fn new(kind: GoalKind, target: usize, current_words: usize) -> Self {
        SessionGoal {
            kind,
            target,
            start_words: current_words,
            start_unix: now_unix(),
            reached: false,
        }
    }

    /// Progress as (current, target). Word goals report net words since the
    /// baseline; time goals report elapsed whole minutes on the wall clock.
    pub fn progress(&self, current_words: usize) -> (usize, usize) {
        match self.kind {
            GoalKind::Words => {
                let written = current_words.saturating_sub(self.start_words);
                (written, self.target)
            }
            GoalKind::Minutes => {
                let elapsed = now_unix().saturating_sub(self.start_unix) / 60;
                (elapsed as usize, self.target)
            }
        }
    }

    /// Whether the goal is now met.
    pub fn is_met(&self, current_words: usize) -> bool {
        let (current, target) = self.progress(current_words);
        current >= target
    }

    /// Persist the goal beside the given document's history so it resumes on
    /// reopen (R2.5). A missing stats root or path is a silent no-op — a goal
    /// that cannot be saved is not worth refusing an edit over.
    pub fn save_for(&self, file_path: Option<&Path>) {
        self.save_in(crate::paths::stats(), file_path);
    }

    fn save_in(&self, root: Option<PathBuf>, file_path: Option<&Path>) {
        let Some(path) = goal_path(root, file_path) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = GoalFile {
            kind: self.kind,
            target: self.target,
            baseline: self.start_words,
            start_unix: self.start_unix,
            reached: self.reached,
        };
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            let _ = crate::paths::write_atomic(&path, json.as_bytes());
        }
    }

    /// Load a persisted goal for the given document, if one is active (R2.5).
    pub fn load_for(file_path: Option<&Path>) -> Option<Self> {
        Self::load_in(crate::paths::stats(), file_path)
    }

    fn load_in(root: Option<PathBuf>, file_path: Option<&Path>) -> Option<Self> {
        let path = goal_path(root, file_path)?;
        let data = std::fs::read_to_string(path).ok()?;
        let file: GoalFile = serde_json::from_str(&data).ok()?;
        Some(SessionGoal {
            kind: file.kind,
            target: file.target,
            start_words: file.baseline,
            start_unix: file.start_unix,
            reached: file.reached,
        })
    }
}

/// The on-disk location of a document's persisted goal: `stats/<key>-goal.json`.
fn goal_path(root: Option<PathBuf>, file_path: Option<&Path>) -> Option<PathBuf> {
    match (root, file_path) {
        (Some(dir), Some(fp)) => {
            let key = crate::paths::path_key(fp);
            Some(dir.join(format!("{key}-goal.json")))
        }
        _ => None,
    }
}

// --- Daily history -----------------------------------------------------------

/// One finished sprint, kept alongside the daily totals (R3.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SprintRecord {
    /// UTC date the sprint finished on (YYYY-MM-DD).
    pub date: String,
    /// Net prose words written during the sprint.
    pub words: i64,
    pub seconds: u64,
    /// Whether the sprint met its own terms.
    pub met_target: bool,
}

/// How many sprint records a document's history keeps. A writer's recent
/// sprints are useful; their thousandth-from-last is not, and the file stays
/// small enough to rewrite atomically on every save.
const MAX_SPRINTS: usize = 100;

/// On-disk shape of a document's history.
///
/// `deny_unknown_fields` is load-bearing: without it, the original bare
/// `{"2026-08-19": 812}` map would parse "successfully" as a record with no
/// days at all, and a writer's history would silently vanish on first save.
/// Failing to parse is what routes legacy files to the fallback below.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryFile {
    #[serde(default)]
    days: BTreeMap<String, i64>,
    #[serde(default)]
    sprints: Vec<SprintRecord>,
}

/// Per-day net-words record, keyed by ISO date string (YYYY-MM-DD), plus the
/// document's sprint log.
#[derive(Debug, Clone, Default)]
pub struct DailyHistory {
    pub entries: BTreeMap<String, i64>,
    pub sprints: Vec<SprintRecord>,
    path: Option<PathBuf>,
}

impl DailyHistory {
    /// Load history for a given file from the stats directory.
    pub fn load(file_path: Option<&Path>) -> Self {
        Self::load_in(crate::paths::stats(), file_path)
    }

    /// Load history from an explicit stats root, keyed by the document's path.
    /// Production passes [`crate::paths::stats`]; tests pass a temporary root.
    pub fn load_in(root: Option<PathBuf>, file_path: Option<&Path>) -> Self {
        let path = match (root, file_path) {
            (Some(dir), Some(fp)) => {
                let key = crate::paths::path_key(fp);
                Some(dir.join(format!("{key}.json")))
            }
            _ => None,
        };
        let file = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|data| parse_history(&data))
            .unwrap_or_default();
        DailyHistory {
            entries: file.days,
            sprints: file.sprints,
            path,
        }
    }

    /// Record a net-word delta for today.
    pub fn record_delta(&mut self, delta: i64) {
        if delta == 0 {
            return;
        }
        let today = today_key();
        let entry = self.entries.entry(today).or_insert(0);
        *entry += delta;
    }

    /// Record a finished sprint (R3.2), keeping the most recent
    /// [`MAX_SPRINTS`].
    pub fn record_sprint(&mut self, words: i64, seconds: u64, met_target: bool) {
        self.sprints.push(SprintRecord {
            date: today_key(),
            words,
            seconds,
            met_target,
        });
        let excess = self.sprints.len().saturating_sub(MAX_SPRINTS);
        self.sprints.drain(..excess);
    }

    /// The last N sprints, most recent first.
    pub fn recent_sprints(&self, n: usize) -> Vec<&SprintRecord> {
        self.sprints.iter().rev().take(n).collect()
    }

    /// Persist history to disk.
    pub fn save(&self) -> io::Result<()> {
        let Some(ref path) = self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = HistoryFile {
            days: self.entries.clone(),
            sprints: self.sprints.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(io::Error::other)?;
        crate::paths::write_atomic(path, json.as_bytes())
    }

    /// Total words for a given date key.
    pub fn words_on(&self, date: &str) -> i64 {
        self.entries.get(date).copied().unwrap_or(0)
    }

    /// Today's net words.
    pub fn today(&self) -> i64 {
        self.words_on(&today_key())
    }

    /// The last N days of entries (most recent first).
    pub fn recent(&self, n: usize) -> Vec<(&str, i64)> {
        self.entries
            .iter()
            .rev()
            .take(n)
            .map(|(k, &v)| (k.as_str(), v))
            .collect()
    }
}

/// Read a history file in either the current shape or the pre-sprint one (a
/// bare `{date: words}` map). An unreadable file yields an empty history rather
/// than an error: losing a statistic is not worth refusing to open a document.
fn parse_history(data: &str) -> HistoryFile {
    if let Ok(file) = serde_json::from_str::<HistoryFile>(data) {
        return file;
    }
    if let Ok(days) = serde_json::from_str::<BTreeMap<String, i64>>(data) {
        return HistoryFile {
            days,
            sprints: Vec::new(),
        };
    }
    HistoryFile::default()
}

fn today_key() -> String {
    // Use UTC to avoid timezone rollover surprises mid-session.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = now / 86400;
    // Convert days since epoch to YYYY-MM-DD.
    let (year, month, day) = days_to_date(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since the Unix epoch to a UTC `(year, month, day)`. Shared with the
/// snapshot store, which stamps filenames from the same calendar math.
pub fn days_to_date(days_since_epoch: u64) -> (u32, u32, u32) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days_since_epoch + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    #[test]
    fn prose_words_excludes_notes_and_markers() {
        assert_eq!(prose_words_in_line(".. this is a note"), 0);
        assert_eq!(prose_words_in_line("A **bold** and *fine* day."), 5);
        assert_eq!(prose_words_in_line("## Heading"), 1);
        assert_eq!(prose_words_in_line("plain line here"), 3);
    }

    #[test]
    fn full_count_matches_line_by_line() {
        let text = "# Chapter One\n.. note to self\nHello **world**.\nGoodbye.\n";
        let rope = Rope::from_str(text);
        let stats = DocStats::from_rope(&rope);
        // "Chapter One" (heading, 2) + "Hello world." (2) + "Goodbye." (1) = 5
        // note line excluded
        assert_eq!(stats.words, 5);
    }

    #[test]
    fn incremental_update_tracks_edit() {
        let text = "Hello world.\nSecond line.\n";
        let rope = Rope::from_str(text);
        let mut stats = DocStats::from_rope(&rope);
        let initial = stats.words;

        // Simulate editing line 0 to add a word.
        let mut rope2 = rope.clone();
        rope2.remove(0..12); // remove "Hello world."
        rope2.insert(0, "Hello bright world.");
        stats.invalidate_lines(&rope2, 0, 1);
        assert_eq!(stats.words, initial + 1);
    }

    #[test]
    fn full_recount_corrects_drift() {
        let text = "One two three.\n";
        let rope = Rope::from_str(text);
        let mut stats = DocStats::from_rope(&rope);
        // Artificially introduce drift.
        stats.words = 999;
        stats.generation += 1;
        assert!(stats.needs_recount());
        assert!(stats.full_recount(&rope));
        assert_eq!(stats.words, 3);
    }

    #[test]
    fn selection_count() {
        let rope = Rope::from_str("Hello world, this is a test.\n");
        let (words, _chars) = count_slice(&rope, 0, 11); // "Hello world"
        assert_eq!(words, 2);
    }

    #[test]
    fn session_goal_words() {
        let goal = SessionGoal::new(GoalKind::Words, 100, 50);
        assert!(!goal.is_met(100));
        assert!(goal.is_met(150));
        let (progress, target) = goal.progress(120);
        assert_eq!(progress, 70);
        assert_eq!(target, 100);
    }

    #[test]
    fn word_goal_bounds_are_enforced() {
        assert!(SessionGoal::validate_words(0).is_err());
        assert_eq!(SessionGoal::validate_words(1), Ok(1));
        assert_eq!(
            SessionGoal::validate_words(MAX_WORD_GOAL),
            Ok(MAX_WORD_GOAL)
        );
        assert!(SessionGoal::validate_words(MAX_WORD_GOAL + 1).is_err());
    }

    #[test]
    fn minute_goal_bounds_are_enforced() {
        assert!(SessionGoal::validate_minutes(0).is_err());
        assert_eq!(SessionGoal::validate_minutes(1), Ok(1));
        assert_eq!(
            SessionGoal::validate_minutes(MAX_MINUTE_GOAL),
            Ok(MAX_MINUTE_GOAL)
        );
        assert!(SessionGoal::validate_minutes(MAX_MINUTE_GOAL + 1).is_err());
    }

    #[test]
    fn minute_goal_progress_reads_the_wall_clock() {
        // A goal started 90 seconds ago should read one whole elapsed minute,
        // independently of any word count.
        let mut goal = SessionGoal::new(GoalKind::Minutes, 30, 0);
        goal.start_unix = goal.start_unix.saturating_sub(90);
        let (elapsed, target) = goal.progress(0);
        assert_eq!(elapsed, 1);
        assert_eq!(target, 30);
        assert!(!goal.is_met(0));
    }

    #[test]
    fn session_goal_round_trips_target_baseline_and_start() {
        let dir = std::env::temp_dir().join(format!("pstar-goal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let doc = dir.join("chapter.md");

        // A word goal set with the writer 120 words in.
        let mut goal = SessionGoal::new(GoalKind::Words, 500, 120);
        goal.reached = false;
        goal.save_in(Some(dir.clone()), Some(&doc));

        let reloaded =
            SessionGoal::load_in(Some(dir.clone()), Some(&doc)).expect("a saved goal reloads");
        assert_eq!(reloaded.kind, GoalKind::Words);
        assert_eq!(reloaded.target, 500);
        assert_eq!(reloaded.start_words, 120);
        assert_eq!(reloaded.start_unix, goal.start_unix);
        assert!(!reloaded.reached);
        // Net-words-since-baseline resumes from where it left off.
        assert_eq!(reloaded.progress(300), (180, 500));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_persisted_goal_loads_as_none() {
        let dir = std::env::temp_dir().join(format!("pstar-goal-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let doc = dir.join("chapter.md");
        assert!(SessionGoal::load_in(Some(dir), Some(&doc)).is_none());
    }

    #[test]
    fn daily_history_round_trip() {
        let mut h = DailyHistory::default();
        h.record_delta(50);
        h.record_delta(30);
        assert_eq!(h.today(), 80);
    }

    #[test]
    fn sprint_records_accumulate_newest_last_and_are_capped() {
        let mut h = DailyHistory::default();
        h.record_sprint(300, 900, true);
        h.record_sprint(120, 600, false);

        assert_eq!(h.sprints.len(), 2);
        assert_eq!(h.recent_sprints(1)[0].words, 120);
        assert_eq!(h.recent_sprints(5).len(), 2);
        assert!(h.sprints[0].met_target && !h.sprints[1].met_target);
        assert_eq!(h.sprints[0].date, today_key());

        for i in 0..MAX_SPRINTS {
            h.record_sprint(i as i64, 60, true);
        }
        assert_eq!(h.sprints.len(), MAX_SPRINTS, "oldest records retire");
        assert_eq!(
            h.sprints.last().unwrap().words,
            MAX_SPRINTS as i64 - 1,
            "the newest is kept"
        );
    }

    #[test]
    fn history_survives_a_save_and_reload_on_disk() {
        let dir = std::env::temp_dir().join(format!("pstar-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let doc = dir.join("chapter.md");

        let mut history = DailyHistory::load_in(Some(dir.clone()), Some(&doc));
        history.record_delta(812);
        history.record_sprint(520, 754, true);
        history.save().unwrap();

        let reloaded = DailyHistory::load_in(Some(dir.clone()), Some(&doc));
        assert_eq!(reloaded.today(), 812);
        assert_eq!(reloaded.sprints.len(), 1);
        assert_eq!(reloaded.recent_sprints(1)[0].words, 520);
        assert_eq!(reloaded.recent_sprints(1)[0].seconds, 754);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn history_round_trips_days_and_sprints_through_the_file_shape() {
        let mut h = DailyHistory::default();
        h.record_delta(812);
        h.record_sprint(520, 754, true);
        let file = HistoryFile {
            days: h.entries.clone(),
            sprints: h.sprints.clone(),
        };

        let json = serde_json::to_string(&file).unwrap();
        let parsed = parse_history(&json);
        assert_eq!(parsed.days, h.entries);
        assert_eq!(parsed.sprints, h.sprints);
    }

    #[test]
    fn a_pre_sprint_history_file_still_loads_its_days() {
        // The original format was a bare {date: words} map. Reading it as the
        // current shape must not silently produce an empty history.
        let parsed = parse_history("{\"2026-08-19\": 812, \"2026-08-18\": -40}");
        assert_eq!(parsed.days.get("2026-08-19"), Some(&812));
        assert_eq!(parsed.days.get("2026-08-18"), Some(&-40));
        assert!(parsed.sprints.is_empty());
    }

    #[test]
    fn an_unreadable_history_file_is_empty_not_fatal() {
        let parsed = parse_history("{ not json at all");
        assert!(parsed.days.is_empty());
        assert!(parsed.sprints.is_empty());
    }

    #[test]
    fn date_conversion_sanity() {
        // 2024-01-01 is 19723 days since epoch
        let (y, m, d) = days_to_date(19723);
        assert_eq!((y, m, d), (2024, 1, 1));
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    fn text_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just(' '),
                Just('\n'),
                Just('\t'),
                Just('#'),
                Just('*'),
                Just('_'),
                Just('.'),
                Just('a'),
                Just('Z'),
                Just('é'),
                Just('中'),
                Just('🙂'),
            ],
            0..=96,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    fn prose_line_strategy() -> impl Strategy<Value = (String, usize, usize)> {
        let word = prop::string::string_regex("[a-z]{1,8}").expect("valid word regex");
        prop::collection::vec(word, 1..=4).prop_flat_map(|words| {
            let content = words.join(" ");
            let prose_words = words.len();
            let prose_chars = content.chars().count();

            prop_oneof![
                Just((content.clone(), prose_words, prose_chars)),
                Just((format!("## {content}"), prose_words, prose_chars)),
                Just((format!("**{content}**"), prose_words, prose_chars)),
                Just((format!("*{content}*"), prose_words, prose_chars)),
                Just((format!("`{content}`"), prose_words, prose_chars)),
                Just((format!(".. {content}"), 0, 0)),
                Just((format!("   .. {content}"), 0, 0)),
            ]
        })
    }

    fn date_strategy() -> impl Strategy<Value = String> {
        prop::string::string_regex(r"[0-9]{4}-[0-9]{2}-[0-9]{2}").expect("valid date regex")
    }

    // Feature: pro-writer-10-star, Property 5
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            ..ProptestConfig::default()
        })]

        #[test]
        fn incremental_word_count_matches_full_recount(
            initial in text_strategy(),
            edits in prop::collection::vec(
                (any::<usize>(), any::<usize>(), text_strategy()),
                1..=64,
            ),
        ) {
            let mut rope = Rope::from_str(&initial);
            let mut stats = DocStats::from_rope(&rope);

            for (position, delete_len, insert) in edits {
                let len = rope.len_chars();
                let at = if len == 0 { 0 } else { position % (len + 1) };
                let end = at.saturating_add(delete_len).min(len);
                let from_line = rope.char_to_line(at);

                rope.remove(at..end);
                rope.insert(at, &insert);

                let inserted_end = at + insert.chars().count();
                let to_line = rope
                    .char_to_line(inserted_end.min(rope.len_chars()))
                    .saturating_add(1);
                stats.invalidate_lines(&rope, from_line, to_line);

                let full = DocStats::from_rope(&rope);
                prop_assert_eq!(stats.words, full.words);
                prop_assert_eq!(stats.chars, full.chars);
            }
        }

        // Feature: pro-writer-10-star, Property 6
        #[test]
        fn prose_count_excludes_note_lines_and_markdown_markers(
            lines in prop::collection::vec(prose_line_strategy(), 1..=32),
        ) {
            let text = lines
                .iter()
                .map(|(line, _, _)| line.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let rope = Rope::from_str(&text);
            let stats = DocStats::from_rope(&rope);
            let expected_words: usize = lines.iter().map(|(_, words, _)| words).sum();
            let expected_chars: usize = lines.iter().map(|(_, _, chars)| chars).sum();

            prop_assert_eq!(stats.words, expected_words);
            prop_assert_eq!(stats.chars, expected_chars);

            for (line, words, chars) in &lines {
                prop_assert_eq!(prose_words_in_line(line), *words);
                prop_assert_eq!(prose_chars_in_line(line), *chars);
            }
        }

        // Feature: pro-writer-10-star, Property 7
        #[test]
        fn session_goal_progress_equals_net_words_since_baseline(
            baseline in 0usize..=100_000,
            inserted_words in 0usize..=10_000,
            deleted_words in 0usize..=100_000,
            elapsed_minutes in 0usize..MAX_MINUTE_GOAL,
        ) {
            // Keep the generated edit valid for a word-count API that accepts
            // the current count as usize, while still exercising deletions.
            let deleted_words = deleted_words.min(baseline);
            let current_words = baseline - deleted_words + inserted_words;
            let net_words = current_words.saturating_sub(baseline);

            // Exercise all meaningful word-goal boundaries: below, exactly
            // at, and above the net progress (with minimum target handling).
            let word_targets = [
                net_words.saturating_sub(1).max(1),
                net_words.max(1),
                net_words.saturating_add(1).min(MAX_WORD_GOAL),
            ];
            for target in word_targets {
                let goal = SessionGoal::new(GoalKind::Words, target, baseline);
                let (done, reported_target) = goal.progress(current_words);
                prop_assert_eq!(done, net_words);
                prop_assert_eq!(reported_target, target);
                prop_assert_eq!(goal.is_met(current_words), net_words >= target);
            }

            // Pin elapsed-minute boundaries without sleeping: the one-second
            // offset keeps the generated elapsed value stable during the test.
            let elapsed = elapsed_minutes as u64;
            let start_unix = now_unix().saturating_sub(elapsed * 60 + 1);
            let minute_targets = [
                elapsed_minutes.saturating_sub(1).max(1),
                elapsed_minutes.max(1),
                elapsed_minutes.saturating_add(1).min(MAX_MINUTE_GOAL),
            ];
            for target in minute_targets {
                let mut goal = SessionGoal::new(GoalKind::Minutes, target, current_words);
                goal.start_unix = start_unix;
                let (done, reported_target) = goal.progress(current_words);
                prop_assert_eq!(done, elapsed_minutes);
                prop_assert_eq!(reported_target, target);
                prop_assert_eq!(goal.is_met(current_words), elapsed_minutes >= target);
            }
        }

        // Feature: pro-writer-10-star, Property 8
        #[test]
        fn daily_history_round_trips_current_and_legacy_shapes(
            days in prop::collection::btree_map(
                date_strategy(),
                -1_000_000i64..=1_000_000,
                0..=32,
            ),
            sprints in prop::collection::vec(
                (-1_000_000i64..=1_000_000, 0u64..=86_400, any::<bool>()),
                0..=150,
            ),
            legacy_days in prop::collection::btree_map(
                date_strategy(),
                -1_000_000i64..=1_000_000,
                0..=32,
            ),
            token in any::<u64>(),
        ) {
            let root = std::env::temp_dir().join(format!(
                "pstar-stats-property8-{}-{token}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let doc = root.join("chapter.md");

            let mut history = DailyHistory::load_in(Some(root.clone()), Some(&doc));
            history.entries = days.clone();
            for (words, seconds, met_target) in sprints {
                history.record_sprint(words, seconds, met_target);
            }
            let expected_sprints = history.sprints.clone();
            history.save().expect("current history shape saves");

            let history_path = root.join(format!("{}.json", crate::paths::path_key(&doc)));
            let current_json = std::fs::read_to_string(&history_path)
                .expect("current history shape is written");
            let current_file: HistoryFile =
                serde_json::from_str(&current_json).expect("current history shape parses");
            prop_assert_eq!(&current_file.days, &days);
            prop_assert_eq!(&current_file.sprints, &expected_sprints);

            let reloaded = DailyHistory::load_in(Some(root.clone()), Some(&doc));
            prop_assert_eq!(&reloaded.entries, &days);
            prop_assert_eq!(&reloaded.sprints, &expected_sprints);

            // The pre-sprint format was a bare date-to-delta JSON object.
            let legacy_json = serde_json::to_vec(&legacy_days).expect("legacy shape serializes");
            std::fs::write(&history_path, legacy_json).expect("legacy history shape writes");
            let legacy = DailyHistory::load_in(Some(root.clone()), Some(&doc));
            prop_assert_eq!(&legacy.entries, &legacy_days);
            prop_assert!(legacy.sprints.is_empty());

            let _ = std::fs::remove_dir_all(root);
        }
    }
}
