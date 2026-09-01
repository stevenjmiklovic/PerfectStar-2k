# Requirements Document

## Introduction

PerfectStar 2k is already a capable daily-driver writing tool: WordStar chord editing, persistent blocks/bookmarks/jump ring, a never-lose undo log, a shared kill ring, per-file sessions, inline Markdown, Reveal Codes, bundled Hunspell spellcheck, split windows, DOS-accurate themes, macros, and standard-manuscript RTF export.

That set makes it an excellent single-document editor. The gap between "a great editor" and a "10-star" application a professional writer chooses over Scrivener, Ulysses, or Word is not more editing commands — it is the apparatus *around* the prose: managing a book made of many files, knowing whether today's session hit its target, revising with confidence across drafts, keeping research and characters at hand, and getting clean output in the formats agents and publishers actually accept.

This document defines the requirements for that apparatus. It is scoped to what a working novelist, non-fiction author, or long-form journalist needs to run an entire project inside `pstar` — not just edit one file in it.

### Personas

- **Nadia — the novelist.** Writing a 110k-word fantasy novel across ~40 chapter files. Cares about project navigation, word-count goals, consistent character names, and a clean submission manuscript.
- **Marcus — the non-fiction author.** Writing a business book with heavy research notes and citations. Cares about linking notes to prose, footnotes, and DOCX/EPUB output for his editor.
- **Priya — the long-form journalist.** 6,000-word feature on deadline. Cares about sprints/timers, distraction-free focus, revision history against an editor's cuts, and fast multi-file search.

### Design Constraints (inherited, non-negotiable)

- **C1 — Keyboard-only, hands on home row.** Every new capability must be reachable via Ctrl-chord commands consistent with the existing WordStar keymap (ADR-002). No feature may require a mouse.
- **C2 — Terminal-native TUI.** All UI is ratatui inside a terminal. New surfaces are panels, overlays, or panes — never external windows.
- **C3 — Never lose work.** Consistent with the never-lose undo model (ADR-003) and atomic saves, no new feature may introduce a path that can silently destroy or corrupt a writer's text.
- **C4 — Plain files on disk.** The manuscript stays as plain text / Markdown files the user owns; pstar metadata lives outside the manuscript folder (as sessions already do, ADR-004). No proprietary opaque container.
- **C5 — Offline and dependency-light.** Core writing features work with no network and no external binaries, consistent with bundled spellcheck (ADR-005) and hand-generated RTF (ADR-006).
- **C6 — Performance.** The editor must stay responsive (sub-frame keystroke latency) on manuscripts of at least 300,000 words, respecting the ropey choice (ADR-001).

### Priority Key

- **P0** = table-stakes for professional use (ship first)
- **P1** = strong differentiators
- **P2** = polish / delight

### Out of Scope

Real-time collaboration / multi-user editing; cloud account sync as a hosted service; AI text generation; a GUI front-end; PDF typesetting engine (handled via export handoff). These may be future specs.

## Glossary

- **Project**: A named collection of manuscript files (chapters, parts, front/back matter, notes) that pstar treats as one book.
- **Binder**: The navigable tree/list of a project's documents.
- **Sprint**: A timed or word-count-bounded writing session.
- **Snapshot**: A point-in-time saved copy of a document's text for later comparison or restoration.
- **Session_Target**: A word-count or time goal for the current sitting.
- **Style_Issue**: A prose-quality flag (passive voice, adverb, filter word, crutch/overused word, long sentence) distinct from a spelling error.
- **Annotation**: An inline comment/editorial mark anchored to a position or block in the document, excluded from exports.
- **Manifest**: The project configuration file that defines binder order and project metadata.
- **Focus_Mode**: A distraction-free presentation that hides all chrome except the text.

## Prioritization Summary

| Priority | Requirements |
|----------|-------------|
| P0 | R1 (Binder), R2 (Statistics), R4 (Snapshots), R6 (Search), R7 (Export), R11 (Recovery) |
| P1 | R3 (Sprints), R5 (Notes), R8 (Style), R9 (Annotations) |
| P2 | R10 (Dictionary), R12 (Help) |

## Requirements

### Requirement 1: Project / Manuscript Management (Binder) [P0]

**User Story:** As Nadia (novelist), I want to open a whole book made of many chapter files as one project, so that I can navigate, reorder, and write across chapters without juggling file paths.

#### Acceptance Criteria

1. THE Binder_System SHALL support defining a Project as an ordered collection of document files, persisted in a Manifest stored outside the manuscript folder (per C4)
2. WHEN the user opens a Project, THE Binder_System SHALL present a binder panel (Ctrl-chord toggle) listing the Project's documents in author-defined order, displaying per-document the title (derived from the document's first Markdown heading, falling back to the filename if no heading exists) and word count
3. WHEN the user selects a document in the Binder, THE Binder_System SHALL save the currently open document (consistent with existing autosave behavior and C3), then open the selected document in the active pane, preserving that document's session state (cursor, bookmarks, undo) per ADR-004
4. THE Binder_System SHALL allow reordering documents in the Binder via keyboard commands, and SHALL persist the new order to the Manifest atomically
5. THE Binder_System SHALL allow adding an existing file to, and removing a file from, a Project without moving or deleting the file on disk unless the user explicitly requests deletion
6. WHEN the user requests "compile," THE Binder_System SHALL produce a single concatenated document from the binder order with configurable separators between documents, defaulting to a page-break separator per document
7. IF a file referenced by the Manifest is missing at open time, THEN THE Binder_System SHALL show it as "missing" in the Binder and SHALL NOT fail to open the rest of the Project
8. WHERE no Project is defined, THE Binder_System SHALL continue to operate as a single-file editor with no behavior change (backward compatibility)
9. THE Binder_System SHALL provide a command to create a new Project from a user-specified name, producing an empty Manifest to which documents can then be added (per criterion 5)
10. IF the Manifest file is missing, unreadable, or contains malformed data at open time, THEN THE Binder_System SHALL report the error to the user and SHALL NOT discard or overwrite the existing Manifest file

### Requirement 2: Writing Statistics and Goals [P0]

**User Story:** As Priya (journalist) on deadline, I want live word counts and a session target with progress, so that I know when I've hit my goal for the day.

#### Acceptance Criteria

1. THE Statistics_Engine SHALL display the word count and character count for the current document and for the whole Project in the status bar, toggled between always-visible and on-demand via a keyboard command; WHEN displayed on demand, the counts SHALL appear for at least 3 seconds or until dismissed
2. WHILE a selection/block is active, THE Statistics_Engine SHALL display the word and character count of the selection in the status bar, replacing or augmenting the document-level counts
3. THE Statistics_Engine SHALL let the user set a Session_Target expressed as a word-count goal (minimum 1 word, maximum 1,000,000 words) or a time goal (minimum 1 minute, maximum 480 minutes), and SHALL display live progress toward it in the status bar, updated within 1 second of each edit or each elapsed minute respectively; progress SHALL be measured as net words added (insertions minus deletions) since the target was set
4. WHEN a Session_Target is reached, THE Statistics_Engine SHALL display a non-blocking, non-modal notification in the status bar that does not interrupt typing and does not require dismissal (per C1/C3); the notification SHALL remain visible for at least 5 seconds
5. IF the user quits or the application terminates while a Session_Target is active, THEN THE Statistics_Engine SHALL persist the session state (target value, baseline word count, start time) so that reopening the same project resumes progress tracking from where it left off
6. THE Statistics_Engine SHALL record per-day words-written totals (delta of net words added, not gross keystrokes) and SHALL make the daily history viewable via a keyboard command; history SHALL be retained for at least 365 days and stored as a plain-text file outside the manuscript folder (per C4)
7. THE Statistics_Engine SHALL exclude `..` note lines and Markdown syntax markers (heading markers, emphasis delimiters, link syntax) from the prose word count and character count, consistent with export stripping, and SHALL count words as runs of alphanumeric/Unicode characters and graphemes correctly for the buffer's Unicode content (per ADR-001)
8. THE Statistics_Engine SHALL NOT introduce more than 16ms of additional keystroke-to-screen latency on a 300,000-word Project (per C6); counts MAY be updated incrementally or debounced rather than recomputed from scratch on every edit

### Requirement 3: Sprints, Timers, and Focus [P1]

**User Story:** As Priya (journalist), I want a writing sprint with a countdown and a distraction-free view, so that I can produce a rough draft fast without fiddling.

#### Acceptance Criteria

1. THE Sprint_System SHALL provide a sprint command that accepts a duration (1 to 180 minutes) and/or a word goal (1 to 100,000 words), and SHALL display remaining time and/or remaining words in the status area without obscuring the text editing region.
2. IF both a duration and a word goal are set, THEN THE Sprint_System SHALL end the sprint when either target is reached first.
3. WHEN a sprint ends (by reaching its target or by the user issuing a cancel command), THE Sprint_System SHALL display a non-modal summary showing words written during the sprint and elapsed time, and SHALL append these figures to the session history (R2.5).
4. IF the user cancels a sprint before its target is reached, THEN THE Sprint_System SHALL still record words written and elapsed time up to the cancellation point.
5. THE Sprint_System SHALL provide a Focus_Mode toggled by a single chord command that hides the status bar, hint bar, prefix menus, and binder panel — leaving only the text area and the sprint timer (if a sprint is active) visible.
6. WHERE Focus_Mode is active, THE Sprint_System MAY optionally dim all lines except the current paragraph, and this dimming SHALL be configurable off via `config.toml`.
7. THE Sprint_System SHALL treat sprint and focus states as purely presentational and SHALL NOT alter the text buffer or saved files.

### Requirement 4: Revision History, Snapshots, and Drafts [P0]

**User Story:** As Marcus (non-fiction author), I want to snapshot a chapter before a big revision and compare it later, so that I can revise fearlessly and recover cut material.

#### Acceptance Criteria

1. THE Snapshot_System SHALL let the user take a named Snapshot of the current document on demand, stored outside the manuscript folder (C4), with labels of up to 80 characters; there SHALL be no limit on the number of manual Snapshots per document
2. THE Snapshot_System SHALL take an automatic Snapshot at a configurable cadence (default: every 10 minutes of active editing) and/or on each save, retaining a configurable number of automatic Snapshots (default: 20 per document); WHEN the retention limit is exceeded, THE Snapshot_System SHALL delete the oldest automatic Snapshot first (FIFO)
3. THE Snapshot_System SHALL list a document's Snapshots with timestamp, optional label, word count, and type (manual or automatic), sorted newest-first
4. WHEN the user selects two versions (a Snapshot and current, or two Snapshots), THE Snapshot_System SHALL show a diff highlighting added and removed text with distinct visual markers, and SHALL allow keyboard navigation between diff hunks (next/previous change)
5. WHEN the user chooses to restore a Snapshot, THE Snapshot_System SHALL replace the current buffer with the Snapshot content as a single undoable operation (never-lose undo, ADR-003) — restoration SHALL be reversible via the standard undo command
6. IF disk space or write fails while creating a Snapshot, THEN THE Snapshot_System SHALL display a warning indicating the failure reason, SHALL NOT lose the working buffer (C3), and SHALL NOT interrupt the current editing session
7. THE Snapshot_System SHALL store Snapshots as plain text so they remain recoverable without pstar (C4/C5)
8. Snapshot creation SHALL NOT introduce perceptible typing latency on documents up to 300,000 words (C6); writing the Snapshot to disk MAY occur asynchronously after the in-memory copy is captured

### Requirement 5: Notes, Research, and Metadata Sidecar [P1]

**User Story:** As Marcus (non-fiction author), I want research notes, a synopsis, and character sheets attached to my project and reachable while I write, so that continuity details are one keystroke away.

#### Acceptance Criteria

1. THE Notes_System SHALL let each document carry a synopsis (maximum 500 characters, truncated in display contexts) and freeform notes stored as plain-text sidecar metadata outside the manuscript folder (C4), using the same platform data directory as sessions.
2. THE Notes_System SHALL provide project-level note documents (e.g., characters, places, timeline) stored as plain-text files (C4), editable within pstar with full editing capabilities (undo, session persistence, spellcheck, split-pane support), supporting at least 100 note documents per project.
3. THE Notes_System SHALL show a document's synopsis in the Binder as a secondary line, toggled visible or hidden via a Ctrl-chord command consistent with the existing keymap (C1).
4. THE Notes_System SHALL provide a Ctrl-chord command to open a split pane (per existing ^OK) showing a chosen note document alongside the manuscript, navigable and selectable via keyboard (C1).
5. WHERE the user invokes a quick-lookup command on a word or selection, THE Notes_System MAY surface matching note entries in a non-modal overlay within 200ms of invocation, and this SHALL NOT block typing or insert latency into the keystroke path.
6. THE Notes_System SHALL autosave note and metadata edits using the same idle-timeout mechanism as the existing autosave model (config `autosave_secs`), with atomic writes so a failed save never leaves a partial sidecar file (C3).
7. IF a sidecar metadata file is missing or unreadable when a document is opened, THEN THE Notes_System SHALL treat the synopsis and notes as empty, SHALL NOT prevent the document from opening, and SHALL recreate the sidecar on the next autosave cycle.
8. IF a project-level note document file is missing from disk, THEN THE Notes_System SHALL display it as "missing" in the notes listing (consistent with R1.7 binder behavior) and SHALL NOT fail to load the rest of the project's notes.

### Requirement 6: Multi-File Search and Replace [P0]

**User Story:** As Nadia (novelist), I want to find every place a character's name appears across all chapters and rename it, so that a late naming change doesn't mean opening 40 files by hand.

#### Acceptance Criteria

1. THE Search_Engine SHALL provide search across the whole Project, returning a navigable results list showing each match with its file name, line number, and up to 40 characters of surrounding context on either side of the match
2. WHEN the user selects a search result, THE Search_Engine SHALL open that document and place the cursor at the start of the match, restoring that document's session state per ADR-004
3. THE Search_Engine SHALL support project-wide replace with per-match confirm (Y/N/A to replace-all-remaining/Q to quit, consistent with existing ^QA options), whole-word, and case-sensitive/case-insensitive matching
4. WHEN a project-wide replace is confirmed, THE Search_Engine SHALL modify each affected file via the existing atomic-save path, and each change SHALL be undoable within its document's undo log independently of other files
5. IF an atomic save fails for one file during a project-wide replace, THEN THE Search_Engine SHALL report the failure for that file, SHALL preserve the in-memory buffer intact, SHALL continue processing remaining files, and SHALL present a summary of which files succeeded and which failed
6. THE Search_Engine SHALL support the current incremental-search ergonomics for the in-document case (no regression to ^QF/^L)
7. IF a project-wide replace touches an unopened file, THEN THE Search_Engine SHALL open it into a buffer to apply the edit as an undoable operation; the modified buffer SHALL be marked dirty and SHALL NOT be persisted to disk until the user explicitly saves or confirms (no silent unreviewable writes, per C3)
8. THE Search_Engine SHALL support both literal string search and simple wildcard patterns (consistent with the existing single-file ^QF pattern support); full regular-expression search is not required
9. THE Search_Engine SHALL display the first batch of project-wide search results within 500 milliseconds on a 300k-word / ~40-file Project (C6); additional results MAY stream in incrementally as they are found

### Requirement 7: Professional Export Formats [P0]

**User Story:** As Marcus (non-fiction author), I want to export my book to DOCX and EPUB (in addition to the existing manuscript RTF), so that I can hand my editor and my distributor the formats they require.

#### Acceptance Criteria

1. THE Export_System SHALL retain the existing standard-manuscript-format RTF export (^KM) with no regression in output fidelity, smart typography, note-line stripping, or chapter page-break behavior (ADR-006)
2. THE Export_System SHALL provide a keyboard-invoked export command that lets the user choose the target format (RTF, DOCX, EPUB, HTML, or plain text) and confirm or change the output file path before writing
3. THE Export_System SHALL export a document or a compiled Project to DOCX, mapping Markdown heading levels 1–3 to the corresponding Word heading styles, preserving bold and italic emphasis as character runs, and maintaining paragraph breaks as distinct paragraphs
4. THE Export_System SHALL export a compiled Project to EPUB 3 with a navigation table of contents containing one entry per heading level 1 or level 2 in binder order
5. THE Export_System SHALL export to HTML (structural tags only: headings, paragraphs, em, strong — no inline styles or scripts) and to a plain-text copy with note lines stripped (existing ^KE behavior retained)
6. WHEN exporting a Project, THE Export_System SHALL concatenate documents in binder order and insert the per-document compile separator (page break or configurable marker) between documents
7. THE Export_System SHALL strip note lines (lines beginning with `..`) and upgrade straight quotes and hyphens to typographic curly quotes, em dashes, and ellipses, consistent with existing manuscript export behavior
8. WHERE an export format requires an external tool or crate not bundled with the binary, THE Export_System SHALL state the missing dependency name in the status area and SHALL skip that format in the format-selection list rather than presenting it and failing
9. WHEN an export completes, THE Export_System SHALL display the absolute output path in the status area; IF export fails, THEN THE Export_System SHALL display an error message indicating the cause, SHALL leave any previously exported file at that path intact, and SHALL NOT write a partial or corrupt output file
10. THE Export_System SHALL produce DOCX and EPUB output for a 300,000-word compiled Project within 30 seconds on the reference hardware, and SHALL not block editor input during export (export MAY run asynchronously with a progress indication)

### Requirement 8: Style and Readability Checking [P1]

**User Story:** As Priya (journalist), I want to see passive voice, adverbs, filter words, and overused words flagged like spelling is, so that I can self-edit a tighter draft.

#### Acceptance Criteria

1. THE Style_Checker SHALL provide optional style checks detecting at minimum: passive voice constructions, adverbs ending in -ly, a bundled list of at least 200 filter/crutch words (configurable), and sentences exceeding a configurable word count threshold (default: 40 words)
2. WHERE style checking is enabled, THE Style_Checker SHALL visually distinguish Style_Issues from spelling errors using a different underline color or marker style, consistent with the existing spellcheck rendering approach
3. THE Style_Checker SHALL provide a command to jump to the next Style_Issue in document order, analogous to ^QN for spelling, wrapping from end to beginning
4. THE Style_Checker SHALL compute readability statistics (Flesch-Kincaid grade level, average sentence length in words, adverb-per-1000-words ratio, and percentage of sentences flagged as long) for the document or selection on demand, displayed in an overlay panel
5. THE Style_Checker SHALL provide an overused-word / word-frequency report showing words appearing more than a configurable threshold (default: 3 times per 1000 words) for the document or Project, sorted by frequency descending
6. THE Style_Checker SHALL be fully offline (C5) and SHALL NOT introduce more than 16ms of additional keystroke-to-screen latency on a 300,000-word document (C6); analysis SHALL run on changed paragraphs only (debounced, not full-document rescan on each keystroke)
7. THE Style_Checker SHALL make each style-check category (passive voice, adverbs, filter words, long sentences) independently toggleable via config.toml, with all categories enabled by default

### Requirement 9: Editorial Annotations and Comments [P1]

**User Story:** As Marcus (non-fiction author) incorporating an editor's feedback, I want inline comments and editorial marks that never appear in the exported prose, so that I can track revision tasks alongside the text.

#### Acceptance Criteria

1. THE Annotation_System SHALL let the user attach an inline Annotation anchored to a contiguous character range (one or more characters) in the document, where the Annotation body is freeform text up to 2000 characters
2. THE Annotation_System SHALL display Annotations distinctly (e.g., margin marker or dimmed panel) without altering the prose flow, consistent with how note lines render dimmed; WHILE the cursor is within an annotated range, the system SHALL show the Annotation body in a non-overlapping panel or status area
3. THE Annotation_System SHALL exclude Annotations from all prose exports (per R7), the same way note lines are stripped
4. THE Annotation_System SHALL provide navigation to the next/previous Annotation and a list of all Annotations in the document/Project, where each list entry shows the anchored text (first 40 characters), the Annotation body (first 60 characters), and the document name
5. THE Annotation_System SHALL adjust Annotation anchors across edits so they stay attached to the intended text, consistent with how block marks and bookmarks are adjusted across edits
6. IF the anchored text is deleted, THEN THE Annotation_System SHALL preserve the comment as orphaned rather than lose it silently (C3), and SHALL display orphaned Annotations in a distinct "orphaned" state in the Annotation list so the user can relocate or dismiss them
7. THE Annotation_System SHALL let the user edit the body of an existing Annotation, and delete an Annotation, each as a single undoable operation
8. THE Annotation_System SHALL persist Annotations in sidecar storage outside the manuscript folder (per C4), keyed to the document, and SHALL load them when the document is opened
9. IF the user creates an Annotation with an empty body, THEN THE Annotation_System SHALL reject the creation and indicate that a non-empty body is required

### Requirement 10: Dictionary, Thesaurus, and Autocorrect [P2]

**User Story:** As any writer, I want a thesaurus and definition lookup and smart autocorrect for the word under the cursor, so that I don't break flow to open a browser.

#### Acceptance Criteria

1. THE Lookup_System SHALL provide a thesaurus lookup for the word under the cursor or a selection, presented in an overlay panel dismissed by Escape or selection, working offline (C5), displaying results within 500 milliseconds of invocation
2. THE Lookup_System SHALL provide a definition lookup for the word under the cursor, presented in an overlay panel dismissed by Escape, working offline (C5)
3. WHEN the user chooses a synonym from the lookup, THE Lookup_System SHALL replace the word as a single undoable edit
4. THE Lookup_System SHALL support autocorrect/expansion rules that fire immediately after the user types a word separator (space, punctuation, or Enter) following a matching token, are user-configurable via the existing config mechanism, support at least 500 rules, and can be globally disabled
5. WHEN autocorrect replaces a word, THE Lookup_System SHALL treat the replacement as a single undoable edit so that a single undo command immediately after the substitution restores the original typed text
6. THE Lookup_System SHALL perform smart typographic substitution (straight quotes to curly quotes, double hyphens to em-dash, triple periods to ellipsis) as the user types, consistent with the existing export substitution rules, toggleable independently of autocorrect via configuration
7. WHERE a lookup resource (thesaurus or definition dictionary) is not bundled or fails to load, THE Lookup_System SHALL display a non-blocking message in the status area indicating which resource is unavailable and SHALL disable the corresponding command until the resource is available, rather than producing an unhandled error
8. Autocorrect and typographic substitution SHALL NOT introduce perceptible typing latency on a 300,000-word document (C6); rule matching SHALL complete within the keystroke processing budget

### Requirement 11: Backup, Crash Recovery, and Data Safety [P0]

**User Story:** As any professional whose livelihood is the manuscript, I want absolute confidence that a crash, a bad edit, or a full disk never loses my work, so that I can trust pstar with a career's worth of writing.

#### Acceptance Criteria

1. THE Recovery_System SHALL write a crash-recovery file for each dirty buffer after every edit group boundary and after each autosave interval elapses, such that after an abnormal termination, reopening the same document offers to restore unsaved changes from the most recent recovery file
2. THE Recovery_System SHALL keep the existing one-time .bak on save and atomic-write behavior (write to temp file in same directory, then rename over original) with no regression
3. THE Recovery_System SHALL support a configurable rolling backup (timestamped copies) of edited documents, stored outside the manuscript folder, retaining at most 10 copies per document by default (oldest deleted when the limit is exceeded), with the retention count configurable via config.toml
4. IF a save fails (disk full, permissions, read-only media), THEN THE Recovery_System SHALL display an error message indicating the failure reason, SHALL retain the in-memory buffer and dirty state intact, and SHALL prompt for an alternate file path via the same text-input mechanism used for other file operations
5. THE Recovery_System SHALL never leave a manuscript file partially written; a failed write to the temp file or a failed rename SHALL leave the previous good file untouched (atomic replace)
6. THE Recovery_System SHALL store crash-recovery files and rolling backups as plain UTF-8 text identical in format to the manuscript source, recoverable by opening them directly with any text editor without requiring pstar
7. IF the Recovery_System cannot write or update the crash-recovery file (e.g., state directory is full or inaccessible), THEN THE Recovery_System SHALL display a status-bar warning indicating that crash recovery is unavailable for the current session

### Requirement 12: Discoverability, Onboarding, and Help [P2]

**User Story:** As a writer new to WordStar chords, I want the deep feature set to be discoverable without a manual, so that the power doesn't come at the cost of a cliff-like learning curve.

#### Acceptance Criteria

1. THE Help_System SHALL include every command added by new features in the BINDINGS table with a human-readable name and chord, so that the command palette (searchable by name or chord string) and prefix menus automatically list them without additional UI code
2. WHILE help_level is 0, THE Help_System SHALL hide all prefix menus, hint bars, and first-use hints for new features, leaving only the text editing area and status bar visible; WHILE help_level is 1, THE Help_System SHALL show delayed prefix menus (including any new prefix groups such as ^P) when the prefix key is held; WHILE help_level is 2, THE Help_System SHALL additionally show the hint bar listing available commands for new features
3. WHEN a new capability is invoked for the first time, THE Help_System MAY display a one-line hint in the status bar area that does not steal keyboard focus and does not obscure document text; the hint SHALL auto-dismiss after 4 seconds or immediately on any keypress, and SHALL not reappear for that capability once dismissed
4. IF the user sets help_level to 0, THEN THE Help_System SHALL suppress all first-use hints globally regardless of whether individual capabilities have been used before
5. THE Help_System SHALL make all new features reachable from the command palette by descriptive name (the `name` field in BINDINGS) even if the user does not know the chord, ensuring keyboard-only discoverability (C1)
