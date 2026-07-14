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
        let context: String = chars[line_start..line_end].iter().collect();
        let context = context.trim().to_string();

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
    fn search_skips_missing_files() {
        let docs = vec![(
            0,
            String::from("Ghost"),
            PathBuf::from("/nonexistent/ghost.md"),
        )];
        let results = search_project(&docs, "anything", false, None, None);
        assert!(results.is_empty());
    }
}
