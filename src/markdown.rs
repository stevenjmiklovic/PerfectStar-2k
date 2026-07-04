//! Line-local inline Markdown scanning: bold, italic, code spans, and
//! heading lines. Markers stay visible (dimmed) — no cursor trickery — and
//! the same scan drives both the styled view and Reveal Codes.

/// What a run of characters is, in char offsets within the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdKind {
    /// Markup punctuation: `**`, `*`, backticks, leading `#`s.
    Marker,
    Bold,
    Italic,
    Code,
    Heading,
}

/// Char-range annotations for one line. Ranges are non-overlapping and
/// ascending; unannotated chars are plain text.
pub fn scan_line(text: &str) -> Vec<(usize, usize, MdKind)> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();

    // Heading: 1-6 #'s then a space.
    let hashes = chars.iter().take_while(|&&c| c == '#').count();
    if (1..=6).contains(&hashes) && chars.get(hashes) == Some(&' ') {
        out.push((0, hashes + 1, MdKind::Marker));
        out.push((hashes + 1, n, MdKind::Heading));
        return out;
    }

    let mut i = 0;
    while i < n {
        match chars[i] {
            '`' => {
                if let Some(j) = find_char(&chars, '`', i + 1) {
                    out.push((i, i + 1, MdKind::Marker));
                    if j > i + 1 {
                        out.push((i + 1, j, MdKind::Code));
                    }
                    out.push((j, j + 1, MdKind::Marker));
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
            '*' if i + 1 < n && chars[i + 1] == '*' => {
                if let Some(j) = find_double_star(&chars, i + 2) {
                    if j > i + 2 {
                        out.push((i, i + 2, MdKind::Marker));
                        out.push((i + 2, j, MdKind::Bold));
                        out.push((j, j + 2, MdKind::Marker));
                        i = j + 2;
                    } else {
                        i += 2;
                    }
                } else {
                    i += 2;
                }
            }
            '*' => {
                match find_char(&chars, '*', i + 1) {
                    Some(j)
                        if j > i + 1
                            && !chars[i + 1].is_whitespace()
                            && !chars[j - 1].is_whitespace() =>
                    {
                        out.push((i, i + 1, MdKind::Marker));
                        out.push((i + 1, j, MdKind::Italic));
                        out.push((j, j + 1, MdKind::Marker));
                        i = j + 1;
                    }
                    _ => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// If `text` is a heading line ("# Title" through "###### Title"), its level
/// (1-6) and trimmed title text.
pub fn heading_level(text: &str) -> Option<(u8, String)> {
    let hashes = text.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && text.chars().nth(hashes) == Some(' ') {
        let title: String = text.chars().skip(hashes + 1).collect();
        Some((hashes as u8, title.trim_end().to_string()))
    } else {
        None
    }
}

fn find_char(chars: &[char], target: char, from: usize) -> Option<usize> {
    (from..chars.len()).find(|&k| chars[k] == target)
}

fn find_double_star(chars: &[char], from: usize) -> Option<usize> {
    let n = chars.len();
    let mut k = from;
    while k + 1 < n {
        if chars[k] == '*' && chars[k + 1] == '*' {
            return Some(k);
        }
        k += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading() {
        let spans = scan_line("## Title here");
        assert_eq!(spans, vec![(0, 3, MdKind::Marker), (3, 13, MdKind::Heading)]);
    }

    #[test]
    fn not_a_heading_without_space() {
        assert!(scan_line("#hashtag").is_empty());
    }

    #[test]
    fn bold_and_italic() {
        let spans = scan_line("a **b** and *c*");
        assert_eq!(
            spans,
            vec![
                (2, 4, MdKind::Marker),
                (4, 5, MdKind::Bold),
                (5, 7, MdKind::Marker),
                (12, 13, MdKind::Marker),
                (13, 14, MdKind::Italic),
                (14, 15, MdKind::Marker),
            ]
        );
    }

    #[test]
    fn code_span_suppresses_inner_markers() {
        let spans = scan_line("`**x**`");
        assert_eq!(
            spans,
            vec![
                (0, 1, MdKind::Marker),
                (1, 6, MdKind::Code),
                (6, 7, MdKind::Marker),
            ]
        );
    }

    #[test]
    fn unclosed_markers_stay_plain() {
        assert!(scan_line("2 * 3 = 6").is_empty());
        assert!(scan_line("a ** b").is_empty());
        assert!(scan_line("`code").is_empty());
    }

    #[test]
    fn italic_needs_tight_edges() {
        // "* 3 *" has spaces inside — not emphasis.
        assert!(scan_line("2 * 3 * 4").is_empty());
    }

    #[test]
    fn heading_level_parses() {
        assert_eq!(heading_level("# Chapter One"), Some((1, String::from("Chapter One"))));
        assert_eq!(heading_level("### Scene   "), Some((3, String::from("Scene"))));
        assert_eq!(heading_level("#nope"), None);
        assert_eq!(heading_level("just text"), None);
    }
}
