#!/usr/bin/env python3
"""End-to-end smoke test for sprints and focus mode (R3, Phase 8).

Drives the real binary in a PTY: toggle focus mode (^OF) and check the chrome
actually leaves and comes back, then run a word-target sprint (^OP) to
completion by typing and check it reports.

A finished sprint appends to the document's stats file under the platform
metadata root, which is not redirectable on macOS, so this script records the
stats files present before the run and removes only the ones it created.

Usage: .harness-venv/bin/python tests/harness/smoke_sprint_focus.py
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
    """The same directory `paths::stats()` resolves to."""
    if sys.platform == "darwin":
        return pathlib.Path.home() / "Library/Application Support/perfectstar2k/stats"
    return pathlib.Path.home() / ".local/state/perfectstar2k/stats"


def main():
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)
    root = stats_root()
    before = set(root.iterdir()) if root.exists() else set()

    workdir = pathlib.Path(tempfile.mkdtemp(prefix="pstar-smoke-"))
    chapter = workdir / "chapter.md"
    chapter.write_text("The knife was on the table.\n")
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
            check("Ln 1" in pty.text(), "status line missing before focus mode", pty)

            # ^OF: focus mode strips the chrome (R3.3).
            pty.send_ctrl("o")
            pty.send("f")
            pty.wait_for("Focus mode", timeout=5)
            # The banner sits on the status row; typing clears it, and then the
            # row should be blank — no Ln/Col, no filename, no word count.
            pty.send_ctrl("q")
            pty.send("c")  # document end: a movement, so no text is changed
            check("Ln 1" not in pty.text(), "focus mode left the status chrome up", pty)
            check("words" not in pty.text(), "focus mode left the word count up", pty)
            check(
                "The knife was on the table." in pty.text(),
                "focus mode hid the prose it exists to show",
                pty,
            )

            # ^OF again: the chrome comes back.
            pty.send_ctrl("o")
            pty.send("f")
            pty.wait_for("Focus mode off", timeout=5)
            pty.send_ctrl("q")
            pty.send("c")
            check("Ln " in pty.text(), "leaving focus mode left the chrome off", pty)

            # ^OP: a three-word sprint, finished by typing (R3.1, R3.2).
            pty.send_ctrl("o")
            pty.send("p")
            pty.wait_for("Sprint:", timeout=5)
            pty.send("/3")
            pty.send("\r")
            pty.wait_for("Sprint started", timeout=5)
            check("0/3" in pty.text(), "sprint countdown not shown", pty)

            # wait_for, not a bare screen read: the app has to process the keys
            # and redraw before the chip changes.
            pty.send(" one two")
            pty.wait_for("2/3", timeout=5)
            # The third word completes the sprint mid-typing; the report has to
            # survive the keystrokes that follow it (R3.2).
            pty.send(" three and more besides")
            pty.wait_for("Sprint done", timeout=5)
            screen = pty.text()
            check("3 words" in screen, "sprint report missing the word count", pty)
            check("✓" in screen, "sprint met its target but says otherwise", pty)
            check("0/3" not in screen, "finished sprint still shows a countdown", pty)

            # Quit, abandoning the unsaved text.
            pty.send_ctrl("k")
            pty.send("q")
            pty.send("y")
    finally:
        shutil.rmtree(workdir, ignore_errors=True)
        if root.exists():
            for created in set(root.iterdir()) - before:
                created.unlink() if created.is_file() else shutil.rmtree(
                    created, ignore_errors=True
                )

    if failures:
        print("\n\n".join(failures))
        print(f"FAIL: {len(failures)} check(s) failed")
        return 1
    print("PASS: focus mode on/off · sprint start → progress → report")
    return 0


if __name__ == "__main__":
    sys.exit(main())
