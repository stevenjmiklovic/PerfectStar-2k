use std::io;

use super::zip::{self, Entry};
use super::{Block, CompiledDoc, Exporter, TextRun, escape_xml};

pub struct DocxExporter;

impl Exporter for DocxExporter {
    fn render(&self, doc: &CompiledDoc) -> io::Result<Vec<u8>> {
        let document = document_xml(doc);
        zip::archive(&[
            Entry {
                name: "[Content_Types].xml",
                data: CONTENT_TYPES.as_bytes(),
            },
            Entry {
                name: "_rels/.rels",
                data: RELS.as_bytes(),
            },
            Entry {
                name: "word/_rels/document.xml.rels",
                data: DOCUMENT_RELS.as_bytes(),
            },
            Entry {
                name: "word/document.xml",
                data: document.as_bytes(),
            },
            Entry {
                name: "word/styles.xml",
                data: STYLES.as_bytes(),
            },
        ])
    }
}

fn document_xml(doc: &CompiledDoc) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>",
    );
    for block in &doc.blocks {
        match block {
            Block::Heading { level, runs } => {
                out.push_str(&format!(
                    "<w:p><w:pPr><w:pStyle w:val=\"Heading{level}\"/></w:pPr>"
                ));
                render_runs(&mut out, runs);
                out.push_str("</w:p>");
            }
            Block::Paragraph(runs) => {
                out.push_str("<w:p>");
                render_runs(&mut out, runs);
                out.push_str("</w:p>");
            }
            Block::PageBreak => out.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>"),
            Block::Separator => {
                // Visual separator between compiled documents — an empty paragraph.
                out.push_str(
                    "<w:p><w:pPr><w:spacing w:before=\"240\" w:after=\"240\"/></w:pPr></w:p>",
                );
            }
        }
    }
    out.push_str("<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/></w:sectPr></w:body></w:document>");
    out
}

fn render_runs(out: &mut String, runs: &[TextRun]) {
    for run in runs {
        out.push_str("<w:r>");
        if run.bold || run.italic || run.code {
            out.push_str("<w:rPr>");
            if run.bold {
                out.push_str("<w:b/>");
            }
            if run.italic {
                out.push_str("<w:i/>");
            }
            if run.code {
                out.push_str("<w:rFonts w:ascii=\"Courier New\" w:hAnsi=\"Courier New\"/>");
            }
            out.push_str("</w:rPr>");
        }
        out.push_str("<w:t xml:space=\"preserve\">");
        out.push_str(&escape_xml(&run.text));
        out.push_str("</w:t></w:r>");
    }
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/></Types>";
const RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";
const DOCUMENT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>";
const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/><w:pPr><w:outlineLvl w:val=\"0\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"32\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:outlineLvl w:val=\"1\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"28\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading3\"><w:name w:val=\"heading 3\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:outlineLvl w:val=\"2\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"26\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading4\"><w:name w:val=\"heading 4\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:outlineLvl w:val=\"3\"/></w:pPr><w:rPr><w:b/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading5\"><w:name w:val=\"heading 5\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:outlineLvl w:val=\"4\"/></w:pPr><w:rPr><w:b/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading6\"><w:name w:val=\"heading 6\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:outlineLvl w:val=\"5\"/></w:pPr><w:rPr><w:b/></w:rPr></w:style></w:styles>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_xml_matches_golden() {
        let doc = CompiledDoc::from_text("# One\nA **bold** & *fine* -- line.\n.. note\n");
        assert_eq!(
            document_xml(&doc),
            include_str!("../../tests/fixtures/export/document.xml.golden")
        );
        assert!(
            DocxExporter
                .render(&doc)
                .unwrap()
                .starts_with(b"PK\x03\x04")
        );
    }
}
