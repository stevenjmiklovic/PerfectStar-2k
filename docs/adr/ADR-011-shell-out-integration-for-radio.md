# ADR-011: Shell-Out Integration Pattern for pstar-radio

**Date:** 2025-07-14
**Status:** Draft
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

`pstar-radio` is a standalone TUI binary that the `pstar` editor should be able to launch via `^OY`. The integration must allow the writer to switch to the radio, interact with it, and return to their document seamlessly. The two programs share a terminal — they cannot both draw to it simultaneously without one of them yielding control.

## Decision Drivers

- pstar owns the terminal (raw mode, alternate screen) during editing — it cannot share this with another full-screen TUI without one of them suspending
- The radio's state persists on disk (state.json) — it does not need to stay resident in memory between invocations
- Writers will use the radio briefly (pick a track, skip, adjust) then return to writing — latency of the switch matters but not critically
- Future evolution may embed the radio in-process as an overlay panel, but the v1 integration should be simple and decoupled
- The pattern must work on all platforms (macOS, Linux, Windows)

## Considered Options

1. **Shell-out: suspend terminal, spawn child process, restore terminal on exit** — like `$EDITOR` invocation or `git commit` opening an editor
2. **In-process embedding** — the radio runs as a module within pstar's event loop, rendering to a sub-area of the terminal
3. **Background daemon** — pstar-radio runs persistently in the background; pstar sends it commands via a local socket
4. **Tmux/terminal multiplexer integration** — open pstar-radio in a split pane of the terminal multiplexer

## Decision Outcome

**Chosen option:** Shell-out (option 1), because it provides full decoupling between the two binaries, requires no shared state beyond the filesystem, works identically on all platforms, and matches established Unix patterns (editors shelling out to other tools). The radio gets full terminal control during its invocation and yields it back cleanly.

### Positive Consequences

- Zero coupling between the two binaries' internal architectures — they share only the state file format
- Each binary can evolve independently (different release cadence, different dependencies)
- The pattern is well-understood: `ratatui::restore()` → `Command::new("pstar-radio").status()` → `ratatui::init()` — three lines of integration code
- No shared memory, no IPC protocol between editor and radio, no lifetime entanglement
- If pstar-radio crashes, pstar is unaffected (it just re-initializes its terminal)
- The workspace structure (ADR-008) means both binaries are built together, so `^OY` can find the radio binary adjacent to itself

### Negative Consequences

- Terminal switch has visible flicker (alternate screen exit/enter) — brief but perceptible
- Audio playback stops when the user quits pstar-radio (mpv is a child of the radio process); the user must re-invoke `^OY` to resume — this is ambient-radio behavior (start/stop), not persistent background music
- Cannot show "now playing" in pstar's status bar while writing (that would require option 2 or 3) — this is explicitly deferred to a future version
- On Windows, `Command::new` requires the binary to be in PATH or adjacent — slightly more fragile than in-process

## Future Evolution

The workspace structure and the radio's modular architecture (`state.rs`, `mpv.rs`, `ui.rs`) are designed so that a future ADR can promote the radio to a library crate (`pstar-radio-lib`) embedded in pstar's event loop as an overlay. The shell-out integration would remain as a fallback / standalone mode.

## Links and References

- Spec: `.claude/specs/pstar-radio/requirements.md` (R9, Task 11)
- ADR-008: Cargo workspace restructure (co-location of binaries)
- Keybinding: `^OY` — `Cmd::Radio` at `Pref(O, 'y')` in keymap.rs
