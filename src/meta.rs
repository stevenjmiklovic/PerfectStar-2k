//! Per-document sidecar metadata: the synopsis that shows in the binder, and
//! the freeform notes that travel with a chapter (R5).
//!
//! Both live outside the manuscript folder (C4), keyed by the same canonical-path
//! hash as sessions and snapshots:
//!
//! ```text
//! perfectstar2k/meta/<stem>-<hash>.json         synopsis (structured fields)
//! perfectstar2k/meta/<stem>-<hash>-notes.md     freeform notes (plain text)
//! ```
//!
//! **Why two files.** The design called for one JSON record holding both. A
//! synopsis is one line of structured metadata and belongs in JSON, but notes
//! are *prose* — and prose trapped in a JSON string can only be edited through a
//! prompt, with no undo, no search, no spellcheck, and no crash journal. Keeping
//! notes as a plain Markdown document means they open in an ordinary pane and
//! get the whole editor, they are autosaved by the machinery that already exists
//! (R5.6), and they stay readable without `pstar` (C5). The JSON record keeps the
//! structured fields, and is where Phase 10's annotations will land.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths;

/// Suffix of a document's freeform notes file.
const NOTES_SUFFIX: &str = "-notes.md";

/// The structured sidecar record for one document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocMeta {
    /// One-line summary, shown as the binder's secondary line (R5.3).
    #[serde(default)]
    pub synopsis: String,
    /// Anything a later version wrote is carried through untouched, so reading
    /// and rewriting this file with an older binary can't destroy fields it
    /// doesn't know about — Phase 10's annotations share this record.
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

/// Path of a document's JSON sidecar within `root`.
pub fn meta_path(root: &Path, doc: &Path) -> PathBuf {
    root.join(format!("{}.json", paths::path_key(doc)))
}

/// Path of a document's freeform notes file within `root`.
pub fn notes_path(root: &Path, doc: &Path) -> PathBuf {
    root.join(format!("{}{NOTES_SUFFIX}", paths::path_key(doc)))
}

/// Load a document's sidecar record. A missing or unreadable file reads as an
/// empty record: metadata is a convenience, and never a reason to fail to open
/// a manuscript.
pub fn load(root: &Path, doc: &Path) -> DocMeta {
    std::fs::read_to_string(meta_path(root, doc))
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

/// Write a document's sidecar record atomically (R11.5).
pub fn save(root: &Path, doc: &Path, meta: &DocMeta) -> io::Result<()> {
    std::fs::create_dir_all(root)?;
    let json = serde_json::to_string_pretty(meta).map_err(io::Error::other)?;
    paths::write_atomic(&meta_path(root, doc), json.as_bytes())
}

/// Read just the synopsis, for the binder.
pub fn synopsis(root: &Path, doc: &Path) -> String {
    load(root, doc).synopsis
}

/// Set (or clear) a document's synopsis, preserving every other field.
pub fn set_synopsis(root: &Path, doc: &Path, text: &str) -> io::Result<()> {
    let mut meta = load(root, doc);
    meta.synopsis = one_line(text);
    save(root, doc, &meta)
}

/// Collapse a synopsis to a single line.
///
/// It is displayed as one row of the binder, so a pasted paragraph would
/// otherwise render as a broken line or blow the panel's layout apart.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir(tag: &str) -> PathBuf {
        let id = SCRATCH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("pstar-meta-{tag}-{}-{id}", std::process::id()))
    }

    #[test]
    fn sidecars_live_outside_the_manuscript_folder_under_a_path_key() {
        let root = Path::new("/tmp/pstar-meta-root");
        let doc = Path::new("/tmp/pstar-manuscripts/chapter1.md");
        let key = paths::path_key(doc);

        assert_eq!(meta_path(root, doc), root.join(format!("{key}.json")));
        assert_eq!(notes_path(root, doc), root.join(format!("{key}-notes.md")));
        // C4: never beside the writer's own files.
        assert!(!meta_path(root, doc).starts_with(doc.parent().unwrap()));
        // Two documents sharing a stem don't share a sidecar.
        let other = Path::new("/tmp/elsewhere/chapter1.md");
        assert_ne!(meta_path(root, doc), meta_path(root, other));
    }

    #[test]
    fn a_synopsis_round_trips_and_can_be_cleared() {
        let dir = scratch_dir("synopsis");
        let doc = dir.join("chapter.md");

        assert_eq!(synopsis(&dir, &doc), "", "no sidecar yet");
        set_synopsis(&dir, &doc, "Marcus finds the knife.").unwrap();
        assert_eq!(synopsis(&dir, &doc), "Marcus finds the knife.");

        set_synopsis(&dir, &doc, "").unwrap();
        assert_eq!(synopsis(&dir, &doc), "", "an empty synopsis clears it");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nothing_is_written_until_a_synopsis_is_set() {
        let dir = scratch_dir("lazy");
        let doc = dir.join("chapter.md");

        assert_eq!(synopsis(&dir, &doc), "");
        assert!(!dir.exists(), "reading metadata must not create anything");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pasted_paragraph_is_stored_as_one_line() {
        let dir = scratch_dir("one-line");
        let doc = dir.join("chapter.md");
        set_synopsis(
            &dir,
            &doc,
            "  Marcus finds the knife.\n\nHe says nothing.\t ",
        )
        .unwrap();

        assert_eq!(
            synopsis(&dir, &doc),
            "Marcus finds the knife. He says nothing."
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fields_written_by_a_later_version_survive_a_rewrite() {
        let dir = scratch_dir("forward-compat");
        let doc = dir.join("chapter.md");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            meta_path(&dir, &doc),
            r#"{"synopsis": "old", "annotations": [{"anchor": 42, "text": "check this"}]}"#,
        )
        .unwrap();

        set_synopsis(&dir, &doc, "new").unwrap();

        let raw = std::fs::read_to_string(meta_path(&dir, &doc)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["synopsis"], "new");
        assert_eq!(
            value["annotations"][0]["text"], "check this",
            "an unknown field must not be dropped: {raw}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_corrupt_sidecar_reads_as_empty_rather_than_failing() {
        let dir = scratch_dir("corrupt");
        let doc = dir.join("chapter.md");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(meta_path(&dir, &doc), "{ not json").unwrap();

        assert_eq!(synopsis(&dir, &doc), "");
        // ...and setting one repairs the file rather than refusing.
        set_synopsis(&dir, &doc, "recovered").unwrap();
        assert_eq!(synopsis(&dir, &doc), "recovered");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_leaves_no_temp_file_behind() {
        let dir = scratch_dir("atomic");
        let doc = dir.join("chapter.md");
        set_synopsis(&dir, &doc, "written atomically").unwrap();

        let mut temporary = meta_path(&dir, &doc).into_os_string();
        temporary.push(".tmp~");
        assert!(!Path::new(&temporary).exists());

        let _ = std::fs::remove_dir_all(dir);
    }
}
