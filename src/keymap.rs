//! The central command table: every command's name and chord in one place.
//! The key dispatcher, the delayed prefix menus, the command palette, and the
//! help screen are all generated from this table.

/// A prefix key (^K, ^Q, ^O, ^P) awaiting its second key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefix {
    K,
    Q,
    O,
    P,
}

impl Prefix {
    pub fn label(self) -> &'static str {
        match self {
            Prefix::K => "^K Block & File",
            Prefix::Q => "^Q Quick",
            Prefix::O => "^O Onscreen",
            Prefix::P => "^P Project",
        }
    }
}

/// Every editor command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    // Movement — the diamond and friends
    Up,
    Down,
    Left,
    Right,
    WordLeft,
    WordRight,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    // Movement — ^Q quick
    LineStart,
    LineEnd,
    ScreenTop,
    ScreenBottom,
    DocStart,
    DocEnd,
    SentenceBack,
    SentenceFwd,
    ParaBack,
    ParaFwd,
    PrevPosition,
    Outline,
    NextMisspelling,
    // Editing
    DeleteRight,
    DeleteLeft,
    DeleteWordRight,
    DeleteLine,
    DeleteToLineEnd,
    InsertBlankLine,
    TransposeWords,
    TransposeChars,
    Undo,
    // Search
    FindIncremental,
    FindReplace,
    FindNext,
    // Macros & palette
    MacroRecord,
    MacroPlay,
    Palette,
    // Blocks & kill ring
    BlockBegin,
    BlockEnd,
    BlockCopy,
    BlockMove,
    BlockDelete,
    BlockWrite,
    BlockRead,
    BlockHide,
    BlockPrev,
    Put,
    CopyFromOther,
    JumpBlockBegin,
    JumpBlockEnd,
    JumpBlockSource,
    // File
    Save,
    SaveExit,
    Quit,
    ExportClean,
    ExportManuscript,
    // Onscreen
    CycleTheme,
    RevealCodes,
    ToggleWrap,
    SetWrapMargin,
    ToggleInsert,
    CycleHelpLevel,
    ToggleSpellcheck,
    AddToDictionary,
    ToggleTypewriter,
    OtherWindow,
}

/// How a command is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chord {
    /// Ctrl+letter, no prefix.
    Bare(char),
    /// Prefix then a second key (typed plain or with Ctrl held).
    Pref(Prefix, char),
}

pub struct Binding {
    pub cmd: Cmd,
    pub chord: Chord,
    pub name: &'static str,
}

use Chord::{Bare, Pref};
use Prefix::{K, O, P, Q};

pub const BINDINGS: &[Binding] = &[
    // The diamond
    Binding { cmd: Cmd::Up, chord: Bare('e'), name: "cursor up" },
    Binding { cmd: Cmd::Down, chord: Bare('x'), name: "cursor down" },
    Binding { cmd: Cmd::Left, chord: Bare('s'), name: "cursor left" },
    Binding { cmd: Cmd::Right, chord: Bare('d'), name: "cursor right" },
    Binding { cmd: Cmd::WordLeft, chord: Bare('a'), name: "word left" },
    Binding { cmd: Cmd::WordRight, chord: Bare('f'), name: "word right" },
    Binding { cmd: Cmd::ScrollUp, chord: Bare('w'), name: "scroll up" },
    Binding { cmd: Cmd::ScrollDown, chord: Bare('z'), name: "scroll down" },
    Binding { cmd: Cmd::PageUp, chord: Bare('r'), name: "page up" },
    Binding { cmd: Cmd::PageDown, chord: Bare('c'), name: "page down" },
    // Editing
    Binding { cmd: Cmd::DeleteRight, chord: Bare('g'), name: "delete char" },
    Binding { cmd: Cmd::DeleteLeft, chord: Bare('h'), name: "delete left" },
    Binding { cmd: Cmd::DeleteWordRight, chord: Bare('t'), name: "delete word" },
    Binding { cmd: Cmd::DeleteLine, chord: Bare('y'), name: "delete line" },
    Binding { cmd: Cmd::InsertBlankLine, chord: Bare('n'), name: "insert line" },
    Binding { cmd: Cmd::Undo, chord: Bare('u'), name: "undo" },
    Binding { cmd: Cmd::FindNext, chord: Bare('l'), name: "find next" },
    Binding { cmd: Cmd::ToggleInsert, chord: Bare('v'), name: "insert/overtype" },
    // ^Q Quick
    Binding { cmd: Cmd::LineStart, chord: Pref(Q, 's'), name: "line start" },
    Binding { cmd: Cmd::LineEnd, chord: Pref(Q, 'd'), name: "line end" },
    Binding { cmd: Cmd::ScreenTop, chord: Pref(Q, 'e'), name: "screen top" },
    Binding { cmd: Cmd::ScreenBottom, chord: Pref(Q, 'x'), name: "screen bottom" },
    Binding { cmd: Cmd::DocStart, chord: Pref(Q, 'r'), name: "document top" },
    Binding { cmd: Cmd::DocEnd, chord: Pref(Q, 'c'), name: "document end" },
    Binding { cmd: Cmd::SentenceBack, chord: Pref(Q, ','), name: "sentence back" },
    Binding { cmd: Cmd::SentenceFwd, chord: Pref(Q, '.'), name: "sentence forward" },
    Binding { cmd: Cmd::ParaBack, chord: Pref(Q, '['), name: "paragraph back" },
    Binding { cmd: Cmd::ParaFwd, chord: Pref(Q, ']'), name: "paragraph forward" },
    Binding { cmd: Cmd::PrevPosition, chord: Pref(Q, 'p'), name: "previous position" },
    Binding { cmd: Cmd::Outline, chord: Pref(Q, 'o'), name: "outline / headings" },
    Binding { cmd: Cmd::NextMisspelling, chord: Pref(Q, 'n'), name: "next misspelling" },
    Binding { cmd: Cmd::FindIncremental, chord: Pref(Q, 'f'), name: "find" },
    Binding { cmd: Cmd::FindReplace, chord: Pref(Q, 'a'), name: "find & replace" },
    Binding { cmd: Cmd::DeleteToLineEnd, chord: Pref(Q, 'y'), name: "delete to line end" },
    Binding { cmd: Cmd::TransposeWords, chord: Pref(Q, 't'), name: "transpose words" },
    Binding { cmd: Cmd::TransposeChars, chord: Pref(Q, 'g'), name: "transpose chars" },
    Binding { cmd: Cmd::MacroRecord, chord: Pref(Q, 'm'), name: "record macro" },
    Binding { cmd: Cmd::MacroPlay, chord: Pref(Q, 'j'), name: "play macro" },
    Binding { cmd: Cmd::JumpBlockBegin, chord: Pref(Q, 'b'), name: "to block begin" },
    Binding { cmd: Cmd::JumpBlockEnd, chord: Pref(Q, 'k'), name: "to block end" },
    Binding { cmd: Cmd::JumpBlockSource, chord: Pref(Q, 'v'), name: "to block source" },
    // ^K Block & File
    Binding { cmd: Cmd::BlockBegin, chord: Pref(K, 'b'), name: "mark block begin" },
    Binding { cmd: Cmd::BlockEnd, chord: Pref(K, 'k'), name: "mark block end" },
    Binding { cmd: Cmd::BlockCopy, chord: Pref(K, 'c'), name: "copy block here" },
    Binding { cmd: Cmd::BlockMove, chord: Pref(K, 'v'), name: "move block here" },
    Binding { cmd: Cmd::BlockDelete, chord: Pref(K, 'y'), name: "delete block" },
    Binding { cmd: Cmd::BlockWrite, chord: Pref(K, 'w'), name: "write block to file" },
    Binding { cmd: Cmd::BlockRead, chord: Pref(K, 'r'), name: "read file here" },
    Binding { cmd: Cmd::BlockHide, chord: Pref(K, 'h'), name: "hide/show block" },
    Binding { cmd: Cmd::BlockPrev, chord: Pref(K, 'u'), name: "previous block" },
    Binding { cmd: Cmd::Put, chord: Pref(K, 'p'), name: "put (cycle clippings)" },
    Binding { cmd: Cmd::CopyFromOther, chord: Pref(K, 'a'), name: "copy block from other window" },
    Binding { cmd: Cmd::Save, chord: Pref(K, 'd'), name: "save" },
    Binding { cmd: Cmd::Save, chord: Pref(K, 's'), name: "save" },
    Binding { cmd: Cmd::SaveExit, chord: Pref(K, 'x'), name: "save & exit" },
    Binding { cmd: Cmd::Quit, chord: Pref(K, 'q'), name: "quit" },
    Binding { cmd: Cmd::ExportClean, chord: Pref(K, 'e'), name: "export (strip notes)" },
    Binding { cmd: Cmd::ExportManuscript, chord: Pref(K, 'm'), name: "export manuscript RTF" },
    // ^O Onscreen
    Binding { cmd: Cmd::CycleTheme, chord: Pref(O, 'b'), name: "cycle theme" },
    Binding { cmd: Cmd::RevealCodes, chord: Pref(O, 'd'), name: "reveal codes" },
    Binding { cmd: Cmd::ToggleWrap, chord: Pref(O, 'w'), name: "word wrap on/off" },
    Binding { cmd: Cmd::SetWrapMargin, chord: Pref(O, 'r'), name: "set wrap margin" },
    Binding { cmd: Cmd::CycleHelpLevel, chord: Pref(O, 'h'), name: "cycle help level" },
    Binding { cmd: Cmd::ToggleSpellcheck, chord: Pref(O, 's'), name: "spellcheck on/off" },
    Binding { cmd: Cmd::AddToDictionary, chord: Pref(O, 'a'), name: "add word to dictionary" },
    Binding { cmd: Cmd::ToggleTypewriter, chord: Pref(O, 't'), name: "typewriter scrolling on/off" },
    Binding { cmd: Cmd::OtherWindow, chord: Pref(O, 'k'), name: "other window (open/switch)" },
];

/// Human-readable chord for display in menus, the palette, and help.
pub fn chord_label(chord: Chord) -> String {
    match chord {
        Bare(c) => format!("^{}", c.to_ascii_uppercase()),
        Pref(p, c) => {
            let p = match p {
                K => 'K',
                Q => 'Q',
                O => 'O',
                P => 'P',
            };
            format!("^{p}{}", c.to_ascii_uppercase())
        }
    }
}

/// Palette entries whose names contain `query` (case-insensitive).
pub fn filtered_entries(query: &str) -> Vec<(Cmd, &'static str, String)> {
    let q = query.to_lowercase();
    palette_entries()
        .into_iter()
        .filter(|(_, name, chord)| {
            q.is_empty() || name.to_lowercase().contains(&q) || chord.to_lowercase().contains(&q)
        })
        .collect()
}

/// All commands for the palette/help, deduplicated, in table order.
pub fn palette_entries() -> Vec<(Cmd, &'static str, String)> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for b in BINDINGS {
        if !seen.contains(&b.cmd) {
            seen.push(b.cmd);
            out.push((b.cmd, b.name, chord_label(b.chord)));
        }
    }
    out
}

pub fn lookup_bare(c: char) -> Option<Cmd> {
    BINDINGS.iter().find_map(|b| match b.chord {
        Bare(k) if k == c => Some(b.cmd),
        _ => None,
    })
}

pub fn lookup_prefixed(prefix: Prefix, c: char) -> Option<Cmd> {
    BINDINGS.iter().find_map(|b| match b.chord {
        Pref(p, k) if p == prefix && k == c => Some(b.cmd),
        _ => None,
    })
}

/// Menu entries for a prefix, in table order, deduplicated by command.
pub fn menu_entries(prefix: Prefix) -> Vec<(char, &'static str)> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for b in BINDINGS {
        if let Pref(p, k) = b.chord {
            if p == prefix && !seen.contains(&b.cmd) {
                seen.push(b.cmd);
                out.push((k, b.name));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_prefix_has_a_label() {
        assert_eq!(Prefix::P.label(), "^P Project");
    }

    #[test]
    fn project_prefix_has_no_commands_yet() {
        // Task 0.4 adds only the prefix; commands come in later phases. The
        // menu machinery must handle an empty prefix without panicking so the
        // delayed menu simply shows nothing.
        assert!(menu_entries(Prefix::P).is_empty());
        assert!(lookup_prefixed(Prefix::P, 'p').is_none());
    }

    #[test]
    fn existing_prefixes_still_populated() {
        // Guard against the new prefix accidentally shadowing the others.
        assert!(!menu_entries(Prefix::K).is_empty());
        assert!(!menu_entries(Prefix::Q).is_empty());
        assert!(!menu_entries(Prefix::O).is_empty());
    }

    #[test]
    fn chord_label_renders_p_prefix() {
        assert_eq!(chord_label(Pref(P, 'b')), "^PB");
    }

    #[test]
    fn palette_entries_are_deduplicated() {
        // Save is bound to both ^KD and ^KS; it must appear once.
        let saves = palette_entries()
            .into_iter()
            .filter(|(cmd, _, _)| *cmd == Cmd::Save)
            .count();
        assert_eq!(saves, 1);
    }
}
