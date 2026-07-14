//! Spellchecking: a bundled Hunspell-compatible en_US dictionary (via the
//! `spellbook` crate), plus a personal dictionary — a global, plain-text
//! wordlist, shared across every document, for the words no dictionary
//! will ever have: character names, invented terms, a writer's own coinages.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use spellbook::Dictionary;

// SPDX-style provenance: assets/en_US.{aff,dic} are the LibreOffice-packaged
// SCOWL en_US Hunspell dictionary; see THIRD-PARTY-NOTICES.md for full
// attribution and license text (Kevin Atkinson / SCOWL, Geoff Kuenning /
// Ispell — both permissive, attribution-only).
const AFF: &str = include_str!("../assets/en_US.aff");
const DIC: &str = include_str!("../assets/en_US.dic");

pub struct Spellchecker {
    dict: Dictionary,
    personal: HashSet<String>,
    personal_path: Option<PathBuf>,
}

impl Spellchecker {
    pub fn load() -> Self {
        Self::load_with_personal_path(personal_dict_path())
    }

    fn load_with_personal_path(personal_path: Option<PathBuf>) -> Self {
        let dict = Dictionary::new(AFF, DIC).expect("bundled en_US dictionary failed to parse");
        let personal = personal_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| {
                s.lines()
                    .map(|w| w.trim().to_lowercase())
                    .filter(|w| !w.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Spellchecker {
            dict,
            personal,
            personal_path,
        }
    }

    /// Whether `word` is spelled correctly. All-caps tokens (acronyms) and
    /// anything containing a digit are always accepted without a dictionary
    /// lookup.
    pub fn check(&self, word: &str) -> bool {
        if word.is_empty() {
            return true;
        }
        if word.chars().any(|c| c.is_ascii_digit()) {
            return true;
        }
        if !word.chars().any(|c| c.is_alphabetic()) {
            return true;
        }
        if word
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(char::is_uppercase)
        {
            return true;
        }
        if self.personal.contains(&word.to_lowercase()) {
            return true;
        }
        self.dict.check(word)
    }

    /// Add `word` to the personal dictionary, in memory and on disk.
    pub fn learn(&mut self, word: &str) {
        let w = word.to_lowercase();
        if w.is_empty() || !self.personal.insert(w.clone()) {
            return;
        }
        let Some(path) = &self.personal_path else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{w}");
        }
    }
}

fn personal_dict_path() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("perfectstar2k")
            .join("personal_dict.txt"),
    )
}

/// Runs of "word" characters (letters, digits, apostrophes, underscores) in
/// `text`, as (start_char, end_char) spans — the same character class used
/// for cursor word-motion (^A/^F), so spellcheck operates on exactly what
/// those commands select.
pub fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    let mut i = 0;
    for c in text.chars() {
        let is_word = c.is_alphanumeric() || c == '_' || c == '\'';
        match (is_word, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                spans.push((s, i));
                start = None;
            }
            _ => {}
        }
        i += 1;
    }
    if let Some(s) = start {
        spans.push((s, i));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_spans_basic() {
        assert_eq!(word_spans("one two, three!"), vec![(0, 3), (4, 7), (9, 14)]);
    }

    #[test]
    fn word_spans_contraction_and_underscore() {
        assert_eq!(
            word_spans("don't stop my_var"),
            vec![(0, 5), (6, 10), (11, 17)]
        );
    }

    #[test]
    fn word_spans_empty_and_punct_only() {
        assert!(word_spans("").is_empty());
        assert!(word_spans("... -- !!").is_empty());
    }

    #[test]
    fn dictionary_loads_and_checks_common_words() {
        let sc = Spellchecker::load();
        assert!(sc.check("hello"));
        assert!(sc.check("running")); // affix-derived inflection
        assert!(sc.check("Hello")); // sentence-initial capitalization
        assert!(!sc.check("zzxxqqvv"));
    }

    #[test]
    fn acronyms_and_numbers_pass() {
        let sc = Spellchecker::load();
        assert!(sc.check("NASA"));
        assert!(sc.check("COVID19"));
        assert!(sc.check("123"));
    }

    #[test]
    fn personal_dictionary_overrides() {
        // A process-unique temp path: `learn()` must never touch the real
        // global personal dictionary as a side effect of running tests.
        let tmp = std::env::temp_dir().join(format!("pstar-test-dict-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let mut sc = Spellchecker::load_with_personal_path(Some(tmp.clone()));
        assert!(!sc.check("zorathia"));
        sc.learn("Zorathia");
        assert!(sc.check("zorathia"));
        assert!(sc.check("Zorathia"));
        assert_eq!(std::fs::read_to_string(&tmp).unwrap().trim(), "zorathia");
        let _ = std::fs::remove_file(&tmp);
    }
}
