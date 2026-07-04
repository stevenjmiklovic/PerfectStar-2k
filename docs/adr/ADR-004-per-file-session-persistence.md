# ADR-004: Per-File Session Persistence Keyed by Path Hash

**Date:** 2025-06-15
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

A writer reopening a manuscript should find their "fingers in the pages" — cursor position, bookmarks, block marks, the jump ring, and full undo history restored exactly as left. This state must persist without polluting the document's directory (manuscripts live in synced folders, git repos, or Dropbox — no dotfiles or sidecars). The persistence format must also handle the case where the file was edited externally between sessions.

## Decision Drivers

- Session state stored outside the document's folder to keep manuscript directories clean
- Keyed by canonical file path so the same file always maps to the same session regardless of how it's opened (relative vs. absolute path)
- Stale detection: if the file changed externally, positions are invalid and the session must be discarded rather than restored to garbage offsets
- Must serialize the full undo history (`Vec<EditGroup>`) for never-lose-undo continuity across sessions
- Platform-appropriate storage location (`dirs::state_dir` or `dirs::data_local_dir`)

## Considered Options

1. **JSON files in platform state dir, keyed by path hash** — `{stem}-{hash}.json` under `~/.local/state/perfectstar2k/sessions/`
2. **SQLite database** — single DB file with a row per document
3. **Sidecar files (`.pstar-session`)** — stored alongside the manuscript
4. **No persistence** — start fresh every time

## Decision Outcome

**Chosen option:** JSON files keyed by the canonical path's `DefaultHasher` hash, because it keeps session storage invisible to the user, requires no database dependency, and maps naturally to serde's `Serialize`/`Deserialize` derives already on the session types.

### Positive Consequences

- Manuscript folders stay completely clean — no dotfiles, no sidecars, no metadata pollution
- `DefaultHasher` on the canonicalized path handles symlinks, relative paths, and case differences (on case-sensitive filesystems)
- Stale detection via `len_chars` comparison: if the file's char count changed since the session was saved, the entire session is discarded safely
- JSON format is human-readable for debugging and trivially extensible with `#[serde(default)]`
- The undo history survives across sessions — closing and reopening doesn't lose undo states

### Negative Consequences

- `DefaultHasher` is not guaranteed stable across Rust versions — a toolchain upgrade could orphan existing session files (acceptable: sessions are a convenience, not critical data)
- No garbage collection: session files for deleted manuscripts accumulate until manually purged
- Large undo histories produce large JSON files; no compaction or pruning is applied
- Renaming a file creates a new session key — the old session becomes orphaned

## Links and References

- Implementation: `src/session.rs` (`Session` struct, `load`, `save`, `session_path`)
- Integration: `src/app.rs` (`restore_session`, `save_session` called on init and quit)
- Relates to: [ADR-003](./ADR-003-never-lose-undo.md) (undo history is part of the persisted session)
- Branch: main
