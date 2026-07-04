# ADR-001: Use Ropey for Text Storage

**Date:** 2025-06-15
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

PerfectStar 2k is a TUI text editor designed for writers working with prose manuscripts. The text buffer must support efficient insertion and deletion at arbitrary positions (typing mid-document), fast line indexing (for rendering visible lines), and char-index-based cursor addressing (for grapheme-correct movement). Manuscripts can reach hundreds of thousands of characters, ruling out naive `String` or `Vec<char>` representations.

## Decision Drivers

- O(log n) insert/delete at arbitrary positions for responsive editing in large documents
- Fast line-to-char and char-to-line conversion for viewport rendering
- Direct char-index addressing to unify cursor position, block marks, bookmarks, and undo history
- Memory efficiency for documents that stay open for hours
- Pure Rust, no FFI — aligns with the project's zero-dependency-on-C goal for spellcheck (spellbook) and clipboard (arboard)

## Considered Options

1. **Ropey** — a rope (balanced tree of text chunks) with built-in line and char indexing
2. **xi-rope** — the rope from the xi-editor project, more complex API, designed for async multi-view
3. **Gap buffer** — classic editor data structure (used by Emacs), simple but O(n) line indexing
4. **Vec\<String\> (line array)** — trivial to implement, O(n) for char-index lookup across lines

## Decision Outcome

**Chosen option:** Ropey, because it provides O(log n) edits, built-in char↔line conversion, streaming file I/O (`from_reader`/`write_to`), and a small, stable API that maps directly to the editor's char-index addressing model.

### Positive Consequences

- All positions (cursor, bookmarks, block marks, undo edits, jump stack) are plain `usize` char indices — a single coordinate system across the entire codebase
- `Rope::from_reader` and `Rope::write_to` enable streaming open/save without loading the full file into a contiguous `String`
- Line indexing (`char_to_line`, `line_to_char`) is O(log n), making viewport rendering fast regardless of document size
- Slice iteration (`rope.slice(range)`) avoids allocating for read-only operations like search and word counting

### Negative Consequences

- Grapheme-cluster boundaries are not built into Ropey — the Buffer layer wraps every movement in `unicode-segmentation` calls (`prev_grapheme`, `next_grapheme`)
- Char-index semantics mean off-by-one errors are possible at line terminators; the `line_end` helper must trim `\n`/`\r\n` manually
- The rope's chunk boundaries are invisible but can split multi-byte characters, requiring care when interfacing with byte-oriented APIs (none currently needed)

## Links and References

- Implementation: `src/buffer.rs` (the `Buffer` struct wraps `ropey::Rope`)
- Ropey crate: https://crates.io/crates/ropey
- Branch: main
