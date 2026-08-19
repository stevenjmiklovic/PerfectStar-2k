#!/usr/bin/env python3
"""End-to-end smoke test for the notes sidecar (R5, Phase 9).

Drives the real binary in a PTY: set a synopsis (^PI) and check it comes back
pre-filled, open the document's notes in a split (^PT) and write to them, then
open a project binder (^PP, ^PB) and check the synopsis appears as a secondary
line and a document can be marked as a note (^PM).

This also exercises the ^P prefix dispatch for the new letters, which the unit
tests bypass by calling the commands directly.

Sidecars land under the platform metadata root, which is not redirectable on
macOS, so the script records what was there before and removes only what it
created.

Usage: .harness-venv/bin/python tests/harness/smoke_notes.py
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


def meta_root():
    """The same directory `paths::meta()` resolves to."""
    if sys.platform == "darwin":
        return pathlib.Path.home() / "Library/Application Support/perfectstar2k/meta"
    return pathlib.Path.home() / ".local/state/perfectstar2k/meta"


def main():
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)
    root = meta_root()
    before = set(root.iterdir()) if root.exists() else set()

    workdir = pathlib.Path(tempfile.mkdtemp(prefix="pstar-smoke-"))
    chapter = workdir / "chapter1.md"
    chapter.write_text("The knife was on the table.\n")
    (workdir / "characters.md").write_text("Marcus: left-handed.\n")
    (workdir / "book.pstarproj").write_text(
        "name = 'Book'\n\n"
        "[[docs]]\npath = 'chapter1.md'\ntitle = 'Chapter One'\n\n"
        "[[docs]]\npath = 'characters.md'\ntitle = 'Characters'\n"
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
            pty.wait_for("chapter1.md", timeout=10)

            # ^PI: set a synopsis (R5.1).
            pty.send_ctrl("p")
            pty.send("i")
            pty.wait_for("Synopsis", timeout=5)
            pty.send("Marcus finds the knife")
            pty.send("\r")
            pty.wait_for("Synopsis saved", timeout=5)

            # ...and it comes back pre-filled for editing, not blank.
            pty.send_ctrl("p")
            pty.send("i")
            pty.wait_for("Synopsis:", timeout=5)
            check(
                "Marcus finds the knife" in pty.text(),
                "the synopsis prompt was not pre-filled",
                pty,
            )
            # Esc to leave it alone. A bare ESC byte is ambiguous until the next
            # byte arrives — send another key too soon and the terminal layer
            # reads the pair as Alt+<key>, so wait for the prompt to actually go.
            pty.send_raw(b"\x1b")
            pty.wait_for("chapter1.md", timeout=5)

            # ^PT: notes open in a split as an ordinary document (R5.4).
            pty.send_ctrl("p")
            pty.send("t")
            pty.wait_for("notes.md", timeout=5)
            screen = pty.text()
            check("notes.md" in screen, "notes did not open in a split", pty)
            check(
                "The knife was on the table." in screen,
                "the manuscript should still be visible beside the notes",
                pty,
            )
            pty.send("Marcus is left-handed.")
            pty.send_ctrl("k")
            pty.send("s")
            pty.wait_for("Saved", timeout=5)

            # ^PP: open the project, ^PB: the binder shows the synopsis (R5.3).
            pty.send_ctrl("p")
            pty.send("p")
            pty.wait_for("Open project", timeout=5)
            pty.send(str(workdir / "book.pstarproj"))
            pty.send("\r")
            pty.wait_for("Book", timeout=5)
            pty.send_ctrl("p")
            pty.send("b")
            pty.wait_for("Binder", timeout=5)
            screen = pty.text()
            check("Chapter One" in screen, "binder missing its documents", pty)
            check(
                "Marcus finds the knife" in screen,
                "binder missing the synopsis secondary line",
                pty,
            )

            # ^PM: mark the second document as a note (R5.2).
            pty.send_raw(b"\x1b[B")  # Down
            pty.send_ctrl("p")
            pty.send("m")
            pty.wait_for("excluded from compile", timeout=5)
            check("[note]" in pty.text(), "binder does not mark the note", pty)

            pty.send_raw(b"\x1b")
            pty.wait_for("Ln ", timeout=5)
            pty.send_ctrl("k")
            pty.send("q")
            pty.send("y")
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
    print("PASS: synopsis → prefill → notes split → binder synopsis → mark note")
    return 0


if __name__ == "__main__":
    sys.exit(main())
