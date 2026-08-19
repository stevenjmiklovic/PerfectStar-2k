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
//! structured fields — the synopsis and the editorial annotations of R9, whose
//! text living here rather than in the manuscript is what makes it impossible
//! for a comment to reach an export (R9.3).

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::block::adjust_pos;
use crate::paths;

/// Suffix of a document's freeform notes file.
const NOTES_SUFFIX: &str = "-notes.md";

/// An editorial comment anchored to a span of the document (R9.1).
///
/// The comment text lives here, never in the manuscript, which is what makes it
/// impossible for an annotation to reach an export (R9.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// Char index where the annotated span begins.
    pub anchor: usize,
    /// Length of the annotated span in chars. Zero means a point annotation —
    /// attached to a position rather than a stretch of text.
    #[serde(default)]
    pub len: usize,
    /// What the editor (or the writer) wrote.
    pub text: String,
    /// The annotated text has been deleted. The comment is kept anyway (R9.6):
    /// losing an editorial note because the sentence it was about got cut is
    /// exactly the kind of silent loss C3 forbids.
    #[serde(default)]
    pub orphaned: bool,
}

impl Annotation {
    pub fn new(anchor: usize, len: usize, text: String) -> Self {
        Annotation {
            anchor,
            len,
            text,
            orphaned: false,
        }
    }

    /// The span's end (exclusive).
    pub fn end(&self) -> usize {
        self.anchor + self.len
    }

    /// Whether `pos` falls inside the annotated span (or exactly on a point
    /// annotation).
    pub fn covers(&self, pos: usize) -> bool {
        if self.orphaned {
            return false;
        }
        if self.len == 0 {
            pos == self.anchor
        } else {
            pos >= self.anchor && pos < self.end()
        }
    }

    /// Move this annotation across an edit that replaced `del` chars at `at`
    /// with `ins` chars (R9.5).
    ///
    /// Both ends go through the same [`adjust_pos`](crate::block::adjust_pos)
    /// that keeps block marks and bookmarks attached, so an annotation drifts
    /// exactly as a mark on the same text would. When the edit swallows the
    /// whole span the annotation is orphaned rather than dropped (R9.6).
    pub fn adjust(&mut self, at: usize, del: usize, ins: usize) {
        if self.orphaned {
            return;
        }
        let start = adjust_pos(self.anchor, at, del, ins);
        let end = adjust_pos(self.end(), at, del, ins);
        // A span that had length and now has none lost its text to the edit.
        if self.len > 0 && end <= start {
            self.orphaned = true;
            self.anchor = start;
            self.len = 0;
            return;
        }
        // A point annexed by a deletion lands at the deletion's start, which is
        // still the right place to say "this used to be here".
        self.anchor = start;
        self.len = end - start;
    }
}

/// The structured sidecar record for one document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocMeta {
    /// One-line summary, shown as the binder's secondary line (R5.3).
    #[serde(default)]
    pub synopsis: String,
    /// Editorial comments anchored into the text (R9.1).
    #[serde(default)]
    pub annotations: Vec<Annotation>,
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

/// Read a document's annotations (R9.1).
pub fn annotations(root: &Path, doc: &Path) -> Vec<Annotation> {
    load(root, doc).annotations
}

/// Write a document's annotations, preserving every other field.
pub fn set_annotations(root: &Path, doc: &Path, annotations: &[Annotation]) -> io::Result<()> {
    let mut meta = load(root, doc);
    meta.annotations = annotations.to_vec();
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

    fn note(anchor: usize, len: usize) -> Annotation {
        Annotation::new(anchor, len, String::from("check this"))
    }

    #[test]
    fn an_edit_before_the_span_shifts_it_whole() {
        // "the knife" at 4..13; insert 6 chars at 0.
        let mut a = note(4, 9);
        a.adjust(0, 0, 6);
        assert_eq!((a.anchor, a.len), (10, 9));
        assert!(!a.orphaned);

        // ...and a deletion before it pulls it back.
        a.adjust(0, 6, 0);
        assert_eq!((a.anchor, a.len), (4, 9));
    }

    #[test]
    fn an_edit_after_the_span_leaves_it_alone() {
        let mut a = note(4, 9);
        a.adjust(20, 3, 7);
        assert_eq!((a.anchor, a.len), (4, 9));
        assert!(!a.orphaned);
    }

    #[test]
    fn an_edit_inside_the_span_grows_or_shrinks_it() {
        let mut a = note(4, 9);
        a.adjust(6, 0, 4); // insert inside
        assert_eq!((a.anchor, a.len), (4, 13));

        a.adjust(6, 4, 0); // take it back out
        assert_eq!((a.anchor, a.len), (4, 9));
        assert!(!a.orphaned, "editing inside is not losing the anchor");
    }

    #[test]
    fn deleting_the_anchored_text_orphans_the_comment_rather_than_losing_it() {
        // R9.6/C3: the sentence goes, the editorial note stays.
        let mut a = note(4, 9);
        a.adjust(4, 9, 0);

        assert!(a.orphaned);
        assert_eq!(a.len, 0);
        assert_eq!(a.anchor, 4, "it remembers where the text had been");
        assert_eq!(a.text, "check this");
    }

    #[test]
    fn a_deletion_spanning_the_whole_paragraph_orphans_too() {
        let mut a = note(10, 5);
        a.adjust(0, 40, 0);

        assert!(a.orphaned);
        assert_eq!(a.anchor, 0);
    }

    #[test]
    fn a_partly_deleted_span_survives_with_what_is_left() {
        let mut a = note(4, 9);
        a.adjust(8, 5, 0); // delete the tail half

        assert!(!a.orphaned, "some anchored text remains");
        assert_eq!((a.anchor, a.len), (4, 4));
    }

    #[test]
    fn an_orphan_stops_moving_and_covers_nothing() {
        let mut a = note(4, 9);
        a.adjust(4, 9, 0);
        let before = a.clone();
        a.adjust(0, 0, 100);

        assert_eq!(a, before, "an orphan has no anchor left to adjust");
        assert!(!a.covers(4));
    }

    #[test]
    fn a_point_annotation_tracks_its_position() {
        let mut a = note(10, 0);
        assert!(a.covers(10));
        assert!(!a.covers(11));

        a.adjust(0, 0, 5);
        assert_eq!((a.anchor, a.len), (15, 0));
        assert!(!a.orphaned, "a point is never orphaned by a shift");

        // Deleting the text around a point collapses it to the edit site and
        // keeps it: there is still something to say about that place.
        a.adjust(12, 6, 0);
        assert_eq!((a.anchor, a.len), (12, 0));
        assert!(!a.orphaned);
    }

    #[test]
    fn covers_marks_the_span_but_not_its_end() {
        let a = note(4, 3);
        assert!(!a.covers(3));
        assert!(a.covers(4) && a.covers(6));
        assert!(!a.covers(7), "the end is exclusive");
    }

    #[test]
    fn annotations_round_trip_through_the_sidecar() {
        let dir = scratch_dir("annotations");
        let doc = dir.join("chapter.md");

        assert!(annotations(&dir, &doc).is_empty());
        let mut written = vec![note(4, 9), note(40, 0)];
        written[1].orphaned = true;
        set_annotations(&dir, &doc, &written).unwrap();

        assert_eq!(annotations(&dir, &doc), written);
        // ...and they share the record with the synopsis rather than fighting it.
        set_synopsis(&dir, &doc, "Marcus finds the knife.").unwrap();
        assert_eq!(annotations(&dir, &doc), written);
        assert_eq!(synopsis(&dir, &doc), "Marcus finds the knife.");

        let _ = std::fs::remove_dir_all(dir);
    }
}
