# ADR-014: Adopt the `similar` Crate for Revision Diffing

**Date:** 2026-08-19
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

R4 asks for a revision viewer: a writer takes a snapshot before a big revision, then later compares that snapshot against the current draft (or against another snapshot) with added and removed text highlighted, and optionally restores it. That needs a diff.

Prose diffing is not the same problem as source diffing. A revised paragraph is one very long "line" whose words moved; a naive longest-common-subsequence over lines reports the whole paragraph as deleted-then-added, which tells the writer nothing. Getting useful output means a real diff algorithm with the usual practical refinements — common-prefix/suffix trimming, unique-element heuristics for large inputs, and the option to diff at word granularity within a changed region.

The Phase 7 specification called this decision "ADR-010," but that number is already occupied by the mpv IPC decision. This record uses the next available repository number.

## Decision Drivers

- Diff quality on prose, where paragraphs are long and edits are word-level
- Fully offline operation, in keeping with every other subsystem
- Latency budget: a chapter-sized diff must feel instant inside the TUI's draw loop
- The repository's standing bias against dependency weight (ADR-006, ADR-013)
- Correctness is verifiable but tedious: Myers with heuristics is a well-known source of subtle bugs
- Restore must feed the existing `EditGroup` history path, which constrains nothing about the diff itself

## Considered Options

1. Adopt `similar` (pure Rust, line/word/character granularity, no runtime dependencies)
2. Hand-roll a Myers line diff
3. Hand-roll a full word-level diff with heuristics

## Decision Outcome

**Chosen option:** adopt `similar`.

This is a deliberate exception to the hand-generation precedent set by ADR-006 and ADR-013, and the distinction is worth stating: those decisions concerned *output formats*, where the standard is written down and hand-generation is mostly transcription. A diff is an *algorithm* whose quality is a research result, not a transcription — and unlike an RTF header, a subtly wrong diff misleads the writer about what they changed.

`similar` v3 is pure Rust, needs no network or system library, and adds **zero runtime dependencies** to the build graph with its default `std` + `text` features (`bstr` appears in `Cargo.lock` only for the crate's own optional/dev features and is never compiled into `pstar`). `src/diff.rs` wraps it behind a small `DiffLine`/`DiffTag` model so the rest of the editor — and any future replacement of the engine — talks to `pstar`'s own types rather than to `similar`'s.

### Positive Consequences

- Diff quality is a solved problem rather than a maintenance surface
- Zero runtime dependencies added; offline operation is unaffected
- Word- and character-level granularity is available for later inline highlighting without another decision
- The `DiffLine` wrapper keeps `similar` out of `app.rs`/`ui.rs`, so the engine is replaceable
- License is MIT/Apache-2.0, compatible with the project and consistent with the existing notices

### Negative Consequences

- First third-party algorithm dependency in the editor core; `THIRD-PARTY-NOTICES.md` must track it
- The crate is larger than the line diff actually used today
- Diff output shape is the crate's to define; changing engines later means re-verifying the snapshot tests

## Links and References

- Diff wrapper: `src/diff.rs`
- Snapshot store: `src/snapshot.rs`
- Revision list and diff view: `src/app.rs`, `src/ui.rs`
- Undo contract for restore: `docs/adr/ADR-003-never-lose-undo.md`
- Hand-generation precedent this deviates from: `docs/adr/ADR-006-hand-generated-rtf-manuscript-export.md`, `docs/adr/ADR-013-hand-generated-docx-and-epub.md`
