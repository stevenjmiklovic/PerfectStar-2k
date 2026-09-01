//! Per-file session persistence: reopening a manuscript restores your
//! fingers-in-the-pages state — cursor, bookmarks, block marks, jump stack,
//! and undo history — stored under the user's data dir, keyed by file path,
//! without polluting the document's folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::block::BlockMarks;
use crate::history::{EditGroup, History};
use crate::paths;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Session {
    /// Char count of the file when the session was written; a mismatch on
    /// load means the file changed elsewhere and positions are stale.
    pub len_chars: usize,
    pub cursor: usize,
    pub top_line: usize,
    pub bookmarks: Vec<Option<usize>>,
    pub block: BlockMarks,
    pub jump_stack: Vec<usize>,
    #[serde(default)]
    pub history_log: Vec<EditGroup>,
    #[serde(default)]
    pub undo_ptr: Option<usize>,
}

fn session_path(file: &Path) -> Option<PathBuf> {
    let name = format!("{}.json", paths::path_key(file));
    Some(paths::sessions()?.join(name))
}

pub fn load(file: &Path, len_chars: usize) -> Option<Session> {
    let path = session_path(file)?;
    let data = std::fs::read_to_string(path).ok()?;
    let session: Session = serde_json::from_str(&data).ok()?;
    // Stale session: the file was edited by something else. Positions can't
    // be trusted, so start fresh rather than restore garbage.
    if session.len_chars != len_chars {
        return None;
    }
    Some(session)
}

pub fn save(file: &Path, session: &Session) {
    let Some(path) = session_path(file) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string(session) {
        let _ = std::fs::write(path, data);
    }
}

impl Session {
    pub fn restore_history(&mut self) -> History {
        History::restore(std::mem::take(&mut self.history_log), self.undo_ptr)
    }
}
