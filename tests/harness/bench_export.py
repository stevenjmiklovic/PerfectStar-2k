#!/usr/bin/env python3
"""Export-throughput check for `pstar` on a 300k-word manuscript (R7.10).

R7.10 requires the export system to produce DOCX and EPUB output for a
300,000-word compiled Project within 30 seconds on the reference hardware. This
bench drives the REAL binary through the PTY harness: it opens the large fixture
in the release build, then for each of DOCX and EPUB sends the direct export
chord, types an output path into the prompt, presses Enter, and waits for the
success status line — timing the whole export end-to-end from chord to success.

The 300k single-doc fixture exercises the same exporter throughput path as a
compiled book-length Project: after compilation the exporter receives one big
`CompiledDoc` either way, so timing the single-doc export of 300k words is an
honest measure of the R7.10 throughput bound. The measured seconds are reported
so a caller sees the real number, not just a pass/fail.

The export chords used are the direct per-format ones, which export the ACTIVE
document when no project is loaded (src/app.rs: `Cmd::ExportDocx` = ^KJ,
`Cmd::ExportEpub` = ^KG; the path prompt is `InputAction::Export{Docx,Epub}`).
On success the status area shows "DOCX exported to <path>" / "EPUB exported to
<path>" (the `finish_export` format string), which is what we wait on.

The PTY gotchas (DA1 reply, incremental UTF-8 decode, continuous draining) are
handled inside `pty_harness`. One belongs here, mirrored from `bench_latency.py`:
**cargo sometimes reports `Fresh` and skips recompiling after a real source edit
on this machine**, so a bench could silently measure a stale binary. This script
rebuilds and verifies the binary's mtime is newer than the newest source file
before trusting a run, and prints the binary path so a caller can confirm.

Writing an export touches only the output files this script names (under the
system temp dir), which it deletes on exit. It does not save the document or
create a project, so the metadata tree is not written; `MetadataGuard` still
wraps the run as a belt-and-suspenders guard so an unexpected metadata write
never leaks. The 300k fixture is never modified or removed.

Usage:
    .harness-venv/bin/python tests/harness/bench_export.py
    .harness-venv/bin/python tests/harness/bench_export.py --budget-s 30 --words 300000
"""

import argparse
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from pty_harness import MetadataGuard, PtyHarness  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE = REPO_ROOT / "tests" / "fixtures" / "manuscript-300k.md"
BINARY = REPO_ROOT / "target" / "release" / "pstar"

# Direct per-format export chords (^K prefix, second key) and the success
# status token each prints. These export the active document when no project is
# loaded — exactly the single-doc 300k case this bench measures.
FORMATS = [
    # (label, ^K second key, success needle, output extension)
    ("DOCX", "j", "DOCX exported to", ".docx"),
    ("EPUB", "g", "EPUB exported to", ".epub"),
]


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


def measure(outdir, budget_s):
    """Open the fixture once, export to each format in turn, time each export.

    Returns a list of (label, seconds, ok) tuples. `ok` is False if the success
    status never appeared within the budget (a TimeoutError inside `wait_for`),
    which is itself a R7.10 failure.
    """
    results = []
    with PtyHarness([str(BINARY), str(FIXTURE)], rows=40, cols=120) as h:
        # The splash screen swallows the first keystroke; dismiss it, then wait
        # for the editor to paint. The status line shows the live word count
        # ("…w/…c") once the buffer is up.
        h.wait_for("Press any key", timeout=20.0)
        h.send_raw(b" ")
        h.wait_for("w/", timeout=20.0)
        time.sleep(0.3)

        for label, key, needle, ext in FORMATS:
            out_path = outdir / f"bench-export-300k{ext}"
            # Give each export its own timeout headroom just past the budget so
            # a genuine overrun is reported as a FAIL rather than masked by a
            # short harness timeout. +5s of slack over the R7.10 budget.
            timeout = budget_s + 5.0

            t0 = time.monotonic()
            # Direct export chord: ^K then the format key.
            h.send_ctrl("k")
            h.send(key)
            h.wait_for(f"Export {label} to file", timeout=5.0)
            # Type the output path and confirm.
            h.send(str(out_path))
            h.send_raw(b"\r")
            try:
                h.wait_for(needle, timeout=timeout)
                elapsed = time.monotonic() - t0
                ok = True
            except TimeoutError:
                elapsed = time.monotonic() - t0
                ok = False
            results.append((label, elapsed, ok, out_path))
    return results


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--budget-s", type=float, default=30.0,
                    help="per-format export time budget in seconds (R7.10)")
    ap.add_argument("--words", type=int, default=300_000,
                    help="fixture word count to generate if missing")
    args = ap.parse_args()

    ensure_binary()
    ensure_fixture(args.words)

    print(f"Exporting the {args.words:,}-word fixture to DOCX and EPUB "
          f"(R7.10 budget: {args.budget_s:g}s each).")

    outdir = Path(tempfile.mkdtemp(prefix="pstar-bench-export-"))
    written = []
    try:
        with MetadataGuard():
            results = measure(outdir, args.budget_s)
        written = [out for (_, _, _, out) in results]

        print(f"\nExport time on {args.words:,}-word doc:")
        for label, elapsed, ok, out in results:
            wrote = out.exists() and out.stat().st_size > 0
            note = "" if ok and wrote else "  (no success status / empty output)"
            print(f"  {label:5s}: {elapsed:7.2f} s{note}")
        print(f"  budget : {args.budget_s:7.2f} s each")

        failures = []
        for label, elapsed, ok, out in results:
            wrote = out.exists() and out.stat().st_size > 0
            if not ok or not wrote:
                failures.append(f"{label} did not report a successful export "
                                f"within {args.budget_s + 5:.0f}s")
            elif elapsed > args.budget_s:
                failures.append(f"{label} took {elapsed:.2f}s, over the "
                                f"{args.budget_s:.0f}s R7.10 budget")

        if failures:
            print("\nFAIL (R7.10):")
            for f in failures:
                print(f"  - {f}")
            return 1

        worst = max(elapsed for (_, elapsed, _, _) in results)
        print(f"\nPASS: DOCX and EPUB export of a {args.words:,}-word manuscript "
              f"both within {args.budget_s:g}s (R7.10); slowest was {worst:.2f}s.")
        return 0
    finally:
        for out in written:
            try:
                out.unlink()
            except OSError:
                pass
        try:
            outdir.rmdir()
        except OSError:
            pass


if __name__ == "__main__":
    sys.exit(main())
