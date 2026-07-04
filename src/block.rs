//! Persistent block marks — the long-hand page. Marks are plain char
//! positions kept valid across edits by `adjust_pos`; setting one is never a
//! prelude to an immediate action.

use serde::{Deserialize, Serialize};

/// Shift a mark to account for an edit that replaced `del` chars at `at`
/// with `ins` chars. Positions inside the deleted range collapse to `at`.
pub fn adjust_pos(pos: usize, at: usize, del: usize, ins: usize) -> usize {
    if pos <= at {
        pos
    } else if pos <= at + del {
        at
    } else {
        pos - del + ins
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockMarks {
    pub begin: Option<usize>,
    pub end: Option<usize>,
    /// Highlight toggled off with ^KH (marks stay).
    pub hidden: bool,
    /// Where the block was last moved/copied from (^QV).
    pub source: Option<usize>,
    /// The previously marked block, for ^KU toggling.
    pub previous: Option<(usize, usize)>,
}

impl BlockMarks {
    /// A well-formed marked range, if both ends are set in order.
    pub fn range(&self) -> Option<(usize, usize)> {
        match (self.begin, self.end) {
            (Some(b), Some(e)) if b < e => Some((b, e)),
            _ => None,
        }
    }

    /// The range to paint, honoring ^KH.
    pub fn visible_range(&self) -> Option<(usize, usize)> {
        if self.hidden {
            None
        } else {
            self.range()
        }
    }

    /// Swap the current block with the remembered previous one (^KU).
    pub fn toggle_previous(&mut self) -> bool {
        let current = self.range();
        match self.previous {
            Some((b, e)) => {
                self.previous = current;
                self.begin = Some(b);
                self.end = Some(e);
                true
            }
            None => false,
        }
    }

    /// Remember the current block as "previous" before it is replaced.
    pub fn remember(&mut self) {
        if let Some(r) = self.range() {
            self.previous = Some(r);
        }
    }

    pub fn adjust(&mut self, at: usize, del: usize, ins: usize) {
        if let Some(b) = self.begin {
            self.begin = Some(adjust_pos(b, at, del, ins));
        }
        if let Some(e) = self.end {
            self.end = Some(adjust_pos(e, at, del, ins));
        }
        if let Some(s) = self.source {
            self.source = Some(adjust_pos(s, at, del, ins));
        }
        if let Some((b, e)) = self.previous {
            self.previous = Some((adjust_pos(b, at, del, ins), adjust_pos(e, at, del, ins)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjust_shifts_after_insert() {
        assert_eq!(adjust_pos(10, 5, 0, 3), 13);
        assert_eq!(adjust_pos(5, 5, 0, 3), 5); // at the edit point: stays
        assert_eq!(adjust_pos(3, 5, 0, 3), 3);
    }

    #[test]
    fn adjust_collapses_deleted_range() {
        assert_eq!(adjust_pos(7, 5, 4, 0), 5); // inside deleted range
        assert_eq!(adjust_pos(12, 5, 4, 0), 8); // beyond it
    }

    #[test]
    fn marks_survive_edit_before_block() {
        let mut m = BlockMarks {
            begin: Some(10),
            end: Some(20),
            ..Default::default()
        };
        m.adjust(0, 0, 5); // insert 5 chars at doc start
        assert_eq!(m.range(), Some((15, 25)));
    }

    #[test]
    fn toggle_previous_swaps() {
        let mut m = BlockMarks {
            begin: Some(1),
            end: Some(4),
            previous: Some((8, 12)),
            ..Default::default()
        };
        assert!(m.toggle_previous());
        assert_eq!(m.range(), Some((8, 12)));
        assert_eq!(m.previous, Some((1, 4)));
    }
}
