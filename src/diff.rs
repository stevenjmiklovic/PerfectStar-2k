//! Revision diffing: what changed between a snapshot and the draft (R4.4).
//!
//! The algorithm comes from the `similar` crate ([ADR-014]); this module is the
//! wrapper that keeps it there. Everything above it — the diff view, the
//! revisions list, the tests — speaks in [`DiffLine`]s, so swapping engines
//! later is a change to one file.
//!
//! Diffs are line-oriented with elided context. A chapter is mostly unchanged
//! when you compare it to this morning's snapshot, and a screen of identical
//! prose tells the writer nothing; unchanged runs beyond a few lines of context
//! collapse into a [`DiffTag::Gap`] marker so what actually moved stays on
//! screen.
//!
//! [ADR-014]: ../../docs/adr/ADR-014-similar-crate-for-revision-diff.md

use similar::{ChangeTag, TextDiff};

/// Unchanged lines kept either side of a change, for orientation.
const CONTEXT_LINES: usize = 3;

/// What a rendered diff line represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTag {
    /// Present in both versions.
    Equal,
    /// Only in the newer version.
    Insert,
    /// Only in the older version.
    Delete,
    /// A run of unchanged lines that was elided.
    Gap,
}

/// One line of a rendered diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub tag: DiffTag,
    /// Line text with its end-of-line stripped (a `Gap` carries its summary
    /// here instead, e.g. "⋯ 42 unchanged lines ⋯").
    pub text: String,
    /// 1-based line number in the older version, when it has one.
    pub old_line: Option<usize>,
    /// 1-based line number in the newer version, when it has one.
    pub new_line: Option<usize>,
}

/// How much changed, for the revisions list and the diff view's title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffSummary {
    pub added: usize,
    pub removed: usize,
}

/// Diff two versions with the default context. Returns an empty vector when the
/// versions are identical — there is nothing to show, not even a gap.
pub fn lines(old: &str, new: &str) -> Vec<DiffLine> {
    lines_with_context(old, new, CONTEXT_LINES)
}

/// Diff two versions, keeping `context` unchanged lines around each change.
pub fn lines_with_context(old: &str, new: &str, context: usize) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let old_total = diff.old_len();
    // Context wider than the document means "show everything", and the engine
    // does arithmetic on the radius that overflows for extreme values — clamp
    // rather than hand it a number that can panic.
    let context = context.min(old_total.max(diff.new_len()).max(1));
    let groups = diff.grouped_ops(context);
    let mut out = Vec::new();
    // Where the previous hunk stopped in the *old* text, so the run of
    // unchanged lines before the next hunk can be counted and elided.
    let mut old_cursor = 0usize;

    for group in &groups {
        let Some(first) = group.first() else { continue };
        let hunk_start = first.old_range().start;
        if let Some(gap) = gap_line(hunk_start.saturating_sub(old_cursor)) {
            out.push(gap);
        }
        for op in group {
            for change in diff.iter_changes(op) {
                out.push(DiffLine {
                    tag: match change.tag() {
                        ChangeTag::Equal => DiffTag::Equal,
                        ChangeTag::Insert => DiffTag::Insert,
                        ChangeTag::Delete => DiffTag::Delete,
                    },
                    text: strip_eol(change.value()),
                    old_line: change.old_index().map(|i| i + 1),
                    new_line: change.new_index().map(|i| i + 1),
                });
            }
        }
        old_cursor = group
            .last()
            .map(|op| op.old_range().end)
            .unwrap_or(old_cursor);
    }

    // Anything unchanged after the last hunk.
    if !out.is_empty()
        && let Some(gap) = gap_line(old_total.saturating_sub(old_cursor))
    {
        out.push(gap);
    }
    out
}

/// Count the added and removed lines in a rendered diff.
pub fn summarize(lines: &[DiffLine]) -> DiffSummary {
    let mut summary = DiffSummary::default();
    for line in lines {
        match line.tag {
            DiffTag::Insert => summary.added += 1,
            DiffTag::Delete => summary.removed += 1,
            DiffTag::Equal | DiffTag::Gap => {}
        }
    }
    summary
}

fn gap_line(skipped: usize) -> Option<DiffLine> {
    if skipped == 0 {
        return None;
    }
    let unit = if skipped == 1 { "line" } else { "lines" };
    Some(DiffLine {
        tag: DiffTag::Gap,
        text: format!("⋯ {skipped} unchanged {unit} ⋯"),
        old_line: None,
        new_line: None,
    })
}

/// Drop the line terminator so the view can style a whole row without a stray
/// newline, and so `\n` vs `\r\n` doesn't read as a change in the rendering.
fn strip_eol(line: &str) -> String {
    line.trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(lines: &[DiffLine]) -> Vec<(DiffTag, &str)> {
        lines
            .iter()
            .map(|line| (line.tag, line.text.as_str()))
            .collect()
    }

    #[test]
    fn identical_versions_produce_no_diff() {
        let text = "Chapter One\n\nThe knife was where he left it.\n";
        assert!(lines(text, text).is_empty());
        assert_eq!(summarize(&lines(text, text)), DiffSummary::default());
    }

    #[test]
    fn insertions_and_deletions_are_tagged_with_line_numbers() {
        let old = "one\ntwo\nthree\n";
        let new = "one\ntwo and a half\nthree\n";
        let diff = lines(old, new);

        assert_eq!(
            tags(&diff),
            [
                (DiffTag::Equal, "one"),
                (DiffTag::Delete, "two"),
                (DiffTag::Insert, "two and a half"),
                (DiffTag::Equal, "three"),
            ]
        );
        let deleted = diff.iter().find(|l| l.tag == DiffTag::Delete).unwrap();
        assert_eq!((deleted.old_line, deleted.new_line), (Some(2), None));
        let inserted = diff.iter().find(|l| l.tag == DiffTag::Insert).unwrap();
        assert_eq!((inserted.old_line, inserted.new_line), (None, Some(2)));
        assert_eq!(
            summarize(&diff),
            DiffSummary {
                added: 1,
                removed: 1
            }
        );
    }

    #[test]
    fn pure_addition_and_pure_deletion_are_symmetric() {
        let short = "one\n";
        let long = "one\ntwo\n";
        assert_eq!(
            summarize(&lines(short, long)),
            DiffSummary {
                added: 1,
                removed: 0
            }
        );
        assert_eq!(
            summarize(&lines(long, short)),
            DiffSummary {
                added: 0,
                removed: 1
            }
        );
    }

    #[test]
    fn the_inserted_lines_reconstruct_the_new_version() {
        // A diff is only correct if what it calls "new" *is* the new text.
        let old = "alpha\nbravo\ncharlie\ndelta\n";
        let new = "alpha\nbravo revised\ncharlie\ndelta\necho\n";
        let rebuilt = lines_with_context(old, new, usize::MAX)
            .iter()
            .filter(|line| matches!(line.tag, DiffTag::Equal | DiffTag::Insert))
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(rebuilt, new.trim_end_matches('\n'));
    }

    #[test]
    fn long_unchanged_runs_collapse_into_a_gap() {
        let mut old = String::new();
        for i in 0..40 {
            old.push_str(&format!("line {i}\n"));
        }
        let new = old.replace("line 20\n", "line twenty\n");
        let diff = lines(&old, &new);

        // Leading and trailing unchanged runs are summarized, not printed.
        assert_eq!(diff.first().unwrap().tag, DiffTag::Gap);
        assert_eq!(diff.last().unwrap().tag, DiffTag::Gap);
        assert!(diff.first().unwrap().text.contains("17 unchanged lines"));
        assert!(
            diff.iter().filter(|l| l.tag == DiffTag::Equal).count() <= CONTEXT_LINES * 2,
            "context should be bounded, got {diff:#?}"
        );
        assert_eq!(
            summarize(&diff),
            DiffSummary {
                added: 1,
                removed: 1
            }
        );
    }

    #[test]
    fn a_gap_of_one_line_reads_singular() {
        let gap = gap_line(1).unwrap();
        assert_eq!(gap.text, "⋯ 1 unchanged line ⋯");
        assert!(gap_line(0).is_none());
    }

    #[test]
    fn empty_versions_are_handled() {
        assert!(lines("", "").is_empty());
        assert_eq!(
            summarize(&lines("", "first words\n")),
            DiffSummary {
                added: 1,
                removed: 0
            }
        );
        assert_eq!(
            summarize(&lines("everything\n", "")),
            DiffSummary {
                added: 0,
                removed: 1
            }
        );
    }

    #[test]
    fn line_endings_do_not_read_as_changes_in_the_text() {
        let diff = lines("one\ntwo\n", "one\r\ntwo\r\n");
        // The bytes differ, so the diff is non-empty, but no rendered line
        // carries a stray terminator that would garble the view.
        assert!(diff.iter().all(|line| !line.text.contains(['\n', '\r'])));
    }

    #[test]
    fn prose_paragraph_rewrite_is_reported_as_one_change() {
        let old = "He walked into the room and saw the knife on the table.\n";
        let new = "He entered the room. The knife lay on the table.\n";
        assert_eq!(
            summarize(&lines(old, new)),
            DiffSummary {
                added: 1,
                removed: 1
            }
        );
    }
}
