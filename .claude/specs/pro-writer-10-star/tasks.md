# Tasks — "10-Star" Feature Set for Professional Writers

- **Feature slug:** `pro-writer-10-star`
- **Status:** Draft (tasks phase)
- **Requirements:** [`requirements.md`](./requirements.md)
- **Design:** [`design.md`](./design.md)
- **Date:** 2026-07-04

---

## How to read this

Tasks are grouped into **phases** that respect the dependency order the design
calls out: the structural foundations (metadata root, `Project` layer,
`CompiledDoc`/export refactor) come first because the P0 features R1/R6/R7 all
build on them. Within that, sequencing follows requirement priority
**P0 → P1 → P2**.

- Each task is a checkbox, sized to a focused, independently reviewable change.
- **`Req:`** links back to acceptance criteria (e.g. `R7.3`); **`Design:`**
  links to the design section; **`ADR:`** flags a decision to record first.
- **`Files:`** names the modules touched — new files per the design's flat
  `src/` layout, existing files where extended.
- A task marked **[gate]** must land (and its tests pass) before dependents.
- Every feature touches the same three seams unless noted: a `Cmd` variant +
  `Binding` row (`keymap.rs`), a dispatch arm (`app.rs::execute`), and
  rendering/mode handling (`ui.rs`). Palette + help coverage is then automatic.

Effort key: **S** ≤ half day · **M** ~1–2 days · **L** ~3–5 days.

---

## Phase 0 — Foundations (enable P0 features) — **[gate]** ✅ complete

- [x] **0.1 Metadata-root helper.** Factor the base-dir logic out of
  `session.rs` into a shared `paths.rs` (or `meta.rs`) exposing the
  `perfectstar2k/` root and typed subdir accessors: `sessions/`, `projects/`,
  `snapshots/`, `meta/`, `recovery/`, `stats/`. Reuse the canonical-path-hash
  keying. Migrate `session.rs` to consume it (no behavior change).
  · Design §3, §6 · Files: `paths.rs` (new), `session.rs` · **M**

- [x] **0.2 Atomic-write utility.** Extract the temp-then-rename discipline from
  `Buffer::save` into a reusable `write_atomic(path, bytes)` helper so manifest,
  snapshot index, meta sidecar, and export writers all share the invariant
  (previous good file never truncated). `Buffer::save` calls it.
  · Req: R11.5 · Design §6 · Files: `buffer.rs`, `paths.rs` · **S**

- [x] **0.3 Shared prose-normalizer.** Extract the `..`-note stripping, Markdown
  marker handling, and typographic quote/dash substitution from `rtf.rs` into
  one predicate/normalizer used by stats, export, and word-count so "prose"
  means the same thing everywhere. No behavior change to `^KM`/`^KE`.
  · Req: R2.6, R7.6, R9.3 · Design §4.2, §4.7 · Files: `rtf.rs`,
  `markdown.rs`, new `normalize.rs` · **M**

- [x] **0.4 `^P` "Project" prefix.** Extend `Prefix` enum (+`label()`), the
  prefix-timeout dispatch, and the menu machinery to support a fourth prefix.
  Verify existing prefix menus/palette/help still render. No commands yet.
  · Design §5 · Files: `keymap.rs`, `app.rs` · **S**

- [x] **0.5 300k-word test fixture + harness bench.** Add a large-manuscript
  fixture and a PTY-harness timing check for keystroke latency, to guard C6 as
  features land. Follow the harness gotchas (cargo-clean, DA1 reply, incremental
  UTF-8 decode, keep draining). · Req: C6/R2.7/R6.7 · Design §6 · **M**

---

## Phase 1 — P0: Project & Binder (R1)

- [x] **1.1 [gate] Manifest model + persistence.** Define `ProjectManifest`,
  `DocEntry`, `Project`; serde round-trip; atomic load/save via 0.2. **Record
  [ADR-012] first** (manifest format & location). · Req: R1.1, R1.4 ·
  ADR: D1 · Design §4.1, §7.1 · Files: `project.rs` (new) · **M**

- [x] **1.2 `App.project` wiring.** Add `project: Option<Project>` to `App`;
  `ProjectOpen` command + `InputAction` prompt; guard all project paths on
  `Some` so bare-file launch is unchanged. · Req: R1.8 · Files: `app.rs`,
  `keymap.rs` · **S**

- [x] **1.3 Binder panel.** `Mode::Binder { entries, selected }` rendered as a
  side panel (reuse split-pane rendering); per-doc title + word count; open
  selected doc via `Pane::open` (session restore automatic). · Req: R1.2, R1.3
  · Design §4.1 · Files: `ui.rs`, `app.rs`, `project.rs` · **M**

- [x] **1.4 Reorder / add / remove.** `BinderMoveUp/Down`, `ProjectAddDoc`,
  `ProjectRemoveDoc`; atomic manifest rewrite; remove never deletes the file on
  disk. · Req: R1.4, R1.5 · Files: `project.rs`, `app.rs`, `keymap.rs` · **M**

- [x] **1.5 Missing-file resilience.** Flag missing docs in the binder; open the
  rest of the project regardless. · Req: R1.7 · Files: `project.rs`, `ui.rs` · **S**

- [x] **1.6 Compile.** Concatenate `include_in_compile` docs in order with the
  configured separator into an in-memory `CompiledDoc` (feeds Phase 3 export).
  · Req: R1.6 · Design §4.1, §4.7 · Files: `project.rs` · **M**

- [x] **1.7 Tests.** Manifest round-trip; reorder persistence; missing-file
  open; compile order/separators. · **S**

---

## Phase 2 — P0: Data safety (R11)

*Scheduled early: safety underpins every write-heavy feature that follows.*

- [x] **2.1 [gate] Save-failure handling.** Catch `io::Error` from
  `Buffer::save` in the `Cmd::Save` arm; keep buffer intact; surface the error;
  offer alternate save location via `InputAction`. Document the never-truncate
  invariant (satisfied by 0.2). · Req: R11.4, R11.5 · Design §4.11 ·
  Files: `app.rs`, `buffer.rs` · **M**

- [x] **2.2 Crash-recovery journal.** Write buffer text to
  `recovery/<stem>-<hash>` on dirty transitions, throttled on the autosave
  tick; clear on clean save/exit. · Req: R11.1, R11.6 · Files: `recovery.rs`
  (new), `app.rs` · **M**

- [x] **2.3 Recovery-on-startup prompt.** If a recovery file newer than the
  on-disk file exists at open, offer restore via `Mode::ConfirmRecover`.
  · Req: R11.1 · Files: `app.rs`, `ui.rs`, `recovery.rs` · **S**

- [x] **2.4 Rolling backups.** Timestamped copies under `recovery/` (distinct
  from user snapshots), depth from config `backup_depth`; prune oldest.
  · Req: R11.3 · Files: `recovery.rs`, `config.rs` · **S**

- [x] **2.5 Tests.** Recovery-journal restore after simulated crash; save-fail
  keeps buffer; backup rotation. · **M**

---

## Phase 3 — P0: Professional exports (R7)

*Depends on 0.3 (normalizer) and 1.6 (compile).*

- [x] **3.1 [gate] `CompiledDoc` + `Exporter` trait.** Define the normalized
  intermediate (paragraphs, headings, emphasis runs; notes/annotations stripped;
  typographic subst applied via 0.3) and the `Exporter` trait. Refactor `rtf.rs`
  to implement it with **no `^KM` regression** (golden-file test).
  · Req: R7.1, R7.6 · Design §4.7 · Files: `export/mod.rs` (new), `rtf.rs` · **L**

- [x] **3.2 HTML + plain-text exporters.** Hand-generated, dependency-free;
  plain-text retains `^KE` behavior. · Req: R7.4 · Files: `export/html.rs`
  (new) · **S**

- [x] **3.3 [ADR-013] DOCX/EPUB decision.** Record hand-generated zip+XML vs.
  bundled crate before implementing. · ADR: D2 · Design §7.2 · **S**

- [x] **3.4 DOCX exporter.** Per 3.3; headings, emphasis, paragraph structure.
  · Req: R7.2 · Files: `export/docx.rs` (new) · **L**

- [x] **3.5 EPUB exporter.** TOC from headings/binder order. · Req: R7.3 ·
  Files: `export/epub.rs` (new) · **L**

- [x] **3.6 Export commands + safety.** `ExportDocx/Epub/Html` via path
  prompts; honor binder order; report output path; temp-then-rename so a prior
  good export is never clobbered on failure; graceful degradation + documented
  deps. · Req: R7.5, R7.7, R7.8 · Files: `app.rs`, `keymap.rs`, `export/mod.rs`
  · **M**

- [x] **3.7 Tests.** Golden-file per format; compile-order fidelity; failed
  export leaves prior output intact. · **M**

---

## Phase 4 — P0: Statistics & goals (R2)

- [x] **4.1 [gate] Incremental word/char count.** Maintain authoritative count
  on load; update over changed line-range through `insert`/`delete_range`;
  debounced full recount on idle to correct drift. Prove equivalence to full
  count in tests. Use shared normalizer (0.3) for prose count. · Req: R2.1,
  R2.6, R2.7 · Design §4.2 · Files: `stats.rs` (new), `buffer.rs`, `app.rs` · **L**

- [x] **4.2 Status-line + selection counts.** Doc/project/selection counts in
  the status area; always-on toggle. · Req: R2.1, R2.2 · Files: `ui.rs`,
  `stats.rs` · **S**

- [x] **4.3 Session goal + progress + notify.** `SetGoal` (words/minutes),
  live progress, non-blocking completion banner. · Req: R2.3, R2.4 · Files:
  `stats.rs`, `app.rs` · **M**

- [x] **4.4 Daily words-written history.** Net-words per day per doc/project;
  persist to `stats/`; `Mode::Stats` overlay. · Req: R2.5 · Files: `stats.rs`,
  `ui.rs` · **M**

- [x] **4.5 Tests.** Incremental-vs-full equivalence; net-words delta;
  goal-reached notification. · **S**

---

## Phase 5 — P0: Project-wide search & replace (R6)

*Depends on Phase 1 (manifest).*

- [x] **5.1 [gate] Project search.** Search each manifest doc (open rope or
  streamed from disk); `Mode::ProjectSearch { results, selected }`; stream
  results in. In-document `^QF`/`^L` unchanged. · Req: R6.1, R6.5, R6.7 ·
  Design §4.6 · Files: `projsearch.rs` (new), `app.rs`, `ui.rs` · **L**

- [x] **5.2 Jump to result.** Open the doc, place cursor at the match. · Req:
  R6.2 · Files: `app.rs`, `projsearch.rs` · **S**

- [x] **5.3 Project replace (undoable, reviewable).** Per-match confirm +
  whole-word/case; edits to unopened files go through a transient pane context
  so each is a proper `EditGroup` + atomic save — never a silent raw rewrite;
  per-file summary. · Req: R6.3, R6.4, R6.6 · Design §4.6 · Files:
  `projsearch.rs`, `app.rs` · **L**

- [x] **5.4 Tests.** Cross-file match set; replace produces undoable per-file
  edits; unopened-file edit reviewable; latency on the 300k fixture. · **M**

---

## Phase 6 — P0 exit criterion

- [x] **6.1 P0 acceptance pass.** Verify every P0 criterion (R1, R2, R4*, R6,
  R7, R11) against `requirements.md` via the PTY harness on a real multi-file
  project. *(\*R4 snapshots land in Phase 7; if P0 milestone must be
  self-contained, pull 7.1–7.3 forward.)* · **M**
  **Result:** R1 8/8 ✓, R2 7/7 ✓, R6 7/7 ✓, R7 8/8 ✓, R11 6/6 ✓.
  151 tests pass. R4 deferred to Phase 7 per spec note.

> **Note on R4 (P0):** Snapshots/revisions are P0 in requirements but grouped
> with the revision UI in Phase 7 for cohesion. If shipping a strict P0
> milestone, promote tasks **7.1–7.4** into Phase 5.5 before 6.1.

---

## Phase 7 — P0: Snapshots & revision diff (R4)

- [x] **7.1 [gate] Snapshot store.** Plain-text copies under
  `snapshots/<stem>-<hash>/`; JSON index (label/timestamp/word-count); write
  failure warns and never touches the buffer. · Req: R4.1, R4.3, R4.6, R4.7 ·
  Design §4.4 · Files: `snapshot.rs` (new) · **M**

- [x] **7.2 Manual + auto snapshots.** `Snapshot` command (optional label
  prompt); auto-snapshot on save/interval with retention `snapshot_keep`.
  · Req: R4.1, R4.2 · Files: `snapshot.rs`, `app.rs`, `config.rs` · **S**
  **Note:** retention applies to *automatic* snapshots only, per R4.2's wording —
  a labelled snapshot is never pruned. `snapshot_keep = 0` disables new
  automatic snapshots without deleting existing ones (same contract as
  `backup_depth`); `autosnapshot_secs = 0` (default) leaves automatic snapshots
  to saves alone.

- [x] **7.3 [ADR-010] Diff engine + revisions list.** Record diff decision
  (adopt `similar`); `Mode::Revisions` list. · Req: R4.3 · ADR: D3 ·
  Design §7.3 · Files: `diff.rs` (new), `ui.rs` · **M**
  **Note:** recorded as **ADR-014** — ADR-010…013 were taken by the radio work
  after this spec was written. Chords: `^KN` snapshot, `^KO` revisions (design
  §5 proposed `^KL`, which is export HTML).

- [x] **7.4 Diff view + restore.** `Mode::Diff` add/remove highlighting;
  restore replaces buffer as a single undoable `EditGroup` (reversible).
  · Req: R4.4, R4.5 · Design §4.4 · Files: `diff.rs`, `app.rs`, `ui.rs` · **M**

- [x] **7.5 Tests.** Snapshot round-trip + retention; diff correctness; restore
  is one undo step and reversible. · **S**
  **Result:** 47 tests across the phase — 20 store, 9 diff engine, 12 app-level
  (command → prompt → capture, save-triggered retention, revisions list, two-way
  diff, restore/undo/redo, failure paths), 4 overlay renders via `TestBackend`,
  2 config. 198 tests pass overall. Plus a PTY end-to-end walk of the whole
  flow: `tests/harness/smoke_snapshots.py` (^KN → revise → ^KO → diff → ^R → ^U).

---

## Phase 8 — P1: Sprints & focus (R3)

- [ ] **8.1 Sprint timer.** `SprintStart` (duration and/or word goal),
  unobtrusive countdown, end report appended to `StatsStore`. · Req: R3.1,
  R3.2 · Files: `sprint.rs` (new), `app.rs` · **M**

- [ ] **8.2 Focus mode.** `FocusMode` forces help-level 0, hides chrome;
  optional dim-non-current-paragraph (config `focus_dim`, reuses typewriter
  emphasis); purely presentational, no file effects. · Req: R3.3, R3.4, R3.5 ·
  Files: `sprint.rs`, `ui.rs`, `config.rs` · **M**

- [ ] **8.3 Tests.** Sprint report accuracy; focus mode leaves buffer/files
  untouched. · **S**

---

## Phase 9 — P1: Notes / research sidecar (R5)

- [ ] **9.1 [gate] Sidecar meta model.** Per-doc synopsis + notes as JSON under
  `meta/`, keyed by path hash; autosave on idle tick. · Req: R5.1, R5.6 ·
  Design §4.5 · Files: `meta.rs` (new), `app.rs` · **M**

- [ ] **9.2 Project note docs.** `role = Note` flag on `DocEntry`; note docs
  open like any file and are excluded from compile. · Req: R5.2 · Files:
  `project.rs`, `meta.rs` · **S**

- [ ] **9.3 Synopsis in binder + open-in-split.** Secondary binder line;
  command to open a chosen note in a split via existing `^OK`. · Req: R5.3,
  R5.4 · Files: `ui.rs`, `app.rs` · **S**

- [ ] **9.4 Tests.** Sidecar round-trip; note docs excluded from compile. · **S**

---

## Phase 10 — P1: Editorial annotations (R9)

*Extends `meta.rs`; depends on Phase 9.*

- [ ] **10.1 [gate] Annotation model + anchor adjustment.** `Annotation { anchor,
  len, text, orphaned }`; run anchors through the existing `adjust_pos`
  machinery on every position-shifting edit; orphan (never drop) on deletion.
  · Req: R9.1, R9.5, R9.6 · Design §4.9 · Files: `meta.rs`, `app.rs` · **L**

- [ ] **10.2 Render + navigate + list.** Dimmed margin markers/panel (like `..`
  notes); `NextAnnotation/PrevAnnotation`; annotation-list overlay. · Req:
  R9.2, R9.4 · Files: `ui.rs`, `app.rs`, `keymap.rs` · **M**

- [ ] **10.3 Exclude from exports.** Ensure `CompiledDoc` strip step drops
  annotations. · Req: R9.3 · Files: `export/mod.rs`, `normalize.rs` · **S**

- [ ] **10.4 Tests.** Anchor survives edits; deletion orphans not loses;
  excluded from every export. · **M**

---

## Phase 11 — P1: Style & readability (R8)

- [ ] **11.1 [ADR-011] Style rule engine.** Record fixed-vs-extensible decision.
  · ADR: D4 · Design §7.4 · **S**

- [ ] **11.2 [gate] Style engine.** `style: Option<StyleEngine>` on `App`
  (mirrors `spell`); rules: passive voice, `-ly` adverbs, bundled filter/crutch
  list, long-sentence threshold; debounced over changed ranges; offline.
  · Req: R8.1, R8.6 · Design §4.8 · Files: `style.rs` (new), `app.rs` · **L**

- [ ] **11.3 Distinct markers + next-issue.** Render style issues distinctly
  from spelling; `NextStyleIssue` (parallels `^QN`); `ToggleStyle` (`^OY`) with
  per-check config toggles. · Req: R8.2, R8.3, R8.7 · Files: `ui.rs`,
  `app.rs`, `keymap.rs`, `config.rs` · **M**

- [ ] **11.4 Readability + word-frequency.** On-demand grade level, avg sentence
  length, adverb ratio, overused-word report into the shared stats overlay.
  · Req: R8.4, R8.5 · Files: `style.rs`, `ui.rs` · **M**

- [ ] **11.5 Tests.** Rule detection fixtures; latency on 300k fixture;
  frequency report. · **S**

---

## Phase 12 — P2: Thesaurus / definitions / autocorrect (R10)

- [ ] **12.1 [ADR-012] Lookup dataset.** Record dataset choice + license/size.
  · ADR: D5 · Design §7.5 · **S**

- [ ] **12.2 Thesaurus + definition lookup.** Bundled offline dataset;
  `Mode::Lookup` overlay; choosing a synonym replaces the word as one undoable
  edit; graceful degradation if absent. · Req: R10.1, R10.2, R10.3, R10.6 ·
  Files: `lookup.rs` (new), `app.rs`, `ui.rs` · **L**

- [ ] **12.3 Autocorrect / expansion + typographic subst.** User-configurable
  rule map on word boundary, disable-able; typographic substitution reuses the
  normalizer and is toggleable. · Req: R10.4, R10.5 · Files: `lookup.rs`,
  `config.rs`, `normalize.rs` · **M**

- [ ] **12.4 Tests.** Synonym replace undoable; autocorrect rules
  apply/disable; absent-dataset degradation. · **S**

---

## Phase 13 — P2: Discoverability & onboarding (R12)

- [ ] **13.1 Palette/help audit.** Confirm every new `Cmd` has a descriptive
  `name` and appears in palette + help (automatic via `BINDINGS`); fill gaps.
  · Req: R12.1, R12.4 · Files: `keymap.rs` · **S**

- [ ] **13.2 Tiered help for new menus.** Verify help levels 0/1/2 behave for
  the `^P` prefix and all new menu entries. · Req: R12.2 · Files: `app.rs`,
  `ui.rs` · **S**

- [ ] **13.3 One-time hints.** `App`-level seen-features set persisted in
  config; dismissible non-blocking banner on first use. · Req: R12.3 · Files:
  `app.rs`, `config.rs` · **S**

---

## Phase 14 — Release wrap

- [ ] **14.1 Config documentation.** Document all new keys (`snapshot_keep`,
  `autosnapshot_secs`, `backup_depth`, `style` + per-check toggles,
  `autocorrect`, `daily_goal`, `focus_dim`) in README. · Design §6 · **S**

- [ ] **14.2 README + key tables.** Document project/binder, stats/goals,
  sprints, snapshots, project search, new exports, style, annotations, lookup;
  add `^P` prefix table. · **M**

- [ ] **14.3 Changelog fragments.** One `changes/~slug.feature` fragment per
  shipped capability (matches the repo's `changelogging` workflow). · **S**

- [ ] **14.4 ADR index.** Ensure ADR-008…012 are added to
  `docs/adr/README.md`. · **S**

- [ ] **14.5 Full acceptance pass.** Re-verify all requirements (P0+P1+P2)
  end-to-end via the PTY harness on a real multi-file project. · **M**

---

## Dependency map (critical path)

```
Phase 0 (foundations) ──┬─► Phase 1 (project) ──► Phase 5 (project search)
                        │        └──► Phase 3 (export, +0.3 normalizer)
                        ├─► Phase 2 (data safety)
                        └─► Phase 4 (stats)
Phase 1 + 3 ──► Phase 7 (snapshots/diff, P0 — see Phase 6 note)
Phase 9 (notes) ──► Phase 10 (annotations)
Phase 3 (CompiledDoc) ──► Phase 10.3 (exclude annotations from export)
Phases 8, 11, 12, 13 are largely independent (P1/P2), gated only by their
own foundations.
```

## Milestones

| Milestone | Phases | Delivers |
|-----------|--------|----------|
| **M1 — Book-scale core (P0)** | 0–7 | Projects/binder, data safety, DOCX/EPUB, stats/goals, project search, snapshots — a writer can run a whole book in `pstar` and trust it. |
| **M2 — Differentiators (P1)** | 8–11 | Sprints/focus, notes/research, annotations, style/readability. |
| **M3 — Polish (P2)** | 12–14 | Thesaurus/autocorrect, onboarding, docs/changelog/release. |

## Traceability

Every task carries `Req:` (requirement criteria) and `Design:` (design section)
back-references; ADR tasks (1.1, 3.3, 7.3, 11.1, 12.1) gate their features and
map to design §7 decisions D1–D5. Requirements coverage is complete: R1→P1,
R2→P4, R3→P8, R4→P7, R5→P9, R6→P5, R7→P3, R8→P11, R9→P10, R10→P12, R11→P2,
R12→P13.
