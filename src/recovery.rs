//! Plain-text crash-recovery journals for dirty buffers.
//!
//! Journals live outside the manuscript directory under the shared metadata
//! root. Each update uses the same temp-then-rename helper as manuscript saves,
//! so a crash or failed write leaves either the previous complete journal or
//! the new complete journal, never a partial record.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ropey::Rope;

use crate::paths;

static UNTITLED_COUNTER: AtomicU64 = AtomicU64::new(0);
static BACKUP_COUNTER: AtomicU64 = AtomicU64::new(0);
const BACKUPS_DIR: &str = "backups";
const BACKUP_PREFIX: &str = "backup-";
const BACKUP_SUFFIX: &str = ".txt";

/// Copy a successfully saved manuscript into its path-keyed rolling-backup
/// directory and prune older copies to `depth`.
///
/// A depth of zero disables new backups and deliberately leaves existing
/// copies untouched: changing a setting must not silently destroy recovery
/// data. Backup failures are returned to the caller so the save can remain
/// successful while the UI warns that the secondary safety copy failed.
pub fn write_rolling_backup(
    root: &Path,
    source: &Path,
    depth: usize,
) -> io::Result<Option<PathBuf>> {
    if depth == 0 {
        return Ok(None);
    }

    // Open the committed manuscript before creating metadata directories. The
    // bytes copied below are therefore exactly the bytes from the successful
    // atomic save, not a second rendering of the in-memory rope.
    let mut input = std::fs::File::open(source)?;
    let dir = backup_dir(root, source);
    std::fs::create_dir_all(&dir)?;

    let destination = loop {
        let candidate = dir.join(backup_name());
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut output) => {
                if let Err(error) =
                    io::copy(&mut input, &mut output).and_then(|_| output.sync_all())
                {
                    drop(output);
                    let _ = std::fs::remove_file(&candidate);
                    return Err(error);
                }
                break candidate;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };

    prune_backups(&dir, depth)?;
    Ok(Some(destination))
}

fn backup_dir(root: &Path, source: &Path) -> PathBuf {
    root.join(BACKUPS_DIR).join(paths::path_key(source))
}

fn backup_name() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let sequence = BACKUP_COUNTER.fetch_add(1, Ordering::Relaxed);
    // Fixed-width epoch components preserve chronological lexical ordering.
    // PID + process-local sequence make same-tick names collision-safe; the
    // create_new loop remains the final no-overwrite guard across processes.
    format!(
        "{BACKUP_PREFIX}{:020}-{:09}-{:010}-{:020}{BACKUP_SUFFIX}",
        timestamp.as_secs(),
        timestamp.subsec_nanos(),
        std::process::id(),
        sequence
    )
}

fn prune_backups(dir: &Path, depth: usize) -> io::Result<()> {
    let mut backups = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(BACKUP_PREFIX)
            && name.ends_with(BACKUP_SUFFIX)
            && entry.file_type()?.is_file()
        {
            backups.push(entry.path());
        }
    }
    backups.sort_by(|left, right| left.file_name().cmp(&right.file_name()));

    let remove_count = backups.len().saturating_sub(depth);
    for path in backups.into_iter().take(remove_count) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod rolling_backup_tests {
    use super::*;
    use proptest::prelude::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let id = UNTITLED_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pstar-backup-{tag}-{}-{id}", std::process::id()))
    }

    fn backup_files(root: &Path, source: &Path) -> Vec<PathBuf> {
        let dir = backup_dir(root, source);
        let mut files = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        files.sort();
        files
    }

    #[test]
    fn rolling_backups_are_plain_text_path_keyed_and_distinct_from_journal() {
        let dir = scratch_dir("layout");
        let root = dir.join("recovery");
        let source = dir.join("manuscripts").join("chapter.md");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "plain text — 日本語\n").unwrap();

        let backup = write_rolling_backup(&root, &source, 10).unwrap().unwrap();
        assert_eq!(
            backup.parent(),
            Some(
                root.join(BACKUPS_DIR)
                    .join(paths::path_key(&source))
                    .as_path()
            )
        );
        assert_ne!(backup, journal_path(&root, Some(&source), "unused"));
        assert!(
            backup
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(BACKUP_PREFIX)
        );
        assert_eq!(
            std::fs::read_to_string(backup).unwrap(),
            "plain text — 日本語\n"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rotation_keeps_newest_copies_with_unique_ordered_names() {
        let dir = scratch_dir("rotation");
        let root = dir.join("recovery");
        let source = dir.join("chapter.md");
        std::fs::create_dir_all(&dir).unwrap();

        for text in ["first", "second", "third"] {
            std::fs::write(&source, text).unwrap();
            write_rolling_backup(&root, &source, 2).unwrap();
        }

        let files = backup_files(&root, &source);
        assert_eq!(files.len(), 2);
        assert_ne!(files[0].file_name(), files[1].file_name());
        assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "second");
        assert_eq!(std::fs::read_to_string(&files[1]).unwrap(), "third");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn zero_depth_disables_creation_without_deleting_existing_backups() {
        let dir = scratch_dir("disabled");
        let root = dir.join("recovery");
        let source = dir.join("chapter.md");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, "kept").unwrap();
        write_rolling_backup(&root, &source, 1).unwrap();
        let before = backup_files(&root, &source);

        std::fs::write(&source, "not backed up").unwrap();
        assert_eq!(write_rolling_backup(&root, &source, 0).unwrap(), None);
        assert_eq!(backup_files(&root, &source), before);
        assert_eq!(std::fs::read_to_string(&before[0]).unwrap(), "kept");

        let _ = std::fs::remove_dir_all(dir);
    }

    // Feature: pro-writer-10-star, Property 21: Rolling-backup rotation keeps the newest N and depth 0 disables without deleting
    // Validates: Requirements 11.3
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn rolling_backup_rotation_keeps_newest_n_and_depth_zero_preserves_existing(
            depth in 0usize..=16,
            sequence in prop::collection::vec("[a-z]{1,24}", 1..=24),
        ) {
            let dir = scratch_dir("property-21");
            let root = dir.join("recovery");
            let source = dir.join("chapter.md");
            std::fs::create_dir_all(&dir).unwrap();

            std::fs::write(&source, "seed").unwrap();
            write_rolling_backup(&root, &source, 1).unwrap();
            let existing = backup_files(&root, &source)
                .into_iter()
                .map(|path| (path.clone(), std::fs::read_to_string(path).unwrap()))
                .collect::<Vec<_>>();

            let mut expected_contents = vec![String::from("seed")];
            for (index, value) in sequence.into_iter().enumerate() {
                let content = format!("{index}-{value}");
                std::fs::write(&source, &content).unwrap();
                let result = write_rolling_backup(&root, &source, depth).unwrap();

                if depth == 0 {
                    prop_assert_eq!(result, None);
                    let current = backup_files(&root, &source)
                        .into_iter()
                        .map(|path| (path.clone(), std::fs::read_to_string(path).unwrap()))
                        .collect::<Vec<_>>();
                    prop_assert_eq!(current, existing.clone());
                } else {
                    prop_assert!(result.is_some());
                    expected_contents.push(content);
                }
            }

            if depth > 0 {
                let actual_contents = backup_files(&root, &source)
                    .into_iter()
                    .map(|path| std::fs::read_to_string(path).unwrap())
                    .collect::<Vec<_>>();
                let keep_from = expected_contents.len().saturating_sub(depth);
                let expected_contents = expected_contents
                    .into_iter()
                    .skip(keep_from)
                    .collect::<Vec<_>>();
                prop_assert_eq!(actual_contents, expected_contents);
            }

            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// The recovery record associated with one open pane.
#[derive(Debug)]
pub struct Journal {
    path: Option<PathBuf>,
    source: Option<PathBuf>,
    last_written_edit: Option<Instant>,
}

impl Journal {
    /// Create a journal for a named or unnamed buffer.
    ///
    /// Named buffers use the canonical-path key shared with sessions. Unnamed
    /// buffers receive a process-local synthetic identity so unsaved new work
    /// is journaled too; startup recovery can discover these `untitled-*`
    /// records by scanning the recovery directory.
    pub fn new(source: Option<&Path>) -> Self {
        let path = paths::recovery().map(|root| journal_path(&root, source, &untitled_key()));
        Self {
            path,
            source: source.map(Path::to_path_buf),
            last_written_edit: None,
        }
    }

    /// Whether this pane can journal at all this session.
    ///
    /// A journal has no path when the platform exposed no state/data directory
    /// (`paths::recovery()` returned `None`). In that case every
    /// [`write_if_changed`](Self::write_if_changed) is a silent no-op, so the
    /// UI must tell the writer crash recovery is unavailable for the session
    /// (R11.7) rather than let them assume their work is protected.
    pub fn is_available(&self) -> bool {
        self.path.is_some()
    }

    /// Load a recovery record only when it is newer than the manuscript.
    ///
    /// Missing records are ordinary. Stale records are removed so they cannot
    /// prompt again. Read errors (including invalid UTF-8) are returned while
    /// leaving the record untouched for manual recovery.
    pub fn recoverable_text(&mut self) -> io::Result<Option<String>> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let journal_metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let journal_modified = journal_metadata.modified()?;
        let source_modified = match &self.source {
            Some(source) => match std::fs::metadata(source) {
                Ok(metadata) => Some(metadata.modified()?),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            },
            None => None,
        };

        if source_modified.is_some_and(|modified| journal_modified <= modified) {
            self.clear()?;
            return Ok(None);
        }

        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Atomically write the current UTF-8 rope if this edit has not already
    /// been journaled on an earlier idle tick.
    pub fn write_if_changed(&mut self, rope: &Rope, last_edit: Instant) -> io::Result<bool> {
        if self.last_written_edit == Some(last_edit) {
            return Ok(false);
        }
        let Some(path) = &self.path else {
            return Ok(false);
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        paths::write_atomic_with(path, |file| rope.write_to(io::BufWriter::new(file)))?;
        self.last_written_edit = Some(last_edit);
        Ok(true)
    }

    /// Remove a journal after a successful clean save or clean exit.
    pub fn clear(&mut self) -> io::Result<()> {
        let Some(path) = &self.path else {
            self.last_written_edit = None;
            return Ok(());
        };
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.last_written_edit = None;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn unavailable() -> Self {
        Self {
            path: None,
            source: None,
            last_written_edit: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn in_root(root: &Path, source: Option<&Path>, untitled: &str) -> Self {
        Self {
            path: Some(journal_path(root, source, untitled)),
            source: source.map(Path::to_path_buf),
            last_written_edit: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

fn journal_path(root: &Path, source: Option<&Path>, untitled: &str) -> PathBuf {
    let key = match source {
        Some(path) => paths::path_key(path),
        None => paths::path_key(Path::new(untitled)),
    };
    root.join(key)
}

fn untitled_key() -> String {
    let sequence = UNTITLED_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("untitled-{}-{nanos}-{sequence}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let id = UNTITLED_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pstar-recovery-{tag}-{}-{id}", std::process::id()))
    }

    #[test]
    fn named_journal_is_plain_text_at_path_key() {
        let dir = scratch_dir("named");
        let source = dir.join("manuscripts").join("chapter.md");
        let root = dir.join("recovery");
        let mut journal = Journal::in_root(&root, Some(&source), "unused");
        let edit = Instant::now();

        assert!(
            journal
                .write_if_changed(&Rope::from_str("plain text — 日本語\n"), edit)
                .unwrap()
        );
        assert_eq!(
            journal.path(),
            Some(root.join(paths::path_key(&source)).as_path())
        );
        assert_eq!(
            std::fs::read_to_string(journal.path().unwrap()).unwrap(),
            "plain text — 日本語\n"
        );
        let mut temporary = journal.path().unwrap().as_os_str().to_owned();
        temporary.push(".tmp~");
        assert!(!Path::new(&temporary).exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn idle_ticks_write_only_after_new_edits() {
        let dir = scratch_dir("throttle");
        let mut journal = Journal::in_root(&dir, None, "untitled-test");
        let first_edit = Instant::now();

        assert!(
            journal
                .write_if_changed(&Rope::from_str("first"), first_edit)
                .unwrap()
        );
        assert!(
            !journal
                .write_if_changed(&Rope::from_str("not rewritten"), first_edit)
                .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(journal.path().unwrap()).unwrap(),
            "first"
        );

        let second_edit = first_edit + std::time::Duration::from_nanos(1);
        assert!(
            journal
                .write_if_changed(&Rope::from_str("second"), second_edit)
                .unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(journal.path().unwrap()).unwrap(),
            "second"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unnamed_journals_have_distinct_untitled_keys() {
        let dir = scratch_dir("untitled");
        let one = Journal::in_root(&dir, None, "untitled-one");
        let two = Journal::in_root(&dir, None, "untitled-two");

        assert_ne!(one.path(), two.path());
        assert!(
            one.path()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("untitled-one-")
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_removes_journal_and_is_idempotent() {
        let dir = scratch_dir("clear");
        let mut journal = Journal::in_root(&dir, None, "untitled-clear");
        journal
            .write_if_changed(&Rope::from_str("recover me"), Instant::now())
            .unwrap();
        let path = journal.path().unwrap().to_path_buf();

        journal.clear().unwrap();
        assert!(!path.exists());
        journal.clear().unwrap();

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn recoverable_text_requires_a_newer_journal_and_clears_stale_data() {
        let dir = scratch_dir("freshness");
        let source = dir.join("chapter.md");
        let root = dir.join("recovery");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, "saved version").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        let mut journal = Journal::in_root(&root, Some(&source), "unused");
        journal
            .write_if_changed(&Rope::from_str("newer unsaved version"), Instant::now())
            .unwrap();
        assert_eq!(
            journal.recoverable_text().unwrap().as_deref(),
            Some("newer unsaved version")
        );

        journal.clear().unwrap();
        journal
            .write_if_changed(
                &Rope::from_str("now stale"),
                Instant::now() + std::time::Duration::from_nanos(1),
            )
            .unwrap();
        let stale_path = journal.path().unwrap().to_path_buf();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&source, "newest saved version").unwrap();

        assert_eq!(journal.recoverable_text().unwrap(), None);
        assert!(!stale_path.exists(), "stale journal should be discarded");

        let _ = std::fs::remove_dir_all(dir);
    }

    // Feature: pro-writer-10-star, Property 22: Recovery offers a journal only when it is newer than the manuscript
    // Validates: Requirements 11.1
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn recovery_offers_only_newer_journals_and_cleans_up_stale_records(
            manuscript in "[a-z]{1,32}",
            recovered in "[a-z]{1,32}",
            stale in "[a-z]{1,32}",
            saved_after in "[a-z]{1,32}",
        ) {
            let dir = scratch_dir("property-22");
            let root = dir.join("recovery");
            let source = dir.join("chapter.md");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&source, &manuscript).unwrap();

            // A journal written after the manuscript is offered and retained.
            std::thread::sleep(std::time::Duration::from_millis(5));
            let mut fresh = Journal::in_root(&root, Some(&source), "unused");
            fresh
                .write_if_changed(&Rope::from_str(&recovered), Instant::now())
                .unwrap();
            let fresh_recovery = fresh.recoverable_text().unwrap();
            prop_assert_eq!(fresh_recovery.as_deref(), Some(recovered.as_str()));
            prop_assert!(fresh.path().unwrap().exists());

            // A manuscript saved after its journal makes that journal stale;
            // recovery rejects it and removes it so it cannot prompt again.
            fresh.clear().unwrap();
            fresh
                .write_if_changed(&Rope::from_str(&stale), Instant::now())
                .unwrap();
            let stale_path = fresh.path().unwrap().to_path_buf();
            std::thread::sleep(std::time::Duration::from_millis(5));
            std::fs::write(&source, &saved_after).unwrap();

            prop_assert_eq!(fresh.recoverable_text().unwrap(), None);
            prop_assert!(!stale_path.exists(), "stale journal should be discarded");

            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn missing_manuscript_can_recover_but_missing_record_is_ignored() {
        let dir = scratch_dir("missing");
        let source = dir.join("deleted-chapter.md");
        let root = dir.join("recovery");
        let mut journal = Journal::in_root(&root, Some(&source), "unused");

        assert_eq!(journal.recoverable_text().unwrap(), None);
        journal
            .write_if_changed(&Rope::from_str("only surviving copy"), Instant::now())
            .unwrap();
        assert_eq!(
            journal.recoverable_text().unwrap().as_deref(),
            Some("only surviving copy")
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_record_is_reported_and_preserved() {
        let dir = scratch_dir("corrupt");
        let source = dir.join("chapter.md");
        let root = dir.join("recovery");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&source, "saved version").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        let mut journal = Journal::in_root(&root, Some(&source), "unused");
        std::fs::create_dir_all(&root).unwrap();
        let record = journal.path().unwrap().to_path_buf();
        std::fs::write(&record, [0xff, 0xfe]).unwrap();

        assert_eq!(
            journal.recoverable_text().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(record.exists(), "unreadable recovery data must be retained");

        let _ = std::fs::remove_dir_all(dir);
    }
}
