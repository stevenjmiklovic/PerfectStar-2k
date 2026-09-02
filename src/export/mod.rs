//! Shared, fully-offline document export infrastructure.
//!
//! Rich exporters consume [`CompiledDoc`], a line-oriented normalized model.
//! Notes are removed once, Markdown emphasis becomes explicit runs, and smart
//! typography is applied everywhere except literal inline-code runs. The
//! `clean_text` stream intentionally retains Markdown markers for compatibility
//! with the long-standing `^KE` plain-text command.

pub mod docx;
pub mod epub;
pub mod html;
mod zip;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::buffer::Buffer;
use crate::markdown::{self, MdKind};
use crate::normalize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<TextRun>),
    Heading {
        level: u8,
        runs: Vec<TextRun>,
    },
    PageBreak,
    /// Visual separation between compiled documents (from binder separators).
    /// Rich exporters render this as blank space or a rule; RTF emits a page
    /// break for PageBreak separators and blank lines for BlankLines.
    Separator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledDoc {
    pub blocks: Vec<Block>,
    /// Notes removed, otherwise byte-for-byte source-compatible with `^KE`.
    pub clean_text: String,
}

impl CompiledDoc {
    pub fn from_buffer(buf: &Buffer) -> Self {
        Self::build(&buf.rope.to_string(), false)
    }

    #[allow(dead_code)] // Used in tests and available for future callers.
    pub fn from_text(source: &str) -> Self {
        Self::build(source, false)
    }

    /// Build from project compilation output where form-feed lines are
    /// structural page breaks (from `Separator::PageBreak`) and blank-line
    /// runs of 4+ are document separators (from `Separator::BlankLines`).
    pub fn from_compiled(source: &str) -> Self {
        Self::build(source, true)
    }

    fn build(source: &str, interpret_separators: bool) -> Self {
        let mut blocks = Vec::new();
        let mut clean_text = String::with_capacity(source.len());

        for source_line in source.split_inclusive('\n') {
            let line = source_line.strip_suffix('\n').unwrap_or(source_line);
            let line = line.strip_suffix('\r').unwrap_or(line);
            if normalize::is_note_line(line) {
                continue;
            }
            clean_text.push_str(source_line);

            if interpret_separators && line.trim_matches([' ', '\t']) == "\u{c}" {
                blocks.push(Block::PageBreak);
                continue;
            }
            if line.trim().is_empty() {
                // In compiled project output, runs of blank lines from
                // Separator::BlankLines become explicit Separator blocks.
                if interpret_separators {
                    // Only emit one Separator per consecutive blank-line run.
                    if !matches!(blocks.last(), Some(Block::Separator)) {
                        blocks.push(Block::Separator);
                    }
                }
                continue;
            }
            if let Some((level, title)) = markdown::heading_level(line) {
                // Apply smart typography to heading text (R7.6) — same treatment
                // as body paragraphs so curly quotes/em dashes appear in all
                // export formats.
                let smartened = normalize::smart_typography(&title);
                blocks.push(Block::Heading {
                    level,
                    runs: vec![TextRun {
                        text: smartened,
                        bold: false,
                        italic: false,
                        code: false,
                    }],
                });
            } else {
                blocks.push(Block::Paragraph(parse_runs(line)));
            }
        }

        // A leading or trailing Separator from blank lines in the separator
        // padding is cosmetic noise — trim them.
        if matches!(blocks.first(), Some(Block::Separator)) {
            blocks.remove(0);
        }
        if matches!(blocks.last(), Some(Block::Separator)) {
            blocks.pop();
        }

        Self { blocks, clean_text }
    }

    pub fn plain_text(&self) -> &str {
        &self.clean_text
    }
}

fn parse_runs(line: &str) -> Vec<TextRun> {
    let chars: Vec<char> = line.chars().collect();
    let spans = markdown::scan_line(line);
    let mut tagged = Vec::with_capacity(chars.len());

    for (i, &ch) in chars.iter().enumerate() {
        let mut bold = false;
        let mut italic = false;
        let mut code = false;
        let mut marker = false;
        for &(start, end, kind) in &spans {
            if i >= start && i < end {
                match kind {
                    MdKind::Marker => marker = true,
                    MdKind::Bold => bold = true,
                    MdKind::Italic => italic = true,
                    MdKind::Code => code = true,
                    MdKind::Heading => {}
                }
            }
        }
        if !marker {
            tagged.push((ch, bold, italic, code));
        }
    }

    let source: Vec<char> = tagged.iter().map(|item| item.0).collect();
    let mut normalized = Vec::with_capacity(tagged.len());
    let mut i = 0;
    while i < tagged.len() {
        let (ch, bold, italic, code) = tagged[i];
        if code {
            normalized.push((ch, bold, italic, code));
            i += 1;
            continue;
        }
        let prev = if i == 0 { None } else { Some(source[i - 1]) };
        if let Some(sub) = normalize::smart_char(&source, i, prev) {
            normalized.push((sub.ch, bold, italic, code));
            i += sub.consumed;
        } else {
            normalized.push((ch, bold, italic, code));
            i += 1;
        }
    }

    let mut runs: Vec<TextRun> = Vec::new();
    for (ch, bold, italic, code) in normalized {
        if let Some(last) = runs.last_mut()
            && (last.bold, last.italic, last.code) == (bold, italic, code)
        {
            last.text.push(ch);
        } else {
            runs.push(TextRun {
                text: ch.to_string(),
                bold,
                italic,
                code,
            });
        }
    }
    runs
}

/// The export targets the unified format-selection step (R7.2) can offer.
/// Each variant maps to an existing single-document / project export path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Plain text with note lines stripped (the long-standing `^KE`).
    PlainText,
    /// Standard-manuscript-format RTF (`^KM`).
    Rtf,
    /// Word DOCX (`^KJ` / project `^PD`).
    Docx,
    /// EPUB 3 (`^KG` / project `^PF`).
    Epub,
    /// Structural HTML (`^KL` / project `^PH`).
    Html,
}

impl Target {
    /// The human-readable name shown in the selection list and status area.
    pub fn label(self) -> &'static str {
        match self {
            Target::PlainText => "Plain text",
            Target::Rtf => "Manuscript RTF",
            Target::Docx => "DOCX",
            Target::Epub => "EPUB",
            Target::Html => "HTML",
        }
    }

    /// The dependency this format needs beyond the bundled binary, if any.
    ///
    /// Every current exporter is hand-generated with no external tool or crate
    /// (ADR-006, ADR-013), so this is always `None` today. It is the single
    /// enforcement point for R7.8: a format whose dependency is unbundled
    /// returns its name here, is omitted from the selection list, and has that
    /// name stated in the status area rather than being offered and failing.
    pub fn missing_dependency(self) -> Option<&'static str> {
        match self {
            Target::PlainText | Target::Rtf | Target::Docx | Target::Epub | Target::Html => None,
        }
    }

    /// Whether this format can be produced by the current build.
    pub fn is_available(self) -> bool {
        self.missing_dependency().is_none()
    }
}

/// Every export target in selection-list order. The single source of truth the
/// format-selection step (R7.2) and its availability gate (R7.8) draw from.
pub const TARGETS: &[Target] = &[
    Target::Rtf,
    Target::Docx,
    Target::Epub,
    Target::Html,
    Target::PlainText,
];

/// A format offered in the selection list: an available target (R7.2). Formats
/// whose dependency is unbundled are omitted here and reported separately via
/// [`unavailable_targets`] (R7.8).
pub fn available_targets() -> Vec<Target> {
    TARGETS
        .iter()
        .copied()
        .filter(|t| t.is_available())
        .collect()
}

/// The (target, missing-dependency) pairs omitted from the selection list, so a
/// caller can state the missing dependency name in the status area (R7.8).
pub fn unavailable_targets() -> Vec<(Target, &'static str)> {
    TARGETS
        .iter()
        .copied()
        .filter_map(|t| t.missing_dependency().map(|dep| (t, dep)))
        .collect()
}

pub trait Exporter {
    /// Produce the complete destination bytes without touching the output path.
    fn render(&self, doc: &CompiledDoc) -> io::Result<Vec<u8>>;

    /// Replace `out` only after complete rendering and a successful temp write.
    fn export(&self, doc: &CompiledDoc, out: &Path) -> io::Result<()> {
        let bytes = self.render(doc)?;
        atomic_write(out, &bytes)
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_export_with(path, |file| file.write_all(bytes))
}

/// Atomic export primitive exposed for failure-path tests and format writers.
pub fn atomic_export_with<F>(path: &Path, write: F) -> io::Result<()>
where
    F: FnOnce(&mut fs::File) -> io::Result<()>,
{
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "export path has no file name")
    })?;
    let mut temp = PathBuf::from(parent);
    temp.push(format!(
        ".{}.pstar-export-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        write(&mut file)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(crate) fn runs_text(runs: &[TextRun]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

pub(crate) fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn compiles_structure_emphasis_typography_and_notes() {
        let doc =
            CompiledDoc::from_text("# Chapter\n.. private\nA **bold** and *fine* -- `x--y`.\n");
        assert_eq!(doc.blocks.len(), 2);
        assert!(matches!(&doc.blocks[0], Block::Heading { level: 1, .. }));
        let Block::Paragraph(runs) = &doc.blocks[1] else {
            panic!("paragraph expected")
        };
        assert!(runs.iter().any(|run| run.bold && run.text == "bold"));
        assert!(runs.iter().any(|run| run.italic && run.text == "fine"));
        assert!(runs.iter().any(|run| run.text.contains('—')));
        assert!(runs.iter().any(|run| run.code && run.text == "x--y"));
        assert!(!doc.clean_text.contains("private"));
        assert!(doc.clean_text.contains("**bold**"));
    }

    #[test]
    fn project_compile_order_and_separator_reach_export_model() {
        use crate::project::{DocEntry, DocRole, Project, ProjectManifest, Separator};

        let dir = std::env::temp_dir().join(format!("pstar-export-order-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("second.md"), "# Second\nBravo.\n").unwrap();
        fs::write(dir.join("first.md"), "# First\nAlpha.\n").unwrap();
        let project = Project {
            manifest_path: dir.join("book.pstarproj"),
            manifest: ProjectManifest {
                name: String::from("Book"),
                docs: vec![
                    DocEntry {
                        path: dir.join("first.md"),
                        title: String::from("First"),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("second.md"),
                        title: String::from("Second"),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::PageBreak,
            },
        };

        let compiled = project.compile();
        let doc = CompiledDoc::from_compiled(&compiled.text);
        let headings: Vec<String> = doc
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Heading { runs, .. } => Some(runs_text(runs)),
                _ => None,
            })
            .collect();
        assert_eq!(headings, ["First", "Second"]);
        assert!(
            doc.blocks
                .iter()
                .any(|block| matches!(block, Block::PageBreak))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_order_and_separator_survive_all_exporters() {
        use crate::project::{DocEntry, DocRole, Project, ProjectManifest, Separator};

        let dir = std::env::temp_dir().join(format!("pstar-export-all-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("first.md"), "# First\n.. private note\nAlpha.\n").unwrap();
        fs::write(dir.join("second.md"), "# Second\nBravo.\n").unwrap();
        let project = Project {
            manifest_path: dir.join("book.pstarproj"),
            manifest: ProjectManifest {
                name: String::from("Book"),
                docs: vec![
                    DocEntry {
                        path: dir.join("first.md"),
                        title: String::from("First"),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("second.md"),
                        title: String::from("Second"),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::PageBreak,
            },
        };
        let compiled = project.compile();
        assert!(compiled.skipped.is_empty());
        let doc = CompiledDoc::from_compiled(&compiled.text);
        let outputs = [
            ("DOCX", docx::DocxExporter.render(&doc).unwrap()),
            ("EPUB", epub::EpubExporter.render(&doc).unwrap()),
            ("HTML", html::HtmlExporter.render(&doc).unwrap()),
            ("plain text", html::PlainTextExporter.render(&doc).unwrap()),
            (
                "RTF",
                crate::rtf::RtfExporter {
                    font: crate::rtf::ManuscriptFont::TimesNewRoman,
                }
                .render(&doc)
                .unwrap(),
            ),
        ];
        for (label, bytes) in &outputs {
            let first = bytes
                .windows(b"First".len())
                .position(|window| window == b"First")
                .unwrap_or_else(|| panic!("{label} omitted the first document"));
            let second = bytes
                .windows(b"Second".len())
                .position(|window| window == b"Second")
                .unwrap_or_else(|| panic!("{label} omitted the second document"));
            assert!(first < second, "{label} changed binder order");
            assert!(
                !bytes
                    .windows(b"private note".len())
                    .any(|window| window == b"private note"),
                "{label} exported a note line"
            );
        }
        assert!(
            outputs[0]
                .1
                .windows(b"w:type=\"page\"".len())
                .any(|w| w == b"w:type=\"page\"")
        );
        assert!(
            outputs[1]
                .1
                .windows(b"class=\"page-break\"".len())
                .any(|w| w == b"class=\"page-break\"")
        );
        assert!(
            outputs[2]
                .1
                .windows(b"class=\"page-break\"".len())
                .any(|w| w == b"class=\"page-break\"")
        );
        assert!(outputs[3].1.contains(&b'\x0c'));
        assert!(
            outputs[4]
                .1
                .windows(b"\\page".len())
                .any(|w| w == b"\\page")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blank_lines_separator_becomes_separator_block() {
        use crate::project::{DocEntry, DocRole, Project, ProjectManifest, Separator};

        let dir = std::env::temp_dir().join(format!("pstar-export-blsep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.md"), "Alpha.\n").unwrap();
        fs::write(dir.join("b.md"), "Bravo.\n").unwrap();
        let project = Project {
            manifest_path: dir.join("book.pstarproj"),
            manifest: ProjectManifest {
                name: String::from("Book"),
                docs: vec![
                    DocEntry {
                        path: dir.join("a.md"),
                        title: String::from("A"),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                    DocEntry {
                        path: dir.join("b.md"),
                        title: String::from("B"),
                        include_in_compile: true,
                        role: DocRole::Manuscript,
                    },
                ],
                separator: Separator::BlankLines,
            },
        };
        let compiled = project.compile();
        let doc = CompiledDoc::from_compiled(&compiled.text);
        assert!(
            doc.blocks.iter().any(|b| matches!(b, Block::Separator)),
            "BlankLines separator should produce a Separator block"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn form_feed_in_single_doc_is_not_page_break() {
        // A user document with a literal form-feed line should NOT become a
        // PageBreak in a single-doc export (only project compile interprets it).
        let doc = CompiledDoc::from_text("Before.\n\x0c\nAfter.\n");
        assert!(
            !doc.blocks.iter().any(|b| matches!(b, Block::PageBreak)),
            "Single-doc export must not interpret form-feed as PageBreak"
        );
    }

    #[test]
    fn all_five_targets_are_available_today() {
        // Every exporter is hand-generated (ADR-006/ADR-013), so the selection
        // list offers all five formats and omits none (R7.2).
        let available = available_targets();
        assert_eq!(available.len(), 5);
        for t in [
            Target::Rtf,
            Target::Docx,
            Target::Epub,
            Target::Html,
            Target::PlainText,
        ] {
            assert!(available.contains(&t), "{} should be available", t.label());
            assert!(t.is_available());
            assert_eq!(t.missing_dependency(), None);
        }
        assert!(unavailable_targets().is_empty());
    }

    #[test]
    fn every_target_has_a_display_label() {
        // The selection list and the R7.8 status message both need a name.
        for t in TARGETS {
            assert!(!t.label().is_empty(), "{t:?} has no label");
        }
    }

    fn prose_line_strategy() -> impl Strategy<Value = String> {
        prop::string::string_regex("[a-z]{1,8}")
            .expect("valid word regex")
            .prop_map(|word| format!("{word} \"quoted\" wait--stop and wait...stop '{word}'"))
    }

    fn note_line_strategy() -> impl Strategy<Value = String> {
        (
            prop::sample::select(vec!["", " ", "\t  "]),
            prop::string::string_regex("[a-z]{1,8}").expect("valid word regex"),
        )
            .prop_map(|(indent, word)| {
                format!("{indent}.. NOTE_ONLY_{word} \"secret\" -- hidden...")
            })
    }

    fn source_text_strategy() -> impl Strategy<Value = String> {
        (
            prose_line_strategy(),
            prop::collection::vec(
                (any::<bool>(), prose_line_strategy(), note_line_strategy()),
                0..=8,
            ),
        )
            .prop_map(|(first_prose, rest)| {
                let mut lines = vec![first_prose];
                lines.extend(
                    rest.into_iter()
                        .map(|(is_note, prose, note)| if is_note { note } else { prose }),
                );
                format!("{}\n", lines.join("\n"))
            })
    }

    // Feature: pro-writer-10-star, Property 17: Annotations never reach a prose export
    // Validates: Requirements 9.3
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn annotation_bodies_never_reach_any_prose_export(
            source in source_text_strategy(),
            annotation_bodies in prop::collection::vec(
                prop::string::string_regex("[a-z]{1,24}").expect("valid annotation body regex"),
                1..=8,
            ),
        ) {
            let mut sidecar = crate::meta::DocMeta::default();
            sidecar.annotations = annotation_bodies
                .iter()
                .enumerate()
                .map(|(index, body)| {
                    crate::meta::Annotation::new(
                        index,
                        0,
                        format!("ANNOTATION_BODY_{index}_{body}"),
                    )
                })
                .collect();
            let doc = CompiledDoc::from_text(&source);
            let outputs = [
                ("DOCX", docx::DocxExporter.render(&doc).unwrap()),
                ("EPUB", epub::EpubExporter.render(&doc).unwrap()),
                ("HTML", html::HtmlExporter.render(&doc).unwrap()),
                ("plain text", html::PlainTextExporter.render(&doc).unwrap()),
                (
                    "RTF",
                    crate::rtf::RtfExporter {
                        font: crate::rtf::ManuscriptFont::TimesNewRoman,
                    }
                    .render(&doc)
                    .unwrap(),
                ),
            ];

            prop_assert_eq!(sidecar.annotations.len(), annotation_bodies.len());
            for annotation in &sidecar.annotations {
                for (label, output) in &outputs {
                    prop_assert!(
                        !output
                            .windows(annotation.text.len())
                            .any(|window| window == annotation.text.as_bytes()),
                        "{label} exported annotation body {:?}",
                        annotation.text,
                    );
                }
            }
        }
    }

    // Feature: pro-writer-10-star, Property 18: Export normalization strips notes and upgrades typography
    // Validates: Requirements 7.5, 7.7
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn export_normalization_strips_notes_and_upgrades_typography(
            source in source_text_strategy(),
        ) {
            let doc = CompiledDoc::from_text(&source);
            let expected: Vec<String> = source
                .lines()
                .filter(|line| !normalize::is_note_line(line))
                .map(normalize::smart_typography)
                .collect();
            let actual: Vec<String> = doc
                .blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Paragraph(runs) | Block::Heading { runs, .. } => {
                        Some(runs_text(runs))
                    }
                    Block::PageBreak | Block::Separator => None,
                })
                .collect();

            prop_assert_eq!(&actual, &expected);
            prop_assert!(
                !actual.iter().any(|line| line.contains("NOTE_ONLY_")),
                "note-only source lines must not reach the normalized export model"
            );
            let open_quote = '\u{201c}';
            let close_quote = '\u{201d}';
            let em_dash = '\u{2014}';
            let ellipsis = '\u{2026}';
            prop_assert!(actual.iter().all(|line| line.contains(open_quote)));
            prop_assert!(actual.iter().all(|line| line.contains(close_quote)));
            prop_assert!(actual.iter().all(|line| line.contains(em_dash)));
            prop_assert!(actual.iter().all(|line| line.contains(ellipsis)));
        }
    }

    // Feature: pro-writer-10-star, Property 19: Failed export preserves any previously exported file
    // Validates: Requirements 7.9
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn failed_export_preserves_any_previous_output(
            previous_output in prop::collection::vec(any::<u8>(), 0..=256),
            partial_output in prop::collection::vec(any::<u8>(), 1..=256),
        ) {
            let dir = std::env::temp_dir().join(format!(
                "pstar-export-failure-property-{}",
                std::process::id(),
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let out = dir.join("book.export");
            fs::write(&out, &previous_output).unwrap();

            let err = atomic_export_with(&out, |file| {
                file.write_all(&partial_output)?;
                Err(io::Error::other("simulated renderer failure"))
            });
            prop_assert!(err.is_err());
            prop_assert_eq!(fs::read(&out).unwrap(), previous_output);

            let leftovers: Vec<_> = fs::read_dir(&dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .filter(|name| name.to_string_lossy().contains("pstar-export"))
                .collect();
            prop_assert!(
                leftovers.is_empty(),
                "temporary export files remain: {leftovers:?}"
            );
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn failed_atomic_export_preserves_previous_output() {
        let dir = std::env::temp_dir().join(format!("pstar-export-failure-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("book.html");
        fs::write(&out, b"previous good export").unwrap();

        let err = atomic_export_with(&out, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("simulated renderer failure"))
        });
        assert!(err.is_err());
        assert_eq!(fs::read(&out).unwrap(), b"previous good export");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains("pstar-export"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary export files remain: {leftovers:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
