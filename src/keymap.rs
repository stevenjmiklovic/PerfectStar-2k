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
    ExportDocx,
    ExportEpub,
    ExportHtml,
    ExportProjectDocx,
    ExportProjectEpub,
    ExportProjectHtml,
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
    // Project
    ProjectNew,
    ProjectOpen,
    BinderToggle,
    BinderMoveUp,
    BinderMoveDown,
    ProjectAddDoc,
    ProjectRemoveDoc,
    /// Toggle always-on word count display (R2.1).
    WordCount,
    /// Set a session writing goal (R2.3).
    SetGoal,
    /// Show daily writing history overlay (R2.5).
    StatsOverlay,
    /// Search across all project documents (R6.1).
    ProjectFind,
    /// Replace across all project documents (R6.3).
    ProjectReplace,
    /// Snapshots & revisions
    /// Take a snapshot of the current document, with an optional label (R4.1).
    Snapshot,
    /// List this document's snapshots (R4.3).
    RevisionsList,
    /// Start (or stop) a timed / word-target writing sprint (R3.1).
    SprintStart,
    /// Distraction-free view: text only (R3.3).
    FocusMode,
    /// Notes & research (R5)
    /// Edit this document's one-line synopsis (R5.1, R5.3).
    EditSynopsis,
    /// Open this document's freeform notes in a split (R5.1, R5.4).
    OpenNotes,
    /// Mark the selected binder document as a note, or back (R5.2).
    ToggleDocRole,
    /// Open the selected binder document in a split (R5.4).
    BinderOpenSplit,
    /// Editorial annotations (R9)
    /// Comment on the marked block, or at the cursor (R9.1).
    Annotate,
    /// List every comment in the document (R9.4).
    AnnotationList,
    /// Go to the next comment (R9.4).
    NextAnnotation,
    /// Go to the previous comment (R9.4).
    PrevAnnotation,
    /// Style & readability (R8)
    /// Turn style checking on or off (R8.1).
    ToggleStyle,
    /// Jump to the next style issue, as ^QN does for spelling (R8.3).
    NextStyleIssue,
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
    Binding {
        cmd: Cmd::Up,
        chord: Bare('e'),
        name: "cursor up",
    },
    Binding {
        cmd: Cmd::Down,
        chord: Bare('x'),
        name: "cursor down",
    },
    Binding {
        cmd: Cmd::Left,
        chord: Bare('s'),
        name: "cursor left",
    },
    Binding {
        cmd: Cmd::Right,
        chord: Bare('d'),
        name: "cursor right",
    },
    Binding {
        cmd: Cmd::WordLeft,
        chord: Bare('a'),
        name: "word left",
    },
    Binding {
        cmd: Cmd::WordRight,
        chord: Bare('f'),
        name: "word right",
    },
    Binding {
        cmd: Cmd::ScrollUp,
        chord: Bare('w'),
        name: "scroll up",
    },
    Binding {
        cmd: Cmd::ScrollDown,
        chord: Bare('z'),
        name: "scroll down",
    },
    Binding {
        cmd: Cmd::PageUp,
        chord: Bare('r'),
        name: "page up",
    },
    Binding {
        cmd: Cmd::PageDown,
        chord: Bare('c'),
        name: "page down",
    },
    // Editing
    Binding {
        cmd: Cmd::DeleteRight,
        chord: Bare('g'),
        name: "delete char",
    },
    Binding {
        cmd: Cmd::DeleteLeft,
        chord: Bare('h'),
        name: "delete left",
    },
    Binding {
        cmd: Cmd::DeleteWordRight,
        chord: Bare('t'),
        name: "delete word",
    },
    Binding {
        cmd: Cmd::DeleteLine,
        chord: Bare('y'),
        name: "delete line",
    },
    Binding {
        cmd: Cmd::InsertBlankLine,
        chord: Bare('n'),
        name: "insert line",
    },
    Binding {
        cmd: Cmd::Undo,
        chord: Bare('u'),
        name: "undo",
    },
    Binding {
        cmd: Cmd::FindNext,
        chord: Bare('l'),
        name: "find next",
    },
    Binding {
        cmd: Cmd::ToggleInsert,
        chord: Bare('v'),
        name: "insert/overtype",
    },
    // ^Q Quick
    Binding {
        cmd: Cmd::LineStart,
        chord: Pref(Q, 's'),
        name: "line start",
    },
    Binding {
        cmd: Cmd::LineEnd,
        chord: Pref(Q, 'd'),
        name: "line end",
    },
    Binding {
        cmd: Cmd::ScreenTop,
        chord: Pref(Q, 'e'),
        name: "screen top",
    },
    Binding {
        cmd: Cmd::ScreenBottom,
        chord: Pref(Q, 'x'),
        name: "screen bottom",
    },
    Binding {
        cmd: Cmd::DocStart,
        chord: Pref(Q, 'r'),
        name: "document top",
    },
    Binding {
        cmd: Cmd::DocEnd,
        chord: Pref(Q, 'c'),
        name: "document end",
    },
    Binding {
        cmd: Cmd::SentenceBack,
        chord: Pref(Q, ','),
        name: "sentence back",
    },
    Binding {
        cmd: Cmd::SentenceFwd,
        chord: Pref(Q, '.'),
        name: "sentence forward",
    },
    Binding {
        cmd: Cmd::ParaBack,
        chord: Pref(Q, '['),
        name: "paragraph back",
    },
    Binding {
        cmd: Cmd::ParaFwd,
        chord: Pref(Q, ']'),
        name: "paragraph forward",
    },
    Binding {
        cmd: Cmd::PrevPosition,
        chord: Pref(Q, 'p'),
        name: "previous position",
    },
    Binding {
        cmd: Cmd::Outline,
        chord: Pref(Q, 'o'),
        name: "outline / headings",
    },
    Binding {
        cmd: Cmd::NextMisspelling,
        chord: Pref(Q, 'n'),
        name: "next misspelling",
    },
    // Style's next-issue sits beside spelling's, where "next thing to look at"
    // lives (R8.3).
    Binding {
        cmd: Cmd::NextStyleIssue,
        chord: Pref(Q, 'i'),
        name: "next style issue",
    },
    Binding {
        cmd: Cmd::FindIncremental,
        chord: Pref(Q, 'f'),
        name: "find",
    },
    Binding {
        cmd: Cmd::FindReplace,
        chord: Pref(Q, 'a'),
        name: "find & replace",
    },
    Binding {
        cmd: Cmd::DeleteToLineEnd,
        chord: Pref(Q, 'y'),
        name: "delete to line end",
    },
    Binding {
        cmd: Cmd::TransposeWords,
        chord: Pref(Q, 't'),
        name: "transpose words",
    },
    Binding {
        cmd: Cmd::TransposeChars,
        chord: Pref(Q, 'g'),
        name: "transpose chars",
    },
    Binding {
        cmd: Cmd::MacroRecord,
        chord: Pref(Q, 'm'),
        name: "record macro",
    },
    Binding {
        cmd: Cmd::MacroPlay,
        chord: Pref(Q, 'j'),
        name: "play macro",
    },
    Binding {
        cmd: Cmd::JumpBlockBegin,
        chord: Pref(Q, 'b'),
        name: "to block begin",
    },
    Binding {
        cmd: Cmd::JumpBlockEnd,
        chord: Pref(Q, 'k'),
        name: "to block end",
    },
    Binding {
        cmd: Cmd::JumpBlockSource,
        chord: Pref(Q, 'v'),
        name: "to block source",
    },
    // ^K Block & File
    Binding {
        cmd: Cmd::BlockBegin,
        chord: Pref(K, 'b'),
        name: "mark block begin",
    },
    Binding {
        cmd: Cmd::BlockEnd,
        chord: Pref(K, 'k'),
        name: "mark block end",
    },
    Binding {
        cmd: Cmd::BlockCopy,
        chord: Pref(K, 'c'),
        name: "copy block here",
    },
    Binding {
        cmd: Cmd::BlockMove,
        chord: Pref(K, 'v'),
        name: "move block here",
    },
    Binding {
        cmd: Cmd::BlockDelete,
        chord: Pref(K, 'y'),
        name: "delete block",
    },
    Binding {
        cmd: Cmd::BlockWrite,
        chord: Pref(K, 'w'),
        name: "write block to file",
    },
    Binding {
        cmd: Cmd::BlockRead,
        chord: Pref(K, 'r'),
        name: "read file here",
    },
    Binding {
        cmd: Cmd::BlockHide,
        chord: Pref(K, 'h'),
        name: "hide/show block",
    },
    Binding {
        cmd: Cmd::BlockPrev,
        chord: Pref(K, 'u'),
        name: "previous block",
    },
    Binding {
        cmd: Cmd::Put,
        chord: Pref(K, 'p'),
        name: "put (cycle clippings)",
    },
    Binding {
        cmd: Cmd::CopyFromOther,
        chord: Pref(K, 'a'),
        name: "copy block from other window",
    },
    Binding {
        cmd: Cmd::Save,
        chord: Pref(K, 'd'),
        name: "save",
    },
    Binding {
        cmd: Cmd::Save,
        chord: Pref(K, 's'),
        name: "save",
    },
    Binding {
        cmd: Cmd::SaveExit,
        chord: Pref(K, 'x'),
        name: "save & exit",
    },
    Binding {
        cmd: Cmd::Quit,
        chord: Pref(K, 'q'),
        name: "quit",
    },
    Binding {
        cmd: Cmd::ExportClean,
        chord: Pref(K, 'e'),
        name: "export (strip notes)",
    },
    Binding {
        cmd: Cmd::ExportManuscript,
        chord: Pref(K, 'm'),
        name: "export manuscript RTF",
    },
    Binding {
        cmd: Cmd::ExportDocx,
        chord: Pref(K, 'j'),
        name: "export DOCX",
    },
    Binding {
        cmd: Cmd::ExportEpub,
        chord: Pref(K, 'g'),
        name: "export EPUB",
    },
    Binding {
        cmd: Cmd::ExportHtml,
        chord: Pref(K, 'l'),
        name: "export HTML",
    },
    // Snapshots live under ^K with the other file-ish commands. Design §5 asked
    // for ^KL as the revisions list, but ^KL is export HTML; ^KO ("old
    // versions") keeps the group together without stealing a bound chord.
    Binding {
        cmd: Cmd::Snapshot,
        chord: Pref(K, 'n'),
        name: "snapshot now",
    },
    Binding {
        cmd: Cmd::RevisionsList,
        chord: Pref(K, 'o'),
        name: "revisions / snapshots",
    },
    // ^O Onscreen
    Binding {
        cmd: Cmd::CycleTheme,
        chord: Pref(O, 'b'),
        name: "cycle theme",
    },
    Binding {
        cmd: Cmd::RevealCodes,
        chord: Pref(O, 'd'),
        name: "reveal codes",
    },
    Binding {
        cmd: Cmd::ToggleWrap,
        chord: Pref(O, 'w'),
        name: "word wrap on/off",
    },
    Binding {
        cmd: Cmd::SetWrapMargin,
        chord: Pref(O, 'r'),
        name: "set wrap margin",
    },
    Binding {
        cmd: Cmd::CycleHelpLevel,
        chord: Pref(O, 'h'),
        name: "cycle help level",
    },
    Binding {
        cmd: Cmd::ToggleSpellcheck,
        chord: Pref(O, 's'),
        name: "spellcheck on/off",
    },
    Binding {
        cmd: Cmd::AddToDictionary,
        chord: Pref(O, 'a'),
        name: "add word to dictionary",
    },
    Binding {
        cmd: Cmd::ToggleTypewriter,
        chord: Pref(O, 't'),
        name: "typewriter scrolling on/off",
    },
    Binding {
        cmd: Cmd::ToggleStyle,
        chord: Pref(O, 'y'),
        name: "style checking on/off",
    },
    Binding {
        cmd: Cmd::OtherWindow,
        chord: Pref(O, 'k'),
        name: "other window (open/switch)",
    },
    Binding {
        cmd: Cmd::WordCount,
        chord: Pref(O, 'c'),
        name: "toggle word count",
    },
    Binding {
        cmd: Cmd::SetGoal,
        chord: Pref(O, 'g'),
        name: "set session goal",
    },
    Binding {
        cmd: Cmd::StatsOverlay,
        chord: Pref(O, 'i'),
        name: "writing stats",
    },
    // Sprints and focus are screen-and-session concerns, so they sit with the
    // other ^O toggles next to the session goal (^OG).
    Binding {
        cmd: Cmd::SprintStart,
        chord: Pref(O, 'p'),
        name: "writing sprint start/stop",
    },
    Binding {
        cmd: Cmd::FocusMode,
        chord: Pref(O, 'f'),
        name: "focus mode (text only)",
    },
    // ^P Project prefix
    Binding {
        cmd: Cmd::ProjectNew,
        chord: Pref(P, 'n'),
        name: "new project",
    },
    Binding {
        cmd: Cmd::ProjectOpen,
        chord: Pref(P, 'p'),
        name: "open project",
    },
    Binding {
        cmd: Cmd::BinderToggle,
        chord: Pref(P, 'b'),
        name: "toggle binder",
    },
    Binding {
        cmd: Cmd::BinderMoveUp,
        chord: Pref(P, 'e'),
        name: "binder: move doc up",
    },
    Binding {
        cmd: Cmd::BinderMoveDown,
        chord: Pref(P, 'x'),
        name: "binder: move doc down",
    },
    Binding {
        cmd: Cmd::ProjectAddDoc,
        chord: Pref(P, 'a'),
        name: "add document to project",
    },
    Binding {
        cmd: Cmd::ProjectRemoveDoc,
        chord: Pref(P, 'r'),
        name: "remove document from project",
    },
    Binding {
        cmd: Cmd::ExportProjectDocx,
        chord: Pref(P, 'd'),
        name: "export project DOCX",
    },
    Binding {
        cmd: Cmd::ExportProjectEpub,
        chord: Pref(P, 'f'),
        name: "export project EPUB",
    },
    Binding {
        cmd: Cmd::ExportProjectHtml,
        chord: Pref(P, 'h'),
        name: "export project HTML",
    },
    Binding {
        cmd: Cmd::ProjectFind,
        chord: Pref(P, 's'),
        name: "project search",
    },
    Binding {
        cmd: Cmd::ProjectReplace,
        chord: Pref(P, 'w'),
        name: "project replace",
    },
    // Notes & research sit under ^P with the rest of the project furniture.
    Binding {
        cmd: Cmd::EditSynopsis,
        chord: Pref(P, 'i'),
        name: "document synopsis",
    },
    Binding {
        cmd: Cmd::OpenNotes,
        chord: Pref(P, 't'),
        name: "document notes in a split",
    },
    Binding {
        cmd: Cmd::ToggleDocRole,
        chord: Pref(P, 'm'),
        name: "binder: mark document as note",
    },
    Binding {
        cmd: Cmd::BinderOpenSplit,
        chord: Pref(P, 'v'),
        name: "binder: open document in a split",
    },
    // Annotations: Comment, List, Go to next, go Up to previous.
    Binding {
        cmd: Cmd::Annotate,
        chord: Pref(P, 'c'),
        name: "comment on block or cursor",
    },
    Binding {
        cmd: Cmd::AnnotationList,
        chord: Pref(P, 'l'),
        name: "list comments",
    },
    Binding {
        cmd: Cmd::NextAnnotation,
        chord: Pref(P, 'g'),
        name: "next comment",
    },
    Binding {
        cmd: Cmd::PrevAnnotation,
        chord: Pref(P, 'u'),
        name: "previous comment",
    },
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
    fn project_prefix_has_commands() {
        // Task 1.2: ^PP opens a project manifest. The menu should show it.
        assert!(!menu_entries(Prefix::P).is_empty());
        assert_eq!(lookup_prefixed(Prefix::P, 'p'), Some(Cmd::ProjectOpen));
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
    fn snapshot_commands_are_bound_without_stealing_a_chord() {
        assert_eq!(lookup_prefixed(Prefix::K, 'n'), Some(Cmd::Snapshot));
        assert_eq!(lookup_prefixed(Prefix::K, 'o'), Some(Cmd::RevisionsList));
        // The chords they might have collided with are untouched.
        assert_eq!(lookup_prefixed(Prefix::K, 'l'), Some(Cmd::ExportHtml));
        assert_eq!(lookup_bare('n'), Some(Cmd::InsertBlankLine));
    }

    #[test]
    fn every_command_is_reachable_by_name_in_the_palette() {
        // R12.1/R12.4: the palette is generated from BINDINGS, so a command
        // with a chord but no descriptive name would ship undiscoverable.
        let entries = palette_entries();
        for cmd in [Cmd::Snapshot, Cmd::RevisionsList] {
            let entry = entries.iter().find(|(c, _, _)| *c == cmd);
            let (_, name, chord) = entry.expect("command missing from palette");
            assert!(!name.is_empty(), "{cmd:?} has no name");
            assert!(chord.starts_with("^K"), "{cmd:?} chord is {chord}");
        }
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
