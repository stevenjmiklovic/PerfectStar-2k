# ADR-003: Never-Lose Undo Model (Emacs-Style)

**Date:** 2025-06-15
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

Writers work in long sessions and make changes they may want to reverse hours later. A conventional undo/redo stack discards "redo" history the moment the user types after undoing — states become permanently unreachable. For a tool whose philosophy is "never lose work," the undo system must guarantee that every prior buffer state remains accessible regardless of editing after an undo.

## Decision Drivers

- No buffer state may ever become unreachable — a writer must always be able to get back
- Undo must be a single key (`^U`) with no separate "redo" key — repeated `^U` walks backward, and any other command breaks the chain so subsequent `^U` undoes the undos
- Consecutive same-kind edits (a burst of typing, a sequence of backspaces) should coalesce into one undo step so `^U` doesn't step through every individual character
- The full undo log must be serializable for session persistence (see ADR-004)

## Considered Options

1. **Emacs-style append-only log** — undoing appends the inverse edit as a new entry; the log only grows
2. **Linear undo/redo stack** — separate undo and redo stacks; redo is discarded on new edits
3. **Tree-based undo (vim undotree)** — full branching history tree with branch navigation UI

## Decision Outcome

**Chosen option:** Emacs-style append-only log, because it satisfies "never lose" with minimal complexity: the log is a flat `Vec<EditGroup>`, undo appends rather than pops, and there is no separate redo concept to explain or bind.

### Positive Consequences

- No state is ever lost — the log only grows, so any point in history remains reachable
- Single-key operation: `^U` is the only undo/redo key; the `undo_ptr` tracks where in the log the chain is walking
- Coalescing (`EditKind::InsertChar` and `DeleteLeft` runs merge into one group) keeps the log manageable for long typing bursts
- The log serializes trivially to JSON for session persistence (`Vec<EditGroup>` with `#[derive(Serialize, Deserialize)]`)
- `break_chain()` on any non-undo command is a one-line state reset — no complex tree navigation

### Negative Consequences

- The log grows without bound during a session; for extremely long editing sessions the in-memory `Vec` could become large (mitigated: session save/restore keeps it bounded per file across runs)
- No visual "undo tree" — the user cannot see or choose branches; they must undo sequentially
- Coalescing heuristics (`MAX_GROUP_EDITS = 32`, breaking on movement/mode changes) are tuned by feel rather than a formal specification

## Links and References

- Implementation: `src/history.rs` (`History` struct, `EditGroup`, `EditKind`, `record`, `next_undo`, `confirm_undo`)
- Integration: `src/app.rs` (`apply_edit`, `undo`, `break_chain`/`break_group` calls)
- Persistence: `src/session.rs` (serializes `history_log` and `undo_ptr`)
- Branch: main
