"""Exercise the Linux terminal entry point with an isolated library and a real PTY."""
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time


def fixture(path):
    widths = [(22, 23), (37, 39), (70, 74)]
    manifest = {
        "format": "cascii-multisize", "schema_version": 2,
        "frame_count": 2, "fps": 30, "default_columns": 70, "column_map": [22, 70],
        "font_ratio": 0.7, "luminance": 20, "ascii_chars": "ABCDEF", "text": True,
        "color": False, "outputs": ["frames.bin"],
        "variants": [{"columns": w, "rows": [h, h]} for w, h in widths],
    }
    meta = json.dumps(manifest).encode()
    index = bytearray()
    payload = bytearray()
    for frame in range(2):
        for i, (width, height) in enumerate(widths):
            glyph = bytes([ord("A") + frame * 3 + i])
            row = glyph * width if frame == 0 else glyph * (width - 2) + b"  "
            body = (row + b"\n") * height
            index.extend(struct.pack("<IIQII", width, frame, len(payload), len(body), 0))
            payload.extend(body)
    path.write_bytes(struct.pack("<4sIII", b"CSMV", 2, len(meta), 6) + meta + index + payload)


def resize(fd, rows):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, 80, 0, 0))


def expect(fd, status, glyph=None):
    output = bytearray()
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if select.select([fd], [], [], 0.2)[0]:
            data = os.read(fd, 65536)
            if not data:
                break
            output.extend(data)
            while b"\x1b[?2026l" in output:
                update, _, output = output.partition(b"\x1b[?2026l")
                _, begin, paint = update.partition(b"\x1b[?2026h")
                assert begin, "Frame ended without beginning a synchronized update"
                assert b"\x1b[2J" not in paint, "Screen erased during playback"
                if status in paint and (glyph is None or glyph in paint):
                    return paint
    raise AssertionError(f"Missing {status!r} or {glyph!r}: {output[-300:]!r}")


def check(binary, root):
    asset = root / "responsive.bin"
    fixture(asset)
    env = dict(os.environ, XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
    subprocess.run([binary, "save", asset, "responsive"], env=env, check=True)
    pid, fd = pty.fork()
    if pid == 0:
        resize(1, 24)
        os.execve(binary, [binary, "play", "responsive", "--paused"], env)
    try:
        expect(fd, b"frame 1/2 | 22x23 auto", b"A" * 22)
        resize(fd, 40)
        os.kill(pid, signal.SIGWINCH)
        expect(fd, b"frame 1/2 | 37x39 auto", b"B" * 37)
        os.write(fd, b"g")
        expect(fd, b"Go to frame")
        os.write(fd, b"2\r")
        expect(fd, b"frame 2/2 | 37x39 auto", b"E" * 35 + b"  ")
        os.write(fd, b"[")
        smaller = expect(fd, b"frame 2/2 | 22x23 manual", b"D" * 20 + b"  ")
        # The first row used to contain artwork. It is now entirely letterbox.
        assert b"\x1b[1;1H" + b" " * 80 in smaller
        os.write(fd, b"p")
        expect(fd, b"frame 2/2 | 70x74 map", b"F" * 5)
        os.write(fd, b"q")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if select.select([fd], [], [], 0.1)[0]:
                try:
                    os.read(fd, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
            done, status = os.waitpid(pid, os.WNOHANG)
            if done:
                pid = None
                assert os.waitstatus_to_exitcode(status) == 0
                return
        raise AssertionError("Terminal did not exit")
    finally:
        os.close(fd)
        if pid is not None:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)


if __name__ == "__main__":
    if sys.platform != "linux":
        sys.exit("This test uses Linux XDG directories for isolation.")
    with tempfile.TemporaryDirectory() as directory:
        check(str(Path(sys.argv[1]).resolve()), Path(directory))
