//! Never-lose undo, after Emacs: undoing appends the inverse edit to the log
//! as a new entry rather than popping, so no buffer state is ever
//! unreachable. Repeated `^U` walks backward through history; any other
//! command breaks the chain, after which `^U` undoes the undos (= redo).

use serde::{Deserialize, Serialize};

/// One primitive text change: at char index `at`, `deleted` was removed and
/// `inserted` was put in its place.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edit {
    pub at: usize,
    pub deleted: String,
    pub inserted: String,
}

impl Edit {
    pub fn inverse(&self) -> Edit {
        Edit {
            at: self.at,
            deleted: self.inserted.clone(),
            inserted: self.deleted.clone(),
        }
    }
}

/// A group of edits undone/redone as a unit (e.g. a burst of typing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditGroup {
    pub edits: Vec<Edit>,
    pub cursor_before: usize,
    pub cursor_after: usize,
}

const MAX_GROUP_EDITS: usize = 32;

pub struct History {
    log: Vec<EditGroup>,
    /// Index into `log` of the entry the next undo will revert.
    /// `None` means the chain is broken: next undo starts from the end.
    undo_ptr: Option<usize>,
    /// Whether the last group in the log is still open for coalescing.
    group_open: bool,
    /// Kind of the most recent recorded edit, for coalescing.
    last_kind: Option<EditKind>,
}

/// What kind of edit is being recorded, for coalescing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    InsertChar,
    DeleteLeft,
    Other,
}

impl History {
    pub fn new() -> Self {
        History {
            log: Vec::new(),
            undo_ptr: None,
            group_open: false,
            last_kind: None,
        }
    }

    /// Record a user edit. `kind` controls coalescing: consecutive edits of
    /// the same coalescable kind merge into one undo group.
    pub fn record(
        &mut self,
        edit: Edit,
        kind: EditKind,
        cursor_before: usize,
        cursor_after: usize,
    ) {
        self.undo_ptr = None;
        let coalesce = self.group_open
            && kind != EditKind::Other
            && self.last_kind == Some(kind)
            && self
                .log
                .last()
                .is_some_and(|g| g.edits.len() < MAX_GROUP_EDITS);
        if coalesce {
            let group = self
                .log
                .last_mut()
                .expect("group_open implies non-empty log");
            group.edits.push(edit);
            group.cursor_after = cursor_after;
        } else {
            self.log.push(EditGroup {
                edits: vec![edit],
                cursor_before,
                cursor_after,
            });
            self.group_open = kind != EditKind::Other;
        }
        self.last_kind = Some(kind);
    }

    /// Record several edits as one undo group (e.g. a block move). The edits
    /// must have been applied in list order, each `at` in the coordinates of
    /// the buffer state it was applied to.
    pub fn record_group(&mut self, edits: Vec<Edit>, cursor_before: usize, cursor_after: usize) {
        self.undo_ptr = None;
        self.log.push(EditGroup {
            edits,
            cursor_before,
            cursor_after,
        });
        self.group_open = false;
        self.last_kind = None;
    }

    /// Close the current coalescing group (called on movement, mode changes —
    /// anything that should make the next keystroke start a fresh group).
    pub fn break_group(&mut self) {
        self.group_open = false;
        self.last_kind = None;
    }

    /// Any command other than undo breaks the undo chain.
    pub fn break_chain(&mut self) {
        self.undo_ptr = None;
    }

    /// The group the next undo should revert, if any. The caller applies the
    /// inverses to the buffer and then calls `confirm_undo` with the group it
    /// wants appended to the log.
    pub fn next_undo(&mut self) -> Option<EditGroup> {
        let ptr = self.undo_ptr.unwrap_or(self.log.len());
        if ptr == 0 {
            return None;
        }
        Some(self.log[ptr - 1].clone())
    }

    /// Append the inverse group produced by an undo and step the pointer back.
    pub fn confirm_undo(&mut self, inverse: EditGroup) {
        let ptr = self.undo_ptr.unwrap_or(self.log.len());
        self.log.push(inverse);
        self.group_open = false;
        self.last_kind = None;
        self.undo_ptr = Some(ptr - 1);
    }

    /// Export the log for session persistence.
    pub fn export(&self) -> (Vec<EditGroup>, Option<usize>) {
        (self.log.clone(), self.undo_ptr)
    }

    /// Rebuild a history from a persisted session.
    pub fn restore(log: Vec<EditGroup>, undo_ptr: Option<usize>) -> Self {
        let undo_ptr = undo_ptr.filter(|&p| p <= log.len());
        History {
            log,
            undo_ptr,
            group_open: false,
            last_kind: None,
        }
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(at: usize, s: &str) -> Edit {
        Edit {
            at,
            deleted: String::new(),
            inserted: s.to_string(),
        }
    }

    #[test]
    fn typing_coalesces() {
        let mut h = History::new();
        h.record(ins(0, "a"), EditKind::InsertChar, 0, 1);
        h.record(ins(1, "b"), EditKind::InsertChar, 1, 2);
        assert_eq!(h.log.len(), 1);
        assert_eq!(h.log[0].edits.len(), 2);
    }

    #[test]
    fn movement_breaks_group() {
        let mut h = History::new();
        h.record(ins(0, "a"), EditKind::InsertChar, 0, 1);
        h.break_group();
        h.record(ins(1, "b"), EditKind::InsertChar, 1, 2);
        assert_eq!(h.log.len(), 2);
    }

    #[test]
    fn undo_walks_back_and_appends() {
        let mut h = History::new();
        h.record(ins(0, "a"), EditKind::Other, 0, 1);
        h.record(ins(1, "b"), EditKind::Other, 1, 2);

        let g = h.next_undo().unwrap(); // reverts "b"
        assert_eq!(g.edits[0].inserted, "b");
        let inverse = EditGroup {
            edits: g.edits.iter().rev().map(Edit::inverse).collect(),
            cursor_before: 2,
            cursor_after: 1,
        };
        h.confirm_undo(inverse);
        assert_eq!(h.log.len(), 3);

        let g = h.next_undo().unwrap(); // reverts "a"
        assert_eq!(g.edits[0].inserted, "a");
    }

    #[test]
    fn broken_chain_undoes_the_undo() {
        let mut h = History::new();
        h.record(ins(0, "a"), EditKind::Other, 0, 1);
        let g = h.next_undo().unwrap();
        let inverse = EditGroup {
            edits: g.edits.iter().rev().map(Edit::inverse).collect(),
            cursor_before: 1,
            cursor_after: 0,
        };
        h.confirm_undo(inverse);
        // Something else happens; the chain breaks.
        h.break_chain();
        // Now undo reverts the undo itself: "a" comes back.
        let g = h.next_undo().unwrap();
        assert_eq!(g.edits[0].deleted, "a");
    }
}
