# Tasks — pstar-radio (YouTube Radio for PerfectStar 2k)

- **Feature slug:** `pstar-radio`
- **Status:** Draft (tasks phase)
- **Requirements:** [`requirements.md`](./requirements.md)
- **Design:** [`design.md`](./design.md)
- **Date:** 2025-07-14

---

## How to read this

Tasks are ordered for incremental, test-driven delivery. Each task results in a
working, demoable increment. Dependencies flow top-to-bottom; a task marked
**[gate]** must land before its dependents.

- **`Req:`** links back to acceptance criteria.
- **`Design:`** links to the design section.
- **`Files:`** names the modules touched.

Effort key: **S** ≤ half day · **M** ~1–2 days · **L** ~3–5 days.

---

## Phase 1 — Workspace Restructure

- [ ] **1.1 [gate] Convert repo to Cargo workspace.** Create top-level
  `[workspace]` Cargo.toml with members. Move existing `src/` into `pstar/src/`
  with its own Cargo.toml preserving all deps and the `[[bin]]` section. Create
  `pstar-common/` with a no-op `lib.rs`. Create `pstar-radio/` with a
  hello-world `main.rs`. Verify `cargo build --workspace` succeeds and `cargo
  run -p perfectstar2k -- draft.md` launches the editor unchanged.
  · Req: DC1 · Design §2 · ADR: ADR-008
  · Files: `Cargo.toml`, `pstar/Cargo.toml`, `pstar-common/`, `pstar-radio/`
  · **L**

- [ ] **1.2 Extract shared theme into pstar-common.** Move `Theme`, `ThemeKind`,
  and the DOS color constants from the editor's `theme.rs` into
  `pstar-common/src/theme.rs`. The editor re-exports or wraps
  `pstar_common::theme`, adding editor-specific fields. `pstar-radio` depends
  on `pstar-common` for theming. Verify editor renders all three themes
  correctly.
  · Req: DC6 · Design §3.6
  · Files: `pstar-common/src/theme.rs`, `pstar/src/theme.rs`
  · **M**

---

## Phase 2 — Core Data Layer

- [ ] **2.1 [gate] Implement yt-dlp metadata fetcher.** Build
  `pstar-radio/src/ytdlp.rs`: define `Channel`, `Track`, `ChannelKind` structs
  with serde. Implement `fetch_channel(url) -> Result<Channel>`. Handle
  channel-vs-playlist URL normalization. Check yt-dlp is in PATH. Unit test
  parsing a saved JSON fixture. Integration test (ignored) fetching a real
  channel.
  · Req: R1.1–R1.6 · Design §3.2
  · Files: `pstar-radio/src/ytdlp.rs`
  · **M**

- [ ] **2.2 [gate] Implement state persistence.** Build
  `pstar-radio/src/state.rs`: `RadioState` with `BTreeMap<String, Channel>`,
  `last_played`. Atomic save/load (temp + rename). Navigation helpers:
  `all_tracks()`, `next_track()`, `previous_track()`. Version field for future
  migration. Test round-trip save/load, track navigation ordering, graceful
  handling of missing/corrupt files.
  · Req: R5.1–R5.5 · Design §3.3
  · Files: `pstar-radio/src/state.rs`
  · **M**

---

## Phase 3 — mpv IPC

- [ ] **3.1 [gate] Implement platform-abstracted mpv IPC client.** Build
  `platform.rs` (socket abstraction with `IpcStream` trait) and `mpv.rs` (spawn
  mpv, connect with retry loop, send commands, receive events). Non-blocking
  reads. Define `MpvEvent` enum: `PropertyChange`, `EndFile`, `Position(f64)`.
  Unit test parsing sample mpv JSON into events. Integration test (ignored)
  spawning mpv with a short URL.
  · Req: R4.1–R4.6, R10.1–R10.3 · Design §3.4, §3.5
  · Files: `pstar-radio/src/mpv.rs`, `pstar-radio/src/platform.rs`
  · **L**

---

## Phase 4 — TUI Shell

- [ ] **4.1 [gate] Build TUI event loop with polling.** Add `polling` dep.
  Watch stdin + mpv socket fd. Main loop: poll(100ms), dispatch keys or mpv
  events or tick. Define `RadioApp` struct. Wire crossterm alternate-screen +
  raw mode. App starts, shows empty state, quits on `q`.
  · Req: DC4 · Design §3.1
  · Files: `pstar-radio/src/main.rs`
  · **M**

- [ ] **4.2 Implement player UI (status bar + main area).** Render the player
  using ratatui with pstar themes. Layout: status bar (always visible), main
  area (now-playing card or list), controls hint. Marquee bouncing algorithm.
  Elapsed time display. Play-state icons. Theme application from pstar-common.
  Render to TestBackend, assert key spans present in known states.
  · Req: R6.1–R6.7 · Design §3.6
  · Files: `pstar-radio/src/ui.rs`
  · **M**

---

## Phase 5 — Interactive Commands

- [ ] **5.1 Implement all keyboard commands.** Wire up all bindings from R7:
  space (toggle), n/p (next/prev), f/b (seek), +/- (add/remove channel), /
  (pick track), c (pick channel), o (open in browser), q (quit). Input prompts
  for URL entry. List selection with filtering (tracks, channels). Auto-advance
  on track end. Open-in-browser via platform command.
  · Req: R7.1–R7.7, R3.2–R3.3, R10.4 · Design §3.1
  · Files: `pstar-radio/src/main.rs`, `pstar-radio/src/ui.rs`
  · **L**

---

## Phase 6 — Configuration

- [ ] **6.1 Implement TOML config.** Build `pstar-radio/src/config.rs`: parse
  config from `<config_dir>/pstar-radio/config.toml`. Support `theme`,
  `channels`, `seek_seconds`, `seek_large_seconds`. Auto-fetch config channels
  on first launch if not in state. Mark config channels as pinned. Default
  gracefully when no config file exists.
  · Req: R8.1–R8.5 · Design §3.7
  · Files: `pstar-radio/src/config.rs`
  · **S**

---

## Phase 7 — Windows Support

- [ ] **7.1 Cross-platform IPC (Windows named pipes).** Implement the Windows
  side of `platform.rs`. Use `interprocess` crate or raw Win32 API. Named pipe
  at `\\.\pipe\pstar-radio-mpv-<random>`. Same retry logic. `#[cfg(windows)]`
  conditional compilation. Verify compiles for Windows target.
  · Req: R10.1–R10.3 · Design §3.5
  · Files: `pstar-radio/src/platform.rs`
  · **M**

---

## Phase 8 — Editor Integration

- [ ] **8.1 Integration hook in pstar (`^OY`).** Add `Cmd::Radio` +
  `Pref(O, 'y')` binding in the editor. On trigger: find pstar-radio binary
  (adjacent to self, then PATH), suspend terminal (`ratatui::restore()`), spawn
  and wait, re-init terminal. Status message on not-found. Binding appears in
  palette. Error shown when binary missing. Clean round-trip when available.
  · Req: R9.1–R9.5 · Design §3.8
  · Files: `pstar/src/keymap.rs`, `pstar/src/app.rs`
  · **M**

---

## Phase 9 — Polish and Verification

- [ ] **9.1 End-to-end integration test.** Full session: add channel, browse
  tracks, play, skip, seek, remove channel, quit, re-launch and verify state
  resume. Document manual test procedure.
  · Req: all · **M**

- [ ] **9.2 Record remaining ADRs.** Write ADR-009 (polling crate event loop),
  ADR-010 (cross-platform mpv IPC), ADR-011 (shell-out integration for radio).
  Confirm ADR-008 (workspace restructure) is already recorded.
  · **S**
