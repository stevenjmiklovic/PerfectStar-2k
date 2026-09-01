# PTY test harness

End-to-end testing for `pstar` drives the real binary inside a pseudo-terminal
and asserts on the emulated screen, because a TUI's behavior only exists on a
terminal. This directory holds the shared harness plus the performance fixture
and latency bench introduced with the pro-writer feature set (task 0.5) to
guard constraint **C6**: sub-frame keystroke latency on a ≥300k-word manuscript.

## Setup

```sh
python3 -m venv .harness-venv
.harness-venv/bin/pip install -r tests/harness/requirements.txt
```

## Files

| File | Purpose |
|------|---------|
| `pty_harness.py` | Reusable `PtyHarness` — `pty.fork()` + `pyte` screen mirror. Encapsulates the project's hard-won PTY gotchas (see below). |
| `gen_manuscript.py` | Deterministic large-manuscript generator. Default 300k words → `tests/fixtures/manuscript-300k.md` (git-ignored; regenerate on demand). |
| `bench_latency.py` | Opens the fixture, types a burst, measures per-keystroke round-trip latency against a budget. Exits non-zero on regression. |
| `smoke_snapshots.py` | End-to-end walk of the R4 snapshot flow: `^KN` label → revise → `^KO` list → Enter diff → `^R` restore → `^U` undo. Exits non-zero on any failed screen check. |
| `smoke_sprint_focus.py` | End-to-end walk of R3: `^OF` focus mode on/off (chrome actually leaves and returns) and a word-target sprint (`^OP`) driven to completion by typing. |
| `smoke_style.py` | End-to-end walk of R8: `^OY` on/off, `^QI` next style issue with the issue named, and the `^OI` readability and overused-word report. |
| `smoke_notes.py` | End-to-end walk of R5: `^PI` synopsis (including the pre-filled re-prompt), `^PT` notes in a split, and the binder's synopsis line and `^PM` note marking. Also covers `^P` prefix dispatch, which the unit tests bypass. |

## Running the smoke tests

```sh
.harness-venv/bin/python tests/harness/smoke_snapshots.py
.harness-venv/bin/python tests/harness/smoke_sprint_focus.py
.harness-venv/bin/python tests/harness/smoke_notes.py
.harness-venv/bin/python tests/harness/smoke_style.py
```

Both write under the platform metadata root (snapshots, stats), which macOS does
not let a test redirect, so each script records what was there before the run and
deletes only what it created.

Assert with `wait_for`, not a bare screen read after `send`: the app needs a turn
of its event loop to process keys and redraw. Note also that `display()` returns a
*list of rows* — use `text()` for substring checks.

**A bare `ESC` needs the app to settle before the next key.** `ESC` is ambiguous
until the following byte arrives, so `send_raw(b"\x1b")` immediately followed by
another key is read as `Alt+<key>` and both are swallowed. Follow an `ESC` with a
`wait_for` on whatever proves the mode actually changed.

## Running the latency bench

```sh
.harness-venv/bin/python tests/harness/bench_latency.py
# or a smaller/tighter run:
.harness-venv/bin/python tests/harness/bench_latency.py --keys 40 --budget-ms 16
```

The bench builds `--release` and, because cargo on some machines reports
`Fresh` and skips recompiling after a real edit, verifies the binary is newer
than the sources (forcing a clean rebuild if not) before trusting a run.

## The PTY gotchas (baked into `pty_harness.py`)

- **Answer DA1.** `pstar` probes terminal capabilities at startup and blocks
  ~2s waiting for a Device-Attributes reply; the harness answers `\x1b[?1;2c`
  immediately.
- **Incremental UTF-8 decode.** A read can split a multibyte box-drawing char
  across chunks; a naive per-chunk decode corrupts it and desyncs `pyte`. The
  harness feeds bytes to `pyte` verbatim and keeps an incremental text decoder.
- **Keep draining.** An undrained PTY blocks the app's final frame on quit and
  it looks hung; a background thread drains continuously.
- **Set the window size** via `TIOCSWINSZ` so layout is deterministic.
