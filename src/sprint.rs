//! Writing sprints and focus mode (R3) — the two things a writer reaches for
//! when the job is *volume*, not polish.
//!
//! A **sprint** is a bounded push: so many minutes, so many words, or both. It
//! shows an unobtrusive countdown while it runs (R3.1) and, when it ends,
//! reports what was written and appends that to the daily history (R3.2).
//!
//! **Focus mode** strips the screen to the prose (R3.3) and can dim everything
//! outside the paragraph being written (R3.4). Both are *purely presentational*
//! (R3.5): nothing here can touch the buffer or a file. The only editor state
//! focus mode changes is the help level, which it restores on the way out.

use std::time::{Duration, Instant};

/// Longest sprint the prompt will accept, in minutes. A sprint is a sitting,
/// not a schedule; anything beyond a day is a typo.
const MAX_MINUTES: u64 = 24 * 60;

/// A running sprint (R3.1).
#[derive(Debug, Clone)]
pub struct Sprint {
    started: Instant,
    /// Countdown target, if the writer set one.
    duration: Option<Duration>,
    /// Word target, if the writer set one.
    word_target: Option<usize>,
    /// Prose word count when the sprint began, so progress is words *written*
    /// rather than words present.
    start_words: usize,
}

/// What a finished (or cancelled) sprint achieved (R3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// Net prose words written during the sprint; negative if the writer cut
    /// more than they added, which is still an honest number.
    pub words: i64,
    pub elapsed: Duration,
    /// Whether the sprint's own terms were met — its word target, or its full
    /// duration when words weren't the point.
    pub met_target: bool,
}

impl Report {
    /// The one-line result, for the status banner and the stats overlay.
    pub fn summary(&self) -> String {
        format!("{} words in {}", self.words, format_duration(self.elapsed))
    }
}

impl Sprint {
    /// Parse a sprint spec: `minutes`, `minutes/words`, or `/words`.
    ///
    /// Deliberately narrow. A writer starting a sprint is trying to *stop*
    /// fiddling with the tool, so the prompt takes the two numbers that matter
    /// and rejects anything ambiguous with a message that names the format.
    pub fn parse(spec: &str, start_words: usize, now: Instant) -> Result<Self, String> {
        let spec = spec.trim();
        let (minutes_part, words_part) = match spec.split_once('/') {
            Some((minutes, words)) => (minutes.trim(), Some(words.trim())),
            None => (spec, None),
        };

        let minutes = parse_positive(minutes_part, "minutes")?;
        let word_target = match words_part {
            Some(words) => parse_positive(words, "words")?,
            None => None,
        };
        if minutes.is_none() && word_target.is_none() {
            return Err(String::from(
                "Sprint needs minutes and/or words, e.g. 25 or 25/500",
            ));
        }
        if minutes.is_some_and(|m| m > MAX_MINUTES) {
            return Err(format!("Sprint is capped at {MAX_MINUTES} minutes"));
        }

        Ok(Sprint {
            started: now,
            duration: minutes.map(|m| Duration::from_secs(m * 60)),
            word_target: word_target.map(|w| w as usize),
            start_words,
        })
    }

    /// Time left on the clock, or `None` for a words-only sprint.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.duration
            .map(|limit| limit.saturating_sub(now.saturating_duration_since(self.started)))
    }

    /// Net prose words written so far.
    pub fn words_written(&self, current_words: usize) -> i64 {
        current_words as i64 - self.start_words as i64
    }

    /// Whether the sprint is over — the clock ran out, or the word target was
    /// reached. Either satisfies it: hitting 500 words in ten of twenty-five
    /// minutes is a sprint finished, not a sprint abandoned.
    pub fn is_finished(&self, now: Instant, current_words: usize) -> bool {
        let out_of_time = self.remaining(now).is_some_and(|left| left.is_zero());
        let target_met = self
            .word_target
            .is_some_and(|target| self.words_written(current_words) >= target as i64);
        out_of_time || target_met
    }

    /// The result as of `now`.
    pub fn report(&self, now: Instant, current_words: usize) -> Report {
        let words = self.words_written(current_words);
        let met_target = match self.word_target {
            Some(target) => words >= target as i64,
            // A timed sprint's terms are its clock.
            None => self.remaining(now).is_some_and(|left| left.is_zero()),
        };
        Report {
            words,
            elapsed: now.saturating_duration_since(self.started),
            met_target,
        }
    }

    /// The status-line chip: `⏱ 12:34 · 320/500`. Short on purpose — a
    /// countdown that draws the eye defeats the sprint (R3.1).
    pub fn chip(&self, now: Instant, current_words: usize) -> String {
        let mut parts = Vec::new();
        if let Some(left) = self.remaining(now) {
            parts.push(format!("⏱ {}", format_duration(left)));
        }
        match self.word_target {
            Some(target) => parts.push(format!(
                "{}/{target}",
                self.words_written(current_words).max(0)
            )),
            None => parts.push(format!("{} words", self.words_written(current_words))),
        }
        parts.join(" · ")
    }
}

/// Accept a positive count, an empty field (meaning "not set"), or fail with a
/// message naming the field.
fn parse_positive(field: &str, name: &str) -> Result<Option<u64>, String> {
    if field.is_empty() {
        return Ok(None);
    }
    match field.parse::<u64>() {
        Ok(0) => Err(format!("Sprint {name} must be more than zero")),
        Ok(value) => Ok(Some(value)),
        Err(_) => Err(format!("\"{field}\" is not a number of {name}")),
    }
}

/// `MM:SS`, or `H:MM:SS` once a sprint runs past the hour.
pub fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Focus mode (R3.3): the screen stripped to the prose.
///
/// Holds the help level to put back on the way out, so a writer who was running
/// with menus doesn't lose them by taking a focused hour.
#[derive(Debug, Clone, Copy)]
pub struct Focus {
    prior_help_level: u8,
}

impl Focus {
    pub fn enter(prior_help_level: u8) -> Self {
        Focus { prior_help_level }
    }

    pub fn prior_help_level(self) -> u8 {
        self.prior_help_level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sprint_spec() -> impl Strategy<Value = String> {
        prop_oneof![
            (1_u64..=180, 1_usize..=100_000)
                .prop_map(|(minutes, words)| format!("{minutes}/{words}")),
            (1_u64..=180).prop_map(|minutes| minutes.to_string()),
            (1_usize..=100_000).prop_map(|words| format!("/{words}")),
        ]
    }

    fn sprint_case() -> impl Strategy<Value = (String, usize, i64)> {
        (sprint_spec(), 0_usize..=1_000_000).prop_flat_map(|(spec, start_words)| {
            let minimum_delta = -(start_words as i64);
            (Just(spec), Just(start_words), minimum_delta..=100_000_i64)
        })
    }

    // Feature: pro-writer-10-star, Property 9
    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 100,
            ..ProptestConfig::default()
        })]

        #[test]
        fn sprint_ends_at_first_target_and_reports_net_words(
            (spec, start_words, word_delta) in sprint_case(),
            elapsed_secs in 0_u64..=10_800,
        ) {
            let now = Instant::now();
            let sprint = Sprint::parse(&spec, start_words, now).unwrap();
            let current_words = (start_words as i64 + word_delta) as usize;

            let report = sprint.report(at(now, elapsed_secs), current_words);
            prop_assert_eq!(report.words, word_delta);
            prop_assert_eq!(report.elapsed, Duration::from_secs(elapsed_secs));
            let expected_met_target = match sprint.word_target {
                Some(target) => word_delta >= target as i64,
                None => elapsed_secs >= sprint.duration.unwrap().as_secs(),
            };
            prop_assert_eq!(report.met_target, expected_met_target);

            if let Some(target) = sprint.word_target {
                let before_target = start_words + target - 1;
                let before_time = sprint
                    .duration
                    .map_or(now, |duration| at(now, duration.as_secs() - 1));
                prop_assert!(!sprint.is_finished(before_time, before_target));
                prop_assert!(sprint.is_finished(before_time, start_words + target));
            }

            if let Some(duration) = sprint.duration {
                prop_assert!(!sprint.is_finished(at(now, duration.as_secs() - 1), start_words));
                prop_assert!(sprint.is_finished(at(now, duration.as_secs()), start_words));
            }
        }
    }

    fn at(now: Instant, secs: u64) -> Instant {
        now + Duration::from_secs(secs)
    }

    #[test]
    fn a_bare_number_is_minutes() {
        let now = Instant::now();
        let sprint = Sprint::parse("25", 100, now).unwrap();
        assert_eq!(sprint.remaining(now), Some(Duration::from_secs(25 * 60)));
        assert_eq!(sprint.word_target, None);
    }

    #[test]
    fn minutes_and_words_can_both_be_set() {
        let now = Instant::now();
        let sprint = Sprint::parse("25/500", 100, now).unwrap();
        assert_eq!(sprint.remaining(now), Some(Duration::from_secs(1500)));
        assert_eq!(sprint.word_target, Some(500));
    }

    #[test]
    fn a_words_only_sprint_has_no_clock() {
        let now = Instant::now();
        let sprint = Sprint::parse("/500", 100, now).unwrap();
        assert_eq!(sprint.remaining(now), None);
        assert_eq!(sprint.word_target, Some(500));
        // ...and never finishes on time alone.
        assert!(!sprint.is_finished(at(now, 86_400), 100));
    }

    #[test]
    fn nonsense_specs_are_rejected_by_name() {
        let now = Instant::now();
        assert!(Sprint::parse("", 0, now).is_err());
        assert!(Sprint::parse("/", 0, now).is_err());
        assert!(Sprint::parse("0", 0, now).unwrap_err().contains("zero"));
        assert!(Sprint::parse("25/0", 0, now).unwrap_err().contains("zero"));
        assert!(
            Sprint::parse("soon", 0, now)
                .unwrap_err()
                .contains("not a number of minutes")
        );
        assert!(
            Sprint::parse("25/lots", 0, now)
                .unwrap_err()
                .contains("not a number of words")
        );
        assert!(
            Sprint::parse("2000", 0, now)
                .unwrap_err()
                .contains("capped")
        );
    }

    #[test]
    fn the_clock_runs_down_and_finishes() {
        let now = Instant::now();
        let sprint = Sprint::parse("10", 0, now).unwrap();

        assert_eq!(
            sprint.remaining(at(now, 60)),
            Some(Duration::from_secs(540))
        );
        assert!(!sprint.is_finished(at(now, 599), 0));
        assert!(sprint.is_finished(at(now, 600), 0));
        // The clock floors at zero rather than going negative.
        assert_eq!(sprint.remaining(at(now, 900)), Some(Duration::ZERO));
    }

    #[test]
    fn hitting_the_word_target_early_finishes_the_sprint() {
        let now = Instant::now();
        let sprint = Sprint::parse("25/500", 1_000, now).unwrap();

        assert!(!sprint.is_finished(at(now, 60), 1_400));
        assert!(sprint.is_finished(at(now, 60), 1_500));
    }

    #[test]
    fn the_report_counts_words_written_not_words_present() {
        let now = Instant::now();
        let sprint = Sprint::parse("25/500", 1_000, now).unwrap();

        let report = sprint.report(at(now, 754), 1_520);
        assert_eq!(report.words, 520);
        assert_eq!(report.elapsed, Duration::from_secs(754));
        assert!(report.met_target);
        assert_eq!(report.summary(), "520 words in 12:34");
    }

    #[test]
    fn a_timed_sprint_that_ran_out_met_its_terms_whatever_the_count() {
        let now = Instant::now();
        let sprint = Sprint::parse("10", 500, now).unwrap();

        assert!(
            !sprint.report(at(now, 300), 700).met_target,
            "stopped early"
        );
        assert!(sprint.report(at(now, 600), 700).met_target);
        // Cutting more than you wrote is reported honestly.
        assert_eq!(sprint.report(at(now, 600), 480).words, -20);
    }

    #[test]
    fn a_sprint_past_the_hour_reads_in_hours() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0:00");
        assert_eq!(format_duration(Duration::from_secs(59)), "0:59");
        assert_eq!(format_duration(Duration::from_secs(754)), "12:34");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1:00:00");
        assert_eq!(
            format_duration(Duration::from_secs(7 * 3600 + 62)),
            "7:01:02"
        );
    }

    #[test]
    fn the_chip_shows_the_clock_and_the_words() {
        let now = Instant::now();
        let both = Sprint::parse("25/500", 100, now).unwrap();
        assert_eq!(both.chip(at(now, 60), 420), "⏱ 24:00 · 320/500");

        let timed = Sprint::parse("25", 100, now).unwrap();
        assert_eq!(timed.chip(at(now, 60), 420), "⏱ 24:00 · 320 words");

        let counted = Sprint::parse("/500", 100, now).unwrap();
        assert_eq!(counted.chip(at(now, 60), 420), "320/500");

        // Deleting below the starting count never shows a negative target.
        assert_eq!(both.chip(at(now, 60), 50), "⏱ 24:00 · 0/500");
    }

    #[test]
    fn focus_remembers_the_help_level_to_restore() {
        let focus = Focus::enter(2);
        assert_eq!(focus.prior_help_level(), 2);
    }
}
