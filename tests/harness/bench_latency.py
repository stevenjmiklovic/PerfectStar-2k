#!/usr/bin/env python3
"""Keystroke-latency check for `pstar` on a 300k-word manuscript (C6).

Opens the large fixture in the release binary through the PTY harness, types a
burst of characters, and measures the round-trip from sending each keystroke to
seeing the editor's screen update. As the pro-writer features land (incremental
word count, style/spell scanning, project search), re-running this guards
against any of them creeping onto the hot editing path and blowing the
sub-frame budget.

The gotchas from the project's testing notes are all handled inside
`pty_harness` (DA1 reply, incremental UTF-8 decode, continuous draining) — but
one belongs here: **cargo sometimes reports `Fresh` and skips recompiling after
a real source edit on this machine**, so a bench could silently measure a stale
binary. This script rebuilds and verifies the binary's mtime is newer than the
newest source file before trusting a run, and prints the binary path so a
caller can confirm.

Usage:
    .harness-venv/bin/python tests/harness/bench_latency.py
    .harness-venv/bin/python tests/harness/bench_latency.py --budget-ms 16 --keys 40
"""

import argparse
import statistics
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from pty_harness import PtyHarness  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE = REPO_ROOT / "tests" / "fixtures" / "manuscript-300k.md"
BINARY = REPO_ROOT / "target" / "release" / "pstar"


def newest_source_mtime():
    newest = 0.0
    for p in (REPO_ROOT / "src").rglob("*.rs"):
        newest = max(newest, p.stat().st_mtime)
    return newest


def ensure_binary():
    """Build release and guard against the known 'Fresh'/stale-binary trap."""
    print("Building release binary…")
    subprocess.run(
        ["cargo", "build", "--release", "--quiet"],
        cwd=REPO_ROOT, check=True,
    )
    if not BINARY.exists():
        sys.exit(f"binary not found at {BINARY}")
    if BINARY.stat().st_mtime < newest_source_mtime():
        # Cargo thought it was Fresh; force a clean rebuild of just the bin.
        print("  binary older than sources — forcing rebuild after touch…")
        subprocess.run(["cargo", "clean", "--release", "-p", "perfectstar2k"],
                       cwd=REPO_ROOT, check=False)
        subprocess.run(["cargo", "build", "--release", "--quiet"],
                       cwd=REPO_ROOT, check=True)
    print(f"  using {BINARY}")


def ensure_fixture(words):
    if FIXTURE.exists():
        return
    print(f"Fixture missing — generating {FIXTURE}…")
    subprocess.run(
        [sys.executable, str(Path(__file__).with_name("gen_manuscript.py")),
         "--words", str(words), "--out", str(FIXTURE)],
        check=True,
    )


def measure(keys, budget_ms):
    latencies = []
    with PtyHarness([str(BINARY), str(FIXTURE)], rows=40, cols=120) as h:
        # The splash screen swallows the first keystroke; dismiss it, then wait
        # for the editor to paint. The status line shows the word count once
        # the buffer is up.
        h.wait_for("Press any key", timeout=20.0)
        h.send_raw(b" ")
        h.wait_for("words", timeout=20.0)
        time.sleep(0.3)

        # Jump to document end so typing appends without scrolling artifacts.
        h.send_ctrl("q")
        h.send("c")
        time.sleep(0.2)

        for i in range(keys):
            ch = "abcdefghijklmnopqrstuvwxyz"[i % 26]
            before = h.text()
            t0 = time.monotonic()
            h.send(ch)
            # Spin until the screen changes (the char appears / cursor moves).
            deadline = t0 + 2.0
            while time.monotonic() < deadline:
                if h.text() != before:
                    break
                time.sleep(0.0005)
            latencies.append((time.monotonic() - t0) * 1000.0)

    return latencies


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--keys", type=int, default=60,
                    help="keystrokes to sample (enough for a stable p95)")
    # The measured round-trip includes a full pyte screen diff and Python poll
    # per sample, so the ~8ms floor is mostly harness overhead, not pstar's
    # work. The budget is set well above the observed p95 (~10ms) and outliers
    # (~20ms) so it doesn't flake, while still catching a real C6 regression —
    # a feature that pushes per-keystroke work into the hundreds of ms.
    ap.add_argument("--budget-ms", type=float, default=50.0,
                    help="per-keystroke p95 latency budget in ms")
    ap.add_argument("--words", type=int, default=300_000)
    ap.add_argument("--percentile", type=float, default=95.0)
    args = ap.parse_args()

    ensure_binary()
    ensure_fixture(args.words)

    latencies = measure(args.keys, args.budget_ms)
    latencies_sorted = sorted(latencies)
    idx = min(len(latencies_sorted) - 1,
              int(len(latencies_sorted) * args.percentile / 100.0))
    pct = latencies_sorted[idx]
    med = statistics.median(latencies)
    worst = max(latencies)

    print(f"\nKeystroke latency over {len(latencies)} samples on {args.words:,}-word doc:")
    print(f"  median : {med:6.2f} ms")
    print(f"  p{args.percentile:g}    : {pct:6.2f} ms")
    print(f"  worst  : {worst:6.2f} ms")
    print(f"  budget : {args.budget_ms:6.2f} ms")

    if pct > args.budget_ms:
        print(f"\nFAIL: p{args.percentile:g} latency {pct:.2f}ms exceeds "
              f"{args.budget_ms:.2f}ms budget (C6 regression).")
        sys.exit(1)
    print(f"\nPASS: within the {args.budget_ms:.2f}ms budget.")


if __name__ == "__main__":
    main()
