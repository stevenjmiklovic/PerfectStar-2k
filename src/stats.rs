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
use std::time::Instant;

use ropey::Rope;

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

        // If lines were added or removed, rebuild the line vectors.
        if n != self.line_words.len() {
            self.line_words.resize(n, 0);
            self.line_chars.resize(n, 0);
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalKind {
    Words,
    #[allow(dead_code)] // Used by Phase 8 (sprints/timers).
    Minutes,
}

/// A running session goal.
#[derive(Debug, Clone)]
pub struct SessionGoal {
    pub kind: GoalKind,
    pub target: usize,
    pub start_words: usize,
    pub started: Instant,
    pub reached: bool,
}

impl SessionGoal {
    pub fn new(kind: GoalKind, target: usize, current_words: usize) -> Self {
        SessionGoal {
            kind,
            target,
            start_words: current_words,
            started: Instant::now(),
            reached: false,
        }
    }

    /// Progress as (current, target). Returns None if the goal is time-based
    /// and the elapsed check is more appropriate.
    pub fn progress(&self, current_words: usize) -> (usize, usize) {
        match self.kind {
            GoalKind::Words => {
                let written = current_words.saturating_sub(self.start_words);
                (written, self.target)
            }
            GoalKind::Minutes => {
                let elapsed = self.started.elapsed().as_secs() / 60;
                (elapsed as usize, self.target)
            }
        }
    }

    /// Whether the goal is now met.
    pub fn is_met(&self, current_words: usize) -> bool {
        let (current, target) = self.progress(current_words);
        current >= target
    }
}

// --- Daily history -----------------------------------------------------------

/// Per-day net-words record, keyed by ISO date string (YYYY-MM-DD).
#[derive(Debug, Clone, Default)]
pub struct DailyHistory {
    pub entries: BTreeMap<String, i64>,
    path: Option<PathBuf>,
}

impl DailyHistory {
    /// Load history for a given file from the stats directory.
    pub fn load(file_path: Option<&Path>) -> Self {
        let path = match (crate::paths::stats(), file_path) {
            (Some(dir), Some(fp)) => {
                let key = crate::paths::path_key(fp);
                Some(dir.join(format!("{key}.json")))
            }
            _ => None,
        };
        let entries = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        DailyHistory { entries, path }
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

    /// Persist history to disk.
    pub fn save(&self) -> io::Result<()> {
        let Some(ref path) = self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.entries)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
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

fn days_to_date(days_since_epoch: u64) -> (u32, u32, u32) {
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
    fn daily_history_round_trip() {
        let mut h = DailyHistory::default();
        h.record_delta(50);
        h.record_delta(30);
        assert_eq!(h.today(), 80);
    }

    #[test]
    fn date_conversion_sanity() {
        // 2024-01-01 is 19723 days since epoch
        let (y, m, d) = days_to_date(19723);
        assert_eq!((y, m, d), (2024, 1, 1));
    }
}
