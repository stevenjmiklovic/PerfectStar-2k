//! Autocorrect / expansion rules and smart typographic substitution (R10.4,
//! R10.6, R10.8).
//!
//! A writer who types `teh ` wants `the ` without breaking flow, and a writer
//! who types `"` wants a curly quote. This module is the **pure engine** for
//! both, with no `app.rs` wiring: the event-loop integration — firing on a
//! word separator, replacing as a single undoable edit, and gating each half
//! by its config flag — lands in a later task (13.5).
//!
//! Two independent capabilities live here, kept independently toggleable so the
//! app can enable one without the other (R10.6 requires typographic
//! substitution be toggleable independently of autocorrect):
//!
//!   1. **Autocorrect / expansion** — [`Autocorrect::correct_word`] looks up a
//!      just-completed word in a bundled rules table and returns its correction
//!      or expansion, or `None` on a miss. The table is a **bundled, offline
//!      resource** compiled into the binary via `include_str!`, exactly the way
//!      [`crate::lookup`] bundles the thesaurus and [`crate::spellcheck`]
//!      bundles the Hunspell dictionary (ADR-005, ADR-016). Lookup is a single
//!      [`HashMap`] hit, so it stays well inside the keystroke budget on a
//!      300k-word document (R10.8).
//!
//!   2. **Smart typography** — [`Autocorrect::smart_typography`] delegates to
//!      [`crate::normalize::smart_typography`], the single source of truth for
//!      straight→curly quotes, `--`→em dash, and `...`→ellipsis, so on-the-fly
//!      typing and manuscript export smarten text identically (R10.6). This
//!      module does not reimplement that logic.
//!
//! The rules are user-extendable via config (R10.4): [`Autocorrect::from_str`]
//! parses the same plain-text format as the bundled file, and the app can layer
//! user rules on top before construction. The whole autocorrect feature is
//! globally disable-able — that is the caller's decision to simply not invoke
//! [`correct_word`](Autocorrect::correct_word); the engine itself is stateless
//! beyond its table.

use std::collections::HashMap;

/// The bundled autocorrect ruleset, compiled into the binary (ADR-016). Kept
/// beside the thesaurus and Hunspell files under `assets/` so all offline
/// lexical data lives in one place.
const BUNDLED_RULES: &str = include_str!("../assets/autocorrect.txt");

/// The pure autocorrect engine: a case-insensitive map from a trigger token to
/// its correction/expansion, plus a delegating typography helper. Built once at
/// load and queried per word separator.
///
/// Autocorrect and typography are exposed as two separate calls
/// ([`correct_word`](Self::correct_word) and
/// [`smart_typography`](Self::smart_typography)) so the app can gate each by its
/// own config flag (R10.6).
#[derive(Debug, Clone)]
pub struct Autocorrect {
    rules: HashMap<String, String>,
}

impl Autocorrect {
    /// Build from the bundled offline ruleset (ADR-016).
    ///
    /// Because the data is compiled in, this always succeeds with a non-empty
    /// table; the parse path simply skips comments and blank lines.
    pub fn bundled() -> Self {
        Self::from_str(BUNDLED_RULES)
    }

    /// Parse a ruleset from its plain-text form (see `assets/autocorrect.txt`
    /// for the format). Each non-comment line is `trigger <TAB> replacement`.
    /// Comment (`#`) and blank lines are ignored, as are lines missing a
    /// replacement column or whose trigger is empty. Triggers are lowercased on
    /// load for case-insensitive matching; a later duplicate trigger wins,
    /// which is what lets user rules override bundled ones when concatenated.
    pub fn from_str(text: &str) -> Self {
        let mut rules = HashMap::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut cols = line.splitn(2, '\t');
            let trigger = match cols.next() {
                Some(t) => t.trim().to_lowercase(),
                None => continue,
            };
            let replacement = match cols.next() {
                Some(r) => r.trim().to_string(),
                None => continue,
            };
            if trigger.is_empty() || replacement.is_empty() {
                continue;
            }
            rules.insert(trigger, replacement);
        }
        Autocorrect { rules }
    }

    /// Number of rules loaded.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether the ruleset holds no rules.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Look up the correction/expansion for a just-completed word, to be called
    /// when the writer types a word separator (space, punctuation, or Enter)
    /// after `word` (R10.4). Matching is case-insensitive.
    ///
    /// Returns `None` on a miss — an unknown or already-correct word is left
    /// untouched, not an error. The caller applies the returned string as a
    /// single undoable edit (R10.5, wired in task 13.5).
    ///
    /// This is a single [`HashMap`] hit after a cheap key normalization, so it
    /// completes within the keystroke budget even on a very large document
    /// (R10.8). It allocates only the lowercased key and, on a hit, the cloned
    /// replacement; a miss allocates just the key.
    pub fn correct_word(&self, word: &str) -> Option<String> {
        let key = normalize_trigger(word);
        if key.is_empty() {
            return None;
        }
        self.rules.get(&key).cloned()
    }

    /// Smart typographic substitution for on-the-fly typing (R10.6), delegating
    /// to [`crate::normalize::smart_typography`] so it matches manuscript export
    /// exactly. Exposed here as a separate call from
    /// [`correct_word`](Self::correct_word) so the app can toggle typography
    /// independently of autocorrect.
    pub fn smart_typography(&self, text: &str) -> String {
        crate::normalize::smart_typography(text)
    }
}

/// Normalize a word into a lookup trigger: trim surrounding whitespace and any
/// leading/trailing non-alphanumeric characters (a trailing separator, quotes,
/// commas), then lowercase. Internal apostrophes/hyphens survive so `dont` and
/// `well-known` map correctly and `it's` stays intact.
fn normalize_trigger(word: &str) -> String {
    word.trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Bundled ruleset loads and meets the R10.4 size floor ---------------

    #[test]
    fn bundled_ruleset_has_at_least_500_rules() {
        let ac = Autocorrect::bundled();
        assert!(
            ac.len() >= 500,
            "R10.4 requires at least 500 rules, got {}",
            ac.len()
        );
        assert!(!ac.is_empty());
    }

    #[test]
    fn bundled_known_corrections_fire() {
        let ac = Autocorrect::bundled();
        assert_eq!(ac.correct_word("teh").as_deref(), Some("the"));
        assert_eq!(ac.correct_word("recieve").as_deref(), Some("receive"));
        assert_eq!(ac.correct_word("seperate").as_deref(), Some("separate"));
    }

    #[test]
    fn bundled_expansion_fires() {
        let ac = Autocorrect::bundled();
        assert_eq!(
            ac.correct_word("asap").as_deref(),
            Some("as soon as possible")
        );
    }

    #[test]
    fn unknown_word_is_a_miss_not_an_error() {
        let ac = Autocorrect::bundled();
        assert!(ac.correct_word("thisisdefinitelyaword").is_none());
        assert!(
            ac.correct_word("the").is_none(),
            "correct words pass through"
        );
    }

    // ---- Case-insensitive / separator-tolerant matching ---------------------

    #[test]
    fn matching_is_case_insensitive() {
        let ac = ready("teh\tthe");
        assert_eq!(ac.correct_word("TEH").as_deref(), Some("the"));
        assert_eq!(ac.correct_word("Teh").as_deref(), Some("the"));
    }

    #[test]
    fn trailing_separator_and_punctuation_are_trimmed() {
        // The word arrives with the just-typed separator or punctuation.
        let ac = ready("teh\tthe");
        assert_eq!(ac.correct_word("teh ").as_deref(), Some("the"));
        assert_eq!(ac.correct_word("\"teh,\"").as_deref(), Some("the"));
    }

    #[test]
    fn empty_or_punctuation_only_word_is_a_miss() {
        let ac = ready("teh\tthe");
        assert!(ac.correct_word("").is_none());
        assert!(ac.correct_word("   ").is_none());
        assert!(ac.correct_word("!!!").is_none());
    }

    // ---- Parsing: comments, blanks, malformed lines -------------------------

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let ac = ready("# a comment\n\n  \nteh\tthe\n# trailing");
        assert_eq!(ac.len(), 1);
        assert_eq!(ac.correct_word("teh").as_deref(), Some("the"));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        // No tab / no replacement / empty trigger — all skipped, no panic.
        let ac = ready("noreplacement\n\tno trigger\nteh\tthe\n\t\n");
        assert_eq!(ac.len(), 1);
        assert_eq!(ac.correct_word("teh").as_deref(), Some("the"));
    }

    #[test]
    fn later_duplicate_trigger_wins() {
        // Lets user rules override bundled ones when concatenated.
        let ac = ready("color\tcolor\ncolor\tcolour");
        assert_eq!(ac.correct_word("color").as_deref(), Some("colour"));
    }

    #[test]
    fn parsing_never_panics_on_garbage() {
        for garbage in ["\t\t\t", "\n\n\n", "word", "☃\tsnowman", "   \t   "] {
            let _ = Autocorrect::from_str(garbage);
        }
    }

    // ---- Typography delegates to normalize (R10.6) --------------------------

    #[test]
    fn typography_produces_curly_quotes_em_dash_and_ellipsis() {
        let ac = ready("");
        assert_eq!(ac.smart_typography("\"hi\""), "\u{201C}hi\u{201D}");
        assert_eq!(ac.smart_typography("wait--stop"), "wait\u{2014}stop");
        assert_eq!(ac.smart_typography("er...um"), "er\u{2026}um");
    }

    #[test]
    fn typography_matches_normalize_exactly() {
        // The delegation must be byte-for-byte identical to the export path so
        // typing and manuscript export smarten text the same way.
        let ac = ready("");
        for s in ["it's", "--\"yes\"", "a-b", "a..b", "plain text"] {
            assert_eq!(
                ac.smart_typography(s),
                crate::normalize::smart_typography(s)
            );
        }
    }

    #[test]
    fn autocorrect_and_typography_are_independent_calls() {
        // Typography works without any rules; corrections work without invoking
        // typography — the two capabilities are separately callable so 13.5 can
        // gate each by its own config flag.
        let ac = ready("teh\tthe");
        assert_eq!(ac.smart_typography("\"x\""), "\u{201C}x\u{201D}");
        assert_eq!(ac.correct_word("teh").as_deref(), Some("the"));
    }

    // ---- Lookup is pure / allocation-light (R10.8) --------------------------

    #[test]
    fn repeated_lookups_are_pure_and_stable() {
        // Same input always yields the same output; the engine holds no
        // per-call mutable state, so lookups are a plain map hit.
        let ac = Autocorrect::bundled();
        let first = ac.correct_word("teh");
        for _ in 0..1000 {
            assert_eq!(ac.correct_word("teh"), first);
        }
        // A miss on a long non-word stays a miss and does not mutate the table.
        let before = ac.len();
        assert!(ac.correct_word(&"z".repeat(4096)).is_none());
        assert_eq!(ac.len(), before);
    }

    // ---- helper -------------------------------------------------------------

    fn ready(text: &str) -> Autocorrect {
        Autocorrect::from_str(text)
    }
}
