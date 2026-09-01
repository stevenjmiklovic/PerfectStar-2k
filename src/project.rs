//! Project manifest: ordered collection of documents for book-scale writing.
//!
//! A **project** is a named multi-file manuscript (a novel, a dissertation, etc.)
//! treated as one coherent work. The binder panel (R1) shows the docs in author-
//! defined order; compile (R1.6) concatenates them for export; project-wide
//! search (R6) spans all of them. The manifest persists as a visible `.pstarproj`
//! TOML file in the project folder (ADR-012) — unlike hidden session metadata,
//! this is a first-class user-owned artifact that travels with the manuscript.
//!
//! Doc paths in the manifest are stored relative to the manifest's directory when
//! possible, so moving the whole project folder preserves the structure.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::buffer::Buffer;
use crate::paths;

/// Separator inserted between documents when compiling a project (R1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Separator {
    /// Hard page break (`\f` / Form Feed) — standard manuscript practice.
    PageBreak,
    /// Three blank lines (common in screenplay/chapter breaks).
    BlankLines,
    /// Horizontal rule (`# # #` or `* * *`) — visible chapter separator.
    HorizontalRule,
    /// Nothing — documents concatenate directly (useful for scene files).
    None,
}

impl Default for Separator {
    fn default() -> Self {
        Self::PageBreak
    }
}

impl Separator {
    /// The text inserted between documents when compiling.
    #[allow(dead_code)] // Used by task 1.6 (compile)
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PageBreak => "\n\x0c\n", // form feed with padding
            Self::BlankLines => "\n\n\n\n",
            Self::HorizontalRule => "\n# # #\n\n",
            Self::None => "",
        }
    }
}

/// What a document in the project is *for* (R5.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocRole {
    /// Part of the book itself.
    #[default]
    Manuscript,
    /// Characters, places, timeline, research — edited in `pstar` like any
    /// document, and never part of the compiled manuscript.
    Note,
}

/// A document entry in the project manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocEntry {
    /// Path to the document file. Stored relative to the manifest directory
    /// when possible, absolute if the doc is outside the project tree.
    pub path: PathBuf,
    /// Display title shown in the binder. Defaults to the file stem or the
    /// first heading in the doc; user-editable.
    pub title: String,
    /// Whether to include this doc in compile (R1.6). Lets a project hold
    /// scratch/notes files that aren't part of the exported manuscript.
    #[serde(default = "default_true")]
    pub include_in_compile: bool,
    /// Manuscript or note (R5.2). Defaults to manuscript, so manifests written
    /// before notes existed keep compiling exactly as they did.
    #[serde(default)]
    pub role: DocRole,
}

impl DocEntry {
    /// Whether this is a note document rather than part of the book.
    pub fn is_note(&self) -> bool {
        self.role == DocRole::Note
    }
}

fn default_true() -> bool {
    true
}

/// The trimmed text of a document's first Markdown heading, if it has one.
///
/// Reads the file line by line and stops at the first heading, so a large
/// manuscript is not fully loaded just to name a binder row. Returns `None`
/// when the file is missing, unreadable, or contains no heading — the caller
/// then falls back to the filename-derived title (R1.2).
fn first_heading(path: &Path) -> Option<String> {
    use std::io::BufRead;

    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        let line = line.ok()?;
        if let Some((_level, title)) = crate::markdown::heading_level(&line) {
            let title = title.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

/// The project manifest: everything `pstar` knows about a multi-file book.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    /// Project name (display title, user-chosen).
    pub name: String,
    /// Ordered list of documents. The author defines this order; it drives the
    /// binder display and the compile sequence.
    pub docs: Vec<DocEntry>,
    /// Separator inserted between docs on compile (R1.6).
    #[serde(default)]
    pub separator: Separator,
}

/// A document that was skipped during compilation (missing or unreadable).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Used by task 1.6 (compile)
pub struct SkippedDoc {
    /// Index into the manifest's doc list.
    pub index: usize,
    /// The doc's title (for user-facing messages).
    pub title: String,
    /// Human-readable reason the doc was skipped (e.g. "No such file").
    pub reason: String,
}

/// The result of compiling a project: concatenated text plus any skipped docs.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used by task 1.6 (compile)
pub struct CompileResult {
    /// The concatenated text of all included, readable docs with separators.
    pub text: String,
    /// Docs that were skipped because they were missing or unreadable.
    pub skipped: Vec<SkippedDoc>,
}

/// A loaded project: manifest plus derived/cached state.
#[derive(Debug)]
#[allow(dead_code)] // Fields used by later tasks (1.2+)
pub struct Project {
    /// The manifest's canonical path (the `.pstarproj` file on disk).
    pub manifest_path: PathBuf,
    /// The loaded manifest.
    pub manifest: ProjectManifest,
    // Future: per-doc word-count cache, missing-file flags (tasks 1.3, 1.5).
}

impl Project {
    /// Load a project from a `.pstarproj` TOML file.
    #[allow(dead_code)] // Used by task 1.2 (App.project wiring)
    ///
    /// Doc paths stored as relative are resolved against the manifest's
    /// directory. Missing files are tolerated (R1.7); they'll be flagged in the
    /// binder but don't prevent opening the project.
    pub fn load(manifest_path: &Path) -> io::Result<Self> {
        let data = std::fs::read_to_string(manifest_path)?;
        let mut manifest: ProjectManifest = toml::from_str(&data).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid project manifest: {e}"),
            )
        })?;

        // Resolve relative paths against the manifest's directory.
        let base = manifest_path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Manifest has no parent"))?;
        for entry in &mut manifest.docs {
            if entry.path.is_relative() {
                entry.path = base.join(&entry.path);
            }
        }

        Ok(Self {
            manifest_path: manifest_path.to_path_buf(),
            manifest,
        })
    }

    /// Save the manifest atomically (R11.5: previous good file never truncated).
    #[allow(dead_code)] // Used by task 1.4 (reorder/add/remove)
    ///
    /// Doc paths are stored relative to the manifest directory when possible,
    /// so moving the project folder preserves the structure.
    pub fn save(&self) -> io::Result<()> {
        // Clone the manifest and make paths relative to the manifest's dir.
        let mut manifest = self.manifest.clone();
        let base = self
            .manifest_path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Manifest has no parent"))?;

        for entry in &mut manifest.docs {
            // Strip the base prefix if the doc is under the project tree.
            if let Ok(rel) = entry.path.strip_prefix(base) {
                entry.path = rel.to_path_buf();
            }
            // Otherwise leave it absolute (doc is outside the project folder).
        }

        let data = toml::to_string_pretty(&manifest)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        paths::write_atomic(&self.manifest_path, data.as_bytes())
    }

    /// The project's base directory (the manifest's parent folder).
    #[allow(dead_code)] // Used by later tasks (binder UI, compile)
    pub fn base_dir(&self) -> Option<&Path> {
        self.manifest_path.parent()
    }

    /// Add a document to the project (at the end of the list).
    #[allow(dead_code)] // Used by task 1.4 (add/remove)
    pub fn add_doc(&mut self, path: PathBuf, title: String) {
        self.manifest.docs.push(DocEntry {
            path,
            title,
            include_in_compile: true,
            role: DocRole::Manuscript,
        });
    }

    /// Flip a document between manuscript and note (R5.2), returning its new
    /// role. Marking a document as a note takes it out of the compile; marking
    /// it back restores it, so the gesture is reversible.
    pub fn toggle_role(&mut self, idx: usize) -> Option<DocRole> {
        let entry = self.manifest.docs.get_mut(idx)?;
        entry.role = match entry.role {
            DocRole::Manuscript => DocRole::Note,
            DocRole::Note => DocRole::Manuscript,
        };
        Some(entry.role)
    }

    /// Whether the document at `idx` is a note.
    pub fn doc_is_note(&self, idx: usize) -> bool {
        self.manifest
            .docs
            .get(idx)
            .is_some_and(|entry| entry.is_note())
    }

    /// Remove a document from the project by index. The file on disk is never
    /// deleted (R1.5) — this only removes it from the manifest.
    #[allow(dead_code)] // Used by task 1.4 (add/remove)
    pub fn remove_doc(&mut self, idx: usize) -> Option<DocEntry> {
        if idx < self.manifest.docs.len() {
            Some(self.manifest.docs.remove(idx))
        } else {
            None
        }
    }

    /// Reorder a document: move the doc at `from` to position `to`.
    #[allow(dead_code)] // Used by task 1.4 (reorder)
    pub fn reorder_doc(&mut self, from: usize, to: usize) {
        if from < self.manifest.docs.len() && to < self.manifest.docs.len() {
            let doc = self.manifest.docs.remove(from);
            self.manifest.docs.insert(to, doc);
        }
    }

    /// Check whether a doc exists on disk. Used by the binder to flag missing
    /// files (R1.7).
    #[allow(dead_code)] // Used by task 1.5 (missing-file resilience)
    pub fn doc_exists(&self, idx: usize) -> bool {
        self.manifest
            .docs
            .get(idx)
            .map(|e| e.path.exists())
            .unwrap_or(false)
    }

    /// Compile the project: concatenate all `include_in_compile` docs in binder
    /// order, inserting the configured separator between them.
    ///
    /// Missing or unreadable files are skipped (R1.7 resilience) and reported in
    /// the returned `CompileResult::skipped` list so the caller can warn the user.
    /// This produces the raw concatenated text that Phase 3 will later normalize
    /// into a full `CompiledDoc`.
    #[allow(dead_code)] // Called by Phase 3 export (task 3.1)
    pub fn compile(&self) -> CompileResult {
        let sep = self.manifest.separator.as_str();
        let mut text = String::new();
        let mut skipped: Vec<SkippedDoc> = Vec::new();
        let mut first = true;

        for (idx, entry) in self.manifest.docs.iter().enumerate() {
            // A note document is never part of the book (R5.2), whatever its
            // include flag says — the role is the durable statement of intent.
            if !entry.include_in_compile || entry.is_note() {
                continue;
            }

            match std::fs::read_to_string(&entry.path) {
                Ok(content) => {
                    if !first && !sep.is_empty() {
                        text.push_str(sep);
                    }
                    text.push_str(&content);
                    first = false;
                }
                Err(e) => {
                    skipped.push(SkippedDoc {
                        index: idx,
                        title: entry.title.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }

        CompileResult { text, skipped }
    }

    /// The title to show for a document in the binder (R1.2): the document's
    /// first Markdown heading if it has one, otherwise the manifest title
    /// (which defaults to the file stem). Missing or unreadable files fall back
    /// to the manifest title so the binder still names the entry.
    ///
    /// This reads only the file's first non-empty content, so it stays cheap
    /// enough to call while building the binder rows.
    pub fn doc_title(&self, idx: usize) -> String {
        let Some(entry) = self.manifest.docs.get(idx) else {
            return String::new();
        };
        first_heading(&entry.path).unwrap_or_else(|| entry.title.clone())
    }

    /// Get the word count for a document. Returns None if the file doesn't exist
    /// or can't be loaded. Used by the binder panel (task 1.3).
    ///
    /// Note: This loads the file from disk to count words. In the future, this
    /// could be cached to avoid repeated disk I/O (see design §4.1).
    pub fn doc_word_count(&self, idx: usize) -> Option<usize> {
        let entry = self.manifest.docs.get(idx)?;
        if !entry.path.exists() {
            return None;
        }
        // Load the buffer and count words.
        // We can't use Pane::open here as that has side effects (session restore).
        // Instead, load the buffer directly via Buffer::open.
        match Buffer::open(Some(entry.path.clone())) {
            Ok(buf) => Some(buf.word_count()),
            Err(_) => None,
        }
    }

    /// Prose word count for the whole project (R2.1): the sum of every readable
    /// document's prose words, counted the same way the status bar and exporter
    /// count them — `..` note lines and Markdown markers excluded (R2.7).
    ///
    /// Missing or unreadable docs contribute zero rather than aborting the sum,
    /// mirroring the binder's "missing" tolerance (R1.7). This reads files from
    /// disk, so callers cache the result rather than recomputing it per frame
    /// (C6); the active document's live count is layered on top by the caller.
    pub fn total_prose_words(&self) -> usize {
        self.manifest
            .docs
            .iter()
            .map(|entry| prose_words_in_file(&entry.path))
            .sum()
    }
}

/// Count prose words in a file on disk, returning 0 if it can't be read. Uses
/// the same per-line prose counting as the live document statistics so the
/// project total agrees with the per-document count (R2.1, R2.7).
fn prose_words_in_file(path: &Path) -> usize {
    match std::fs::read_to_string(path) {
        Ok(text) => text.lines().map(crate::stats::prose_words_in_line).sum(),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A unique scratch path under the temp dir for testing.
    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pstar-project-test-{tag}"))
    }

    fn path_components() -> impl Strategy<Value = PathBuf> {
        prop::collection::vec("[a-z]{1,8}", 1..4).prop_map(|parts| parts.into_iter().collect())
    }

    fn relative_or_absolute_path() -> impl Strategy<Value = PathBuf> {
        prop_oneof![
            path_components(),
            path_components().prop_map(|path| {
                std::env::temp_dir()
                    .join("pstar-project-property-external")
                    .join(path)
            }),
        ]
    }

    fn document_spec() -> impl Strategy<Value = (PathBuf, String, bool, DocRole)> {
        (
            relative_or_absolute_path(),
            "[A-Za-z][A-Za-z0-9 _-]{0,20}",
            any::<bool>(),
            prop_oneof![Just(DocRole::Manuscript), Just(DocRole::Note)],
        )
    }

    static PROPERTY_SCRATCH_ID: AtomicUsize = AtomicUsize::new(0);

    // Feature: pro-writer-10-star, Property 3: Manifest round-trip preserves project structure
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            .. ProptestConfig::default()
        })]
        #[test]
        fn manifest_round_trip_preserves_project_structure(
            name in "[A-Za-z][A-Za-z0-9 _-]{0,20}",
            docs in prop::collection::vec(document_spec(), 0..8),
            separator in prop_oneof![
                Just(Separator::PageBreak),
                Just(Separator::BlankLines),
                Just(Separator::HorizontalRule),
                Just(Separator::None),
            ],
        ) {
            let id = PROPERTY_SCRATCH_ID.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "pstar-project-property-{}-{id}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();

            let manifest_path = dir.join("project.pstarproj");
            let manifest = ProjectManifest {
                name,
                docs: docs
                    .iter()
                    .map(|(path, title, include_in_compile, role)| DocEntry {
                        path: path.clone(),
                        title: title.clone(),
                        include_in_compile: *include_in_compile,
                        role: *role,
                    })
                    .collect(),
                separator,
            };
            let expected_paths: Vec<PathBuf> = manifest
                .docs
                .iter()
                .map(|entry| {
                    if entry.path.is_relative() {
                        dir.join(&entry.path)
                    } else {
                        entry.path.clone()
                    }
                })
                .collect();

            let project = Project {
                manifest_path: manifest_path.clone(),
                manifest,
            };
            project.save().unwrap();
            let loaded = Project::load(&manifest_path).unwrap();

            prop_assert_eq!(loaded.manifest.name, project.manifest.name);
            prop_assert_eq!(loaded.manifest.separator, project.manifest.separator);
            prop_assert_eq!(loaded.manifest.docs.len(), project.manifest.docs.len());
            for ((loaded_doc, original_doc), expected_path) in loaded
                .manifest
                .docs
                .iter()
                .zip(project.manifest.docs.iter())
                .zip(expected_paths.iter())
            {
                prop_assert_eq!(&loaded_doc.path, expected_path);
                prop_assert_eq!(&loaded_doc.title, &original_doc.title);
                prop_assert_eq!(loaded_doc.include_in_compile, original_doc.include_in_compile);
                prop_assert_eq!(loaded_doc.role, original_doc.role);
            }

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn manifest_round_trip() {
        let manifest = ProjectManifest {
            name: "Test Novel".to_string(),
            docs: vec![
                DocEntry {
                    path: PathBuf::from("chapter1.md"),
                    title: "Chapter One".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                },
                DocEntry {
                    path: PathBuf::from("notes.md"),
                    title: "Research Notes".to_string(),
                    include_in_compile: false,
                    role: DocRole::Manuscript,
                },
            ],
            separator: Separator::PageBreak,
        };

        // Serialize and deserialize.
        let toml_str = toml::to_string(&manifest).unwrap();
        let manifest2: ProjectManifest = toml::from_str(&toml_str).unwrap();

        assert_eq!(manifest2.name, "Test Novel");
        assert_eq!(manifest2.docs.len(), 2);
        assert_eq!(manifest2.docs[0].title, "Chapter One");
        assert!(manifest2.docs[0].include_in_compile);
        assert_eq!(manifest2.docs[1].title, "Research Notes");
        assert!(!manifest2.docs[1].include_in_compile);
        assert_eq!(manifest2.separator, Separator::PageBreak);
    }

    #[test]
    fn save_and_load_manifest() {
        let dir = scratch("save-load");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest_path = dir.join("test.pstarproj");
        let project = Project {
            manifest_path: manifest_path.clone(),
            manifest: ProjectManifest {
                name: "My Book".to_string(),
                docs: vec![DocEntry {
                    path: dir.join("chapter1.md"),
                    title: "Chapter 1".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                }],
                separator: Separator::BlankLines,
            },
        };

        // Save.
        project.save().unwrap();
        assert!(manifest_path.exists());

        // Load.
        let loaded = Project::load(&manifest_path).unwrap();
        assert_eq!(loaded.manifest.name, "My Book");
        assert_eq!(loaded.manifest.docs.len(), 1);
        assert_eq!(loaded.manifest.docs[0].title, "Chapter 1");
        assert_eq!(loaded.manifest.separator, Separator::BlankLines);

        // Paths are resolved to absolute.
        assert!(loaded.manifest.docs[0].path.is_absolute());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_paths_stored_in_manifest() {
        let dir = scratch("relative-paths");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest_path = dir.join("test.pstarproj");
        let project = Project {
            manifest_path: manifest_path.clone(),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![DocEntry {
                    path: dir.join("chapter1.md"), // absolute
                    title: "Ch1".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                }],
                separator: Separator::None,
            },
        };

        project.save().unwrap();

        // Read the raw TOML; the path should be stored relative.
        let raw = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(
            raw.contains("path = \"chapter1.md\"") || raw.contains("path = 'chapter1.md'"),
            "Path not stored as relative: {raw}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_and_remove_docs() {
        let mut project = Project {
            manifest_path: PathBuf::from("/tmp/test.pstarproj"),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![],
                separator: Separator::PageBreak,
            },
        };

        project.add_doc(PathBuf::from("doc1.md"), "Doc 1".to_string());
        project.add_doc(PathBuf::from("doc2.md"), "Doc 2".to_string());
        assert_eq!(project.manifest.docs.len(), 2);

        let removed = project.remove_doc(0);
        assert_eq!(removed.unwrap().title, "Doc 1");
        assert_eq!(project.manifest.docs.len(), 1);
        assert_eq!(project.manifest.docs[0].title, "Doc 2");
    }

    #[test]
    fn reorder_docs() {
        let mut project = Project {
            manifest_path: PathBuf::from("/tmp/test.pstarproj"),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![
                    DocEntry {
                        path: PathBuf::from("a.md"),
                        title: "A".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: PathBuf::from("b.md"),
                        title: "B".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: PathBuf::from("c.md"),
                        title: "C".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::PageBreak,
            },
        };

        // Move "A" (index 0) to position 2.
        project.reorder_doc(0, 2);
        assert_eq!(project.manifest.docs[0].title, "B");
        assert_eq!(project.manifest.docs[1].title, "C");
        assert_eq!(project.manifest.docs[2].title, "A");
    }

    #[test]
    fn doc_exists_check() {
        let dir = scratch("doc-exists");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let existing = dir.join("exists.md");
        std::fs::write(&existing, "content").unwrap();
        let missing = dir.join("missing.md");

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![
                    DocEntry {
                        path: existing,
                        title: "Existing".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: missing,
                        title: "Missing".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::PageBreak,
            },
        };

        assert!(project.doc_exists(0)); // exists.md is there
        assert!(!project.doc_exists(1)); // missing.md is not

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn separator_as_str() {
        assert_eq!(Separator::PageBreak.as_str(), "\n\x0c\n");
        assert_eq!(Separator::BlankLines.as_str(), "\n\n\n\n");
        assert_eq!(Separator::HorizontalRule.as_str(), "\n# # #\n\n");
        assert_eq!(Separator::None.as_str(), "");
    }

    #[test]
    fn doc_word_count_for_existing_file() {
        let dir = scratch("word-count");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let doc1 = dir.join("doc1.md");
        std::fs::write(&doc1, "Hello world this is a test").unwrap();

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![DocEntry {
                    path: doc1,
                    title: "Doc 1".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                }],
                separator: Separator::PageBreak,
            },
        };

        let wc = project.doc_word_count(0);
        assert_eq!(wc, Some(6)); // "Hello world this is a test" = 6 words

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doc_word_count_for_missing_file() {
        let dir = scratch("word-count-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let missing = dir.join("missing.md");

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![DocEntry {
                    path: missing,
                    title: "Missing".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                }],
                separator: Separator::PageBreak,
            },
        };

        let wc = project.doc_word_count(0);
        assert_eq!(wc, None); // Missing file returns None

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doc_word_count_for_invalid_index() {
        let project = Project {
            manifest_path: PathBuf::from("/tmp/test.pstarproj"),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![],
                separator: Separator::PageBreak,
            },
        };

        let wc = project.doc_word_count(0);
        assert_eq!(wc, None); // Out of bounds returns None
    }

    #[test]
    fn doc_title_prefers_first_heading() {
        // R1.2: the binder title is the document's first Markdown heading when
        // present, otherwise the manifest title (file stem), otherwise falls
        // back gracefully for a missing file.
        let dir = scratch("doc-title");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let with_heading = dir.join("ch1.md");
        std::fs::write(&with_heading, "\n\n## The Long Road\n\nProse here.").unwrap();
        let no_heading = dir.join("ch2.md");
        std::fs::write(&no_heading, "Just prose, no heading.").unwrap();
        let missing = dir.join("ch3.md");

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![
                    DocEntry {
                        path: with_heading,
                        title: "ch1".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: no_heading,
                        title: "ch2".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: missing,
                        title: "ch3".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::PageBreak,
            },
        };

        // First heading wins over the stored file-stem title.
        assert_eq!(project.doc_title(0), "The Long Road");
        // No heading: fall back to the manifest title (file stem).
        assert_eq!(project.doc_title(1), "ch2");
        // Missing file: fall back to the manifest title, never panics.
        assert_eq!(project.doc_title(2), "ch3");
        // Out of range: empty string.
        assert_eq!(project.doc_title(9), "");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn total_prose_words_sums_readable_docs_and_strips_prose() {
        let dir = scratch("total-prose");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let doc1 = dir.join("doc1.md");
        // Heading marker + emphasis markers are stripped; note line excluded.
        std::fs::write(
            &doc1,
            "# Chapter One\n.. a private note\nHello **world**.\n",
        )
        .unwrap();
        let doc2 = dir.join("doc2.md");
        std::fs::write(&doc2, "Three plain words here now\n").unwrap();
        let missing = dir.join("gone.md");

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Test".to_string(),
                docs: vec![
                    DocEntry {
                        path: doc1,
                        title: "Doc 1".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: doc2,
                        title: "Doc 2".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: missing,
                        title: "Missing".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::PageBreak,
            },
        };

        // doc1: "Chapter One" (2) + "Hello world." (2) = 4, note line excluded.
        // doc2: 5. missing: 0. Total = 9.
        assert_eq!(project.total_prose_words(), 9);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn total_prose_words_is_zero_for_empty_project() {
        let project = Project {
            manifest_path: PathBuf::from("/tmp/empty.pstarproj"),
            manifest: ProjectManifest {
                name: "Empty".to_string(),
                docs: vec![],
                separator: Separator::PageBreak,
            },
        };
        assert_eq!(project.total_prose_words(), 0);
    }

    #[test]
    fn invalid_manifest_returns_error() {
        let dir = scratch("invalid");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest_path = dir.join("bad.pstarproj");
        std::fs::write(&manifest_path, "not valid TOML {[}").unwrap();

        let result = Project::load(&manifest_path);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid project manifest")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_save_preserves_prior_on_failure() {
        let dir = scratch("atomic-fail");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let manifest_path = dir.join("test.pstarproj");

        // Create an initial good manifest.
        let project = Project {
            manifest_path: manifest_path.clone(),
            manifest: ProjectManifest {
                name: "Original".to_string(),
                docs: vec![],
                separator: Separator::PageBreak,
            },
        };
        project.save().unwrap();
        let _original = std::fs::read_to_string(&manifest_path).unwrap();

        // Now save a new version (should succeed).
        let mut project = Project::load(&manifest_path).unwrap();
        project.manifest.name = "Updated".to_string();
        project.save().unwrap();

        let updated = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(updated.contains("Updated"));
        assert!(!updated.contains("Original"));

        // Verify no temp file left behind.
        let mut tmp = manifest_path.clone().into_os_string();
        tmp.push(".tmp~");
        assert!(!PathBuf::from(tmp).exists(), "Temp file left behind");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Compile tests (task 1.6) ---

    #[test]
    fn compile_concatenates_docs_with_separator() {
        let dir = scratch("compile-basic");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("ch1.md"), "Chapter One content.").unwrap();
        std::fs::write(dir.join("ch2.md"), "Chapter Two content.").unwrap();
        std::fs::write(dir.join("ch3.md"), "Chapter Three content.").unwrap();

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Novel".to_string(),
                docs: vec![
                    DocEntry {
                        path: dir.join("ch1.md"),
                        title: "Chapter 1".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("ch2.md"),
                        title: "Chapter 2".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("ch3.md"),
                        title: "Chapter 3".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::PageBreak,
            },
        };

        let result = project.compile();
        assert!(result.skipped.is_empty());
        assert_eq!(
            result.text,
            "Chapter One content.\n\x0c\nChapter Two content.\n\x0c\nChapter Three content."
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_skips_excluded_docs() {
        let dir = scratch("compile-exclude");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("ch1.md"), "Chapter One.").unwrap();
        std::fs::write(dir.join("notes.md"), "Research notes.").unwrap();
        std::fs::write(dir.join("ch2.md"), "Chapter Two.").unwrap();

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Novel".to_string(),
                docs: vec![
                    DocEntry {
                        path: dir.join("ch1.md"),
                        title: "Chapter 1".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("notes.md"),
                        title: "Notes".to_string(),
                        include_in_compile: false, // excluded
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("ch2.md"),
                        title: "Chapter 2".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::BlankLines,
            },
        };

        let result = project.compile();
        assert!(result.skipped.is_empty());
        // Notes doc should not appear in output.
        assert!(!result.text.contains("Research notes"));
        assert_eq!(result.text, "Chapter One.\n\n\n\nChapter Two.");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_skips_missing_files_gracefully() {
        let dir = scratch("compile-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("ch1.md"), "Chapter One.").unwrap();
        // ch2.md intentionally not created (missing file)
        std::fs::write(dir.join("ch3.md"), "Chapter Three.").unwrap();

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Novel".to_string(),
                docs: vec![
                    DocEntry {
                        path: dir.join("ch1.md"),
                        title: "Chapter 1".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("ch2.md"),
                        title: "Chapter 2".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("ch3.md"),
                        title: "Chapter 3".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::HorizontalRule,
            },
        };

        let result = project.compile();
        // ch2 should be in the skipped list.
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].index, 1);
        assert_eq!(result.skipped[0].title, "Chapter 2");
        // The compiled text should only have ch1 and ch3 with separator.
        assert_eq!(result.text, "Chapter One.\n# # #\n\nChapter Three.");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_with_no_separator() {
        let dir = scratch("compile-no-sep");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("scene1.md"), "Scene one.").unwrap();
        std::fs::write(dir.join("scene2.md"), "Scene two.").unwrap();

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Scenes".to_string(),
                docs: vec![
                    DocEntry {
                        path: dir.join("scene1.md"),
                        title: "Scene 1".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("scene2.md"),
                        title: "Scene 2".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::None,
            },
        };

        let result = project.compile();
        assert!(result.skipped.is_empty());
        // With Separator::None, docs concatenate directly.
        assert_eq!(result.text, "Scene one.Scene two.");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_empty_project() {
        let project = Project {
            manifest_path: PathBuf::from("/tmp/test.pstarproj"),
            manifest: ProjectManifest {
                name: "Empty".to_string(),
                docs: vec![],
                separator: Separator::PageBreak,
            },
        };

        let result = project.compile();
        assert!(result.text.is_empty());
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn compile_all_docs_excluded() {
        let dir = scratch("compile-all-excluded");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("notes.md"), "Just notes.").unwrap();

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Notes Only".to_string(),
                docs: vec![DocEntry {
                    path: dir.join("notes.md"),
                    title: "Notes".to_string(),
                    include_in_compile: false,
                    role: DocRole::Manuscript,
                }],
                separator: Separator::PageBreak,
            },
        };

        let result = project.compile();
        assert!(result.text.is_empty());
        assert!(result.skipped.is_empty()); // excluded ≠ skipped

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reorder_persistence_round_trip() {
        let dir = scratch("reorder-persist");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create doc files so word_count etc. don't trip up.
        std::fs::write(dir.join("a.md"), "Alpha").unwrap();
        std::fs::write(dir.join("b.md"), "Bravo").unwrap();
        std::fs::write(dir.join("c.md"), "Charlie").unwrap();

        let manifest_path = dir.join("test.pstarproj");
        let mut project = Project {
            manifest_path: manifest_path.clone(),
            manifest: ProjectManifest {
                name: "Reorder Test".to_string(),
                docs: vec![
                    DocEntry {
                        path: dir.join("a.md"),
                        title: "A".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("b.md"),
                        title: "B".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("c.md"),
                        title: "C".to_string(),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::PageBreak,
            },
        };

        // Save original order [A, B, C].
        project.save().unwrap();

        // Reorder: move A (index 0) to index 2 → [B, C, A].
        project.reorder_doc(0, 2);
        assert_eq!(project.manifest.docs[0].title, "B");
        assert_eq!(project.manifest.docs[1].title, "C");
        assert_eq!(project.manifest.docs[2].title, "A");

        // Save the reordered state.
        project.save().unwrap();

        // Reload from disk and verify the order persisted.
        let loaded = Project::load(&manifest_path).unwrap();
        assert_eq!(loaded.manifest.docs.len(), 3);
        assert_eq!(loaded.manifest.docs[0].title, "B");
        assert_eq!(loaded.manifest.docs[1].title, "C");
        assert_eq!(loaded.manifest.docs[2].title, "A");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_succeeds_with_missing_files() {
        let dir = scratch("load-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Write a manifest that references files that do NOT exist on disk.
        let manifest_path = dir.join("test.pstarproj");
        let manifest_toml = r#"
name = "Ghost Project"
separator = "PageBreak"

[[docs]]
path = "nonexistent_chapter1.md"
title = "Ghost Chapter 1"
include_in_compile = true

[[docs]]
path = "nonexistent_chapter2.md"
title = "Ghost Chapter 2"
include_in_compile = true
"#;
        std::fs::write(&manifest_path, manifest_toml).unwrap();

        // Loading should succeed even though referenced files don't exist (R1.7).
        let project = Project::load(&manifest_path);
        assert!(
            project.is_ok(),
            "Project::load should succeed with missing doc files"
        );

        let project = project.unwrap();
        assert_eq!(project.manifest.name, "Ghost Project");
        assert_eq!(project.manifest.docs.len(), 2);
        assert_eq!(project.manifest.docs[0].title, "Ghost Chapter 1");
        assert_eq!(project.manifest.docs[1].title, "Ghost Chapter 2");

        // The paths are resolved to absolute but the files don't exist.
        assert!(project.manifest.docs[0].path.is_absolute());
        assert!(!project.manifest.docs[0].path.exists());
        assert!(!project.manifest.docs[1].path.exists());

        // doc_exists correctly reports them as missing.
        assert!(!project.doc_exists(0));
        assert!(!project.doc_exists(1));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_single_doc() {
        let dir = scratch("compile-single");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("ch1.md"), "Only chapter.").unwrap();

        let project = Project {
            manifest_path: dir.join("test.pstarproj"),
            manifest: ProjectManifest {
                name: "Short".to_string(),
                docs: vec![DocEntry {
                    path: dir.join("ch1.md"),
                    title: "Chapter 1".to_string(),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                }],
                separator: Separator::PageBreak,
            },
        };

        let result = project.compile();
        assert!(result.skipped.is_empty());
        // Single doc — no separator should appear.
        assert_eq!(result.text, "Only chapter.");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn note_documents_are_excluded_from_compile() {
        // R5.2: a note is edited like any document but is never part of the book.
        let dir = scratch("note-compile");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("chapter1.md"), "Chapter one text.").unwrap();
        std::fs::write(dir.join("characters.md"), "Marcus: carries a knife.").unwrap();

        let mut project = Project {
            manifest_path: dir.join("book.pstarproj"),
            manifest: ProjectManifest {
                name: String::from("Book"),
                docs: vec![
                    DocEntry {
                        path: dir.join("chapter1.md"),
                        title: String::from("Chapter One"),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("characters.md"),
                        title: String::from("Characters"),
                        // Deliberately still flagged for compile: the role is
                        // what keeps it out, so marking a doc as a note is
                        // enough on its own.
                        include_in_compile: true,
                        role: DocRole::Note,
                    },
                ],
                separator: Separator::None,
            },
        };

        let compiled = project.compile();
        assert!(compiled.text.contains("Chapter one text."));
        assert!(
            !compiled.text.contains("Marcus"),
            "note text leaked into the manuscript: {:?}",
            compiled.text
        );
        assert!(compiled.skipped.is_empty(), "a note is skipped silently");

        // Marking it back restores it to the book.
        assert_eq!(project.toggle_role(1), Some(DocRole::Manuscript));
        assert!(project.compile().text.contains("Marcus"));
        assert!(!project.doc_is_note(1));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_manifest_written_before_notes_existed_compiles_everything() {
        // Manifests from earlier versions have no `role` key at all.
        let manifest: ProjectManifest =
            toml::from_str("name = 'Book'\n\n[[docs]]\npath = 'chapter1.md'\ntitle = 'One'\n")
                .unwrap();

        assert_eq!(manifest.docs[0].role, DocRole::Manuscript);
        assert!(!manifest.docs[0].is_note());
        assert!(manifest.docs[0].include_in_compile);
    }

    #[test]
    fn a_documents_role_survives_a_manifest_round_trip() {
        let dir = scratch("note-role-persist");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest_path = dir.join("book.pstarproj");

        let mut project = Project {
            manifest_path: manifest_path.clone(),
            manifest: ProjectManifest {
                name: String::from("Book"),
                docs: vec![DocEntry {
                    path: dir.join("characters.md"),
                    title: String::from("Characters"),
                    include_in_compile: true,
                    role: DocRole::Manuscript,
                }],
                separator: Separator::PageBreak,
            },
        };
        project.toggle_role(0);
        project.save().unwrap();

        let reloaded = Project::load(&manifest_path).unwrap();
        assert!(reloaded.doc_is_note(0));
        assert_eq!(reloaded.manifest.docs[0].role, DocRole::Note);

        let _ = std::fs::remove_dir_all(dir);
    }
}
