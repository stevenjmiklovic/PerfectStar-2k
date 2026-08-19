//! Point-in-time document snapshots — the store behind "revise fearlessly" (R4).
//!
//! A snapshot is a **plain UTF-8 copy** of the buffer text, written outside the
//! manuscript folder (C4) into a path-keyed directory under the shared metadata
//! root:
//!
//! ```text
//! perfectstar2k/snapshots/<stem>-<hash>/20260819T134501.372Z-before-the-cut.txt
//! perfectstar2k/snapshots/<stem>-<hash>/index.json
//! ```
//!
//! Plain text is the whole point: a writer who loses `pstar` — or this index —
//! still has readable copies of every version (R4.7, C5). The JSON index is
//! therefore a *cache*, not the record of truth. It carries the label, the
//! timestamp, and the word count the revisions list needs (R4.3), and
//! [`SnapshotStore::in_dir`] reconciles it against the directory on load:
//! entries whose file vanished are dropped, and files the index doesn't know
//! about are adopted. A corrupt index costs metadata, never a snapshot.
//!
//! Nothing here can harm the working buffer (R4.6): the rope is borrowed
//! immutably, each snapshot goes to a freshly created unique filename so no
//! existing file is ever overwritten, and the index is rewritten through the
//! shared temp-then-rename helper (R11.5). Failures come back as `io::Error`
//! for the caller to surface as a warning.

// The commands that drive this store (manual `Snapshot`, auto-snapshot with
// retention) land in task 7.2, and restore-from-snapshot in 7.4; the store is
// the gate both depend on. Allowed dead until then.
#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ropey::Rope;
use serde::{Deserialize, Serialize};

use crate::paths;
use crate::stats;

/// Name of the per-document snapshot index.
const INDEX_FILE: &str = "index.json";
/// Extension every snapshot file carries — plain text, plainly named.
const SNAPSHOT_EXT: &str = "txt";
/// Width of the `YYYYMMDDTHHMMSS.sssZ` filename stamp.
const STAMP_LEN: usize = 20;
/// Separates the stamp from the collision counter, on the rare occasion one is
/// needed. `~` sorts after `-`, so a counted name always follows the plain one.
const COUNTER_MARK: char = '~';
/// Longest label slug allowed in a filename. Labels are a writer's shorthand
/// ("before the cut"), not prose; capping keeps paths well inside every
/// filesystem's limit while leaving the full label intact in the index.
const MAX_LABEL_SLUG: usize = 48;
/// Filename slug marking an automatic snapshot. It's in the name, not just the
/// index, so retention still knows which versions it may prune after a lost
/// index — and so a writer browsing the directory can tell the machine's copies
/// from their own.
const AUTO_SLUG: &str = "auto";

/// One snapshot, as listed in the revisions view (R4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    /// File name within the snapshot directory — not a full path, so the
    /// directory (and the whole metadata root) can be moved or copied.
    pub file: String,
    /// The label the writer typed, verbatim. The filename carries a slugified
    /// form; this is what the revisions list shows.
    #[serde(default)]
    pub label: Option<String>,
    /// Seconds since the Unix epoch, for ordering and display.
    pub timestamp: u64,
    /// Prose word count at capture time, counted the same way the status line
    /// and the exporters count it (notes and Markdown markers excluded).
    #[serde(default)]
    pub words: usize,
    /// Taken by the editor rather than asked for by the writer. Only automatic
    /// snapshots are subject to retention (R4.2) — a snapshot someone chose to
    /// take is theirs to delete.
    #[serde(default)]
    pub auto: bool,
}

impl SnapshotEntry {
    /// `YYYY-MM-DD HH:MM` in UTC, for the revisions list.
    pub fn display_time(&self) -> String {
        let (year, month, day, hour, minute, _) = utc_parts(self.timestamp);
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
    }

    /// How this version reads in the revisions list: the writer's label, or
    /// "auto" for an editor-taken snapshot, or nothing at all.
    pub fn display_label(&self) -> &str {
        match (&self.label, self.auto) {
            (Some(label), _) => label,
            (None, true) => AUTO_SLUG,
            (None, false) => "",
        }
    }
}

/// On-disk shape of the index. A struct rather than a bare array so later
/// tasks can add fields without invalidating existing indexes.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SnapshotIndex {
    #[serde(default)]
    snapshots: Vec<SnapshotEntry>,
}

/// The snapshot store for a single document.
#[derive(Debug)]
pub struct SnapshotStore {
    dir: PathBuf,
    /// Entries in chronological order — oldest first, so pruning drains the
    /// front and the newest snapshot is always last. Views that want
    /// most-recent-first reverse it.
    entries: Vec<SnapshotEntry>,
}

impl SnapshotStore {
    /// Open the store for a manuscript file inside a snapshots root, keyed by
    /// the file's canonical path.
    ///
    /// The root is passed in rather than resolved here — production hands it
    /// [`paths::snapshots`], tests hand it a temporary directory, exactly as
    /// recovery journals do. No directory is created until the first capture.
    pub fn for_file_in(root: &Path, source: &Path) -> Self {
        Self::in_dir(root.join(paths::path_key(source)))
    }

    /// Open the store rooted at an explicit directory, loading and reconciling
    /// the index. Never fails: an unreadable or corrupt index is rebuilt from
    /// whatever snapshot files are actually there.
    pub fn in_dir(dir: PathBuf) -> Self {
        let mut store = SnapshotStore {
            entries: read_index(&dir),
            dir,
        };
        store.reconcile();
        store
    }

    /// The directory holding this document's snapshots.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Snapshots in chronological order, oldest first.
    pub fn entries(&self) -> &[SnapshotEntry] {
        &self.entries
    }

    /// The most recent snapshot, if any.
    pub fn latest(&self) -> Option<&SnapshotEntry> {
        self.entries.last()
    }

    /// Full path of a snapshot file.
    pub fn path_of(&self, entry: &SnapshotEntry) -> PathBuf {
        self.dir.join(&entry.file)
    }

    /// Read a snapshot back as text (for the diff view and restore).
    pub fn read_text(&self, entry: &SnapshotEntry) -> io::Result<String> {
        std::fs::read_to_string(self.path_of(entry))
    }

    /// Capture the current buffer text as a new snapshot (R4.1).
    ///
    /// The text goes to a freshly created file whose name no existing snapshot
    /// can hold, so a capture never overwrites an earlier version; a failure
    /// part-way through removes the partial file and leaves the store as it
    /// was. The borrowed rope is never touched (R4.6).
    ///
    /// If the snapshot lands but the index rewrite fails, the error names the
    /// file that *was* written and the snapshot stays on disk — losing metadata
    /// is recoverable (the next load adopts the orphan), losing the copy is not.
    pub fn capture(&mut self, rope: &Rope, label: Option<&str>) -> io::Result<SnapshotEntry> {
        self.capture_kind(rope, label, false)
    }

    /// Capture an automatic snapshot — the kind taken on save or on the idle
    /// cadence (R4.2). Named `auto` on disk and flagged in the index so
    /// [`prune_auto`](Self::prune_auto) can retire it without ever touching a
    /// version the writer asked for.
    pub fn capture_auto(&mut self, rope: &Rope) -> io::Result<SnapshotEntry> {
        self.capture_kind(rope, None, true)
    }

    fn capture_kind(
        &mut self,
        rope: &Rope,
        label: Option<&str>,
        auto: bool,
    ) -> io::Result<SnapshotEntry> {
        std::fs::create_dir_all(&self.dir)?;

        let millis = now_millis();
        let slug = match (auto, label.map(slugify).filter(|s| !s.is_empty())) {
            (_, Some(slug)) => Some(slug),
            (true, None) => Some(String::from(AUTO_SLUG)),
            (false, None) => None,
        };
        let (file, mut handle) = self.create_unique(millis, slug.as_deref())?;
        let path = self.dir.join(&file);

        if let Err(error) = rope
            .write_to(io::BufWriter::new(&mut handle))
            .and_then(|()| handle.sync_all())
        {
            drop(handle);
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        drop(handle);

        let entry = SnapshotEntry {
            file,
            label: label
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned),
            timestamp: millis / 1_000,
            words: prose_words_in_rope(rope),
            auto,
        };
        self.entries.push(entry.clone());

        if let Err(error) = self.save_index() {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "snapshot saved to {} but its index entry could not be written: {error}",
                    path.display()
                ),
            ));
        }
        Ok(entry)
    }

    /// Keep the newest `keep` **automatic** snapshots and delete the older ones.
    /// Returns how many were removed.
    ///
    /// Manual snapshots are never pruned — R4.2 scopes retention to automatic
    /// versions, and a snapshot a writer deliberately labelled "before the cut"
    /// must not evaporate because the editor took twenty of its own since.
    /// `keep == 0` is likewise a deliberate no-op rather than "delete
    /// everything": turning retention off must not destroy history, the same
    /// rule rolling backups follow. Deletion errors don't abort the sweep —
    /// every snapshot that can go, goes, the index is rewritten to match what
    /// survives, and the first error is returned for the caller to warn about.
    pub fn prune_auto(&mut self, keep: usize) -> io::Result<usize> {
        let auto_count = self.entries.iter().filter(|entry| entry.auto).count();
        if keep == 0 || auto_count <= keep {
            return Ok(0);
        }

        let mut excess = auto_count - keep;
        let mut first_error = None;
        let mut removed = 0usize;
        let mut kept = Vec::with_capacity(self.entries.len());

        for entry in std::mem::take(&mut self.entries) {
            if !entry.auto || excess == 0 {
                kept.push(entry);
                continue;
            }
            excess -= 1;
            match std::fs::remove_file(self.dir.join(&entry.file)) {
                Ok(()) => removed += 1,
                // Already gone: still drop it from the index.
                Err(error) if error.kind() == io::ErrorKind::NotFound => removed += 1,
                Err(error) => {
                    first_error.get_or_insert(error);
                    kept.push(entry);
                }
            }
        }
        self.entries = kept;

        let saved = self.save_index();
        match first_error.or_else(|| saved.err()) {
            Some(error) => Err(error),
            None => Ok(removed),
        }
    }

    /// Create the snapshot file, reserving a name nothing else holds.
    ///
    /// `create_new` makes the reservation atomic against a second `pstar`
    /// snapshotting the same document in the same millisecond; the counter
    /// suffix only ever appends, so an existing snapshot can't be clobbered.
    /// The name always starts with the timestamp, which both orders the
    /// directory chronologically and keeps it clear of Windows' reserved device
    /// names no matter what the writer typed as a label.
    fn create_unique(
        &self,
        millis: u64,
        slug: Option<&str>,
    ) -> io::Result<(String, std::fs::File)> {
        let stamp = stamp(millis);
        for attempt in 1u32.. {
            let mut name = stamp.clone();
            if attempt > 1 {
                name.push(COUNTER_MARK);
                name.push_str(&attempt.to_string());
            }
            if let Some(slug) = slug {
                name.push('-');
                name.push_str(slug);
            }
            name.push('.');
            name.push_str(SNAPSHOT_EXT);

            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.dir.join(&name))
            {
                Ok(file) => return Ok((name, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!("attempt counter is unbounded")
    }

    /// Rewrite the index atomically, so a crash mid-write leaves the previous
    /// complete index rather than a truncated one (R11.5).
    fn save_index(&self) -> io::Result<()> {
        let index = SnapshotIndex {
            snapshots: self.entries.clone(),
        };
        let json = serde_json::to_string_pretty(&index).map_err(io::Error::other)?;
        paths::write_atomic(&self.dir.join(INDEX_FILE), json.as_bytes())
    }

    /// Make the in-memory listing agree with the directory: drop entries whose
    /// file is gone, adopt snapshot files the index doesn't mention, and sort
    /// chronologically. Read-only — the index is rewritten on the next capture.
    fn reconcile(&mut self) {
        let listing = match std::fs::read_dir(&self.dir) {
            Ok(listing) => listing,
            // No directory yet (the common case before the first snapshot):
            // nothing on disk, so nothing to list.
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.entries.clear();
                return;
            }
            // A transient failure (permissions, a busy volume) is no reason to
            // forget the index we already parsed. Leave the listing as-is.
            Err(_) => return,
        };

        let mut on_disk = Vec::new();
        for entry in listing.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(&format!(".{SNAPSHOT_EXT}")) {
                continue; // the index, and any temp-file debris
            }
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            on_disk.push((name, entry.path()));
        }

        self.entries
            .retain(|entry| on_disk.iter().any(|(name, _)| *name == entry.file));
        for (name, path) in on_disk {
            if self.entries.iter().any(|entry| entry.file == name) {
                continue;
            }
            self.entries.push(adopt(&name, &path));
        }

        // The index's timestamp is only second-resolution, so several snapshots
        // from one second tie; the filename breaks the tie, and its
        // millisecond-precision stamp (then counter) orders them exactly as they
        // were written — which is what makes `latest()` and `prune` trustworthy
        // across a reload.
        self.entries
            .sort_by(|a, b| (a.timestamp, &a.file).cmp(&(b.timestamp, &b.file)));
    }
}

/// Parse the index, treating anything unreadable or malformed as empty — the
/// snapshot files themselves are the record of truth.
fn read_index(dir: &Path) -> Vec<SnapshotEntry> {
    std::fs::read_to_string(dir.join(INDEX_FILE))
        .ok()
        .and_then(|data| serde_json::from_str::<SnapshotIndex>(&data).ok())
        .map(|index| index.snapshots)
        .unwrap_or_default()
}

/// Rebuild an entry for a snapshot file the index doesn't know about: label
/// from the filename, timestamp from the file's mtime (cheaper and more honest
/// than reversing the stamp, which a hand-renamed file may not even have), and
/// the word count recounted from the text.
///
/// A file named `…Z-auto.txt` is taken to be an automatic snapshot, so retention
/// keeps working after a lost index. Guessing the other way would be the harmful
/// mistake: an adopted file wrongly marked automatic could be pruned.
fn adopt(name: &str, path: &Path) -> SnapshotEntry {
    let stem = name
        .strip_suffix(&format!(".{SNAPSHOT_EXT}"))
        .unwrap_or(name);
    let label = stem
        .get(STAMP_LEN..)
        .map(|rest| rest.trim_start_matches(|c: char| c == COUNTER_MARK || c.is_ascii_digit()))
        .and_then(|rest| rest.strip_prefix('-'))
        .filter(|rest| !rest.is_empty())
        .map(str::to_owned);
    let timestamp = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or_default();
    let words = std::fs::read_to_string(path)
        .map(|text| prose_words_in_text(&text))
        .unwrap_or_default();

    let auto = label.as_deref() == Some(AUTO_SLUG);
    SnapshotEntry {
        file: name.to_owned(),
        label: label.filter(|_| !auto),
        timestamp,
        words,
        auto,
    }
}

/// Reduce a writer's label to something safe to put in a filename.
///
/// Alphanumerics (including non-ASCII, so a label stays recognizable) are kept
/// lowercased; everything else collapses to a single dash. That is what makes
/// the result traversal-proof: `/`, `\`, and `.` are not alphanumeric, so
/// `../../etc/passwd` slugs to `etc-passwd` and can never leave the snapshot
/// directory. May return an empty string, in which case the caller omits the
/// label from the filename entirely.
fn slugify(label: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in label.chars() {
        if c.is_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            for lowered in c.to_lowercase() {
                slug.push(lowered);
            }
            if slug.chars().count() >= MAX_LABEL_SLUG {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    slug
}

/// Prose word count of a rope, matching the status line and the exporters:
/// `..` note lines and Markdown markers don't count (R2.6).
fn prose_words_in_rope(rope: &Rope) -> usize {
    rope.lines()
        .map(|line| stats::prose_words_in_line(&line.to_string()))
        .sum()
}

fn prose_words_in_text(text: &str) -> usize {
    text.lines().map(stats::prose_words_in_line).sum()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// `YYYYMMDDTHHMMSS.sssZ` — fixed width, so lexical order is chronological
/// order, and still readable to a writer browsing the directory without
/// `pstar`. Milliseconds are there so a burst of snapshots (a save-triggered
/// auto-snapshot right after a manual one) still sorts by when it happened.
fn stamp(millis: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(millis / 1_000);
    let sub = millis % 1_000;
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}.{sub:03}Z")
}

/// Split epoch seconds into UTC calendar parts. UTC throughout, like the daily
/// stats history, so a snapshot taken either side of midnight or a DST shift
/// still sorts by when it was actually taken.
fn utc_parts(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let (year, month, day) = stats::days_to_date(secs / 86_400);
    let time = secs % 86_400;
    (
        year,
        month,
        day,
        (time / 3600) as u32,
        ((time % 3600) / 60) as u32,
        (time % 60) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir(tag: &str) -> PathBuf {
        let id = SCRATCH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pstar-snapshot-{tag}-{}-{id}", std::process::id()))
    }

    fn snapshot_files(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".txt"))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn snapshot_is_plain_text_and_round_trips() {
        let dir = scratch_dir("plain-text");
        let mut store = SnapshotStore::in_dir(dir.clone());
        let rope = Rope::from_str("Chapter One\n\nplain text — 日本語\n");

        let entry = store.capture(&rope, Some("before the cut")).unwrap();
        assert!(
            entry.file.ends_with("-before-the-cut.txt"),
            "{}",
            entry.file
        );
        // The bytes on disk are the buffer text, readable without pstar (R4.7).
        assert_eq!(
            std::fs::read_to_string(store.path_of(&entry)).unwrap(),
            "Chapter One\n\nplain text — 日本語\n"
        );
        assert_eq!(store.read_text(&entry).unwrap(), rope.to_string());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn store_lives_outside_the_manuscript_folder_under_a_path_key() {
        let source = Path::new("/tmp/pstar-manuscripts/chapter1.md");
        let Some(root) = paths::snapshots() else {
            return; // no state dir on this platform; nothing to assert
        };
        let store = SnapshotStore::for_file_in(&root, source);
        assert_eq!(store.dir(), root.join(paths::path_key(source)));
        // C4: never beside the writer's own files.
        assert!(!store.dir().starts_with(source.parent().unwrap()));
        // Opening a store must not create anything on disk.
        assert!(store.entries().is_empty());
        assert!(!store.dir().exists());
    }

    #[test]
    fn index_records_label_timestamp_and_prose_word_count() {
        let dir = scratch_dir("index");
        let mut store = SnapshotStore::in_dir(dir.clone());
        // "Chapter One" (2) + "Hello world." (2) — the `..` note doesn't count.
        let rope = Rope::from_str("# Chapter One\n.. remember the knife\nHello **world**.\n");

        let entry = store.capture(&rope, Some("draft two")).unwrap();
        assert_eq!(entry.words, 4);
        assert_eq!(entry.label.as_deref(), Some("draft two"));
        assert!(entry.timestamp > 0);

        // R4.3: the listing survives a reopen with label, time, and count.
        let reopened = SnapshotStore::in_dir(dir.clone());
        assert_eq!(reopened.entries(), std::slice::from_ref(&entry));
        assert_eq!(reopened.latest(), Some(&entry));
        assert!(!entry.display_time().is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unlabelled_snapshots_are_named_by_timestamp_alone() {
        let dir = scratch_dir("unlabelled");
        let mut store = SnapshotStore::in_dir(dir.clone());

        let entry = store.capture(&Rope::from_str("text\n"), None).unwrap();
        assert_eq!(entry.label, None);
        assert_eq!(entry.file.len(), STAMP_LEN + ".txt".len());
        assert!(entry.file.ends_with("Z.txt"), "{}", entry.file);

        // A label that slugs away to nothing gets no filename suffix either,
        // but the writer's text is still recorded for the revisions list.
        let punctuation = store
            .capture(&Rope::from_str("text\n"), Some("!!!"))
            .unwrap();
        assert_eq!(punctuation.label.as_deref(), Some("!!!"));
        assert!(!punctuation.file.contains('!'), "{}", punctuation.file);
        let stem = punctuation.file.trim_end_matches(".txt");
        assert!(
            stem.len() == STAMP_LEN || stem[STAMP_LEN..].starts_with(COUNTER_MARK),
            "{stem}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn labels_cannot_escape_the_snapshot_directory() {
        let dir = scratch_dir("traversal");
        let mut store = SnapshotStore::in_dir(dir.clone());

        let entry = store
            .capture(&Rope::from_str("safe\n"), Some("../../etc/passwd"))
            .unwrap();
        assert_eq!(
            entry.file,
            format!("{}-etc-passwd.txt", &entry.file[..STAMP_LEN])
        );
        assert_eq!(store.path_of(&entry).parent(), Some(dir.as_path()));
        // The raw label is still what the writer sees in the revisions list.
        assert_eq!(entry.label.as_deref(), Some("../../etc/passwd"));
        assert_eq!(snapshot_files(&dir), [entry.file.as_str()]);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_labels_are_truncated_and_separators_collapse() {
        assert_eq!(slugify("Before  the   Cut!"), "before-the-cut");
        assert_eq!(slugify("---"), "");
        assert_eq!(slugify("日本語 draft"), "日本語-draft");
        let long = slugify(&"word ".repeat(40));
        assert!(long.chars().count() <= MAX_LABEL_SLUG, "{long}");
        assert!(!long.ends_with('-'), "{long}");
    }

    #[test]
    fn repeated_captures_never_overwrite_an_earlier_snapshot() {
        let dir = scratch_dir("collision");
        let mut store = SnapshotStore::in_dir(dir.clone());

        // Same label, same second: the second capture must get its own file.
        let first = store
            .capture(&Rope::from_str("first\n"), Some("cut"))
            .unwrap();
        let second = store
            .capture(&Rope::from_str("second\n"), Some("cut"))
            .unwrap();

        assert_ne!(first.file, second.file);
        assert_eq!(store.read_text(&first).unwrap(), "first\n");
        assert_eq!(store.read_text(&second).unwrap(), "second\n");
        assert_eq!(store.entries().len(), 2);
        // Chronological order: oldest first.
        assert_eq!(store.entries()[0].file, first.file);
        assert_eq!(store.latest().unwrap().file, second.file);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_second_snapshots_keep_their_order_across_a_reload() {
        let dir = scratch_dir("order");
        let mut store = SnapshotStore::in_dir(dir.clone());
        // Labels whose alphabetical order contradicts the write order: reloading
        // must still report the last one written as the latest, or restore and
        // retention would silently operate on the wrong version.
        let first = store
            .capture(&Rope::from_str("first\n"), Some("zulu"))
            .unwrap();
        let second = store
            .capture(&Rope::from_str("second\n"), Some("alpha"))
            .unwrap();

        let reopened = SnapshotStore::in_dir(dir.clone());
        let order = reopened
            .entries()
            .iter()
            .map(|entry| entry.file.as_str())
            .collect::<Vec<_>>();
        assert_eq!(order, [first.file.as_str(), second.file.as_str()]);
        assert_eq!(
            reopened.read_text(reopened.latest().unwrap()).unwrap(),
            "second\n"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_index_is_rebuilt_from_the_snapshot_files() {
        let dir = scratch_dir("corrupt-index");
        let mut store = SnapshotStore::in_dir(dir.clone());
        let entry = store
            .capture(&Rope::from_str("Hello world.\n"), Some("keeper"))
            .unwrap();

        std::fs::write(dir.join(INDEX_FILE), "{ this is not json").unwrap();
        let recovered = SnapshotStore::in_dir(dir.clone());

        assert_eq!(recovered.entries().len(), 1);
        let adopted = &recovered.entries()[0];
        assert_eq!(adopted.file, entry.file);
        assert_eq!(adopted.label.as_deref(), Some("keeper"));
        assert_eq!(adopted.words, 2, "word count recounted from the text");
        assert!(adopted.timestamp > 0, "timestamp from the file's mtime");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deleted_snapshots_drop_out_of_the_listing() {
        let dir = scratch_dir("deleted");
        let mut store = SnapshotStore::in_dir(dir.clone());
        let gone = store.capture(&Rope::from_str("gone\n"), None).unwrap();
        let kept = store
            .capture(&Rope::from_str("kept\n"), Some("keep"))
            .unwrap();

        std::fs::remove_file(store.path_of(&gone)).unwrap();
        let reopened = SnapshotStore::in_dir(dir.clone());

        assert_eq!(reopened.entries().len(), 1);
        assert_eq!(reopened.entries()[0].file, kept.file);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_hand_dropped_text_file_is_adopted() {
        let dir = scratch_dir("adopt");
        std::fs::create_dir_all(&dir).unwrap();
        // No index at all — just a file a writer copied in themselves.
        std::fs::write(dir.join("my-own-copy.txt"), "One two three.\n").unwrap();

        let store = SnapshotStore::in_dir(dir.clone());
        assert_eq!(store.entries().len(), 1);
        assert_eq!(store.entries()[0].file, "my-own-copy.txt");
        assert_eq!(store.entries()[0].words, 3);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_failure_reports_and_leaves_the_store_untouched() {
        let dir = scratch_dir("write-fail");
        // A regular file where the snapshot directory should be: create_dir_all
        // fails, so the capture cannot even begin.
        std::fs::write(&dir, b"not a directory").unwrap();
        let mut store = SnapshotStore::in_dir(dir.clone());
        let rope = Rope::from_str("the working buffer\n");

        let error = store.capture(&rope, Some("doomed")).unwrap_err();
        assert!(!store.entries().iter().any(|e| e.label.is_some()));
        assert!(store.entries().is_empty(), "{error}");
        // R4.6: the buffer is untouched — it was only ever borrowed.
        assert_eq!(rope.to_string(), "the working buffer\n");
        assert_eq!(std::fs::read_to_string(&dir).unwrap(), "not a directory");

        let _ = std::fs::remove_file(dir);
    }

    #[test]
    fn unwritable_directory_leaves_no_partial_snapshot() {
        let dir = scratch_dir("readonly");
        std::fs::create_dir_all(&dir).unwrap();
        let mut permissions = std::fs::metadata(&dir).unwrap().permissions();
        permissions.set_readonly(true);
        if std::fs::set_permissions(&dir, permissions).is_err() {
            let _ = std::fs::remove_dir_all(dir);
            return; // platform doesn't honor it; nothing to prove
        }

        let mut store = SnapshotStore::in_dir(dir.clone());
        let result = store.capture(&Rope::from_str("text\n"), None);

        // Either the create is refused (expected) or the platform allowed it;
        // what must hold in the refusal case is that nothing was left behind.
        if result.is_err() {
            assert!(store.entries().is_empty());
            let mut permissions = std::fs::metadata(&dir).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = std::fs::set_permissions(&dir, permissions);
            assert!(snapshot_files(&dir).is_empty());
        } else {
            let mut permissions = std::fs::metadata(&dir).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = std::fs::set_permissions(&dir, permissions);
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_keeps_the_newest_automatic_snapshots() {
        let dir = scratch_dir("prune");
        let mut store = SnapshotStore::in_dir(dir.clone());
        for text in ["first", "second", "third", "fourth"] {
            store.capture_auto(&Rope::from_str(text)).unwrap();
        }

        assert_eq!(store.prune_auto(2).unwrap(), 2);
        let survivors = store
            .entries()
            .iter()
            .map(|entry| store.read_text(entry).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(survivors, ["third", "fourth"]);
        assert_eq!(snapshot_files(&dir).len(), 2);
        // The index on disk agrees with what survived.
        assert_eq!(
            SnapshotStore::in_dir(dir.clone()).entries(),
            store.entries()
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn retention_never_prunes_a_snapshot_the_writer_asked_for() {
        let dir = scratch_dir("prune-manual");
        let mut store = SnapshotStore::in_dir(dir.clone());
        // The labelled version is the oldest, so an auto-count-blind prune
        // would take it first (R4.2: retention covers automatic snapshots only).
        store
            .capture(&Rope::from_str("before the cut"), Some("before the cut"))
            .unwrap();
        for text in ["auto one", "auto two", "auto three"] {
            store.capture_auto(&Rope::from_str(text)).unwrap();
        }

        assert_eq!(store.prune_auto(1).unwrap(), 2);
        let survivors = store
            .entries()
            .iter()
            .map(|entry| (entry.display_label().to_owned(), entry.auto))
            .collect::<Vec<_>>();
        assert_eq!(
            survivors,
            [
                (String::from("before the cut"), false),
                (String::from("auto"), true),
            ]
        );
        assert_eq!(
            store.read_text(&store.entries()[0]).unwrap(),
            "before the cut"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_is_a_no_op_when_retention_is_off_or_not_reached() {
        let dir = scratch_dir("prune-noop");
        let mut store = SnapshotStore::in_dir(dir.clone());
        store.capture_auto(&Rope::from_str("one")).unwrap();
        store.capture_auto(&Rope::from_str("two")).unwrap();
        let before = store.entries().to_vec();

        // Retention disabled must not destroy existing history.
        assert_eq!(store.prune_auto(0).unwrap(), 0);
        // Nor should a limit the store hasn't reached.
        assert_eq!(store.prune_auto(5).unwrap(), 0);
        assert_eq!(store.entries(), before.as_slice());
        assert_eq!(snapshot_files(&dir).len(), 2);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn automatic_snapshots_are_marked_on_disk_and_survive_a_lost_index() {
        let dir = scratch_dir("auto-marking");
        let mut store = SnapshotStore::in_dir(dir.clone());
        let auto = store.capture_auto(&Rope::from_str("machine copy")).unwrap();
        let manual = store
            .capture(&Rope::from_str("my copy"), Some("mine"))
            .unwrap();
        assert!(auto.file.ends_with("-auto.txt"), "{}", auto.file);
        assert_eq!(auto.display_label(), "auto");
        assert!(auto.auto && !manual.auto);

        // With the index gone, the filename still says which is which, so
        // retention keeps working and the manual version stays protected.
        std::fs::remove_file(dir.join(INDEX_FILE)).unwrap();
        let mut recovered = SnapshotStore::in_dir(dir.clone());
        assert_eq!(
            recovered
                .entries()
                .iter()
                .map(|entry| entry.auto)
                .collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!(recovered.prune_auto(0).unwrap(), 0);
        assert_eq!(recovered.prune_auto(1).unwrap(), 0, "one auto, keep one");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn timestamps_format_as_sortable_utc() {
        // 2024-01-01T00:00:00Z is 19723 days since the epoch.
        let midnight = 19_723 * 86_400 * 1_000;
        assert_eq!(stamp(midnight), "20240101T000000.000Z");
        assert_eq!(
            stamp(midnight + (13 * 3600 + 45 * 60 + 1) * 1_000 + 372),
            "20240101T134501.372Z"
        );
        assert_eq!(stamp(midnight).len(), STAMP_LEN);
        // Lexical order is chronological order, down to the millisecond.
        assert!(stamp(midnight) < stamp(midnight + 1));
        assert!(stamp(midnight + 999) < stamp(midnight + 1_000));

        let entry = SnapshotEntry {
            file: String::new(),
            label: None,
            timestamp: midnight / 1_000 + 13 * 3600 + 45 * 60,
            words: 0,
            auto: false,
        };
        assert_eq!(entry.display_time(), "2024-01-01 13:45");
    }
}
