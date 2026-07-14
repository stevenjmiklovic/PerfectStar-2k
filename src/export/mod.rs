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
        use crate::project::{DocEntry, Project, ProjectManifest, Separator};

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
                    },
                    DocEntry {
                        path: dir.join("second.md"),
                        title: String::from("Second"),
                        include_in_compile: true,
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
    fn blank_lines_separator_becomes_separator_block() {
        use crate::project::{DocEntry, Project, ProjectManifest, Separator};

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
                    },
                    DocEntry {
                        path: dir.join("b.md"),
                        title: String::from("B"),
                        include_in_compile: true,
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
        let _ = fs::remove_dir_all(&dir);
    }
}
