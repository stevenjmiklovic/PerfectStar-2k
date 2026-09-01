//! One editing window: a document plus everything that travels with it.
//!
//! Sawyer's essay singles out WordStar's windows for letting each one keep
//! its own marked block ("imagine ... being told that you could only have
//! one piece cut out at a time. Madness!"). So a pane owns not just the
//! buffer but the cursor, undo history, block marks, bookmarks, and jump
//! ring — two documents side by side each keep their whole long-hand-page
//! state. The kill ring stays on `App`, shared, so clippings move freely
//! between windows.

use std::io;
use std::path::PathBuf;
use std::time::Instant;

use crate::block::BlockMarks;
use crate::buffer::Buffer;
use crate::history::History;
use crate::session::{self, Session};

pub struct Pane {
    pub buf: Buffer,
    /// Cursor as a char index into the rope.
    pub cursor: usize,
    /// Sticky visual column for vertical movement.
    pub goal_col: Option<usize>,
    /// First visible document line.
    pub top_line: usize,
    /// Horizontal scroll offset in visual columns.
    pub left_col: usize,
    pub history: History,
    pub blocks: BlockMarks,
    pub bookmarks: [Option<usize>; 10],
    /// Ring of positions left behind by long-range jumps (^QP walks it).
    pub jump_stack: Vec<usize>,
    /// Editorial comments anchored into this document (R9.1). Loaded from the
    /// sidecar when `App` installs the pane and adjusted alongside the block
    /// marks and bookmarks on every edit (R9.5).
    pub annotations: Vec<crate::meta::Annotation>,
    /// Annotations have moved (or changed) since the sidecar was written. Kept
    /// per pane rather than on `App` so a second window's adjusted anchors can't
    /// be forgotten when the focus moves away from it.
    pub annotations_dirty: bool,
    /// When this pane's buffer last changed (drives idle autosave).
    pub last_edit: Instant,
    /// This pane's text-area size, captured at render time.
    pub view_rows: usize,
    pub view_cols: usize,
}

impl Pane {
    pub fn open(path: Option<PathBuf>) -> io::Result<Self> {
        let buf = Buffer::open(path)?;
        let mut pane = Pane {
            buf,
            cursor: 0,
            goal_col: None,
            top_line: 0,
            left_col: 0,
            history: History::new(),
            blocks: BlockMarks::default(),
            bookmarks: [None; 10],
            jump_stack: Vec::new(),
            annotations: Vec::new(),
            annotations_dirty: false,
            last_edit: Instant::now(),
            view_rows: 24,
            view_cols: 80,
        };
        pane.restore_session();
        Ok(pane)
    }

    fn restore_session(&mut self) {
        let Some(path) = self.buf.path.clone() else {
            return;
        };
        let Some(mut s) = session::load(&path, self.buf.len_chars()) else {
            return;
        };
        let len = self.buf.len_chars();
        self.cursor = s.cursor.min(len);
        self.top_line = s.top_line.min(self.buf.len_lines().saturating_sub(1));
        for (i, b) in s.bookmarks.iter().take(10).enumerate() {
            self.bookmarks[i] = b.filter(|&p| p <= len);
        }
        self.blocks = s.block.clone();
        self.jump_stack = s.jump_stack.iter().copied().filter(|&p| p <= len).collect();
        self.history = s.restore_history();
    }

    pub fn save_session(&self) {
        let Some(path) = &self.buf.path else {
            return;
        };
        let (history_log, undo_ptr) = self.history.export();
        let s = Session {
            len_chars: self.buf.len_chars(),
            cursor: self.cursor,
            top_line: self.top_line,
            bookmarks: self.bookmarks.to_vec(),
            block: self.blocks.clone(),
            jump_stack: self.jump_stack.clone(),
            history_log,
            undo_ptr,
        };
        session::save(path, &s);
    }
}
