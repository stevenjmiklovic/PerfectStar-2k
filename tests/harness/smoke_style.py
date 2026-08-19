#!/usr/bin/env python3
"""End-to-end smoke test for style and readability (R8, Phase 11).

Drives the real binary in a PTY: turn style checking on (^OY), walk to the next
style issue (^QI) and check the status line names it, then open the stats overlay
(^OI) and check the readability and overused-word figures are there.

Exiting writes the day's word delta to the stats directory, which macOS does not
let a test redirect, so the script removes only files it created.

Usage: .harness-venv/bin/python tests/harness/smoke_style.py
"""

import pathlib
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from pty_harness import PtyHarness  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[2]
BINARY = ROOT / "target" / "debug" / "pstar"


def stats_root():
    if sys.platform == "darwin":
        return pathlib.Path.home() / "Library/Application Support/perfectstar2k/stats"
    return pathlib.Path.home() / ".local/state/perfectstar2k/stats"


def main():
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)
    root = stats_root()
    before = set(root.iterdir()) if root.exists() else set()

    workdir = pathlib.Path(tempfile.mkdtemp(prefix="pstar-smoke-"))
    chapter = workdir / "chapter.md"
    chapter.write_text(
        "He took the knife.\n"
        "He walked quietly to the table and the knife was taken.\n"
        "The knife was heavy. The knife was his.\n"
    )
    failures = []

    def check(condition, message, pty):
        if not condition:
            failures.append(
                f"{message}\n----- screen -----\n{pty.text()}\n------------------"
            )

    try:
        with PtyHarness([str(BINARY), str(chapter)]) as pty:
            pty.send_raw(b" ")  # dismiss the splash
            pty.wait_for("chapter.md", timeout=10)

            # ^OY: style checking on (R8.1, off by default).
            pty.send_ctrl("o")
            pty.send("y")
            pty.wait_for("Style checking on", timeout=5)

            # ^QI: the next style issue, named (R8.3).
            pty.send_ctrl("q")
            pty.send("i")
            pty.wait_for("Style:", timeout=5)
            screen = pty.text()
            check("adverb" in screen, "first issue should be the -ly adverb", pty)

            pty.send_ctrl("q")
            pty.send("i")
            pty.wait_for("passive", timeout=5)

            # ^OI: readability and overused words on demand (R8.4, R8.5).
            pty.send_ctrl("o")
            pty.send("i")
            pty.wait_for("Writing Stats", timeout=5)
            screen = pty.text()
            check("Readability (document)" in screen, "no readability figures", pty)
            check("words/sentence" in screen, "no sentence-length figure", pty)
            check("Most repeated" in screen, "no overused-word report", pty)
            check("knife: 4" in screen, "overused word count wrong", pty)

            pty.send_raw(b"\x1b")
            pty.wait_for("Ln ", timeout=5)

            # ^OY again: back off.
            pty.send_ctrl("o")
            pty.send("y")
            pty.wait_for("Style checking off", timeout=5)

            pty.send_ctrl("k")
            pty.send("q")
    finally:
        shutil.rmtree(workdir, ignore_errors=True)
        if root.exists():
            for created in set(root.iterdir()) - before:
                if created.is_file():
                    created.unlink()
                else:
                    shutil.rmtree(created, ignore_errors=True)

    if failures:
        print("\n\n".join(failures))
        print(f"FAIL: {len(failures)} check(s) failed")
        return 1
    print("PASS: style on → next issue → readability report → style off")
    return 0


if __name__ == "__main__":
    sys.exit(main())
