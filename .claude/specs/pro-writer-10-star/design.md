# Design — "10-Star" Feature Set for Professional Writers

- **Feature slug:** `pro-writer-10-star`
- **Status:** Draft (design phase)
- **Requirements:** [`requirements.md`](./requirements.md)
- **Date:** 2026-07-04
- **Applies to:** PerfectStar 2k (`pstar`), Rust 2024 + ratatui 0.30 + ropey

---

## 1. Overview

The requirements define an *apparatus around the prose* that turns a great
single-document editor into a book-scale writing instrument. This document maps
those requirements onto PerfectStar 2k's actual architecture, chooses concrete
mechanisms, and flags the decisions that warrant ADRs.

The guiding architectural fact is the one the memory and code both stress:

> **`App` derefs to the active `Pane`.** Per-document state (buffer, cursor,
> undo, blocks, bookmarks, jump ring) lives in `Pane`; global state (kill ring,
> theme, mode, macros, spell, config-derived toggles) lives on `App`.

Every new capability is placed on one side of that line deliberately. The
central design tension is that the current product has **no concept above the
document** — `App` owns a `Vec<Pane>` of at most two windows, but there is no
"book." The single largest structural addition in this spec is a **`Project`**
layer that sits on `App` *beside* the panes, not inside them.

### 1.1 Design principles carried from requirements

- Respect constraints **C1–C6** (keyboard-only, terminal-native, never-lose-
  work, plain files, offline, 300k-word performance).
- **Additive, not invasive.** The deref-to-Pane engine and the static
  `BINDINGS` table are load-bearing and well-tested. New features extend them
  (new `Cmd` variants, new `Mode` variants, new modules) rather than
  restructuring the hot editing path.
- **Metadata lives outside the manuscript folder** (like `session.rs` already
  does), keyed by canonical path hash, so the writer's directory stays clean
  (C4, ADR-004).
- **Reuse the existing seams:** `Mode` enum for overlays/prompts, `InputAction`
  for text prompts, `Cmd` + `BINDINGS` for dispatch and palette/help, atomic
  `Buffer::save` for all writes.

---

## 2. Current architecture (as-built, for grounding)

| Concern | Where it lives | Notes |
|---------|----------------|-------|
| Text storage | `buffer.rs` — `Buffer { rope, path, dirty, backed_up }` | ropey; atomic save w/ one-shot `.bak` + `.tmp~` rename |
| Per-document state | `pane.rs` — `Pane` | cursor, history, blocks, bookmarks, jump_stack, view size |
| Global app state | `app.rs` — `App { panes, active, kill, theme, mode, spell, … }` | derefs to `panes[active]` |
| Command table | `keymap.rs` — `Cmd`, `Chord`, `BINDINGS` | drives dispatch, menus, palette, help |
| Dispatch | `app.rs::execute(cmd)` | one big `match cmd` |
| Modal UI | `app.rs::Mode` + `InputAction` | Search/Replace/Input/Palette/Outline |
| Persistence | `session.rs` | JSON under state/data dir, keyed by path hash, invalidated by char-count mismatch |
| Config | `config.rs` — `Config` (serde/TOML) | `~/.config/perfectstar2k/config.toml` |
| Rendering | `ui.rs` | status line, ruler, viewport, menus, overlays, reveal codes |
| Exports | `rtf.rs` (manuscript), plain `^KE` | hand-generated RTF (ADR-006) |

Key extension seams this design will use repeatedly:
- **Add a `Cmd` variant** → add a `Binding` row → dispatch arm in `execute`.
  Palette and help pick it up automatically (they iterate `BINDINGS`).
- **Add a `Mode` variant** → render in `ui.rs`, handle keys in the mode
  dispatcher. Used for every new overlay/panel.
- **Add an `InputAction`** → single-line text prompt with a completion arm.

---

## 3. Architectural additions (the big picture)

```
App
├── panes: Vec<Pane>              (unchanged; still ≤2 visible editing windows)
├── active: usize
├── project: Option<Project>     ★ NEW — the "book" layer, beside the panes
│     ├── manifest: ProjectManifest      (ordered docs, titles, separators)
│     ├── meta_store: MetaStore          (synopsis/notes/annotations sidecars)
│     └── stats: StatsStore              (per-day words-written history)
├── kill, theme, mode, spell …   (unchanged globals)
└── (new globals)
      ├── goal: Option<SessionGoal>      ★ R2/R3 sprint & target state
      ├── style: Option<StyleEngine>     ★ R8 style/readability (like spell)
      └── config additions               ★ R2/R3/R4/R8/R10/R11 toggles
```

New modules under `src/` (one concern each, mirroring the existing flat layout):

| Module | Requirements | Responsibility |
|--------|--------------|----------------|
| `project.rs` | R1 | `Project`, `ProjectManifest`, binder ops (add/remove/reorder), compile |
| `stats.rs` | R2 | word/char counting service, session goal tracking, per-day history |
| `sprint.rs` | R3 | sprint timer + focus-mode state (mostly presentational) |
| `snapshot.rs` | R4 | snapshot store, list, restore; auto-snapshot cadence |
| `diff.rs` | R4 | version diff computation for the revision viewer |
| `meta.rs` | R5, R9 | synopsis/notes + annotation sidecar model & persistence |
| `projsearch.rs` | R6 | multi-file search/replace over the manifest |
| `export/` (mod) | R7 | `export/mod.rs`, `export/docx.rs`, `export/epub.rs`, `export/html.rs`; `rtf.rs` folds in or stays alongside |
| `style.rs` | R8 | passive/adverb/filter/long-sentence checks, readability, word-frequency |
| `lookup.rs` | R10 | thesaurus/definition lookup + autocorrect rules |
| `recovery.rs` | R11 | crash-recovery journal + rolling backups |

Persistence root (new): a `projects/` and per-doc `meta/`, `snapshots/`,
`recovery/` tree under the same base dir `session.rs` uses
(`dirs::state_dir().or(data_local_dir)` → `perfectstar2k/…`). This keeps **all**
`pstar` metadata in one discoverable place, none of it in the manuscript folder.

---

## 4. Component designs by requirement

### 4.1 R1 — Project / binder (`project.rs`) — P0

**Model.**
```rust
pub struct ProjectManifest {
    pub name: String,
    pub docs: Vec<DocEntry>,       // author-defined order
    pub separator: Separator,      // between docs on compile (e.g. PageBreak)
}
pub struct DocEntry {
    pub path: PathBuf,             // canonical; stored relative to manifest if possible
    pub title: String,            // display title (defaults to file stem / first heading)
    pub include_in_compile: bool,
}
pub struct Project {
    pub manifest: ProjectManifest,
    pub manifest_path: PathBuf,
    // caches: per-doc word counts, missing-file flags
}
```

**Storage & format — see [ADR proposal D1](#71-adr-d1--project-manifest).**
Manifest is a human-readable file. Two candidate homes (open question 1): a
`.pstar-project.toml` in the manuscript folder (portable, visible, travels with
the book) vs. under the metadata dir keyed by path hash (clean folder, matches
sessions). **Recommendation:** a visible `*.pstarproj` TOML *in the project
folder* — a book is a first-class artifact the writer should see and back up,
unlike per-file session cruft. This is the one place we deviate from "hide all
metadata," and the ADR records why.

**UI.** New `Mode::Binder { entries, selected }` renders a left/side panel
(reuse the split-pane rendering machinery in `ui.rs`). Because binder selection
must swap the active pane's document, the binder is `App`-level, not `Pane`.

**Opening a doc from the binder** reuses `Pane::open(path)` → session restore is
automatic (ADR-004). Reordering mutates `manifest.docs` and calls an atomic
manifest write (same temp-rename pattern as `Buffer::save`).

**Compile (R1.6)** concatenates `include_in_compile` docs in order, inserting
the separator, producing an in-memory rope/string handed to the export layer
(§4.7). Compile does **not** create a pane; it feeds export directly.

**New `Cmd`s:** `ProjectOpen`, `BinderToggle`, `BinderMoveUp`, `BinderMoveDown`,
`ProjectAddDoc`, `ProjectRemoveDoc`, `Compile`. Chord home: the `^O` (Onscreen)
group is nearly full; propose a **new `^P` prefix ("Project")** — see §5.

**Backward compat (R1.8):** `project: Option<Project>` is `None` when `pstar`
is launched on a bare file; every project-aware code path guards on `Some`.

---

### 4.2 R2 — Statistics & goals (`stats.rs`) — P0

**Counting.** `Buffer::word_count` already exists but scans the whole rope. For
live always-on counts on a 300k-word doc (C6/R2.7), full rescans per keystroke
are unacceptable. Design:
- Keep an authoritative count computed once on load and on major operations.
- Maintain it **incrementally**: `execute` already routes all edits through
  `insert`/`delete_range`; hook a lightweight word-delta recompute over just the
  changed line-range (rope gives cheap line access). Fall back to debounced full
  recount on idle to correct any drift.
- Selection counts (R2.2) compute on demand over `blocks.range()` only.
- **Prose count (R2.6)** reuses the same predicate the exporter uses to strip
  `..` notes and Markdown markers, so "prose words" is consistent everywhere —
  factor that predicate into a shared helper used by `stats`, `markdown`, and
  `export`.

**Goals & history.**
```rust
pub struct SessionGoal { pub kind: GoalKind, pub target: u32, pub start_words: usize, pub started: Instant }
pub enum GoalKind { Words, Minutes }
pub struct StatsStore { /* per-day net-words map, persisted JSON */ }
```
Daily net-words (R2.5) = end-of-day authoritative count − start-of-day, tracked
per project and per doc. Persisted alongside project metadata.

**Notification (R2.4)** is a non-blocking `status_msg`-style banner (never a
`Mode` that blocks typing).

**UI.** Counts render in the existing status line; a `Mode::Stats` overlay shows
the daily history and readability (shared with R8).

**New `Cmd`s:** `WordCount` (toggle always-on / show overlay), `SetGoal`
(prompts via `InputAction::SetGoal`).

---

### 4.3 R3 — Sprints & focus (`sprint.rs`) — P1

Sprint is a `SessionGoal` variant with a countdown surfaced unobtrusively in the
status line; on expiry it emits the report banner and appends to `StatsStore`.
Focus mode is **purely presentational**: it forces `help_level = 0`, hides the
ruler/status chrome, and optionally reuses the existing typewriter emphasis to
dim non-current paragraphs. All state is on `App`; no buffer/file effects
(R3.5). **New `Cmd`s:** `SprintStart`, `FocusMode`.

---

### 4.4 R4 — Snapshots & revision diff (`snapshot.rs`, `diff.rs`) — P0

**Snapshot store.** Per-document directory under the metadata root, keyed by the
same path hash `session.rs` uses:
```
perfectstar2k/snapshots/<stem>-<hash>/<timestamp>[-label].txt   (plain text, C4/C5)
```
Snapshots are plain UTF-8 copies of the buffer text — recoverable without
`pstar` (R4.7). A small JSON index per doc holds label/timestamp/word-count
(R4.3).

- **Manual (R4.1):** `Cmd::Snapshot` → optional label prompt → write.
- **Auto (R4.2):** on each `save()` and/or on an interval; retention = keep last
  N (config `snapshot_keep`), prune oldest. Cheap because they're just files.
- **Safety (R4.6):** snapshot write failure warns via `status_msg` and never
  touches the working buffer.

**Diff (R4.5 restore, R4.4 view).** `Mode::Revisions { versions, selected }`
lists snapshots; selecting two opens `Mode::Diff`. **Diff engine — see
[ADR proposal D3](#73-adr-d3--diff-engine).** Recommendation: use the
`similar` crate (mature, pure-Rust, word- and line-level, well-suited to prose)
rather than hand-rolling. Restore replaces the buffer via the **existing insert/
delete history mechanism** as a single `EditGroup`, so it's one undo step and
fully reversible (R4.5, ADR-003) — not a raw rope swap that would bypass undo.

---

### 4.5 R5 — Notes / research sidecar (`meta.rs`) — P1

Per-doc metadata record (synopsis, notes) stored as sidecar JSON keyed by path
hash. Project-level note documents (R5.2: characters/places/timeline) are just
ordinary files listed in the manifest with a `role = Note` flag, so they open in
a pane and edit like anything else and are excluded from compile. Synopsis shows
as the binder's secondary line (R5.3). "Open note in split" (R5.4) reuses the
existing `^OK` split mechanism targeting a note doc. Quick-lookup (R5.5) is
explicitly optional and non-blocking. Autosave (R5.6) piggybacks on the existing
idle-autosave loop.

---

### 4.6 R6 — Project-wide search/replace (`projsearch.rs`) — P0

**Search.** Iterate `manifest.docs`; for each, search the open pane's rope if
loaded, else stream the file from disk (don't force-load 40 ropes). Results:
`Vec<Match { doc_idx, path, char_pos, line, context }>`, surfaced in
`Mode::ProjectSearch { results, selected }`. Streaming results (R6.7) keeps it
responsive on 300k words. In-document search is unchanged — `^QF`/`^L` keep
their exact current behavior (R6.5).

**Replace (R6.3, R6.4, R6.6).** Per-match confirm with whole-word/case options
mirroring `^QA`. The subtle requirement is R6.6: edits to *unopened* files must
be undoable and reviewable, never silent. **Design:** project replace **opens
each affected file into a pane context transiently**, applies edits through the
normal history path (so each file gets a proper undoable `EditGroup` and atomic
save), then can close it. Files are never rewritten by a raw string replace that
bypasses undo/atomic-save. A summary reports counts per file.

**New `Cmd`s:** `ProjectFind`, `ProjectReplace`.

---

### 4.7 R7 — Professional exports (`export/`) — P0

Refactor exports into an `export` module family with a shared trait:
```rust
pub trait Exporter { fn export(&self, doc: &CompiledDoc, out: &Path) -> io::Result<()>; }
```
`CompiledDoc` is the normalized intermediate the compiler (§4.1) produces:
paragraphs, headings, emphasis runs, with `..` notes and annotations already
stripped and typographic substitution applied (R7.6) — reusing the exact
normalization `rtf.rs` performs today so all formats agree.

- **RTF** (`rtf.rs`, retained verbatim — R7.1, ADR-006).
- **HTML / plain text** (`export/html.rs`) — trivial, hand-generated, no deps
  (R7.4).
- **DOCX / EPUB** (`export/docx.rs`, `export/epub.rs`) — **see
  [ADR proposal D2](#72-adr-d2--docxepub-generation).** Both formats are
  zip-of-XML containers. Two options: (a) hand-generate the minimal XML + zip
  (consistent with ADR-006's dependency-free philosophy, fully offline, more
  work) vs. (b) a bundled Rust crate (`docx-rs`, EPUB via `epub-builder`),
  higher fidelity, added dependency weight. **Recommendation:** DOCX/EPUB are
  standardized zip+XML; a thin hand-generated writer sharing `CompiledDoc` is
  achievable and keeps the "no external converter" promise (C5) — but this is a
  real cost/fidelity trade-off the ADR must settle, possibly landing on a
  vetted crate. Either way: honor binder order (R7.5), report output path,
  never overwrite a good export until the new one succeeds (R7.8 — same
  temp-then-rename discipline as `Buffer::save`).

**New `Cmd`s:** `ExportDocx`, `ExportEpub`, `ExportHtml` (each via an
`InputAction` path prompt, like the existing export commands).

---

### 4.8 R8 — Style & readability (`style.rs`) — P1

Modeled on `spellcheck.rs`, which is the proven pattern: a global service on
`App` (`style: Option<StyleEngine>`, like `spell`), a `style_enabled` toggle,
on-the-fly markers rendered distinctly from spelling underlines (R8.2), and a
`NextStyleIssue` command paralleling `NextMisspelling` (R8.3). Checks are rule
functions over sentence/word spans (passive voice, `-ly` adverbs, a bundled
filter/crutch word list, sentence length threshold). Readability stats (R8.4)
and word-frequency (R8.5) compute on demand into the shared stats overlay.
Offline & debounced (R8.6). Per-check toggles live in config (R8.7). **See
[ADR proposal D4](#74-adr-d4--style-rule-engine)** for whether rules are
fixed or user-extensible.

---

### 4.9 R9 — Editorial annotations (`meta.rs`, extends R5) — P1

Annotations are anchored comments stored in the same sidecar as notes:
```rust
pub struct Annotation { pub anchor: usize /*char pos*/, pub len: usize, pub text: String, pub orphaned: bool }
```
**Anchor adjustment (R9.5)** is the critical detail — it must reuse the exact
`adjust_pos` machinery already imported in `app.rs` (`use crate::block::adjust_pos`)
that keeps blocks/bookmarks attached across edits. Every edit that shifts
positions runs annotation anchors through the same adjustment. If the anchored
span is fully deleted, the annotation is marked `orphaned` rather than dropped
(R9.6, C3). Rendered as dimmed margin markers/panel (R9.2, consistent with `..`
notes), excluded from all exports (R9.3, via the `CompiledDoc` strip step).
`NextAnnotation`/`PrevAnnotation` + an annotation list overlay (R9.4).

---

### 4.10 R10 — Thesaurus / definitions / autocorrect (`lookup.rs`) — P2

Thesaurus + definitions from a **bundled offline dataset** (R10.1–2, C5) —
**see [ADR proposal D5](#75-adr-d5--lookup-dataset)** (WordNet vs. Moby
thesaurus; license & binary-size impact, paralleling the Hunspell bundling
decision ADR-005). Overlay `Mode::Lookup` shows synonyms/definitions; choosing a
synonym replaces the word as one undoable edit (R10.3). Autocorrect/expansion
(R10.4) is a user-configurable rule map applied on word-boundary input,
disable-able; typographic substitution (R10.5) reuses the export normalizer and
is toggleable. Graceful degradation if a dataset is absent (R10.6).

---

### 4.11 R11 — Backup & crash recovery (`recovery.rs`) — P0

- **Crash-recovery journal (R11.1).** On dirty-buffer transitions, write the
  buffer text to a recovery file under `perfectstar2k/recovery/<stem>-<hash>`
  (throttled, e.g. piggybacking the autosave idle tick). On startup, if a
  recovery file newer than the on-disk manuscript exists, offer restore via a
  `Mode::ConfirmRecover` prompt. Clear the recovery file on clean save/exit.
- **Existing `.bak` + atomic save (R11.2)** retained unchanged.
- **Rolling backups (R11.3)** = timestamped copies under the metadata root,
  distinct from snapshots (snapshots are user-facing versions; backups are
  automatic safety copies). Configurable depth.
- **Save-failure handling (R11.4–5).** `Buffer::save` currently propagates
  `io::Error`; extend the `Cmd::Save` arm to catch it, keep the buffer intact,
  show the error, and offer an alternate location via an `InputAction`. The
  temp-then-rename already guarantees the previous good file is never truncated
  (R11.5) — verify and document this invariant.
- All recovery/backup data is plain text (R11.6).

---

### 4.12 R12 — Discoverability & onboarding — P2

Because palette and help **iterate `BINDINGS`**, every new `Cmd` with a `name`
is *automatically* searchable in the palette and shown in help (R12.1, R12.4) —
this is why adding features via the binding table matters. Tiered help levels
0/1/2 continue to work for new prefix menus (R12.2). One-time hints (R12.3) are
a small `App`-level "seen features" set persisted in config, shown as
dismissible non-blocking banners.

---

## 5. Keymap strategy

The `^K`/`^Q`/`^O` prefixes are dense. New commands are grouped as follows to
avoid collisions and keep mnemonics honest:

| Group | Prefix | New commands |
|-------|--------|--------------|
| **Project** (new prefix `^P`) | `^P` | `^PP` open project, `^PB` binder, `^PF` project find, `^PA` project replace, `^PC` compile, `^P↑/↓` reorder |
| Stats/goals | `^Q` (Quick) | `^Q=` word count, `^QG` set goal *(note: `^QG` is transpose-chars today — reassign carefully or use palette-only)* |
| Snapshots/revisions | `^K` (Block & File — file-ish) | `^KN` snapshot now, `^KL` revisions list |
| Style | `^O` (Onscreen) | `^OY` style on/off, `^QN`-parallel next-issue under `^Q` |
| Export | `^K` | `^KX`-family already; add palette entries + prompts |

**Design rule:** any command whose chord is contentious ships **palette-first**
(reachable by name, C1/R12.4) and a chord is assigned only where a clean
mnemonic exists. Introducing the `^P` prefix requires extending `Prefix` enum,
its `label()`, the menu machinery, and the prefix-timeout dispatch — a
localized, well-bounded change to `keymap.rs` + `app.rs`.

---

## 6. Cross-cutting concerns

- **Performance (C6).** The only per-keystroke additions are incremental word
  counting and (when enabled) debounced style/spell scanning over changed
  ranges — never full-document rescans on the hot path. Project search streams
  from disk. Benchmarked against a 300k-word fixture in the PTY harness.
- **Persistence layout.** One metadata root, subdirs `sessions/`, `projects/`,
  `snapshots/`, `meta/`, `recovery/`, `stats/`. All keyed by canonical path
  hash where per-file. Project manifest is the sole visible in-folder artifact.
- **Never-lose-work (C3).** Snapshot restore, project replace, and synonym
  replace all go through the history/`EditGroup` path so they're undoable;
  every file write uses temp-then-rename; recovery journal backstops crashes.
- **Testing.** Extends the Python PTY + `pyte` harness (per project memory):
  cargo-clean before binary verification; answer DA1 `\x1b[?1;2c`; incremental
  UTF-8 decode; keep draining PTY. New unit suites: manifest round-trip,
  incremental word-count vs. full-count equivalence, diff correctness,
  annotation anchor adjustment across edits, export golden files, recovery-
  journal restore.
- **Config growth.** New keys (all defaulted for backward compat):
  `snapshot_keep`, `autosnapshot_secs`, `backup_depth`, `style` (+per-check
  toggles), `autocorrect`, `daily_goal`, `focus_dim`.

---

## 7. Decisions requiring ADRs

These resolve the requirements' open questions and should be recorded as ADRs
(next numbers **ADR-008…ADR-012**, MADR format per `docs/adr/README.md`) during
or before implementation.

### 7.1 ADR D1 — Project manifest format & location
Visible `*.pstarproj` TOML in the project folder vs. hidden path-hashed file
under the metadata root. **Leaning:** visible TOML (a book is a first-class,
user-owned, backup-worthy artifact; the deviation from "hide metadata" is
intentional). Resolves requirements open-Q1 & Q6.

### 7.2 ADR D2 — DOCX/EPUB generation
Hand-generated zip+XML (dependency-free, matches ADR-006) vs. bundled crates
(`docx-rs`, `epub-builder`; higher fidelity, more weight). Resolves open-Q2.

### 7.3 ADR D3 — Diff engine
Adopt `similar` (pure-Rust, word/line diff) vs. hand-roll. **Leaning:** adopt.
Resolves open-Q3.

### 7.4 ADR D4 — Style rule engine
Fixed bundled rule set vs. user-extensible rules. Affects config surface.
Resolves open-Q5.

### 7.5 ADR D5 — Lookup dataset
Bundled thesaurus/definition dataset choice (WordNet vs. Moby) + license & size,
paralleling ADR-005. Resolves open-Q4.

---

## 8. Requirements → design traceability

| Req | Module(s) | New `Cmd` / `Mode` / storage |
|-----|-----------|------------------------------|
| R1 | `project.rs` | `ProjectOpen/BinderToggle/BinderMoveUp/Down/AddDoc/RemoveDoc/Compile`; `Mode::Binder`; `*.pstarproj` |
| R2 | `stats.rs`, `buffer.rs` | `WordCount/SetGoal`; `Mode::Stats`; incremental count; `StatsStore` |
| R3 | `sprint.rs` | `SprintStart/FocusMode`; `SessionGoal` |
| R4 | `snapshot.rs`, `diff.rs` | `Snapshot/RevisionsList`; `Mode::Revisions/Diff`; snapshot dir + index; `similar` |
| R5 | `meta.rs`, `project.rs` | sidecar meta; note-role docs; split reuse |
| R6 | `projsearch.rs` | `ProjectFind/ProjectReplace`; `Mode::ProjectSearch`; transient-open replace |
| R7 | `export/` (+`rtf.rs`) | `ExportDocx/Epub/Html`; `CompiledDoc`; `Exporter` trait |
| R8 | `style.rs` | `NextStyleIssue/ToggleStyle`; `StyleEngine` on `App` |
| R9 | `meta.rs` | `NextAnnotation/PrevAnnotation`; `Annotation`; `adjust_pos` reuse |
| R10 | `lookup.rs` | `Lookup/…`; `Mode::Lookup`; bundled dataset |
| R11 | `recovery.rs`, `buffer.rs` | `Mode::ConfirmRecover`; recovery journal; rolling backups; save-fail handling |
| R12 | `keymap.rs`, `config.rs` | palette/help auto-coverage; one-time hints |

---

## 9. Next step

Proceed to **`tasks.md`** — an ordered implementation task list derived from
this design, sequenced **P0 → P1 → P2**, with the enabling structural work
(the `Project` layer, the metadata-root refactor, and the `export`/`CompiledDoc`
refactor) scheduled first because R1/R6/R7 all depend on it. The five ADRs
(D1–D5) are authored as their features come up.
