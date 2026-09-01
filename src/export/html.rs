use std::io;

use super::{Block, CompiledDoc, Exporter, TextRun, escape_xml};

pub struct HtmlExporter;
pub struct PlainTextExporter;

impl Exporter for HtmlExporter {
    fn render(&self, doc: &CompiledDoc) -> io::Result<Vec<u8>> {
        let mut out = String::from(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>PerfectStar Export</title>\n</head>\n<body>\n",
        );
        for block in &doc.blocks {
            match block {
                Block::Heading { level, runs } => {
                    out.push_str(&format!("<h{level}>"));
                    render_runs(&mut out, runs);
                    out.push_str(&format!("</h{level}>\n"));
                }
                Block::Paragraph(runs) => {
                    out.push_str("<p>");
                    render_runs(&mut out, runs);
                    out.push_str("</p>\n");
                }
                Block::PageBreak => out.push_str("<hr class=\"page-break\">\n"),
                Block::Separator => out.push_str("<hr class=\"separator\">\n"),
            }
        }
        out.push_str("</body>\n</html>\n");
        Ok(out.into_bytes())
    }
}

impl Exporter for PlainTextExporter {
    fn render(&self, doc: &CompiledDoc) -> io::Result<Vec<u8>> {
        Ok(doc.plain_text().as_bytes().to_vec())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_matches_golden() {
        let doc = CompiledDoc::from_text("# One\nA **bold** & *fine* -- line.\n.. note\n");
        let actual = String::from_utf8(HtmlExporter.render(&doc).unwrap()).unwrap();
        assert_eq!(
            actual,
            include_str!("../../tests/fixtures/export/book.html.golden")
        );
    }

    #[test]
    fn plain_text_retains_clean_export_behavior() {
        let doc = CompiledDoc::from_text("A **bold** line.\n.. note\n");
        assert_eq!(
            PlainTextExporter.render(&doc).unwrap(),
            include_bytes!("../../tests/fixtures/export/book.txt.golden")
        );
    }
}
