//! Offline dictionary and thesaurus lookup (R10.1, R10.2, R10.7).
//!
//! A writer under the cursor on a word wants a synonym or a definition without
//! breaking flow to open a browser. This module answers both from a **bundled,
//! offline resource** compiled into the binary via `include_str!`, exactly the
//! way [`crate::spellcheck`] bundles the Hunspell dictionary (ADR-005, ADR-016).
//! Lookups are in-memory map hits, so they return well inside the 500ms budget
//! (R10.1/R10.2) and need no network (constraint C5).
//!
//! ### The typed "unavailable" contract (R10.7)
//!
//! R10.7 requires that a missing or unloadable resource never produces an
//! unhandled error: instead the system posts a non-blocking message naming the
//! unavailable resource and disables the corresponding command. That is a
//! statement about the loader's *return type*, not just runtime behaviour, so
//! loading is expressed as a [`LookupState`] — either [`LookupState::Ready`]
//! with a parsed [`LookupResource`], or [`LookupState::Unavailable`] carrying
//! which resource failed and a human-readable reason. Nothing here panics.
//!
//! The overlay, word-under-cursor extraction, and single-undo synonym
//! replacement are the app's job and land in a later task (13.5); this module
//! is the pure lookup engine and its offline resource loading.

use std::collections::HashMap;

/// The bundled lookup resource, compiled into the binary (ADR-016). Kept beside
/// the Hunspell affix/dictionary files under `assets/` so all offline lexical
/// data lives in one place.
const BUNDLED_THESAURUS: &str = include_str!("../assets/thesaurus.txt");

/// A named lookup resource, used in status messages and to decide which command
/// (`Thesaurus` vs `Define`) to disable when a resource is unavailable (R10.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// The combined synonym + definition resource.
    Thesaurus,
}

impl Resource {
    /// Human-readable name for the status-area message (R10.7).
    pub fn name(self) -> &'static str {
        match self {
            Resource::Thesaurus => "thesaurus",
        }
    }
}

/// Why a resource could not be used, surfaced to the writer verbatim (R10.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable {
    /// Which resource is missing/unloadable.
    pub resource: Resource,
    /// A short, human-readable reason for the status area.
    pub reason: String,
}

impl Unavailable {
    fn new(resource: Resource, reason: impl Into<String>) -> Self {
        Unavailable {
            resource,
            reason: reason.into(),
        }
    }

    /// The message shown in the status area, e.g.
    /// `"thesaurus unavailable: resource is empty"`.
    pub fn message(&self) -> String {
        format!("{} unavailable: {}", self.resource.name(), self.reason)
    }
}

/// The outcome of attempting to load a lookup resource.
///
/// This is the R10.7 contract in the type system: a caller either gets a
/// working [`LookupResource`] or a typed [`Unavailable`] it can turn into a
/// non-blocking status message and use to disable the corresponding command —
/// never a panic and never a raw error to surface at the writer.
#[derive(Debug, Clone)]
pub enum LookupState {
    /// The resource loaded and holds at least one entry.
    Ready(LookupResource),
    /// The resource is missing or unusable; carries why.
    Unavailable(Unavailable),
}

impl LookupState {
    /// Borrow the loaded resource, if this state is [`Ready`](LookupState::Ready).
    pub fn resource(&self) -> Option<&LookupResource> {
        match self {
            LookupState::Ready(r) => Some(r),
            LookupState::Unavailable(_) => None,
        }
    }

    /// Borrow the unavailability, if this state is
    /// [`Unavailable`](LookupState::Unavailable).
    pub fn unavailable(&self) -> Option<&Unavailable> {
        match self {
            LookupState::Unavailable(u) => Some(u),
            LookupState::Ready(_) => None,
        }
    }

    /// Whether the resource is ready for lookups.
    pub fn is_ready(&self) -> bool {
        matches!(self, LookupState::Ready(_))
    }
}

/// A definition/thesaurus hit for one word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupResult {
    /// The head-word as stored in the resource (lowercased).
    pub word: String,
    /// Synonyms in resource order; empty on a thesaurus miss.
    pub synonyms: Vec<String>,
    /// The definition; empty on a definition miss.
    pub definition: String,
}

impl LookupResult {
    /// Whether this result carries any synonyms.
    pub fn has_synonyms(&self) -> bool {
        !self.synonyms.is_empty()
    }

    /// Whether this result carries a definition.
    pub fn has_definition(&self) -> bool {
        !self.definition.is_empty()
    }
}

/// One parsed entry: its synonyms and definition.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    synonyms: Vec<String>,
    definition: String,
}

/// The parsed, in-memory lookup resource: a map from lowercased head-word to
/// its synonyms and definition. Built once at load and queried per invocation.
#[derive(Debug, Clone)]
pub struct LookupResource {
    entries: HashMap<String, Entry>,
}

impl LookupResource {
    /// Load the bundled offline resource (ADR-016).
    ///
    /// Because the data is compiled in, this is effectively always
    /// [`LookupState::Ready`]; it can only be `Unavailable` if the bundled file
    /// were ever emptied, which the same parse path reports rather than panics.
    pub fn bundled() -> LookupState {
        Self::from_str(BUNDLED_THESAURUS)
    }

    /// Parse a resource from its plain-text form (see `assets/thesaurus.txt` for
    /// the format). Comment (`#`) and blank lines are ignored. An empty or
    /// entry-less resource yields [`LookupState::Unavailable`] rather than an
    /// empty-but-`Ready` map, so callers never present a command that can only
    /// ever miss (R10.7).
    pub fn from_str(text: &str) -> LookupState {
        let mut entries = HashMap::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // Tab-separated: headword \t synonyms \t definition. Missing later
            // columns are tolerated (a thesaurus-only or definition-only line).
            let mut cols = line.splitn(3, '\t');
            let head = match cols.next() {
                Some(h) => h.trim().to_lowercase(),
                None => continue,
            };
            if head.is_empty() {
                continue;
            }
            let synonyms = cols.next().map(parse_synonyms).unwrap_or_default();
            let definition = cols
                .next()
                .map(|d| d.trim().to_string())
                .unwrap_or_default();
            entries.insert(
                head,
                Entry {
                    synonyms,
                    definition,
                },
            );
        }

        if entries.is_empty() {
            LookupState::Unavailable(Unavailable::new(
                Resource::Thesaurus,
                "resource contains no entries",
            ))
        } else {
            LookupState::Ready(LookupResource { entries })
        }
    }

    /// Number of head-words loaded.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the resource holds no entries. Kept for API completeness; a
    /// `Ready` resource is never empty by construction (see [`from_str`]).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up a word (definition + synonyms), matched case-insensitively after
    /// trimming surrounding whitespace and punctuation. Returns `None` when the
    /// word is not in the resource — a miss, not an error.
    pub fn lookup(&self, word: &str) -> Option<LookupResult> {
        let key = normalize_word(word);
        if key.is_empty() {
            return None;
        }
        self.entries.get(&key).map(|e| LookupResult {
            word: key,
            synonyms: e.synonyms.clone(),
            definition: e.definition.clone(),
        })
    }

    /// Synonyms for a word (thesaurus, R10.1). `None` on a miss; an empty `Vec`
    /// means the word is known but has no synonyms recorded.
    pub fn thesaurus(&self, word: &str) -> Option<Vec<String>> {
        self.lookup(word).map(|r| r.synonyms)
    }

    /// The definition for a word (R10.2). `None` on a miss; an empty `String`
    /// means the word is known but has no definition recorded.
    pub fn define(&self, word: &str) -> Option<String> {
        self.lookup(word).map(|r| r.definition)
    }
}

/// Parse the comma-separated synonym column, trimming each and dropping empties.
fn parse_synonyms(col: &str) -> Vec<String> {
    col.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Normalize a query word for lookup: trim surrounding whitespace and any
/// leading/trailing punctuation (quotes, commas, periods a selection might
/// carry), then lowercase. Internal apostrophes/hyphens are preserved so
/// `it's` and `well-known` survive.
fn normalize_word(word: &str) -> String {
    word.trim()
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Bundled resource loads and answers lookups -------------------------

    #[test]
    fn bundled_resource_is_ready() {
        let state = LookupResource::bundled();
        assert!(state.is_ready(), "bundled resource should load");
        let res = state.resource().unwrap();
        assert!(
            res.len() >= 100,
            "bundled set should be non-trivial, got {}",
            res.len()
        );
    }

    #[test]
    fn bundled_thesaurus_returns_synonyms() {
        let state = LookupResource::bundled();
        let res = state.resource().unwrap();
        let syns = res.thesaurus("happy").expect("happy is in the bundled set");
        assert!(syns.contains(&"joyful".to_string()), "got {syns:?}");
    }

    #[test]
    fn bundled_definition_is_returned() {
        let state = LookupResource::bundled();
        let res = state.resource().unwrap();
        let def = res
            .define("abandon")
            .expect("abandon is in the bundled set");
        assert!(!def.is_empty());
        assert!(def.to_lowercase().contains("give up"), "got {def:?}");
    }

    // ---- Case-insensitive / punctuation-tolerant matching -------------------

    #[test]
    fn lookup_is_case_insensitive() {
        let res = ready(&sample());
        assert!(res.lookup("HAPPY").is_some());
        assert!(res.lookup("Happy").is_some());
    }

    #[test]
    fn lookup_trims_surrounding_punctuation() {
        let res = ready(&sample());
        // A selection often carries trailing punctuation or quotes.
        assert!(res.lookup("\"happy,\"").is_some());
        assert!(res.lookup("  big.  ").is_some());
    }

    #[test]
    fn miss_returns_none_not_error() {
        let res = ready(&sample());
        assert!(res.lookup("zznotaword").is_none());
        assert!(res.thesaurus("zznotaword").is_none());
        assert!(res.define("zznotaword").is_none());
    }

    #[test]
    fn empty_query_is_a_miss() {
        let res = ready(&sample());
        assert!(res.lookup("").is_none());
        assert!(res.lookup("   ").is_none());
        assert!(res.lookup("!!!").is_none());
    }

    // ---- Partial rows: synonyms-only and definition-only --------------------

    #[test]
    fn definition_only_row_has_no_synonyms() {
        let res = ready("solo\t\tOnly a definition here.");
        let r = res.lookup("solo").unwrap();
        assert!(r.has_definition());
        assert!(!r.has_synonyms());
        assert_eq!(r.synonyms, Vec::<String>::new());
    }

    #[test]
    fn synonyms_only_row_has_no_definition() {
        let res = ready("quick\tfast,rapid,swift");
        let r = res.lookup("quick").unwrap();
        assert!(r.has_synonyms());
        assert!(!r.has_definition());
        assert_eq!(r.synonyms, vec!["fast", "rapid", "swift"]);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let res = ready("# a comment\n\n  \nreal\tword\tDefinition.\n# trailing comment");
        assert_eq!(res.len(), 1);
        assert!(res.lookup("real").is_some());
    }

    // ---- The R10.7 typed "unavailable" path (no panic) ----------------------

    #[test]
    fn missing_resource_yields_typed_unavailable() {
        // An empty resource stands in for "not bundled / failed to load".
        let state = LookupResource::from_str("");
        assert!(!state.is_ready());
        let u = state.unavailable().expect("empty resource is unavailable");
        assert_eq!(u.resource, Resource::Thesaurus);
        assert!(state.resource().is_none());
    }

    #[test]
    fn comment_only_resource_is_unavailable() {
        // A file that parses to zero entries is unavailable, not empty-Ready.
        let state = LookupResource::from_str("# only comments\n\n# nothing else");
        assert!(!state.is_ready());
        assert!(state.unavailable().is_some());
    }

    #[test]
    fn unavailable_message_names_the_resource() {
        let state = LookupResource::from_str("");
        let msg = state.unavailable().unwrap().message();
        assert!(msg.contains("thesaurus"), "got {msg:?}");
        assert!(msg.contains("unavailable"), "got {msg:?}");
    }

    #[test]
    fn loading_never_panics_on_garbage() {
        // Rows with stray tabs, no columns, unicode — must not panic, and the
        // loader either yields Ready or a typed Unavailable.
        for garbage in [
            "\t\t\t",
            "\n\n\n",
            "word",
            "☃\tsnowman\tA snowman.",
            "   \t   \t   ",
        ] {
            let _ = LookupResource::from_str(garbage);
        }
        let state = LookupResource::from_str("☃\tsnowman\tA snowman.");
        assert!(state.is_ready());
    }

    // ---- helpers ------------------------------------------------------------

    /// Parse `text` and assert it is `Ready`, returning the resource.
    fn ready(text: &str) -> LookupResource {
        match LookupResource::from_str(text) {
            LookupState::Ready(r) => r,
            LookupState::Unavailable(u) => panic!("expected Ready, got {}", u.message()),
        }
    }

    fn sample() -> String {
        [
            "happy\tjoyful,cheerful,glad\tFeeling pleasure.",
            "big\tlarge,huge,enormous\tOf considerable size.",
            "quick\tfast,rapid,swift\tMoving with speed.",
        ]
        .join("\n")
    }
}
