//! Outline navigation: jump between Markdown headings in the document,
//! browsed with the same searchable-list UI as the command palette.

use crate::buffer::Buffer;
use crate::markdown;

pub struct Entry {
    pub level: u8,
    pub title: String,
    /// 0-indexed document line the heading is on.
    pub line: usize,
    /// Char index of the heading title, just past the "# " marker.
    pub char_pos: usize,
}

/// Every Markdown heading in the document, in document order.
pub fn scan(buf: &Buffer) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in 0..buf.len_lines() {
        let text = buf.line_text(line);
        if let Some((level, title)) = markdown::heading_level(&text) {
            let marker_len = level as usize + 1; // '#' * level, then a space
            out.push(Entry {
                level,
                title,
                line,
                char_pos: buf.line_start(line) + marker_len,
            });
        }
    }
    out
}
