# ADR-012: Project Manifest Format and Location

**Date:** 2026-07-13
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

The project/binder feature (R1) requires persisting a **project manifest** — an ordered collection of document paths, display titles, and compile settings — so that a writer can open a multi-chapter book as one unit. The manifest must be saved somewhere on disk, in some format. Two questions arise: (1) What format should the manifest use? (2) Where should it be stored?

The location choice is particularly significant because it determines whether the manifest is **part of the manuscript artifact** (portable, user-visible, backed up with the book) or **hidden metadata** (like sessions, kept in the platform data dir).

## Decision Drivers

- **C4 — Plain files on disk:** The manuscript stays as plain text/Markdown files the user owns; `pstar` metadata lives outside the manuscript folder (per ADR-004's session model)
- **Portability:** A book (manuscript + project structure) should be portable — movable to a new machine, committable to git, shareable with collaborators
- **User awareness:** Unlike per-file sessions (transient editor state), a project is a **first-class artifact** the writer creates, names, and curates — it is not background scaffolding
- **Backup & version control:** The project structure should travel with the manuscript in backups and version control, not be orphaned in a hidden metadata dir
- **Human-readable format:** The writer should be able to inspect or repair the manifest in an emergency without `pstar`
- **Existing patterns:** Sessions use hidden path-hashed JSON; export goes to user-chosen paths; the manifest is more like a user-facing config than a cache

## Considered Options

1. **Visible TOML file in the project folder** — `MyNovel.pstarproj` alongside the manuscript files, containing the doc list and settings
2. **Hidden path-hashed JSON under the metadata root** — `projects/<stem>-<hash>.json`, consistent with `sessions/` (ADR-004)
3. **Dotfile in the project folder** — `.pstar-project.toml` (visible to `ls -a` but hidden by default)
4. **SQLite database in the metadata root** — single DB file with a row per project

## Decision Outcome

**Chosen option:** Visible TOML file (option 1), named `*.pstarproj` in the project folder alongside the manuscript, because a book is a **first-class, user-owned, backup-worthy artifact** — unlike a per-file session (which is transient editor state), a project is something the writer deliberately creates, curates, and wants to preserve. The deviation from "hide all metadata outside the manuscript folder" is intentional: the project manifest **is part of the manuscript** conceptually, and treating it as such serves the writer better than hiding it.

### Positive Consequences

- **Portable projects:** Moving the book's folder to a new machine, committing it to git, or sharing it with a collaborator brings the project structure along automatically — no orphaned metadata
- **User-visible and inspectable:** The writer can see `MyNovel.pstarproj` in their directory listing, confirming the project exists; can inspect/edit it in a pinch (TOML is human-readable)
- **Backup & version control friendly:** The manifest is versioned alongside the manuscript; restoring a backup or checking out an old commit restores the exact project state
- **Consistent with user expectations:** Other writing tools (Scrivener `.scriv` bundles, Ulysses `.ulysses` packages) make the project container visible; hiding it would be surprising
- **TOML format:** Human-readable, has good Rust serde support (`toml` crate), matches `config.toml` so writers who customize settings already know the syntax
- **Distinct extension (`.pstarproj`):** Makes the file's purpose immediately clear; easy to `.gitignore` or exclude from syncing if desired

### Negative Consequences

- **Breaks the C4 "metadata outside the manuscript folder" pattern** — this is the one intentional exception; sessions/snapshots/recovery/stats remain hidden, but the project manifest is elevated to user-facing status
- **Pollutes the manuscript directory** with one additional file — acceptable because it's a single, clearly-named, conceptually-meaningful file, not scattered dotfiles or sidecars
- **Renaming or moving the folder requires updating doc paths in the manifest** — though paths are stored relative to the manifest when possible, so this is mitigated
- **No automatic garbage collection** — if a project is abandoned, the `.pstarproj` file lingers (but so does the manuscript folder itself, so this is consistent)

## Implementation Details

- **File name:** The manifest can have any name with a `.pstarproj` extension (e.g., `MyNovel.pstarproj`, `Dissertation.pstarproj`). The project's display name is stored **inside** the manifest (the `name` field), not derived from the filename, so renaming the file doesn't lose the project name
- **Path storage:** Document paths in the manifest are stored **relative to the manifest's directory** when possible (if the doc is under the project folder), absolute otherwise — this maximizes portability when the whole folder moves
- **Format:** TOML, serde-serialized from `ProjectManifest` struct
- **Discovery:** `pstar` can be launched with a `.pstarproj` path to open the project, or the user can open a project via the new `^PP` command

## Links and References

- **Requirements:** R1.1, R1.4, R1.8 in [`requirements.md`](../../.claude/specs/pro-writer-10-star/requirements.md)
- **Design:** §4.1, §7.1 (D1) in [`design.md`](../../.claude/specs/pro-writer-10-star/design.md)
- **Tasks:** Task 1.1 in [`tasks.md`](../../.claude/specs/pro-writer-10-star/tasks.md)
- **Related ADRs:** [ADR-004](./ADR-004-per-file-session-persistence.md) (sessions hidden under metadata root; projects are different)
- **Implementation:** `src/project.rs` (new)
