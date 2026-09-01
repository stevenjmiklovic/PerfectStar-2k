# Changelog

<!-- changelogging: start -->

## 0.5.0 (2026-09-01)

The **P1 milestone**: a professional writer's daily workflow now includes snapshots, revision recovery, focused sprints, editorial metadata, style/readability checks, offline lookup and autocorrect, and guided discovery without leaving the keyboard.

### Features

- **Snapshots and revision diff (R4).** Capture named plain-text snapshots, compare revisions, and restore a prior snapshot as one undoable edit without mutating the live buffer during capture.
- **Writing sprints and focus mode (R3).** Set time and/or word targets, track progress, and record completed or cancelled sprint results in session history.
- **Editorial metadata and notes (R5, R9).** Persist synopsis, annotations, note documents, and `..` note-line metadata alongside each manuscript without affecting exported prose.
- **Style and readability checks (R8).** Apply fixed bundled style rules with an overlay for actionable prose feedback.
- **Offline lookup and autocorrect (R10).** Add bundled dictionary/thesaurus lookup, synonym replacement, autocorrect, and independently toggleable smart typography for quotes, dashes, and ellipses.
- **First-use hints (R12).** Show non-modal, one-time feature discovery hints that respect the configured help level and dismiss automatically.

### Internal

- Added revision, metadata, style, lookup, and autocorrect modules plus bundled offline resources.
- Added property-based coverage for snapshot, metadata, and sprint invariants.
- Added latency and export-throughput harnesses for the release performance budgets.
- Added ADRs 014–016 for revision diff, fixed style rules, and bundled offline lookup resources.

## 0.4.0 (2026-07-14)

The **P0 milestone**: a writer can run a whole book in `pstar` and trust it — multi-file project management, professional export, data safety, statistics, and project-wide search.

### Features

- **Project/binder (R1).** Multi-file manuscript management via `.pstarproj` TOML manifests. Binder panel (^PB) with reorder (^PE/^PX), add (^PA), remove (^PR); compile concatenates included docs in binder order with configurable separators (page break, blank lines, horizontal rule, or none). ADR-012.
- **Professional exports (R7).** Shared `CompiledDoc` intermediate model with `Exporter` trait. RTF unchanged (^KM, golden-file proven). New hand-generated HTML (^KL), DOCX (^KJ), and EPUB (^KG) — all fully offline, no external dependencies. Project export (^PD/^PF/^PH) compiles binder order with separator semantics. Atomic replacement so a failed export never clobbers a good one. ADR-013.
- **Writing statistics & goals (R2).** Prose-aware incremental word/char counting (notes and markers excluded, matching export). Cached per-line counts updated on edit, debounced full recount on idle. Status-line shows word count (^OC toggle), selection count when a block is active, and session goal progress. Session goals (^OG) with non-blocking notification. Daily words-written history persisted as JSON, viewable via ^OI overlay.
- **Project-wide search & replace (R6).** Search all project docs (^PS) with smartcase and whole-word matching. Navigable results overlay with file, line, and context. Jump-to-result opens the doc at the match. Replace (^PW) offers per-match confirm (^R) or replace-all (^A), opening each affected file as an undoable edit through atomic save — no silent unreviewable writes.
- **Backup & crash recovery (R11).** Save-failure handling with alternate-path prompt. Crash-recovery journals written on idle and offered on startup as one undoable edit. Rolling timestamped backups with configurable depth. Every file write uses temp-then-rename so a manuscript is never truncated.

### Internal

- New modules: `project.rs`, `recovery.rs`, `export/` (mod, docx, epub, html, zip), `stats.rs`, `projsearch.rs`.
- New `^P` prefix for project commands (^PN, ^PP, ^PB, ^PE, ^PX, ^PA, ^PR, ^PD, ^PF, ^PH, ^PS, ^PW).
- New `^O` commands: ^OC (word count toggle), ^OG (set goal), ^OI (stats overlay).
- `normalize.rs` is the single source of truth for prose definition (notes, markers, typography).
- `paths.rs` extended with `stats()`, `recovery()`, and other metadata-root accessors.
- 151 tests across all modules.

## 0.3.0 (2026-07-04)

### Features

- Split-window support (^OK) with independent per-pane block marks, bookmarks, and undo history.
- Cross-window block copy (^KA): copy the marked block from the other window to the cursor.
- ^KQ/^KX with two windows closes just the active pane rather than quitting.

## 0.2.0 (2026-07-04)

### Features

- Three color themes: WordPerfect-blue (exact IBM CGA/VGA truecolor), WordStar black, and terminal-native.
- User configuration via `~/.config/perfectstar2k/config.toml` (theme, autosave, wrap margin, menu delay, help level).
- Inline Markdown styling (bold, italic, code spans, headings) with visible dimmed markers.
- Outline navigation: jump between Markdown headings via searchable list.
- Bundled en_US Hunspell dictionary with global personal wordlist for character names and coinages.
- Standard manuscript format RTF export (^KM) per William Shunn's submission guide.
- Per-file session persistence: cursor, bookmarks, block marks, jump stack, and full undo history restored on reopen.
- DOS-style block-letter splash screen dismissed by any keypress.
- Central command table with prefix chords (^K, ^Q, ^O) driving menus, palette, and help.
- TUI rendering with ratatui: status line, ruler, word-wrap viewport, prefix menus, command palette, help overlay, and Reveal Codes panel.
- System clipboard integration via arboard.
- Autosave on idle with configurable interval.
- Typewriter scrolling mode (^OT).

## 0.1.0 (2026-07-04)

### Features

- Rope-backed text buffer with grapheme-cluster-aware cursor movement and deletion.
- Never-lose undo/redo log (Emacs model: undo appends inverse, no state unreachable).
- Persistent block marks and ten numbered bookmarks, adjusted across edits.
- 60-item kill ring with put cycling (^KP cycles through older clippings in place).
- Incremental search (^QF) and find & replace (^QA) with wrap detection.
- Jump ring (^QP) for returning to previous positions after long-range navigation.
- Kitty keyboard protocol support for disambiguating ^J/Enter, ^H/Backspace, ^I/Tab.
