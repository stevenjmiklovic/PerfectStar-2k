#!/usr/bin/env python3
"""Reusable PTY harness for driving `pstar` in a real pseudo-terminal.

This is the project's E2E substrate: a `pty.fork()` child running the binary
under an emulated terminal (`pyte`) so tests can send keystrokes and assert on
what the screen shows. It exists as a shared module because the correct way to
talk to `pstar` over a PTY has several non-obvious gotchas, learned the hard
way, that every test must respect:

  * **Answer DA1.** On startup `pstar` calls `supports_keyboard_enhancement`,
    which blocks ~2s waiting for a terminal Device-Attributes reply. We answer
    `\\x1b[?1;2c` immediately so startup isn't gated on a timeout.
  * **Incremental UTF-8 decode.** Box-drawing characters are multibyte and a
    read can split one across a chunk boundary. A naive per-chunk
    `bytes.decode()` corrupts those and desyncs `pyte`. We hold undecoded tail
    bytes and prepend them to the next chunk.
  * **Keep draining.** An undrained PTY buffer blocks the app's final frame on
    quit, so it looks hung. The reader thread drains continuously.
  * **Set the window size.** `TIOCSWINSZ` on the PTY so the app lays out to a
    known geometry.

Requires `pyte` (see tests/harness/requirements.txt); the project convention is
a venv at `.harness-venv/`.
"""

import codecs
import fcntl
import os
import pty
import select
import signal
import struct
import termios
import threading
import time

import pyte

# Device Attributes (DA1) reply: "VT100 with Advanced Video Option". Sent in
# answer to the app's startup capability probe so it doesn't wait out a timeout.
DA1_REPLY = b"\x1b[?1;2c"


class PtyHarness:
    """Drives one `pstar` process in a PTY with a `pyte` screen mirror."""

    def __init__(self, argv, rows=40, cols=120, env=None):
        self.argv = argv
        self.rows = rows
        self.cols = cols
        self.env = env
        self.pid = None
        self.master_fd = None
        self._screen = pyte.Screen(cols, rows)
        self._stream = pyte.ByteStream(self._screen)
        self._decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self._lock = threading.Lock()
        self._reader = None
        self._alive = False

    # -- lifecycle ---------------------------------------------------------

    def start(self):
        pid, fd = pty.fork()
        if pid == 0:  # child
            if self.env is not None:
                os.execvpe(self.argv[0], self.argv, self.env)
            else:
                os.execvp(self.argv[0], self.argv)
            os._exit(127)  # unreachable on success

        self.pid = pid
        self.master_fd = fd
        self._set_winsize(self.rows, self.cols)
        self._alive = True
        self._reader = threading.Thread(target=self._drain, daemon=True)
        self._reader.start()

        # Answer the startup DA1 probe right away.
        self.send_raw(DA1_REPLY)
        return self

    def _set_winsize(self, rows, cols):
        winsize = struct.pack("HHHH", rows, cols, 0, 0)
        fcntl.ioctl(self.master_fd, termios.TIOCSWINSZ, winsize)

    def _drain(self):
        """Continuously read the PTY, feeding a `pyte` byte stream. Bytes go to
        the stream verbatim (it tolerates split multibyte sequences); the
        incremental *text* decoder keeps a coherent plain-text mirror too."""
        while self._alive:
            try:
                r, _, _ = select.select([self.master_fd], [], [], 0.05)
            except (OSError, ValueError):
                break
            if not r:
                continue
            try:
                data = os.read(self.master_fd, 65536)
            except OSError:
                break
            if not data:
                break
            with self._lock:
                self._stream.feed(data)
                # Incremental decode: never let a split multibyte char corrupt
                # the text mirror; the decoder buffers an incomplete tail.
                self._decoder.decode(data)

    # -- input -------------------------------------------------------------

    def send_raw(self, data: bytes):
        os.write(self.master_fd, data)

    def send(self, text: str):
        self.send_raw(text.encode("utf-8"))

    def send_ctrl(self, letter: str):
        """Send a Ctrl-<letter> chord (e.g. 'k' -> 0x0B)."""
        assert len(letter) == 1 and letter.isalpha()
        self.send_raw(bytes([ord(letter.lower()) - ord("a") + 1]))

    # -- screen access -----------------------------------------------------

    def display(self):
        with self._lock:
            return list(self._screen.display)

    def text(self):
        return "\n".join(self.display())

    def wait_for(self, needle: str, timeout=5.0):
        """Block until `needle` appears on screen, or raise TimeoutError."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if needle in self.text():
                return True
            time.sleep(0.02)
        raise TimeoutError(f"never saw {needle!r}; screen was:\n{self.text()}")

    # -- teardown ----------------------------------------------------------

    def stop(self):
        self._alive = False
        if self.pid:
            try:
                os.kill(self.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            # Keep draining briefly so the child isn't blocked on a full PTY.
            deadline = time.monotonic() + 1.0
            while time.monotonic() < deadline:
                try:
                    pid, _ = os.waitpid(self.pid, os.WNOHANG)
                    if pid:
                        break
                except ChildProcessError:
                    break
                try:
                    os.read(self.master_fd, 65536)
                except OSError:
                    pass
                time.sleep(0.02)
            try:
                os.kill(self.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if self.master_fd is not None:
            try:
                os.close(self.master_fd)
            except OSError:
                pass

    def __enter__(self):
        return self.start()

    def __exit__(self, *exc):
        self.stop()
        return False
