use std::io;

use super::zip::{self, Entry};
use super::{Block, CompiledDoc, Exporter, TextRun, escape_xml, runs_text};

pub struct EpubExporter;

impl Exporter for EpubExporter {
    fn render(&self, doc: &CompiledDoc) -> io::Result<Vec<u8>> {
        let content = content_xhtml(doc);
        let nav = nav_xhtml(doc);
        zip::archive(&[
            // EPUB requires this to be the first, uncompressed entry.
            Entry {
                name: "mimetype",
                data: b"application/epub+zip",
            },
            Entry {
                name: "META-INF/container.xml",
                data: CONTAINER.as_bytes(),
            },
            Entry {
                name: "OEBPS/package.opf",
                data: PACKAGE.as_bytes(),
            },
            Entry {
                name: "OEBPS/content.xhtml",
                data: content.as_bytes(),
            },
            Entry {
                name: "OEBPS/nav.xhtml",
                data: nav.as_bytes(),
            },
        ])
    }
}

fn content_xhtml(doc: &CompiledDoc) -> String {
    let mut out = xhtml_start("PerfectStar Export");
    let mut heading = 0usize;
    for block in &doc.blocks {
        match block {
            Block::Heading { level, runs } => {
                heading += 1;
                out.push_str(&format!("<h{level} id=\"heading-{heading}\">"));
                render_runs(&mut out, runs);
                out.push_str(&format!("</h{level}>\n"));
            }
            Block::Paragraph(runs) => {
                out.push_str("<p>");
                render_runs(&mut out, runs);
                out.push_str("</p>\n");
            }
            Block::PageBreak => out.push_str("<hr class=\"page-break\"/>\n"),
            Block::Separator => out.push_str("<hr class=\"separator\"/>\n"),
        }
    }
    out.push_str("</body></html>\n");
    out
}

fn nav_xhtml(doc: &CompiledDoc) -> String {
    let mut out = xhtml_start("Contents");
    out.push_str("<nav epub:type=\"toc\" id=\"toc\"><h1>Contents</h1><ol>\n");
    let mut heading = 0usize;
    for block in &doc.blocks {
        if let Block::Heading { level, runs } = block {
            heading += 1;
            if *level <= 2 {
                out.push_str(&format!(
                    "<li><a href=\"content.xhtml#heading-{heading}\">{}</a></li>\n",
                    escape_xml(&runs_text(runs))
                ));
            }
        }
    }
    out.push_str("</ol></nav>\n</body></html>\n");
    out
}

fn xhtml_start(title: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"en\"><head><meta charset=\"utf-8\"/><title>{}</title></head><body>\n",
        escape_xml(title)
    )
}

fn render_runs(out: &mut String, runs: &[TextRun]) {
    for run in runs {
        if run.bold {
            out.push_str("<strong>");
        }
        if run.italic {
            out.push_str("<em>");
        }
        if run.code {
            out.push_str("<code>");
        }
        out.push_str(&escape_xml(&run.text));
        if run.code {
            out.push_str("</code>");
        }
        if run.italic {
            out.push_str("</em>");
        }
        if run.bold {
            out.push_str("</strong>");
        }
    }
}

const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/package.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
const PACKAGE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"book-id\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"book-id\">urn:uuid:perfectstar-export</dc:identifier><dc:title>PerfectStar Export</dc:title><dc:language>en</dc:language><meta property=\"dcterms:modified\">1980-01-01T00:00:00Z</meta></metadata><manifest><item id=\"content\" href=\"content.xhtml\" media-type=\"application/xhtml+xml\"/><item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/></manifest><spine><itemref idref=\"content\"/></spine></package>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epub_content_and_toc_match_goldens() {
        let doc = CompiledDoc::from_text("# One\nA **bold** & *fine* -- line.\n## Two\n.. note\n");
        assert_eq!(
            content_xhtml(&doc),
            include_str!("../../tests/fixtures/export/content.xhtml.golden")
        );
        assert_eq!(
            nav_xhtml(&doc),
            include_str!("../../tests/fixtures/export/nav.xhtml.golden")
        );
        let epub = EpubExporter.render(&doc).unwrap();
        assert!(epub.starts_with(b"PK\x03\x04"));
        assert!(
            epub.windows(b"application/epub+zip".len())
                .any(|w| w == b"application/epub+zip")
        );
    }

    #[test]
    fn navigation_contains_only_heading_levels_one_and_two() {
        let nav = nav_xhtml(&CompiledDoc::from_text(
            "# One\n### Subsection\n## Two\n#### Detail\n",
        ));
        assert!(nav.contains(">One</a>"));
        assert!(nav.contains("content.xhtml#heading-3\">Two</a>"));
        assert!(!nav.contains("Subsection"));
        assert!(!nav.contains("Detail"));
    }
}
