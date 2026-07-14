//! One definition of "prose" for the whole app.
//!
//! Three separate places used to decide, each in their own way, what counts as
//! deliverable prose versus editing scaffolding: the RTF exporter stripped
//! `..` notes and Markdown markers and smartened quotes; `^KE` and the status
//! line each re-tested `..` by hand. When those definitions drift, the word
//! count disagrees with the export, which disagrees with what the reader sees.
//!
//! This module is the single source of truth for:
//!   - which lines are **notes** (never part of the prose) — [`is_note_line`],
//!   - stripping **Markdown markers** (`*`, `**`, backticks, heading `#`s) so
//!     only the emphasized/plain text remains — [`strip_markers`],
//!   - **typographic substitution** of straight quotes/dashes/ellipses into
//!     their Unicode forms — [`smart_char`] / [`smart_typography`].
//!
//! Stats, export, and word-count all consume these so "prose" means the same
//! thing everywhere. Behavior matches what `rtf.rs` did before extraction, so
//! `^KM`/`^KE` are unchanged.

/// Whether a line is a note-to-self (`..` at the start, after optional
/// leading whitespace) rather than deliverable prose. Notes never reach a
/// reader — they're stripped from every export and excluded from prose counts.
pub fn is_note_line(line: &str) -> bool {
    line.trim_start().starts_with("..")
}

/// Strip Markdown markup punctuation from a line, returning just the text a
/// reader sees: emphasis/code/heading *content* with the `*`, `**`, backtick,
/// and leading `#` markers removed. Uses the same line scanner that drives the
/// styled view and Reveal Codes, so what's stripped here is exactly what's
/// dimmed on screen.
// Consumed by the prose word-count (§4.2) and plain/HTML exports (§4.7), which
// land in later tasks. Allowed dead until then.
#[allow(dead_code)]
pub fn strip_markers(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let spans = crate::markdown::scan_line(line);
    let mut out = String::with_capacity(line.len());
    for (i, &c) in chars.iter().enumerate() {
        let is_marker = spans
            .iter()
            .any(|&(s, e, kind)| i >= s && i < e && kind == crate::markdown::MdKind::Marker);
        if !is_marker {
            out.push(c);
        }
    }
    out
}

/// Whether a character opens (rather than closes) a quotation, judged by what
/// precedes it: start-of-text, whitespace, or an opening bracket / dash.
pub fn opens_quote(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(p) => p.is_whitespace() || matches!(p, '(' | '[' | '{' | '\u{2014}' | '\u{2013}'),
    }
}

/// The result of matching a typographic substitution at some position.
pub struct Sub {
    /// The replacement character to emit.
    pub ch: char,
    /// How many source characters it consumed.
    pub consumed: usize,
}

/// Try to match a typographic substitution starting at `chars[i]`:
/// `--`→em dash, `...`→ellipsis, straight quotes→curly quotes/apostrophes.
///
/// Returns the replacement and how many source chars it consumed, or `None`
/// when the char passes through unchanged. `prev` is the char immediately
/// before `chars[i]` in the *output stream* (for open/close quote decisions).
/// Callers that treat some spans as literal (e.g. inline code) should skip
/// this and pass those chars through verbatim.
pub fn smart_char(chars: &[char], i: usize, prev: Option<char>) -> Option<Sub> {
    match chars[i] {
        '-' if run_len(chars, i, '-') >= 2 => Some(Sub {
            ch: '\u{2014}',
            consumed: run_len(chars, i, '-'),
        }),
        '.' if run_len(chars, i, '.') >= 3 => {
            // Exactly three dots → ellipsis; extra dots stay literal.
            Some(Sub {
                ch: '\u{2026}',
                consumed: 3,
            })
        }
        '"' => Some(Sub {
            ch: if opens_quote(prev) {
                '\u{201C}'
            } else {
                '\u{201D}'
            },
            consumed: 1,
        }),
        '\'' => Some(Sub {
            ch: if opens_quote(prev) {
                '\u{2018}'
            } else {
                '\u{2019}'
            },
            consumed: 1,
        }),
        _ => None,
    }
}

/// Apply [`smart_char`] across a whole string, returning the smartened text.
/// Used where there's no per-character emphasis to preserve (stats, plain
/// exports); `rtf.rs` drives [`smart_char`] directly to carry emphasis through.
// The string-level entry point's consumers (plain/HTML export, autocorrect
// typographic substitution) land in later tasks. Allowed dead until then.
#[allow(dead_code)]
pub fn smart_typography(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let prev = out.chars().last();
        if let Some(sub) = smart_char(&chars, i, prev) {
            out.push(sub.ch);
            i += sub.consumed;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Length of the run of `target` starting at index `i`.
fn run_len(chars: &[char], i: usize, target: char) -> usize {
    let mut n = 0;
    while i + n < chars.len() && chars[i + n] == target {
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_line_detection() {
        assert!(is_note_line(".. fix later"));
        assert!(is_note_line("   .. indented note"));
        assert!(is_note_line("..no space"));
        assert!(!is_note_line("Real prose."));
        assert!(!is_note_line("A sentence ending in ..."));
        assert!(!is_note_line(". single dot"));
    }

    #[test]
    fn strip_markers_removes_emphasis_punctuation() {
        assert_eq!(strip_markers("a **b** and *c*"), "a b and c");
        assert_eq!(strip_markers("run `x` now"), "run x now");
        assert_eq!(strip_markers("## Heading"), "Heading");
    }

    #[test]
    fn strip_markers_leaves_plain_text_untouched() {
        assert_eq!(strip_markers("2 * 3 = 6"), "2 * 3 = 6");
        assert_eq!(strip_markers("plain line"), "plain line");
    }

    #[test]
    fn em_dash_and_ellipsis() {
        assert_eq!(smart_typography("wait--stop"), "wait\u{2014}stop");
        assert_eq!(smart_typography("er...um"), "er\u{2026}um");
        // Four dots: three collapse to an ellipsis, the fourth stays.
        assert_eq!(smart_typography("done...."), "done\u{2026}.");
    }

    #[test]
    fn curly_quotes_open_and_close() {
        assert_eq!(smart_typography("\"hi\""), "\u{201C}hi\u{201D}");
        // Apostrophe mid-word closes (it follows a letter).
        assert_eq!(smart_typography("it's"), "it\u{2019}s");
    }

    #[test]
    fn quote_after_dash_opens() {
        // A quote right after an em dash should open, not close.
        let out = smart_typography("--\"yes\"");
        assert!(out.starts_with("\u{2014}\u{201C}yes"), "got {out}");
    }

    #[test]
    fn single_dash_and_double_dot_are_literal() {
        assert_eq!(smart_typography("a-b"), "a-b");
        assert_eq!(smart_typography("a..b"), "a..b");
    }
}
