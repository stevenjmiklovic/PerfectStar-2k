//! Shared filesystem layout for everything `pstar` persists about a manuscript.
//!
//! Sessions, projects, snapshots, sidecar notes, crash-recovery journals, and
//! writing stats all live under one discoverable root *outside* the manuscript
//! folder (constraint C4), so the writer's own directory stays clean. Per-file
//! artifacts are keyed by a hash of the file's canonical path, so two files
//! that happen to share a stem never collide.
//!
//! This module is the single source of truth for that layout. `session.rs`
//! already established the root-and-hash scheme; the pro-writer subsystems
//! (project, snapshot, meta, recovery, stats) build their paths through the
//! same accessors so the tree stays consistent.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The metadata root: `<state-or-data-dir>/perfectstar2k/`.
///
/// Prefers the platform state dir, falling back to the local data dir — the
/// resolution `session.rs` has always used. Returns `None` when the platform
/// exposes neither (rare); callers degrade gracefully rather than persist.
pub fn meta_root() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Some(base.join("perfectstar2k"))
}

/// A subdirectory of [`meta_root`], created lazily by whoever writes into it.
fn subdir(name: &str) -> Option<PathBuf> {
    Some(meta_root()?.join(name))
}

/// Per-file editing sessions (cursor, bookmarks, undo history).
pub fn sessions() -> Option<PathBuf> {
    subdir("sessions")
}

// The accessors below are defined now so the metadata layout lives in one
// place, but their consumers land in later pro-writer tasks (projects §1,
// snapshots §7, meta §9, recovery/stats §2/§11). Allowed dead until then.

/// Project manifests' hidden home (the visible `*.pstarproj` lives with the
/// manuscript; anything path-hashed about a project lands here).
#[allow(dead_code)]
pub fn projects() -> Option<PathBuf> {
    subdir("projects")
}

/// Point-in-time document snapshots for the revision viewer.
#[allow(dead_code)]
pub fn snapshots() -> Option<PathBuf> {
    subdir("snapshots")
}

/// Sidecar metadata: synopsis, notes, and editorial annotations.
#[allow(dead_code)]
pub fn meta() -> Option<PathBuf> {
    subdir("meta")
}

/// Crash-recovery journals and rolling backups.
#[allow(dead_code)]
pub fn recovery() -> Option<PathBuf> {
    subdir("recovery")
}

/// Per-day words-written history.
#[allow(dead_code)]
pub fn stats() -> Option<PathBuf> {
    subdir("stats")
}

/// A stable, collision-resistant key for a file, derived from its canonical
/// path: `<file-stem>-<16-hex-hash>`.
///
/// The canonical path is hashed so relative/absolute spellings of the same
/// file agree; the human-readable stem is a debugging courtesy. When the file
/// can't be canonicalized (e.g. it doesn't exist yet) the path is used as-is,
/// which is still deterministic. Reused verbatim from the original `session.rs`
/// keying so existing on-disk session files stay valid across this refactor.
pub fn path_key(file: &Path) -> String {
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let mut h = DefaultHasher::new();
    canonical.hash(&mut h);
    format!(
        "{}-{:016x}",
        canonical
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        h.finish()
    )
}

/// Write `bytes` to `path` atomically: stream into a sibling `.tmp~` file,
/// flush it, then rename over the destination.
///
/// The rename is the point — on every platform `pstar` targets it replaces the
/// destination in a single filesystem operation, so a reader either sees the
/// complete previous file or the complete new one, never a half-written mix.
/// A crash or `ENOSPC` mid-write damages only the temp file; the previous good
/// file is never truncated (R11.5). This is the same discipline `Buffer::save`
/// has always used, extracted here so the manifest, snapshot index, meta
/// sidecars, and export writers all share the one invariant.
///
/// The temp file lives beside the destination (not in a global tempdir) so the
/// rename stays within one filesystem — a cross-device rename would fail or,
/// worse, fall back to a non-atomic copy.
// The bytes-oriented entry point; its consumers (manifest, snapshot index,
// meta sidecar) land in later pro-writer tasks. `Buffer::save` uses the
// streaming `write_atomic_with` directly. Allowed dead until then.
#[allow(dead_code)]
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |f| f.write_all(bytes))
}

/// Atomic-write generalized over *how* the bytes are produced: `fill` writes
/// into the temp file, which is then renamed over `path`. Lets a streaming
/// producer (e.g. `Rope::write_to`) keep the same crash-safety without first
/// materializing the whole document in memory.
pub fn write_atomic_with<F>(path: &Path, fill: F) -> io::Result<()>
where
    F: FnOnce(&mut std::fs::File) -> io::Result<()>,
{
    let mut tmp = path.to_path_buf().into_os_string();
    tmp.push(".tmp~");
    let tmp = PathBuf::from(tmp);

    // Scope the file handle so it's closed (and flushed) before the rename;
    // renaming a still-open file is fine on Unix but not on Windows.
    {
        let mut file = std::fs::File::create(&tmp)?;
        // If producing the bytes fails partway, drop the half-written temp
        // rather than leave litter beside the manuscript.
        if let Err(e) = fill(&mut file).and_then(|()| file.sync_all()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    }
    // A failed rename likewise leaves no debris; the destination is untouched.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_named_perfectstar2k() {
        if let Some(root) = meta_root() {
            assert_eq!(root.file_name().unwrap(), "perfectstar2k");
        }
    }

    #[test]
    fn subdirs_are_children_of_root() {
        let Some(root) = meta_root() else {
            return; // no writable base on this platform; nothing to assert
        };
        assert_eq!(sessions(), Some(root.join("sessions")));
        assert_eq!(projects(), Some(root.join("projects")));
        assert_eq!(snapshots(), Some(root.join("snapshots")));
        assert_eq!(meta(), Some(root.join("meta")));
        assert_eq!(recovery(), Some(root.join("recovery")));
        assert_eq!(stats(), Some(root.join("stats")));
    }

    #[test]
    fn path_key_is_deterministic() {
        let p = Path::new("/tmp/pstar-nonexistent-xyz/chapter1.md");
        assert_eq!(path_key(p), path_key(p));
    }

    #[test]
    fn path_key_differs_by_path() {
        let a = Path::new("/tmp/pstar-nonexistent-xyz/chapter1.md");
        let b = Path::new("/tmp/pstar-nonexistent-xyz/chapter2.md");
        assert_ne!(path_key(a), path_key(b));
    }

    #[test]
    fn path_key_shape() {
        let key = path_key(Path::new("/tmp/pstar-nonexistent-xyz/chapter1.md"));
        assert!(key.starts_with("chapter1-"), "stem prefix, got {key}");
        // Trailing `-<16 hex>` hash.
        let hash = &key[key.len() - 16..];
        assert_eq!(hash.len(), 16);
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hex hash, got {hash}"
        );
        assert_eq!(key.as_bytes()[key.len() - 17], b'-');
    }

    #[test]
    fn path_key_handles_missing_stem() {
        // Root path has no file stem; key is just `-<16 hex>`, still stable.
        let key = path_key(Path::new("/"));
        assert_eq!(key.len(), 17);
        assert!(key.starts_with('-'));
    }

    /// A unique scratch path under the temp dir. Uses the test's line number so
    /// concurrent tests don't collide, without needing a random source.
    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pstar-paths-test-{tag}"))
    }

    #[test]
    fn write_atomic_creates_file() {
        let p = scratch("create");
        let _ = std::fs::remove_file(&p);
        write_atomic(&p, b"hello").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn write_atomic_replaces_existing() {
        let p = scratch("replace");
        std::fs::write(&p, b"old contents, longer").unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn write_atomic_leaves_no_temp_file() {
        let p = scratch("no-temp");
        let _ = std::fs::remove_file(&p);
        write_atomic(&p, b"data").unwrap();
        let mut tmp = p.clone().into_os_string();
        tmp.push(".tmp~");
        assert!(!Path::new(&tmp).exists(), "temp file was left behind");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn write_atomic_preserves_prior_file_on_producer_failure() {
        let p = scratch("producer-fail");
        std::fs::write(&p, b"previous good file").unwrap();
        // Producer errors out partway; the destination must be untouched and
        // no temp debris left behind (R11.5: never truncate the good file).
        let err = write_atomic_with(&p, |f| {
            f.write_all(b"partial")?;
            Err(io::Error::other("simulated failure"))
        });
        assert!(err.is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"previous good file");
        let mut tmp = p.clone().into_os_string();
        tmp.push(".tmp~");
        assert!(!Path::new(&tmp).exists(), "temp file was left behind");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn write_atomic_with_streams_bytes() {
        let p = scratch("stream");
        let _ = std::fs::remove_file(&p);
        write_atomic_with(&p, |f| {
            f.write_all(b"chunk1")?;
            f.write_all(b"chunk2")
        })
        .unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"chunk1chunk2");
        let _ = std::fs::remove_file(&p);
    }
}
