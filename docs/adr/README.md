# Architecture Decision Records

This directory captures the significant architectural choices made in PerfectStar 2k — a TUI text editor for writers, built on WordStar's chord-based input language and WordPerfect's focused writing aesthetic.

Each ADR documents the context, the decision, and its consequences so future contributors understand *why* the system is shaped the way it is.

## Index

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [ADR-001](./ADR-001-ropey-for-text-storage.md) | Use Ropey for text storage | Accepted | 2025-06-15 |
| [ADR-002](./ADR-002-wordstar-chord-keymap.md) | WordStar chord-based keymap with static binding table | Accepted | 2025-06-15 |
| [ADR-003](./ADR-003-never-lose-undo.md) | Never-lose undo model (Emacs-style) | Accepted | 2025-06-15 |
| [ADR-004](./ADR-004-per-file-session-persistence.md) | Per-file session persistence keyed by path hash | Accepted | 2025-06-15 |
| [ADR-005](./ADR-005-bundled-hunspell-spellcheck.md) | Bundled Hunspell dictionary via spellbook (offline spellcheck) | Accepted | 2025-06-15 |
| [ADR-006](./ADR-006-hand-generated-rtf-manuscript-export.md) | Hand-generated RTF for manuscript export | Accepted | 2025-06-15 |
| [ADR-007](./ADR-007-dos-accurate-truecolor-themes.md) | DOS-accurate truecolor theme system | Accepted | 2025-06-15 |
| [ADR-008](./ADR-008-cargo-workspace-restructure.md) | Cargo workspace restructure | Accepted | 2025-07-14 |
| [ADR-009](./ADR-009-polling-crate-event-loop.md) | Polling crate for lightweight event loop | Accepted | 2025-07-14 |
| [ADR-010](./ADR-010-cross-platform-mpv-ipc.md) | Cross-platform mpv IPC via trait abstraction | Accepted | 2025-07-14 |
| [ADR-011](./ADR-011-shell-out-integration-for-radio.md) | Shell-out integration pattern for pstar-radio | Accepted | 2025-07-14 |
| [ADR-012](./ADR-012-project-manifest-format-and-location.md) | Project manifest format and location | Accepted | 2025-07-14 |
| [ADR-013](./ADR-013-hand-generated-docx-and-epub.md) | Hand-generated DOCX and EPUB containers | Accepted | 2026-07-14 |

## Format

ADRs follow [MADR](https://adr.github.io/madr/) (Markdown Any Decision Records). Each record includes:

- **Context and Problem Statement** — what prompted the decision
- **Decision Drivers** — constraints and goals that shaped the choice
- **Considered Options** — alternatives evaluated
- **Decision Outcome** — what was chosen and why
- **Consequences** — trade-offs accepted

## Adding a New ADR

1. Assign the next sequential number
2. Name the file `ADR-{NNN}-{kebab-case-title}.md`
3. Use the full or short-form MADR template
4. Update this index table
