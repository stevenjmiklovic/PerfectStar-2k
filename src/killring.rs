//! The kill ring, after Emacs: every substantial deletion or block copy is
//! kept, and ^KP ("put") pastes the newest — pressed again immediately, it
//! cycles the put text through older and older clippings.

use std::collections::VecDeque;

const MAX_ITEMS: usize = 60;

pub struct KillRing {
    /// Front is newest.
    items: VecDeque<String>,
}

/// An in-progress put-cycle: where the last put landed and which ring index
/// it used, so the next ^KP can swap it for the following item.
#[derive(Debug, Clone, Copy)]
pub struct PutCycle {
    pub at: usize,
    pub chars: usize,
    pub index: usize,
}

impl KillRing {
    pub fn new() -> Self {
        KillRing {
            items: VecDeque::new(),
        }
    }

    pub fn push(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        // Mirror to the OS clipboard; ignore failures (headless, etc.).
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(&text);
        }
        self.items.push_front(text);
        self.items.truncate(MAX_ITEMS);
    }

    /// Newest item, falling back to the OS clipboard when the ring is empty.
    pub fn top(&mut self) -> Option<String> {
        if self.items.is_empty() {
            if let Ok(text) = arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
                if !text.is_empty() {
                    self.items.push_front(text);
                }
            }
        }
        self.items.front().cloned()
    }

    pub fn get(&self, index: usize) -> Option<&String> {
        self.items.get(index)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

impl Default for KillRing {
    fn default() -> Self {
        Self::new()
    }
}
