//! State for incremental search (^QF) and find & replace (^QA).

/// Incremental search in progress.
pub struct SearchState {
    pub query: String,
    /// Cursor position when the search began (restored on cancel).
    pub origin: usize,
    /// Char index of the current match, if any.
    pub current: Option<usize>,
    /// Set when the last repeat wrapped around the top of the document.
    pub wrapped: bool,
}

impl SearchState {
    pub fn new(origin: usize) -> Self {
        SearchState {
            query: String::new(),
            origin,
            current: None,
            wrapped: false,
        }
    }
}

/// Which input the replace flow is currently collecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacePhase {
    EnterFind,
    EnterWith,
    EnterOptions,
    /// Interactive stepping; the current match is at this char index.
    Confirm(usize),
}

pub struct ReplaceState {
    pub find: String,
    pub with: String,
    /// Option letters: g = from document top, n = no confirmation,
    /// w = whole words only.
    pub options: String,
    pub phase: ReplacePhase,
    pub count: usize,
}

impl ReplaceState {
    pub fn new() -> Self {
        ReplaceState {
            find: String::new(),
            with: String::new(),
            options: String::new(),
            phase: ReplacePhase::EnterFind,
            count: 0,
        }
    }

    pub fn whole_word(&self) -> bool {
        self.options.contains(['w', 'W'])
    }

    pub fn from_top(&self) -> bool {
        self.options.contains(['g', 'G'])
    }

    pub fn no_ask(&self) -> bool {
        self.options.contains(['n', 'N'])
    }
}
