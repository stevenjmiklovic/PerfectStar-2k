use std::borrow::Cow;
use std::io;
use std::path::PathBuf;

use ropey::Rope;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const TAB_WIDTH: usize = 4;

/// The text buffer: a rope plus its backing file.
///
/// All positions are char indices into the rope. Grapheme-cluster helpers
/// keep cursor movement and deletion from splitting user-perceived characters.
pub struct Buffer {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// A `.bak` copy is made once per run, before the first overwrite.
    backed_up: bool,
}

impl Buffer {
    pub fn open(path: Option<PathBuf>) -> io::Result<Self> {
        let rope = match &path {
            Some(p) if p.exists() => {
                let file = std::fs::File::open(p)?;
                Rope::from_reader(io::BufReader::new(file))?
            }
            _ => Rope::new(),
        };
        Ok(Buffer {
            rope,
            path,
            dirty: false,
            backed_up: false,
        })
    }

    /// Atomic save: back up the original once per run, write to a temp file
    /// in the same directory, then rename over the original.
    pub fn save(&mut self) -> io::Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| io::Error::other("no file name"))?;
        if !self.backed_up && path.exists() {
            let mut bak = path.clone().into_os_string();
            bak.push(".bak");
            let _ = std::fs::copy(&path, PathBuf::from(bak));
            self.backed_up = true;
        }
        // Stream the rope straight into the temp file so a huge manuscript is
        // never fully copied into a byte buffer just to save. The temp-then-
        // rename crash-safety lives in the shared helper (R11.5).
        crate::paths::write_atomic_with(&path, |file| {
            self.rope.write_to(io::BufWriter::new(file))
        })?;
        self.dirty = false;
        Ok(())
    }

    /// Words in the document (runs of word characters).
    pub fn word_count(&self) -> usize {
        let mut count = 0usize;
        let mut in_word = false;
        for c in self.rope.chars() {
            let w = c.is_alphanumeric();
            if w && !in_word {
                count += 1;
            }
            in_word = w;
        }
        count
    }

    pub fn file_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("[no file]"))
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn insert(&mut self, at: usize, text: &str) {
        self.rope.insert(at, text);
        self.dirty = true;
    }

    pub fn delete(&mut self, range: std::ops::Range<usize>) -> String {
        let removed = self.rope.slice(range.clone()).to_string();
        self.rope.remove(range);
        self.dirty = true;
        removed
    }

    pub fn line_of(&self, char_idx: usize) -> usize {
        self.rope.char_to_line(char_idx)
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    /// Char index of the end of a line's text, excluding its newline.
    pub fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        // Trim the line terminator (\n or \r\n) if present.
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        start + len
    }

    /// The line's text without its terminator.
    pub fn line_text(&self, line: usize) -> Cow<'_, str> {
        let start = self.line_start(line);
        let end = self.line_end(line);
        Cow::from(self.rope.slice(start..end))
    }

    /// Previous grapheme boundary before `char_idx` (0 if at start).
    pub fn prev_grapheme(&self, char_idx: usize) -> usize {
        if char_idx == 0 {
            return 0;
        }
        let line = self.line_of(char_idx);
        let start = self.line_start(line);
        if char_idx == start {
            // Step over the previous line's terminator.
            let prev_end = self.line_end(line - 1);
            return prev_end;
        }
        let text = self.line_text(line);
        let col = char_idx - start;
        let mut prev = start;
        for (off, g) in text.grapheme_indices(true) {
            let g_start = text[..off].chars().count();
            let g_chars = g.chars().count();
            if g_start + g_chars >= col {
                prev = start + g_start;
                break;
            }
            prev = start + g_start + g_chars;
        }
        prev
    }

    /// Next grapheme boundary after `char_idx` (len_chars if at end).
    pub fn next_grapheme(&self, char_idx: usize) -> usize {
        let len = self.len_chars();
        if char_idx >= len {
            return len;
        }
        let line = self.line_of(char_idx);
        let end = self.line_end(line);
        if char_idx >= end {
            // Step over this line's terminator to the next line start.
            return (self.line_start(line + 1)).min(len);
        }
        let start = self.line_start(line);
        let text = self.line_text(line);
        let col = char_idx - start;
        for (off, g) in text.grapheme_indices(true) {
            let g_start = text[..off].chars().count();
            if g_start >= col {
                return start + g_start + g.chars().count();
            }
        }
        end
    }

    fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_' || c == '\''
    }

    /// Start of the next word to the right (WordStar ^F).
    pub fn word_right(&self, char_idx: usize) -> usize {
        let len = self.len_chars();
        let mut i = char_idx;
        // Skip the rest of the current word, then any gap, landing on a word start.
        while i < len && Self::is_word_char(self.rope.char(i)) {
            i += 1;
        }
        while i < len && !Self::is_word_char(self.rope.char(i)) {
            i += 1;
        }
        i
    }

    /// Start of the previous word to the left (WordStar ^A).
    pub fn word_left(&self, char_idx: usize) -> usize {
        let mut i = char_idx;
        while i > 0 && !Self::is_word_char(self.rope.char(i - 1)) {
            i -= 1;
        }
        while i > 0 && Self::is_word_char(self.rope.char(i - 1)) {
            i -= 1;
        }
        i
    }

    fn is_blank_line(&self, line: usize) -> bool {
        self.line_text(line).chars().all(|c| c.is_whitespace())
    }

    /// Start of the next paragraph (blank-line separated).
    pub fn para_fwd(&self, char_idx: usize) -> usize {
        let last = self.len_lines().saturating_sub(1);
        let mut line = self.line_of(char_idx);
        while line < last && !self.is_blank_line(line) {
            line += 1;
        }
        while line < last && self.is_blank_line(line) {
            line += 1;
        }
        if line == last && self.is_blank_line(line) {
            return self.len_chars();
        }
        self.line_start(line)
    }

    /// Start of the current paragraph, or the previous one if already there.
    pub fn para_back(&self, char_idx: usize) -> usize {
        let mut line = self.line_of(char_idx);
        // Step out of any blank region first.
        while line > 0 && self.is_blank_line(line) {
            line -= 1;
        }
        while line > 0 && !self.is_blank_line(line - 1) {
            line -= 1;
        }
        let start = self.line_start(line);
        if start < char_idx {
            return start;
        }
        // Already at a paragraph start: go to the previous paragraph.
        if line == 0 {
            return 0;
        }
        let mut line = line - 1;
        while line > 0 && self.is_blank_line(line) {
            line -= 1;
        }
        while line > 0 && !self.is_blank_line(line - 1) {
            line -= 1;
        }
        self.line_start(line)
    }

    fn is_sentence_end(&self, i: usize) -> bool {
        // A sentence ends at . ! or ? (possibly followed by closing quotes or
        // brackets) that is followed by whitespace or end of document.
        if !matches!(self.rope.char(i), '.' | '!' | '?') {
            return false;
        }
        let len = self.len_chars();
        let mut j = i + 1;
        while j < len && matches!(self.rope.char(j), '"' | '\'' | '\u{2019}' | '\u{201d}' | ')' | ']') {
            j += 1;
        }
        j >= len || self.rope.char(j).is_whitespace()
    }

    /// Position just past a sentence end: after the punctuation and any
    /// closing quotes/brackets.
    fn after_sentence_end(&self, i: usize) -> usize {
        let len = self.len_chars();
        let mut j = i + 1;
        while j < len && matches!(self.rope.char(j), '"' | '\'' | '\u{2019}' | '\u{201d}' | ')' | ']') {
            j += 1;
        }
        j
    }

    /// Start of the next sentence.
    pub fn sentence_fwd(&self, char_idx: usize) -> usize {
        let len = self.len_chars();
        let mut i = char_idx;
        while i < len {
            if self.is_sentence_end(i) {
                let mut j = self.after_sentence_end(i);
                while j < len && self.rope.char(j).is_whitespace() {
                    j += 1;
                }
                if j > char_idx && j < len {
                    return j;
                }
                i = j.max(i + 1);
                continue;
            }
            i += 1;
        }
        len
    }

    /// Start of the current sentence, or the previous one if already there.
    pub fn sentence_back(&self, char_idx: usize) -> usize {
        if char_idx == 0 {
            return 0;
        }
        // Walk back to before any whitespace immediately behind the cursor.
        let mut i = char_idx;
        while i > 0 && self.rope.char(i - 1).is_whitespace() {
            i -= 1;
        }
        // Now scan backward for the previous sentence end; the sentence we
        // want starts at the first non-whitespace after it.
        let mut k = i;
        while k > 0 {
            let c = self.rope.char(k - 1);
            if c.is_whitespace() {
                // Candidate boundary: check what precedes the whitespace run.
                let mut w = k - 1;
                while w > 0 && self.rope.char(w - 1).is_whitespace() {
                    w -= 1;
                }
                if w > 0 && self.is_sentence_end_backwards(w) {
                    let start = k;
                    if start < char_idx {
                        return start;
                    }
                }
                k = w;
            } else {
                k -= 1;
            }
        }
        0
    }

    /// Whether the non-whitespace text ending at char index `end` (exclusive)
    /// finishes a sentence.
    fn is_sentence_end_backwards(&self, end: usize) -> bool {
        let mut j = end;
        while j > 0
            && matches!(self.rope.char(j - 1), '"' | '\'' | '\u{2019}' | '\u{201d}' | ')' | ']')
        {
            j -= 1;
        }
        j > 0 && matches!(self.rope.char(j - 1), '.' | '!' | '?')
    }

    /// Find `query` starting at `from` (char index), scanning forward.
    /// Case-insensitive unless the query contains an uppercase letter
    /// ("smartcase"). Returns the char index of the match start.
    pub fn find(&self, query: &str, from: usize, whole_word: bool) -> Option<usize> {
        if query.is_empty() {
            return None;
        }
        let fold = !query.chars().any(|c| c.is_uppercase());
        let q: Vec<char> = if fold {
            query.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
        } else {
            query.chars().collect()
        };
        let len = self.len_chars();
        if q.len() > len {
            return None;
        }
        'outer: for start in from..=(len - q.len()) {
            for (k, qc) in q.iter().enumerate() {
                let mut c = self.rope.char(start + k);
                if fold {
                    c = c.to_lowercase().next().unwrap_or(c);
                }
                if c != *qc {
                    continue 'outer;
                }
            }
            if whole_word {
                let before_ok = start == 0 || !Self::is_word_char(self.rope.char(start - 1));
                let end = start + q.len();
                let after_ok = end >= len || !Self::is_word_char(self.rope.char(end));
                if !(before_ok && after_ok) {
                    continue;
                }
            }
            return Some(start);
        }
        None
    }

    /// Visual (display) column of a char position within its line,
    /// accounting for grapheme widths and tab stops.
    pub fn visual_col(&self, char_idx: usize) -> usize {
        let line = self.line_of(char_idx);
        let start = self.line_start(line);
        let text = self.line_text(line);
        let col = char_idx - start;
        let mut vcol = 0usize;
        let mut chars_seen = 0usize;
        for g in text.graphemes(true) {
            if chars_seen >= col {
                break;
            }
            vcol += grapheme_width(g, vcol);
            chars_seen += g.chars().count();
        }
        vcol
    }

    /// Char index within `line` closest to the given visual column.
    pub fn char_at_visual_col(&self, line: usize, goal: usize) -> usize {
        let start = self.line_start(line);
        let text = self.line_text(line);
        let mut vcol = 0usize;
        let mut chars_seen = 0usize;
        for g in text.graphemes(true) {
            let w = grapheme_width(g, vcol);
            if vcol + w > goal {
                return start + chars_seen;
            }
            vcol += w;
            chars_seen += g.chars().count();
        }
        start + chars_seen
    }
}

/// Display width of a grapheme at a given visual column (tabs advance to the
/// next tab stop).
pub fn grapheme_width(g: &str, at_vcol: usize) -> usize {
    if g == "\t" {
        TAB_WIDTH - (at_vcol % TAB_WIDTH)
    } else {
        UnicodeWidthStr::width(g).max(1)
    }
}

/// Break a line into soft-wrap segments of at most `width` visual columns,
/// preferring to break after a space. Returns contiguous char ranges
/// covering the whole line; an empty line yields one empty segment.
pub fn wrap_segments(text: &str, width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let mut segs: Vec<(usize, usize)> = Vec::new();
    let mut seg_start = 0usize; // char index
    let mut vcol = 0usize;
    let mut char_idx = 0usize;
    let mut break_at: Option<usize> = None; // char idx just after the last space

    for g in text.graphemes(true) {
        let g_chars = g.chars().count();
        let w = grapheme_width(g, vcol);
        if vcol + w > width && char_idx > seg_start {
            // Overflow: break at the last space if we saw one, else here.
            let cut = match break_at {
                Some(b) if b > seg_start => b,
                _ => char_idx,
            };
            segs.push((seg_start, cut));
            seg_start = cut;
            break_at = None;
            // Recompute the column of everything after the cut. Chars from
            // cut..char_idx were already measured; remeasure them from col 0.
            vcol = remeasure(text, cut, char_idx);
        }
        vcol += grapheme_width(g, vcol);
        char_idx += g_chars;
        if g.chars().all(|c| c == ' ') {
            break_at = Some(char_idx);
        }
    }
    segs.push((seg_start, char_idx));
    segs
}

/// Visual column of char offset `to` within a wrap segment starting at char
/// offset `from` (i.e. width of chars in `from..to` measured from column 0).
pub fn segment_vcol(text: &str, from: usize, to: usize) -> usize {
    remeasure(text, from, to)
}

/// Visual width of chars in `from..to` measured from column zero.
fn remeasure(text: &str, from: usize, to: usize) -> usize {
    let mut vcol = 0usize;
    let mut idx = 0usize;
    for g in text.graphemes(true) {
        let n = g.chars().count();
        if idx >= to {
            break;
        }
        if idx >= from {
            vcol += grapheme_width(g, vcol);
        }
        idx += n;
    }
    vcol
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        Buffer {
            rope: Rope::from_str(text),
            path: None,
            dirty: false,
            backed_up: false,
        }
    }

    #[test]
    fn line_end_excludes_newline() {
        let b = buf("hello\nworld\n");
        assert_eq!(b.line_end(0), 5);
        assert_eq!(b.line_end(1), 11);
    }

    #[test]
    fn grapheme_movement_over_newline() {
        let b = buf("ab\ncd");
        assert_eq!(b.next_grapheme(2), 3); // over the \n
        assert_eq!(b.prev_grapheme(3), 2); // back over it
    }

    #[test]
    fn grapheme_movement_combining() {
        // "e" + combining acute is one grapheme, two chars.
        let b = buf("e\u{301}x");
        assert_eq!(b.next_grapheme(0), 2);
        assert_eq!(b.prev_grapheme(2), 0);
    }

    #[test]
    fn word_motion() {
        let b = buf("one two, three");
        assert_eq!(b.word_right(0), 4); // start of "two"
        assert_eq!(b.word_right(4), 9); // start of "three" (past comma+space)
        assert_eq!(b.word_left(9), 4);
        assert_eq!(b.word_left(4), 0);
    }

    #[test]
    fn visual_col_wide_chars() {
        let b = buf("日本語");
        assert_eq!(b.visual_col(2), 4); // two double-width chars
        assert_eq!(b.char_at_visual_col(0, 4), 2);
        // Landing mid-wide-char snaps to its start.
        assert_eq!(b.char_at_visual_col(0, 3), 1);
    }

    #[test]
    fn paragraph_motion() {
        let b = buf("First para line one.\nLine two.\n\nSecond para.\n\n\nThird.\n");
        assert_eq!(b.para_fwd(0), 32); // start of "Second para."
        assert_eq!(b.para_back(35), 32); // mid-second-para -> its start
        assert_eq!(b.para_back(32), 0); // at start -> previous para
    }

    #[test]
    fn sentence_motion() {
        let text = "One two. Three four! \"Five.\" Six?\nSeven.";
        let b = buf(text);
        assert_eq!(b.sentence_fwd(0), 9); // start of "Three"
        assert_eq!(b.sentence_fwd(9), 21); // start of "\"Five..." (after '!')
        assert_eq!(b.sentence_back(9), 0);
        assert_eq!(b.sentence_back(15), 9); // mid-sentence -> its start
    }

    #[test]
    fn find_smartcase() {
        let b = buf("The Word and the word again.");
        assert_eq!(b.find("word", 0, false), Some(4)); // folds case
        assert_eq!(b.find("Word", 0, false), Some(4)); // exact case
        assert_eq!(b.find("word", 5, false), Some(17));
        assert_eq!(b.find("Word", 5, false), None);
    }

    #[test]
    fn find_whole_word() {
        let b = buf("sword word wordy");
        assert_eq!(b.find("word", 0, true), Some(6));
        assert_eq!(b.find("word", 7, true), None);
    }

    #[test]
    fn visual_col_tabs() {
        let b = buf("\tx");
        assert_eq!(b.visual_col(1), TAB_WIDTH);
    }

    #[test]
    fn wrap_breaks_at_spaces() {
        // width 10: "hello brave world" -> "hello " / "brave " / "world"
        let segs = wrap_segments("hello brave world", 10);
        assert_eq!(segs, vec![(0, 6), (6, 12), (12, 17)]);
    }

    #[test]
    fn wrap_hard_breaks_long_words() {
        let segs = wrap_segments("abcdefghij", 4);
        assert_eq!(segs, vec![(0, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn wrap_empty_line_is_one_segment() {
        assert_eq!(wrap_segments("", 10), vec![(0, 0)]);
    }

    #[test]
    fn wrap_exact_fit_no_extra_segment() {
        assert_eq!(wrap_segments("abcd", 4), vec![(0, 4)]);
    }
}
