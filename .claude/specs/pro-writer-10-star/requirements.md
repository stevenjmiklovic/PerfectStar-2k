# Requirements — "10-Star" Feature Set for Professional Writers

- **Feature slug:** `pro-writer-10-star`
- **Status:** Draft (requirements phase)
- **Author:** Spec development workflow
- **Date:** 2026-07-04
- **Applies to:** PerfectStar 2k (`pstar`), Rust + ratatui TUI editor

---

## 1. Introduction

PerfectStar 2k is already a capable daily-driver writing tool: WordStar chord
editing, persistent blocks/bookmarks/jump ring, a never-lose undo log, a shared
kill ring, per-file sessions, inline Markdown, Reveal Codes, bundled Hunspell
spellcheck, split windows, DOS-accurate themes, macros, and standard-manuscript
RTF export.

That set makes it an excellent **single-document editor**. The gap between "a
great editor" and a **"10-star" application a professional writer chooses over
Scrivener, Ulysses, or Word** is not more editing commands — it is the
apparatus *around* the prose: managing a book made of many files, knowing
whether today's session hit its target, revising with confidence across drafts,
keeping research and characters at hand, and getting clean output in the
formats agents and publishers actually accept.

This document defines the requirements for that apparatus. It is scoped to what
a working novelist, non-fiction author, or long-form journalist needs to run an
entire project inside `pstar` — not just edit one file in it. Requirements are
grouped by theme and each theme carries a **priority** so the design and task
phases can be sequenced.

### 1.1 Design constraints (inherited, non-negotiable)

These are properties of the existing product that every requirement below must
respect. They come from the current architecture and ADRs and are stated here
so the design phase treats them as fixed inputs, not open questions.

- **C1 — Keyboard-only, hands on home row.** Every new capability must be
  reachable via Ctrl-chord commands consistent with the existing WordStar
  keymap (ADR-002). No feature may *require* a mouse.
- **C2 — Terminal-native TUI.** All UI is ratatui inside a terminal. New
  surfaces are panels, overlays, or panes — never external windows.
- **C3 — Never lose work.** Consistent with the never-lose undo model
  (ADR-003) and atomic saves, no new feature may introduce a path that can
  silently destroy or corrupt a writer's text.
- **C4 — Plain files on disk.** The manuscript stays as plain text / Markdown
  files the user owns; `pstar` metadata lives outside the manuscript folder
  (as sessions already do, ADR-004). No proprietary opaque container.
- **C5 — Offline and dependency-light.** Core writing features work with no
  network and no external binaries, consistent with bundled spellcheck
  (ADR-005) and hand-generated RTF (ADR-006).
- **C6 — Performance.** The editor must stay responsive (sub-frame keystroke
  latency) on manuscripts of at least 300,000 words, respecting the ropey
  choice (ADR-001).

### 1.2 Personas

- **Nadia — the novelist.** Writing a 110k-word fantasy novel across ~40
  chapter files. Cares about project navigation, word-count goals, consistent
  character names, and a clean submission manuscript.
- **Marcus — the non-fiction author.** Writing a business book with heavy
  research notes and citations. Cares about linking notes to prose, footnotes,
  and DOCX/EPUB output for his editor.
- **Priya — the long-form journalist.** 6,000-word feature on deadline. Cares
  about sprints/timers, distraction-free focus, revision history against an
  editor's cuts, and fast multi-file search.

### 1.3 Out of scope (this feature)

Real-time collaboration / multi-user editing; cloud account sync as a hosted
service; AI text generation; a GUI front-end; PDF typesetting engine
(handled via export handoff, see R7). These may be future specs.

---

## 2. Glossary

- **Project** — a named collection of manuscript files (chapters, parts,
  front/back matter, notes) that `pstar` treats as one book.
- **Binder** — the navigable tree/list of a project's documents.
- **Sprint** — a timed or word-count-bounded writing session.
- **Snapshot** — a point-in-time saved copy of a document's text for later
  comparison or restoration.
- **Session target** — a word-count or time goal for the current sitting.
- **Style issue** — a prose-quality flag (passive voice, adverb, filter word,
  crutch/overused word, long sentence) distinct from a spelling error.

---

## 3. Requirements

Priority key: **P0** = table-stakes for professional use (ship first);
**P1** = strong differentiators; **P2** = polish / delight.

Acceptance criteria use EARS notation:
- *Ubiquitous:* "The system SHALL …"
- *Event-driven:* "WHEN <trigger>, the system SHALL …"
- *State-driven:* "WHILE <state>, the system SHALL …"
- *Optional:* "WHERE <feature enabled>, the system SHALL …"
- *Unwanted:* "IF <condition>, THEN the system SHALL …"

---

### R1 — Project / manuscript management (binder) — **P0**

**User story.** As Nadia, I want to open a whole book made of many chapter
files as one project, so that I can navigate, reorder, and write across
chapters without juggling file paths.

**Acceptance criteria**

1. The system SHALL support defining a **project** as an ordered collection of
   document files, persisted in a project manifest stored outside the
   manuscript folder (per C4).
2. WHEN the user opens a project, the system SHALL present a **binder** panel
   (Ctrl-chord toggle) listing the project's documents in author-defined order
   with title and word count per document.
3. WHEN the user selects a document in the binder, the system SHALL open it in
   the active pane, preserving that document's session state (cursor,
   bookmarks, undo) per ADR-004.
4. The system SHALL allow reordering documents in the binder via keyboard
   commands, and SHALL persist the new order to the manifest atomically.
5. The system SHALL allow adding an existing file to, and removing a file from,
   a project without moving or deleting the file on disk unless the user
   explicitly requests deletion.
6. WHEN the user requests "compile," the system SHALL produce a single
   concatenated document from the binder order (see R7 export) with
   configurable separators between documents (e.g., page break per chapter).
7. IF a file referenced by the manifest is missing at open time, THEN the
   system SHALL show it as "missing" in the binder and SHALL NOT fail to open
   the rest of the project.
8. WHERE no project is defined, the system SHALL continue to operate as a
   single-file editor with no behavior change (backward compatibility).

---

### R2 — Writing statistics & goals — **P0**

**User story.** As Priya on deadline, I want live word counts and a session
target with progress, so that I know when I've hit my goal for the day.

**Acceptance criteria**

1. The system SHALL display, on demand and optionally always-on in the status
   area, the **word count** and **character count** for the current document
   and for the whole project.
2. WHILE a selection/block is active, the system SHALL report the word and
   character count of the selection.
3. The system SHALL let the user set a **session target** expressed as words
   written or minutes elapsed, and SHALL display live progress toward it.
4. WHEN a session target is reached, the system SHALL give a non-blocking,
   non-modal notification that does not interrupt typing (per C1/C3).
5. The system SHALL record per-day **words-written** totals (delta of net
   words added, not gross keystrokes) and SHALL make the daily history
   viewable.
6. Word counting SHALL exclude `..` note lines and Markdown syntax markers
   from the "prose" count, consistent with export stripping, and SHALL count
   graphemes/words correctly for the buffer's Unicode content (per ADR-001).
7. Statistics computation SHALL NOT introduce perceptible typing latency on a
   300k-word project (per C6); counts MAY be updated incrementally/debounced.

---

### R3 — Sprints, timers, and focus — **P1**

**User story.** As Priya, I want a writing sprint with a countdown and a
distraction-free view, so that I can produce a rough draft fast without
fiddling.

**Acceptance criteria**

1. The system SHALL provide a **sprint** command that starts a timer (duration)
   and/or word goal and shows remaining time/words unobtrusively.
2. WHEN a sprint ends, the system SHALL report words written and elapsed time
   for that sprint and append it to session history (R2.5).
3. The system SHALL provide a **focus mode** that hides all chrome except the
   text (extending existing help-level 0 and typewriter scrolling), toggled by
   a single command.
4. WHERE focus mode is active, the system MAY optionally dim all but the
   current sentence/paragraph ("hemingway"/"typewriter" emphasis), and this
   SHALL be configurable off.
5. Sprint and focus states SHALL be purely presentational and SHALL NOT alter
   the text or saved files.

---

### R4 — Revision history, snapshots & drafts — **P0**

**User story.** As Marcus, I want to snapshot a chapter before a big revision
and compare it later, so that I can revise fearlessly and recover cut material.

**Acceptance criteria**

1. The system SHALL let the user take a named **snapshot** of the current
   document on demand, stored outside the manuscript folder (C4).
2. The system SHALL take an automatic snapshot at a configurable cadence and/or
   on each save, retaining a configurable number of automatic snapshots.
3. The system SHALL list a document's snapshots with timestamp, optional label,
   and word count.
4. WHEN the user selects two versions (a snapshot and current, or two
   snapshots), the system SHALL show a **diff** highlighting added and removed
   text.
5. WHEN the user chooses to restore a snapshot, the system SHALL replace the
   current buffer with the snapshot content as a single undoable operation
   (never-lose undo, ADR-003) — restoration SHALL be reversible.
6. IF disk space or write fails while creating a snapshot, THEN the system
   SHALL warn and SHALL NOT lose the working buffer (C3).
7. Snapshots SHALL be stored as plain text so they remain recoverable without
   `pstar` (C4/C5).

---

### R5 — Notes, research & metadata sidecar — **P1**

**User story.** As Marcus, I want research notes, a synopsis, and character
sheets attached to my project and reachable while I write, so that continuity
details are one keystroke away.

**Acceptance criteria**

1. The system SHALL let each document carry a **synopsis/summary** and freeform
   **notes** stored as sidecar metadata (outside the manuscript, C4).
2. The system SHALL provide **project-level note documents** (e.g., characters,
   places, timeline) editable within `pstar` like any document.
3. The system SHALL show a document's synopsis in the binder (R1) as a
   secondary line or on demand.
4. The system SHALL provide a command to open a split pane (per existing
   `^OK`) showing a chosen note document alongside the manuscript.
5. WHERE the user references a note (e.g., a character name), the system MAY
   surface a quick-lookup, but this SHALL be optional and SHALL NOT block
   typing.
6. Note/metadata edits SHALL be autosaved consistent with the existing autosave
   model.

---

### R6 — Multi-file search & replace — **P0**

**User story.** As Nadia, I want to find every place a character's name
appears across all chapters and rename it, so that a late naming change doesn't
mean opening 40 files by hand.

**Acceptance criteria**

1. The system SHALL provide **search across the whole project**, returning a
   navigable results list of matches with file, line, and context.
2. WHEN the user selects a search result, the system SHALL open that document
   and place the cursor at the match.
3. The system SHALL support **project-wide replace** with per-match confirm,
   whole-word, and case options, consistent with the existing `^QA` semantics.
4. WHEN a project-wide replace is confirmed, each affected file SHALL be
   modified via the existing atomic-save path, and each change SHALL be
   undoable within its document.
5. Search SHALL support the current incremental-search ergonomics for the
   in-document case (no regression to `^QF`/`^L`).
6. IF a project-wide replace touches an unopened file, THEN the system SHALL
   either open it to apply an undoable edit or record the change such that the
   user can review it before it is persisted (no silent unreviewable writes,
   per C3).
7. Project search SHALL remain responsive on a 300k-word / ~40-file project
   (C6); results MAY stream in as they are found.

---

### R7 — Professional export formats — **P0**

**User story.** As Marcus, I want to export my book to DOCX and EPUB (in
addition to the existing manuscript RTF), so that I can hand my editor and my
distributor the formats they require.

**Acceptance criteria**

1. The system SHALL retain the existing standard-manuscript-format RTF export
   (`^KM`) with no regression (ADR-006).
2. The system SHALL export a document or a compiled project (R1.6) to **DOCX**
   with headings, emphasis, and paragraph structure preserved.
3. The system SHALL export a compiled project to **EPUB** with a table of
   contents derived from headings/binder order.
4. The system SHALL export to clean **HTML** and to a **plain-text** copy with
   `..` notes stripped (existing `^KE` behavior retained).
5. WHEN exporting a project, the system SHALL honor binder order and per-
   document compile separators (R1.6).
6. Export SHALL strip `..` note lines and upgrade straight quotes/dashes to
   typographic ones, consistent with existing manuscript export.
7. WHERE an export format cannot be produced fully offline with bundled means,
   the system SHALL document the dependency and SHALL degrade gracefully rather
   than fail silently (respecting C5 — prefer dependency-free generation like
   the existing hand-rolled RTF).
8. WHEN an export completes, the system SHALL report the output path; IF export
   fails, THEN the system SHALL report why and SHALL NOT overwrite a prior good
   export in place until the new one succeeds.

---

### R8 — Style & readability checking — **P1**

**User story.** As Priya, I want to see passive voice, adverbs, filter words,
and overused words flagged like spelling is, so that I can self-edit a tighter
draft.

**Acceptance criteria**

1. The system SHALL provide optional **style checks** distinct from spelling:
   at minimum passive voice, adverbs (-ly), filter/crutch words, and very long
   sentences.
2. WHERE style checking is enabled, the system SHALL visually distinguish style
   issues from spelling errors (different underline/marker), consistent with
   the existing spellcheck rendering.
3. The system SHALL provide a command to jump to the next style issue,
   analogous to `^QN` for spelling.
4. The system SHALL compute **readability statistics** (e.g., reading grade
   level, average sentence length, adverb ratio) for the document or selection
   on demand.
5. The system SHALL provide an **overused-word / word-frequency** report for
   the document or project so writers can find crutch words.
6. Style checking SHALL be fully offline (C5) and SHALL NOT add perceptible
   typing latency (C6); analysis MAY be debounced or run on demand.
7. Style checks SHALL be individually toggleable and default to a
   configuration the user can set (consistent with existing `config.toml`).

---

### R9 — Editorial annotations & comments — **P1**

**User story.** As Marcus incorporating an editor's feedback, I want inline
comments and editorial marks that never appear in the exported prose, so that I
can track revision tasks alongside the text.

**Acceptance criteria**

1. The system SHALL let the user attach an **inline comment/annotation**
   anchored to a position or block in the document.
2. The system SHALL display annotations distinctly (e.g., margin marker or
   dimmed panel) without altering the prose flow, consistent with how `..`
   notes render dimmed.
3. Annotations SHALL be excluded from all prose exports (R7), the same way `..`
   notes are stripped.
4. The system SHALL provide navigation to the next/previous annotation and a
   list of all annotations in the document/project.
5. Annotation anchors SHALL be adjusted across edits so they stay attached to
   the intended text, consistent with how block marks and bookmarks are
   adjusted across edits.
6. IF the anchored text is deleted, THEN the system SHALL preserve the comment
   as orphaned rather than lose it silently (C3).

---

### R10 — Dictionary, thesaurus & autocorrect — **P2**

**User story.** As any writer, I want a thesaurus and definition lookup and
smart autocorrect for the word under the cursor, so that I don't break flow to
open a browser.

**Acceptance criteria**

1. The system SHALL provide a **thesaurus lookup** for the word under the cursor
   or a selection, presented in an overlay/panel, working offline (C5).
2. The system SHALL provide a **definition lookup** for the word under the
   cursor, working offline (C5).
3. WHEN the user chooses a synonym from the lookup, the system SHALL replace the
   word as a single undoable edit.
4. The system SHALL support **autocorrect/expansion** rules (e.g., common typos,
   user-defined abbreviations) that are user-configurable and can be disabled.
5. Smart typographic substitution (quotes/dashes) SHALL be consistent with the
   existing export behavior and SHALL be toggleable.
6. WHERE a lookup resource is not bundled, the system SHALL degrade gracefully
   and clearly indicate the resource is unavailable rather than error.

---

### R11 — Backup, crash recovery & data safety — **P0**

**User story.** As any professional whose livelihood is the manuscript, I want
absolute confidence that a crash, a bad edit, or a full disk never loses my
work, so that I can trust `pstar` with a career's worth of writing.

**Acceptance criteria**

1. The system SHALL maintain a **crash-recovery** record such that, after an
   abnormal termination, reopening offers to restore unsaved changes.
2. The system SHALL keep the existing one-time `.bak` on save and atomic-write
   behavior with no regression.
3. The system SHALL support a configurable **rolling backup** (timestamped
   copies) of edited documents, stored outside the manuscript folder (C4).
4. IF a save fails (disk full, permissions, read-only), THEN the system SHALL
   surface the error clearly, SHALL retain the in-memory buffer intact, and
   SHALL offer an alternate save location (C3).
5. The system SHALL never leave a manuscript file partially written; a failed
   write SHALL leave the previous good file untouched (atomic replace).
6. Recovery and backup data SHALL be plain text recoverable without `pstar`
   (C4/C5).

---

### R12 — Discoverability, onboarding & help — **P2**

**User story.** As a writer new to WordStar chords, I want the deep feature set
to be discoverable without a manual, so that the power doesn't come at the cost
of a cliff-like learning curve.

**Acceptance criteria**

1. The system SHALL extend the existing command palette / help overlay to cover
   all new commands with searchable names and their chords.
2. The system SHALL keep the tiered help levels (0/1/2) working for new
   features, so experts get a clean screen and newcomers get menus/hints.
3. WHEN a new capability is used for the first time, the system MAY show a
   one-time non-blocking hint, which SHALL be dismissible and suppressible.
4. All new features SHALL be reachable from the command palette by descriptive
   name even if the user does not know the chord (C1).

---

## 4. Prioritization summary

| Priority | Requirements | Rationale |
|----------|--------------|-----------|
| **P0 — table stakes** | R1 Binder, R2 Stats/Goals, R4 Snapshots/Revisions, R6 Project search/replace, R7 Export (DOCX/EPUB), R11 Data safety | These are the gating capabilities that let a writer run a *whole book* in `pstar` and trust it. Without them the tool is a great editor but not a career instrument. |
| **P1 — differentiators** | R3 Sprints/Focus, R5 Notes/Research, R8 Style/Readability, R9 Annotations | These are what make writers *prefer* `pstar` and match Scrivener/ProWritingAid territory while staying keyboard-native. |
| **P2 — polish** | R10 Thesaurus/Autocorrect, R12 Onboarding | High delight, lower urgency; sequence after the core apparatus exists. |

---

## 5. Open questions (to resolve before / during design)

1. **Project manifest format & location.** TOML/JSON under the user data dir
   (like sessions) vs. a dotfile in the manuscript folder. C4 favors the former;
   portability of the whole project favors a manifest the user can see. Design
   phase decides.
2. **DOCX/EPUB generation strategy.** Hand-generate (consistent with the RTF
   ADR-006 philosophy, fully offline) vs. a bundled Rust crate. Trade-off is
   fidelity vs. dependency weight (C5). Likely a new ADR.
3. **Diff engine** for R4 — line-level vs. word-level; reuse an existing crate
   vs. implement. Affects perceived quality of revision comparison.
4. **Thesaurus/definition data (R10)** — which bundled dataset (e.g., WordNet)
   and its license and size impact on the binary (parallels the Hunspell
   dictionary bundling decision, ADR-005).
5. **Style-check rule engine (R8)** — bundled rule set only, or user-extensible
   rules? Affects config surface.
6. **Scope of "project" vs. current split-window model** — how the binder
   interacts with the existing per-pane architecture (App-derefs-to-Pane).
   Needs an architecture note in design.

---

## 6. Traceability & next steps

- This document is the **requirements** artifact of the spec workflow for
  `pro-writer-10-star`.
- **Next artifact:** `design.md` — technical design addressing the open
  questions above, mapping each requirement to modules (new files under `src/`,
  changes to `app.rs`/`pane.rs`), and proposing ADRs for the DOCX/EPUB, project
  manifest, and diff decisions.
- **Then:** `tasks.md` — an ordered implementation task list derived from the
  design, sequenced by the P0 → P1 → P2 priority above.
- Each requirement Rn is individually traceable so design and tasks can
  reference criteria as `R7.3`, etc.
