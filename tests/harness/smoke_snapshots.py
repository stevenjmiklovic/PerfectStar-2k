#!/usr/bin/env python3
"""End-to-end smoke test for snapshots, revisions, and diff (R4, Phase 7).

Drives the real binary in a PTY: take a labelled snapshot (^KN), revise the
document, list revisions (^KO), open the diff (Enter), restore (^R), and undo
(^U). Asserts on the emulated screen at each step.

macOS gives no way to redirect the metadata root, so this writes into the real
one. `MetadataGuard` removes whatever the run created across every metadata
subdirectory — saving a file touches more of them than a test means to, since it
also triggers the rolling backup, the auto-snapshot, and the session record.

Usage: .harness-venv/bin/python tests/harness/smoke_snapshots.py
"""

import pathlib
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from pty_harness import MetadataGuard, PtyHarness  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[2]
BINARY = ROOT / "target" / "debug" / "pstar"



def main():
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)

    workdir = pathlib.Path(tempfile.mkdtemp(prefix="pstar-smoke-"))
    chapter = workdir / "chapter.md"
    chapter.write_text("The knife was on the table.\n")
    failures = []

    def check(condition, message, screen):
        if not condition:
            failures.append(f"{message}\n----- screen -----\n{screen}\n------------------")

    def shown(pty, needle):
        """Substring test against the whole screen. `display()` is a list of
        lines, so `needle in pty.display()` would silently test line equality."""
        return needle in pty.text()

    try:
        with MetadataGuard(), PtyHarness([str(BINARY), str(chapter)]) as pty:
            # The splash swallows the first keystroke; dismiss it, then wait for
            # the editor proper.
            pty.send_raw(b" ")
            pty.wait_for("chapter.md", timeout=10)

            # ^KN: snapshot with a label.
            pty.send_ctrl("k")
            pty.send("n")
            pty.wait_for("Snapshot label", timeout=5)
            pty.send("before the cut")
            pty.send("\r")
            # "Snapshot" alone would match the prompt that is still on screen.
            pty.wait_for("saved", timeout=5)
            check(
                shown(pty, 'Snapshot "before the cut" saved'),
                "labelled snapshot not confirmed",
                pty.text(),
            )

            # Revise the document: append a sentence.
            pty.send_ctrl("q")
            pty.send("c")  # document end
            pty.send("He picked it up.\n")

            # ^KO: revisions list shows the labelled version.
            pty.send_ctrl("k")
            pty.send("o")
            pty.wait_for("Revisions", timeout=5)
            screen = pty.text()
            check("before the cut" in screen, "revisions list missing the label", screen)
            check("6 words" in screen, "revisions list missing word counts", screen)

            # Enter: diff the snapshot against the current draft.
            pty.send("\r")
            pty.wait_for("current draft", timeout=5)
            screen = pty.text()
            check("+1" in screen, "diff title missing the add count", screen)
            check("+ He picked it up." in screen, "diff missing the added line", screen)
            check(
                "  The knife was on the table." in screen,
                "diff missing the unchanged context line",
                screen,
            )

            # ^R: restore the snapshot, then ^U: undo it.
            pty.send_ctrl("r")
            pty.wait_for("Restored", timeout=5)
            screen = pty.text()
            check(
                "He picked it up." not in screen,
                "restore did not replace the revised text",
                screen,
            )
            check(
                "The knife was on the table." in screen,
                "restore lost the snapshot text",
                screen,
            )
            pty.send_ctrl("u")
            pty.wait_for("He picked it up.", timeout=5)
            check(
                shown(pty, "He picked it up."),
                "undo did not bring the revision back",
                pty.text(),
            )

            # Quit, abandoning the unsaved state.
            pty.send_ctrl("k")
            pty.send("q")
            pty.send("y")
    finally:
        shutil.rmtree(workdir, ignore_errors=True)

    if failures:
        print("\n\n".join(failures))
        print(f"FAIL: {len(failures)} check(s) failed")
        return 1
    print("PASS: snapshot → revisions → diff → restore → undo")
    return 0


if __name__ == "__main__":
    sys.exit(main())
