//! Project-wide search & replace (R6).
//!
//! Searches every document in the project manifest (streaming from disk for
//! unopened files). Results are surfaced as a navigable list; selecting a
//! result opens the document and places the cursor at the match. Replace
//! opens each affected file as a proper pane so edits are undoable and go
//! through atomic save — no silent unreviewable writes (R6.6).

use std::path::{Path, PathBuf};

/// One match in a project-wide search.
#[derive(Debug, Clone)]
#[allow(dead_code)] // doc_idx used in tests and future UI features.
pub struct Match {
    /// Index into `manifest.docs`.
    pub doc_idx: usize,
    /// Document title for display.
    pub title: String,
    /// Document path.
    pub path: PathBuf,
    /// Char offset of the match within the file.
    pub char_pos: usize,
    /// 1-based line number.
    pub line: usize,
    /// Context: the line text containing the match (trimmed).
    pub context: String,
}

/// Search all project documents for `query`. Uses smartcase (case-insensitive
/// unless query contains uppercase) and optional whole-word matching —
/// consistent with `Buffer::find` semantics (R6.5).
///
/// For files not currently open in a pane, reads from disk. For the active
/// pane's file, searches the in-memory rope directly for up-to-date results.
pub fn search_project(
    docs: &[(usize, String, PathBuf)],
    query: &str,
    whole_word: bool,
    active_path: Option<&Path>,
    active_rope: Option<&ropey::Rope>,
) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let fold = !query.chars().any(|c| c.is_uppercase());
    let q: Vec<char> = if fold {
        query
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect()
    } else {
        query.chars().collect()
    };

    let mut results = Vec::new();

    for (doc_idx, title, path) in docs {
        // Use the in-memory rope if this is the active document.
        let use_rope = active_path.is_some() && active_path.unwrap() == path.as_path();
        let text: String;
        let chars: Vec<char>;

        if use_rope {
            let rope = active_rope.unwrap();
            // Collect chars from rope for searching.
            chars = rope.chars().collect();
        } else {
            text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(_) => continue, // Skip unreadable files (R1.7 resilience).
            };
            chars = text.chars().collect();
        }

        find_matches_in(
            &chars,
            &q,
            fold,
            whole_word,
            *doc_idx,
            title.clone(),
            path.clone(),
            &mut results,
        );
    }

    results
}

#[allow(clippy::too_many_arguments)]
fn find_matches_in(
    chars: &[char],
    q: &[char],
    fold: bool,
    whole_word: bool,
    doc_idx: usize,
    title: String,
    path: PathBuf,
    results: &mut Vec<Match>,
) {
    let len = chars.len();
    if q.len() > len {
        return;
    }

    let mut start = 0;
    'outer: while start <= len - q.len() {
        for (k, &qc) in q.iter().enumerate() {
            let mut c = chars[start + k];
            if fold {
                c = c.to_lowercase().next().unwrap_or(c);
            }
            if c != qc {
                start += 1;
                continue 'outer;
            }
        }

        // Whole-word boundary check.
        if whole_word {
            let before_ok = start == 0 || !is_word_char(chars[start - 1]);
            let end = start + q.len();
            let after_ok = end >= len || !is_word_char(chars[end]);
            if !(before_ok && after_ok) {
                start += 1;
                continue;
            }
        }

        // Compute line number and context.
        let line = chars[..start].iter().filter(|&&c| c == '\n').count() + 1;
        let line_start = chars[..start]
            .iter()
            .rposition(|&c| c == '\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        let line_end = chars[start..]
            .iter()
            .position(|&c| c == '\n')
            .map(|p| start + p)
            .unwrap_or(len);
        let context = build_context(chars, line_start, line_end, start, start + q.len());

        results.push(Match {
            doc_idx,
            title: title.clone(),
            path: path.clone(),
            char_pos: start,
            line,
            context,
        });

        start += q.len().max(1);
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Number of context characters shown on each side of a match (R6.1).
const CONTEXT_RADIUS: usize = 40;

/// Build the display context for a match: the surrounding text on the match's
/// line, clipped to at most `CONTEXT_RADIUS` characters on either side of the
/// match (R6.1). A clip on either end is marked with an ellipsis so the reader
/// knows the line continues.
fn build_context(
    chars: &[char],
    line_start: usize,
    line_end: usize,
    match_start: usize,
    match_end: usize,
) -> String {
    // Trim leading whitespace on the line so short lines read cleanly, but keep
    // the window anchored on the match for long lines.
    let mut ctx_start = line_start;
    while ctx_start < match_start && chars[ctx_start].is_whitespace() {
        ctx_start += 1;
    }
    let clip_start = match_start.saturating_sub(CONTEXT_RADIUS).max(ctx_start);
    let clip_end = (match_end + CONTEXT_RADIUS).min(line_end);

    let mut out = String::new();
    if clip_start > ctx_start {
        out.push('…');
    }
    let slice: String = chars[clip_start..clip_end].iter().collect();
    out.push_str(slice.trim_end());
    if clip_end < line_end {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static PROPERTY_SEARCH_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn search_finds_matches_across_files() {
        let dir = std::env::temp_dir().join(format!("pstar-projsearch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ch1.md"), "Alice met Bob.\nBob smiled.\n").unwrap();
        fs::write(dir.join("ch2.md"), "Bob left.\n").unwrap();

        let docs = vec![
            (0, String::from("Chapter 1"), dir.join("ch1.md")),
            (1, String::from("Chapter 2"), dir.join("ch2.md")),
        ];

        let results = search_project(&docs, "Bob", false, None, None);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[1].line, 2);
        assert_eq!(results[2].doc_idx, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_smartcase() {
        let dir = std::env::temp_dir().join(format!("pstar-projsearch-sc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.md"), "Hello hello HELLO.\n").unwrap();

        let docs = vec![(0, String::from("A"), dir.join("a.md"))];

        // Lowercase query → case-insensitive.
        let results = search_project(&docs, "hello", false, None, None);
        assert_eq!(results.len(), 3);

        // Query with uppercase → case-sensitive.
        let results = search_project(&docs, "Hello", false, None, None);
        assert_eq!(results.len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_whole_word() {
        let dir = std::env::temp_dir().join(format!("pstar-projsearch-ww-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.md"), "cat catch concatenate\n").unwrap();

        let docs = vec![(0, String::from("A"), dir.join("a.md"))];

        let results = search_project(&docs, "cat", true, None, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].char_pos, 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn context_is_bounded_to_forty_chars_each_side() {
        let dir = std::env::temp_dir().join(format!("pstar-projsearch-ctx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // A long single line with the match in the middle.
        let before = "x".repeat(100);
        let after = "y".repeat(100);
        fs::write(dir.join("a.md"), format!("{before}NEEDLE{after}\n")).unwrap();

        let docs = vec![(0, String::from("A"), dir.join("a.md"))];
        let results = search_project(&docs, "NEEDLE", false, None, None);
        assert_eq!(results.len(), 1);
        let ctx = &results[0].context;
        // Ellipsis on both ends because the line is clipped.
        assert!(
            ctx.starts_with('…'),
            "context should be clipped left: {ctx}"
        );
        assert!(ctx.ends_with('…'), "context should be clipped right: {ctx}");
        assert!(ctx.contains("NEEDLE"));
        // 40 x's + NEEDLE + 40 y's, plus two ellipsis markers.
        let non_ellipsis = ctx.chars().filter(|&c| c != '…').count();
        assert_eq!(non_ellipsis, 40 + "NEEDLE".chars().count() + 40);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn context_short_line_has_no_ellipsis() {
        let dir =
            std::env::temp_dir().join(format!("pstar-projsearch-ctx2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.md"), "  Alice met Bob here.\n").unwrap();

        let docs = vec![(0, String::from("A"), dir.join("a.md"))];
        let results = search_project(&docs, "Bob", false, None, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].context, "Alice met Bob here.");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_skips_missing_files() {
        let docs = vec![(
            0,
            String::from("Ghost"),
            PathBuf::from("/nonexistent/ghost.md"),
        )];
        let results = search_project(&docs, "anything", false, None, None);
        assert!(results.is_empty());
    }

    fn query_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(vec!['a', 'b', 'c', 'd']), 1..=5)
            .prop_map(|chars| chars.into_iter().collect())
    }

    fn document_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop::sample::select(vec![
                'a', 'b', 'c', 'd', 'A', 'B', 'C', 'D', ' ', '\n', '_', '-', '.', ',',
            ]),
            0..=96,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    fn uppercase_ascii(value: &str) -> String {
        value.chars().map(|c| c.to_ascii_uppercase()).collect()
    }

    fn expected_occurrences(text: &str, query: &str, whole_word: bool) -> Vec<(usize, usize)> {
        let fold = !query.chars().any(|c| c.is_uppercase());
        let haystack = if fold {
            text.to_ascii_lowercase()
        } else {
            text.to_owned()
        };
        let needle = if fold {
            query.to_ascii_lowercase()
        } else {
            query.to_owned()
        };
        let chars: Vec<char> = text.chars().collect();
        let query_len = needle.chars().count();

        haystack
            .match_indices(&needle)
            .filter_map(|(byte_pos, _)| {
                let char_pos = haystack[..byte_pos].chars().count();
                if whole_word {
                    let before_ok = char_pos == 0 || !is_word_char(chars[char_pos - 1]);
                    let end = char_pos + query_len;
                    let after_ok = end >= chars.len() || !is_word_char(chars[end]);
                    if !(before_ok && after_ok) {
                        return None;
                    }
                }
                let line = chars[..char_pos].iter().filter(|&&c| c == '\n').count() + 1;
                Some((char_pos, line))
            })
            .collect()
    }

    // Feature: pro-writer-10-star, Property 20: Project search finds all and only the matching occurrences
    // Validates: Requirements 6.1, 6.3, 1.7
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            .. ProptestConfig::default()
        })]

        #[test]
        fn project_search_finds_all_and_only_matching_occurrences(
            query_base in query_strategy(),
            uppercase_query in any::<bool>(),
            whole_word in any::<bool>(),
            random_bodies in prop::collection::vec(document_strategy(), 1..=4),
        ) {
            let query = if uppercase_query {
                uppercase_ascii(&query_base)
            } else {
                query_base.clone()
            };
            let id = PROPERTY_SEARCH_ID.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "pstar-projsearch-prop-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();

            let texts: Vec<String> = random_bodies
                .into_iter()
                .enumerate()
                .map(|(doc_idx, body)| {
                    let variant = if doc_idx % 2 == 0 {
                        query_base.clone()
                    } else {
                        uppercase_ascii(&query_base)
                    };
                    format!("{body}\n{variant} {variant}x x{variant}\n")
                })
                .collect();
            let docs: Vec<(usize, String, PathBuf)> = texts
                .iter()
                .enumerate()
                .map(|(doc_idx, text)| {
                    let path = dir.join(format!("doc-{doc_idx}.md"));
                    fs::write(&path, text).unwrap();
                    (doc_idx, format!("Document {doc_idx}"), path)
                })
                .chain(std::iter::once((
                    99_999,
                    String::from("Missing"),
                    dir.join("missing.md"),
                )))
                .collect();

            let active_rope = ropey::Rope::from_str(&texts[0]);
            let results = search_project(
                &docs,
                &query,
                whole_word,
                Some(&docs[0].2),
                Some(&active_rope),
            );

            let mut expected = Vec::new();
            for (doc_idx, text) in texts.iter().enumerate() {
                for (char_pos, line) in expected_occurrences(text, &query, whole_word) {
                    expected.push((doc_idx, char_pos, line));
                }
            }
            let actual: Vec<(usize, usize, usize)> = results
                .iter()
                .map(|result| (result.doc_idx, result.char_pos, result.line))
                .collect();

            let _ = fs::remove_dir_all(&dir);
            prop_assert_eq!(actual, expected);
        }
    }
}
