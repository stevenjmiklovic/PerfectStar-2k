//! Style and readability checking: the self-editing pass (R8).
//!
//! Modelled on [`spellcheck`](crate::spellcheck), which is the proven pattern in
//! this editor: a service on `App`, a toggle, and markers resolved while the
//! visible lines are being drawn. Nothing is scanned in the background and no
//! state is cached, so there is nothing to invalidate and nothing to go stale;
//! the cost per frame is bounded by what is on screen, not by the size of the
//! manuscript (R8.6).
//!
//! The rules are fixed and bundled, each individually toggleable — see
//! [ADR-015]. They are also *heuristics*: there is no part-of-speech tagger
//! behind them, so `-ly` matching and be-plus-participle detection carry a known
//! false-positive rate. That is the honest trade for offline, instant, and
//! dependency-free. Style advice is advice; the writer decides.
//!
//! **Scope.** Checks run over one line at a time. In this editor wrapping is
//! visual, so a paragraph is normally a single logical line and line-scoped
//! analysis is paragraph-scoped in practice. A sentence deliberately split
//! across hard line breaks is analysed as its separate pieces.
//!
//! [ADR-015]: ../../docs/adr/ADR-015-fixed-bundled-style-rules.md

use std::collections::HashMap;

use crate::normalize;
use crate::spellcheck::word_spans;

/// Crutch words (intensifiers and hedges that weaken a sentence) and filter
/// words (perception verbs that put a narrator between the reader and the
/// scene). Bundled data rather than logic, so a future config key could extend
/// the list without changing the engine (ADR-015).
const FILLER: &[&str] = &[
    // Intensifiers and hedges.
    "very",
    "really",
    "quite",
    "rather",
    "somewhat",
    "actually",
    "basically",
    "literally",
    "simply",
    "totally",
    "utterly",
    "certainly",
    "definitely",
    "probably",
    "perhaps",
    "maybe",
    "just",
    "even",
    "suddenly",
    "somehow",
    // Filter words: the narrator noticing, instead of the thing itself.
    "seemed",
    "felt",
    "saw",
    "heard",
    "noticed",
    "watched",
    "realized",
    "realised",
    "wondered",
    "thought",
    "decided",
    "knew",
];

/// Forms of "to be" that can open a passive construction.
const BE_FORMS: &[&str] = &[
    "am", "is", "are", "was", "were", "be", "been", "being", "get", "gets", "got",
];

/// Irregular past participles, for the passive check. Regular ones are caught by
/// the `-ed` ending; these are the common ones that aren't.
const IRREGULAR_PARTICIPLES: &[&str] = &[
    "beaten",
    "become",
    "begun",
    "bent",
    "bitten",
    "blown",
    "born",
    "borne",
    "bought",
    "bound",
    "broken",
    "brought",
    "built",
    "burnt",
    "burst",
    "bust",
    "caught",
    "chosen",
    "clung",
    "come",
    "cut",
    "dealt",
    "done",
    "drawn",
    "driven",
    "drunk",
    "eaten",
    "fallen",
    "fed",
    "felt",
    "fought",
    "found",
    "flown",
    "flung",
    "forgiven",
    "forgotten",
    "frozen",
    "given",
    "gone",
    "grown",
    "heard",
    "held",
    "hidden",
    "hit",
    "hung",
    "hurt",
    "kept",
    "known",
    "laid",
    "led",
    "left",
    "lent",
    "let",
    "lit",
    "lost",
    "made",
    "meant",
    "met",
    "paid",
    "put",
    "read",
    "ridden",
    "risen",
    "run",
    "said",
    "seen",
    "sent",
    "set",
    "sewn",
    "shaken",
    "shot",
    "shown",
    "shut",
    "slept",
    "slid",
    "sold",
    "sought",
    "sown",
    "spent",
    "spoken",
    "spun",
    "stolen",
    "struck",
    "stuck",
    "stung",
    "sung",
    "sunk",
    "swept",
    "swum",
    "swung",
    "taken",
    "taught",
    "thrown",
    "told",
    "torn",
    "understood",
    "woken",
    "won",
    "worn",
    "woven",
    "written",
    "wrung",
];

/// Words ending in `-ed` that are ordinarily adjectives after "to be", where
/// "she is tired" is a state and not a passive construction. The rest of the
/// ambiguous cases ("the door was closed") are left flagged: they genuinely are
/// ambiguous, and ADR-015 accepts a documented false-positive rate over shipping
/// a part-of-speech tagger.
const ADJECTIVAL_ED: &[&str] = &[
    "aged",
    "annoyed",
    "ashamed",
    "blessed",
    "bored",
    "confused",
    "crooked",
    "crowded",
    "delighted",
    "disappointed",
    "dressed",
    "embarrassed",
    "excited",
    "exhausted",
    "frustrated",
    "interested",
    "learned",
    "married",
    "naked",
    "pleased",
    "sacred",
    "satisfied",
    "scared",
    "tired",
    "wicked",
    "worried",
];

/// Words ending in `-ly` that are not adverbs. Short list on purpose: the point
/// is to drop the obvious false positives, not to encode English.
const NOT_ADVERBS: &[&str] = &[
    "ally",
    "apply",
    "belly",
    "bully",
    "comply",
    "family",
    "folly",
    "holy",
    "imply",
    "italy",
    "jelly",
    "jolly",
    "july",
    "lily",
    "melancholy",
    "multiply",
    "only",
    "rally",
    "rely",
    "reply",
    "silly",
    "supply",
    "tally",
    "ugly",
];

/// Words too common to be interesting in an overused-word report.
const STOPWORDS: &[&str] = &[
    "a", "об", "about", "after", "all", "an", "and", "any", "are", "as", "at", "be", "been", "but",
    "by", "can", "could", "did", "do", "for", "from", "had", "has", "have", "he", "her", "here",
    "him", "his", "how", "i", "if", "in", "into", "is", "it", "its", "me", "my", "no", "not", "of",
    "on", "one", "or", "our", "out", "över", "over", "said", "she", "so", "than", "that", "the",
    "their", "them", "then", "there", "these", "they", "this", "to", "up", "was", "we", "were",
    "what", "when", "which", "who", "will", "with", "would", "you", "your",
];

/// Which checks are switched on (R8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleChecks {
    pub passive: bool,
    pub adverbs: bool,
    pub filler: bool,
    pub long_sentences: bool,
    /// Words past which a sentence is flagged as very long.
    pub sentence_words: usize,
}

impl Default for StyleChecks {
    fn default() -> Self {
        StyleChecks {
            passive: true,
            adverbs: true,
            filler: true,
            long_sentences: true,
            sentence_words: 40,
        }
    }
}

/// What kind of style issue a marker represents (R8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleKind {
    Passive,
    Adverb,
    Filler,
    LongSentence,
}

impl StyleKind {
    /// How the issue reads in the status line when the cursor is on it.
    pub fn label(self) -> &'static str {
        match self {
            StyleKind::Passive => "passive voice",
            StyleKind::Adverb => "-ly adverb",
            StyleKind::Filler => "filler word",
            StyleKind::LongSentence => "very long sentence",
        }
    }
}

/// One flagged span within a line, in char offsets from the line's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleIssue {
    pub start: usize,
    pub end: usize,
    pub kind: StyleKind,
}

/// The style checker.
#[derive(Debug, Clone)]
pub struct StyleEngine {
    pub checks: StyleChecks,
}

impl StyleEngine {
    pub fn new(checks: StyleChecks) -> Self {
        StyleEngine { checks }
    }

    /// Every style issue in one line of prose.
    ///
    /// Note lines and Markdown markers are excluded the same way the word count
    /// and the exporters exclude them, so "prose" means one thing everywhere
    /// (R2.6). Spans are returned against the original line so the renderer can
    /// mark them directly.
    pub fn issues_in_line(&self, line: &str) -> Vec<StyleIssue> {
        if normalize::is_note_line(line) {
            return Vec::new();
        }
        let mut issues = Vec::new();
        // Materialize the line's chars once. Extracting each word with
        // `chars().skip(start)` would make the whole pass quadratic in the line
        // length — and a soft-wrapped paragraph is one very long line.
        let chars: Vec<char> = line.chars().collect();
        let spans = word_spans(line);
        let words: Vec<String> = spans
            .iter()
            .map(|&(s, e)| lowercase_of(&chars[s..e]))
            .collect();

        if self.checks.adverbs {
            for (i, word) in words.iter().enumerate() {
                if is_ly_adverb(word) {
                    issues.push(StyleIssue {
                        start: spans[i].0,
                        end: spans[i].1,
                        kind: StyleKind::Adverb,
                    });
                }
            }
        }

        if self.checks.filler {
            for (i, word) in words.iter().enumerate() {
                if FILLER.contains(&word.as_str()) {
                    issues.push(StyleIssue {
                        start: spans[i].0,
                        end: spans[i].1,
                        kind: StyleKind::Filler,
                    });
                }
            }
        }

        if self.checks.passive {
            for (i, word) in words.iter().enumerate() {
                if !BE_FORMS.contains(&word.as_str()) {
                    continue;
                }
                // Allow one intervening adverb: "was quietly taken".
                for step in 1..=2 {
                    let Some(next) = words.get(i + step) else {
                        break;
                    };
                    if step == 2 && !is_ly_adverb(&words[i + 1]) {
                        break;
                    }
                    if is_participle(next) {
                        issues.push(StyleIssue {
                            start: spans[i].0,
                            end: spans[i + step].1,
                            kind: StyleKind::Passive,
                        });
                        break;
                    }
                }
            }
        }

        if self.checks.long_sentences {
            // Words are already located; count the ones inside each sentence by
            // walking both sorted lists together rather than re-scanning text.
            let mut next_word = 0usize;
            for (start, end) in sentence_spans(line) {
                while next_word < spans.len() && spans[next_word].0 < start {
                    next_word += 1;
                }
                let mut count = 0usize;
                let mut i = next_word;
                while i < spans.len() && spans[i].1 <= end {
                    count += 1;
                    i += 1;
                }
                if count > self.checks.sentence_words {
                    issues.push(StyleIssue {
                        start,
                        end,
                        kind: StyleKind::LongSentence,
                    });
                }
            }
        }

        issues.sort_by_key(|issue| (issue.start, issue.end));
        issues
    }
}

/// Readability figures for a document or a selection (R8.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Readability {
    pub words: usize,
    pub sentences: usize,
    pub syllables: usize,
    /// Flesch–Kincaid grade level.
    pub grade: f32,
    pub avg_sentence_words: f32,
    /// Share of words that are `-ly` adverbs, as a percentage.
    pub adverb_percent: f32,
    /// Share of sentences longer than the configured threshold, as a percentage.
    pub long_sentence_percent: f32,
}

/// Compute readability over prose (notes and Markdown markers excluded), using
/// the default long-sentence threshold from [`StyleChecks`].
pub fn readability(text: &str) -> Readability {
    readability_with_threshold(text, StyleChecks::default().sentence_words)
}

/// Compute readability using an explicit long-sentence threshold.
///
/// The app passes its configured threshold here so the overlay's percentage
/// agrees with the style markers shown in the document.
pub fn readability_with_threshold(text: &str, sentence_words: usize) -> Readability {
    let mut words = 0usize;
    let mut sentences = 0usize;
    let mut long_sentences = 0usize;
    let mut syllables = 0usize;
    let mut adverbs = 0usize;

    for line in text.lines() {
        if normalize::is_note_line(line) {
            continue;
        }
        let stripped = normalize::strip_markers(line);
        if stripped.trim().is_empty() {
            continue;
        }
        let sentence_ranges = sentence_spans(&stripped);
        sentences += sentence_ranges.len();
        let chars: Vec<char> = stripped.chars().collect();
        let spans = word_spans(&stripped);
        for (s, e) in &spans {
            let word = lowercase_of(&chars[*s..*e]);
            words += 1;
            syllables += syllables_in(&word);
            if is_ly_adverb(&word) {
                adverbs += 1;
            }
        }
        for (start, end) in sentence_ranges {
            let sentence_word_count = spans
                .iter()
                .filter(|&&(word_start, word_end)| word_start >= start && word_end <= end)
                .count();
            if sentence_word_count > sentence_words {
                long_sentences += 1;
            }
        }
    }

    // A document with prose but no terminal punctuation is still one sentence.
    let effective_sentences = sentences.max(if words > 0 { 1 } else { 0 });
    let (grade, avg) = if words == 0 || effective_sentences == 0 {
        (0.0, 0.0)
    } else {
        let wps = words as f32 / effective_sentences as f32;
        let spw = syllables as f32 / words as f32;
        // Flesch–Kincaid grade level.
        (0.39 * wps + 11.8 * spw - 15.59, wps)
    };

    Readability {
        words,
        sentences: effective_sentences,
        syllables,
        grade: (grade * 10.0).round() / 10.0,
        avg_sentence_words: (avg * 10.0).round() / 10.0,
        adverb_percent: if words == 0 {
            0.0
        } else {
            ((adverbs as f32 / words as f32 * 1000.0).round()) / 10.0
        },
        long_sentence_percent: if effective_sentences == 0 {
            0.0
        } else {
            ((long_sentences as f32 / effective_sentences as f32 * 1000.0).round()) / 10.0
        },
    }
}

/// The most-repeated interesting words, commonest first (R8.5).
///
/// Stopwords are excluded — a report whose top entry is "the" tells a writer
/// nothing. Ties break alphabetically so the report is deterministic.
pub fn word_frequency(text: &str, top: usize) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        if normalize::is_note_line(line) {
            continue;
        }
        let stripped = normalize::strip_markers(line);
        let chars: Vec<char> = stripped.chars().collect();
        for (s, e) in word_spans(&stripped) {
            let word = lowercase_of(&chars[s..e]);
            if word.chars().count() < 4 || STOPWORDS.contains(&word.as_str()) {
                continue;
            }
            if word.chars().any(|c| c.is_numeric()) {
                continue;
            }
            *counts.entry(word).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|(a_word, a_count), (b_word, b_count)| {
        b_count.cmp(a_count).then_with(|| a_word.cmp(b_word))
    });
    ranked.truncate(top);
    ranked
}

fn lowercase_of(chars: &[char]) -> String {
    chars.iter().flat_map(|c| c.to_lowercase()).collect()
}

fn is_ly_adverb(word: &str) -> bool {
    word.chars().count() >= 4
        && word.ends_with("ly")
        && word.chars().all(char::is_alphabetic)
        && !NOT_ADVERBS.contains(&word)
}

fn is_participle(word: &str) -> bool {
    if ADJECTIVAL_ED.contains(&word) {
        return false;
    }
    if word.chars().count() >= 4 && word.ends_with("ed") {
        return true;
    }
    IRREGULAR_PARTICIPLES.contains(&word)
}

/// Sentence spans within a line, as char offsets.
///
/// A sentence ends at `.`, `!`, or `?` — including runs like `?!` and a closing
/// quote or bracket after the stop — or at the end of the line. Leading
/// whitespace is skipped so a span starts on a word.
pub fn sentence_spans(line: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if matches!(chars[i], '.' | '!' | '?') {
            let mut end = i + 1;
            while end < chars.len()
                && matches!(
                    chars[end],
                    '.' | '!' | '?' | '"' | '\'' | ')' | ']' | '”' | '’'
                )
            {
                end += 1;
            }
            push_sentence(&chars, &mut spans, start, end);
            start = end;
            i = end;
            continue;
        }
        i += 1;
    }
    push_sentence(&chars, &mut spans, start, chars.len());
    spans
}

fn push_sentence(chars: &[char], spans: &mut Vec<(usize, usize)>, start: usize, end: usize) {
    let mut from = start;
    while from < end && chars[from].is_whitespace() {
        from += 1;
    }
    if from < end && chars[from..end].iter().any(|c| c.is_alphanumeric()) {
        spans.push((from, end));
    }
}

/// Vowel-group syllable estimate, with the usual silent-`e` correction. Good
/// enough for a grade-level figure and free of a pronunciation dictionary.
fn syllables_in(word: &str) -> usize {
    let mut count = 0usize;
    let mut prev_vowel = false;
    for c in word.chars() {
        let vowel = matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
        if vowel && !prev_vowel {
            count += 1;
        }
        prev_vowel = vowel;
    }
    if word.ends_with('e') && count > 1 && !word.ends_with("le") {
        count -= 1;
    }
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(text: &str, start: usize, end: usize) -> String {
        text.chars().skip(start).take(end - start).collect()
    }

    fn engine() -> StyleEngine {
        StyleEngine::new(StyleChecks::default())
    }

    fn kinds(line: &str) -> Vec<(StyleKind, String)> {
        engine()
            .issues_in_line(line)
            .into_iter()
            .map(|issue| (issue.kind, slice(line, issue.start, issue.end)))
            .collect()
    }

    #[test]
    fn adverbs_are_flagged_and_lookalikes_are_not() {
        assert_eq!(
            kinds("He walked quietly."),
            [(StyleKind::Adverb, String::from("quietly"))]
        );
        // -ly words that aren't adverbs stay quiet.
        assert!(
            engine()
                .issues_in_line("The family will apply. Only Italy.")
                .is_empty()
        );
        // Too short to be an adverb.
        assert!(engine().issues_in_line("Fly.").is_empty());
    }

    #[test]
    fn filler_and_filter_words_are_flagged() {
        // "was very tired" is copular, not passive: only -ly adverbs may sit
        // between the verb and a participle, so this reports the filler alone.
        assert_eq!(
            kinds("He was very tired."),
            [(StyleKind::Filler, String::from("very"))]
        );
        assert_eq!(
            kinds("She saw the knife."),
            [(StyleKind::Filler, String::from("saw"))]
        );
        assert_eq!(
            kinds("He just felt it."),
            [
                (StyleKind::Filler, String::from("just")),
                (StyleKind::Filler, String::from("felt")),
            ]
        );
    }

    #[test]
    fn passive_voice_is_flagged_including_across_an_adverb() {
        assert_eq!(
            kinds("The knife was taken."),
            [(StyleKind::Passive, String::from("was taken"))]
        );
        // Issues are reported in document order, so the passive span — which
        // starts at "was" — comes before the adverb inside it.
        assert_eq!(
            kinds("The door was quietly closed."),
            [
                (StyleKind::Passive, String::from("was quietly closed")),
                (StyleKind::Adverb, String::from("quietly")),
            ]
        );
        // Regular participles are caught by the -ed ending.
        assert_eq!(
            kinds("It is decorated."),
            [(StyleKind::Passive, String::from("is decorated"))]
        );
        // Active voice stays quiet.
        assert!(engine().issues_in_line("He took the knife.").is_empty());
        // ...and so does a state described with "to be" plus an adjective that
        // happens to end in -ed.
        assert!(engine().issues_in_line("She is tired.").is_empty());
        assert!(engine().issues_in_line("They were worried.").is_empty());
    }

    #[test]
    fn very_long_sentences_are_flagged_whole() {
        let long = format!("{} end.", "word ".repeat(40));
        let issues = engine().issues_in_line(&long);
        let sentence = issues
            .iter()
            .find(|issue| issue.kind == StyleKind::LongSentence)
            .expect("a 41-word sentence should be flagged");
        assert_eq!(sentence.start, 0);
        assert_eq!(sentence.end, long.chars().count());

        // The requirement's default is 40 words: exactly 40 is allowed, while
        // the 41-word sentence above is flagged.
        let at_default = format!("{} end.", "word ".repeat(39));
        assert!(
            !engine()
                .issues_in_line(&at_default)
                .iter()
                .any(|i| i.kind == StyleKind::LongSentence)
        );

        // Just under the threshold is left alone.
        let ok = format!("{} end.", "word ".repeat(28));
        assert!(
            !engine()
                .issues_in_line(&ok)
                .iter()
                .any(|i| i.kind == StyleKind::LongSentence)
        );
    }

    #[test]
    fn each_check_can_be_switched_off_independently() {
        // R8.7.
        let line = "The door was quietly closed, very slowly.";
        let all = StyleEngine::new(StyleChecks::default());
        assert!(all.issues_in_line(line).len() >= 4);

        let quiet = StyleEngine::new(StyleChecks {
            passive: false,
            adverbs: false,
            filler: false,
            long_sentences: false,
            sentence_words: 30,
        });
        assert!(quiet.issues_in_line(line).is_empty());

        let only_passive = StyleEngine::new(StyleChecks {
            adverbs: false,
            filler: false,
            long_sentences: false,
            ..StyleChecks::default()
        });
        assert_eq!(
            only_passive
                .issues_in_line(line)
                .iter()
                .map(|i| i.kind)
                .collect::<Vec<_>>(),
            [StyleKind::Passive]
        );

        // The long-sentence threshold is a number the writer sets.
        let strict = StyleEngine::new(StyleChecks {
            sentence_words: 5,
            ..StyleChecks::default()
        });
        assert!(
            strict
                .issues_in_line("One two three four five six seven.")
                .iter()
                .any(|i| i.kind == StyleKind::LongSentence)
        );
    }

    #[test]
    fn note_lines_are_not_style_checked() {
        // R2.6 consistency: a note is not prose.
        assert!(
            engine()
                .issues_in_line(".. very quietly, the knife was taken")
                .is_empty()
        );
    }

    #[test]
    fn sentences_split_on_terminal_punctuation() {
        assert_eq!(
            sentence_spans("One. Two! Three?"),
            [(0, 4), (5, 9), (10, 16)]
        );
        assert_eq!(sentence_spans("No terminator here"), [(0, 18)]);
        assert_eq!(sentence_spans("Wait... what?!"), [(0, 7), (8, 14)]);
        assert!(sentence_spans("   ").is_empty());
        assert!(sentence_spans("").is_empty());
        // A closing quote belongs to the sentence it ends.
        assert_eq!(sentence_spans("\"Stop!\" she said."), [(0, 7), (8, 17)]);
    }

    #[test]
    fn readability_reports_grade_and_averages() {
        let plain = readability("The cat sat on the mat. The dog ran.\n");
        assert_eq!(plain.words, 9);
        assert_eq!(plain.sentences, 2);
        assert_eq!(plain.avg_sentence_words, 4.5);
        assert!(
            plain.grade < 4.0,
            "short simple sentences read young, got {}",
            plain.grade
        );

        let dense = readability(
            "The extraordinarily complicated administrative determination \
             necessitated considerable additional deliberation regarding \
             fundamentally incompatible organizational responsibilities.\n",
        );
        assert!(
            dense.grade > plain.grade + 8.0,
            "polysyllabic prose should read much older: {} vs {}",
            dense.grade,
            plain.grade
        );

        let adverbs = readability("He quietly, slowly, sadly left.\n");
        assert!(adverbs.adverb_percent > 50.0, "{}", adverbs.adverb_percent);

        let long = readability_with_threshold(
            "One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty one. Short sentence.\n",
            20,
        );
        assert_eq!(long.long_sentence_percent, 50.0);

        // Empty and note-only input is handled without dividing by zero.
        let empty = readability("");
        assert_eq!((empty.words, empty.sentences, empty.grade), (0, 0, 0.0));
        assert_eq!(readability(".. just a note\n").words, 0);
    }

    #[test]
    fn a_document_without_punctuation_is_one_sentence() {
        let r = readability("three words here\n");
        assert_eq!((r.words, r.sentences), (3, 1));
        assert!(r.grade.is_finite());
    }

    #[test]
    fn the_frequency_report_finds_crutch_words_and_skips_the_obvious() {
        let text = "The knife was on the table. The knife was Marcus's knife. \
                    He looked at the table.\n";
        let report = word_frequency(text, 3);

        assert_eq!(report[0], (String::from("knife"), 3));
        assert_eq!(report[1], (String::from("table"), 2));
        assert!(
            !report.iter().any(|(word, _)| word == "the"),
            "stopwords are not a finding: {report:?}"
        );
        // Short words and numbers aren't interesting either.
        assert!(word_frequency("cat cat cat 1999 1999 1999\n", 5).is_empty());
        // Notes don't count toward the writer's prose habits.
        assert!(word_frequency(".. knife knife knife\n", 5).is_empty());
    }

    #[test]
    fn syllable_estimates_are_sane() {
        assert_eq!(syllables_in("cat"), 1);
        assert_eq!(syllables_in("table"), 2);
        assert_eq!(syllables_in("knife"), 1);
        // Vowel-group counting says five for "extraordinary"; dictionaries say
        // five or six. Close enough for a grade level, and stable.
        assert_eq!(syllables_in("extraordinary"), 5);
        assert_eq!(syllables_in(""), 1, "never zero, to keep the ratio finite");
    }

    #[test]
    fn checking_a_paragraph_sized_line_is_fast_enough_for_the_draw_path() {
        // R8.6/C6: the render path analyses only visible lines, but one line can
        // be a whole soft-wrapped paragraph. Ten thousand words of it must stay
        // far inside a frame.
        let line = "The door was quietly closed and she really saw very little. ".repeat(1_000);
        let words = word_spans(&line).len();
        assert!(words > 10_000, "{words}");

        let start = std::time::Instant::now();
        let issues = engine().issues_in_line(&line);
        let elapsed = start.elapsed();

        assert!(!issues.is_empty());
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "{words} words took {elapsed:?}"
        );
    }
}
