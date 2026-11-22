"""Line tracer of tools/coverage/changed-lines (rule project.new-code-tested).

Started by sitecustomize.py beside this file in every Python process whose environment carries
CHANGED_LINES_DATA (a directory) and a PYTHONPATH that names this directory: the test runner,
and every Python subprocess a test starts, since they inherit both. At exit the process writes
one JSON reading, <data>/<pid>-<random>.json: each source file it ran (resolved path, SHA-256
of its bytes) and the line numbers that ran in it.

sys.monitoring (Python 3.12+) reports each line once and then disables that location, so a
traced suite runs at about the speed of an untraced one; an older interpreter falls back to
sys.settrace (and threading.settrace), which is slower and sees the same lines.

The interpreter's own library (sys.prefix, sys.base_prefix, sys.exec_prefix) is never written.
Files outside the repository are: a test that copies a tool into a scratch repository runs the
copy, and the judge attributes a copy whose bytes equal the working file's.

Not seen: a process started with -I or -E (it ignores PYTHONPATH), one whose environment drops
PYTHONPATH or CHANGED_LINES_DATA, and one that ends through os._exit() or a signal (no atexit).
"""

from __future__ import annotations

import atexit
import hashlib
import json
import os
import secrets
import sys
import threading

_LIBRARY = tuple(
    sorted({os.path.realpath(p) + os.sep for p in (sys.prefix, sys.base_prefix, sys.exec_prefix)})
)


def wanted(filename: str) -> bool:
    """A real source file that is not the interpreter's own library."""
    if not filename or filename.startswith("<"):
        return False
    return not os.path.realpath(filename).startswith(_LIBRARY)


class Recorder:
    """The lines one process ran, by the file name its code objects carry."""

    def __init__(self, data_dir: str) -> None:
        self.data_dir = data_dir
        self.ran: dict[str, set[int]] = {}
        self.tool: int | None = None
        self.mode = ""
        self.stopped = False
        self.previous = (None, None)  # the settrace tracers this one replaced, restored on stop

    def on_line(self, code, line: int):
        """sys.monitoring LINE callback: note the line, then stop reporting this location."""
        self.ran.setdefault(code.co_filename, set()).add(line)
        return sys.monitoring.DISABLE

    def tracer(self, frame, event: str, arg):
        """sys.settrace global tracer: trace frames of wanted files only."""
        if not wanted(frame.f_code.co_filename):
            return None
        return self.local

    def local(self, frame, event: str, arg):
        """sys.settrace local tracer."""
        if event == "line":
            self.ran.setdefault(frame.f_code.co_filename, set()).add(frame.f_lineno)
        return self.local

    def start(self, use_monitoring: bool | None = None) -> None:
        """Begin recording: sys.monitoring when the interpreter has it, unless told otherwise."""
        if use_monitoring is None:
            use_monitoring = hasattr(sys, "monitoring")
        if use_monitoring:
            mon = sys.monitoring
            # The coverage slot first; another tracer (an outer run of this gate) may hold it.
            free = [t for t in (mon.COVERAGE_ID, 0, 1, 2, 3, 4, 5) if mon.get_tool(t) is None]
            if not free:
                raise RuntimeError("changed-lines: every sys.monitoring tool id is taken")
            self.tool = free[0]
            mon.use_tool_id(self.tool, "changed-lines")
            mon.register_callback(self.tool, mon.events.LINE, self.on_line)
            mon.set_events(self.tool, mon.events.LINE)
            self.mode = "monitoring"
        else:
            self.previous = (sys.gettrace(), threading.gettrace())
            sys.settrace(self.tracer)
            threading.settrace(self.tracer)
            self.mode = "settrace"
        atexit.register(self.stop_and_write)

    def stop(self) -> None:
        """Stop recording; a second call does nothing."""
        if self.stopped:
            return
        self.stopped = True
        if self.mode == "monitoring":
            mon = sys.monitoring
            mon.set_events(self.tool, 0)
            mon.register_callback(self.tool, mon.events.LINE, None)
            mon.free_tool_id(self.tool)
        else:
            sys.settrace(self.previous[0])
            threading.settrace(self.previous[1])

    def reading(self) -> dict:
        """The JSON-ready reading: resolved path -> the SHA-256 of its bytes and the lines run."""
        files: dict[str, dict] = {}
        for name, lines in self.ran.items():
            if not wanted(name):
                continue
            path = os.path.realpath(name)
            try:
                with open(path, "rb") as f:
                    sha = hashlib.sha256(f.read()).hexdigest()
            except OSError:
                continue  # a scratch copy removed before exit: nothing left to compare
            entry = files.setdefault(path, {"sha256": sha, "lines": set()})
            entry["lines"].update(lines)
        return {
            "pid": os.getpid(),
            "mode": self.mode,
            "files": {p: {"sha256": e["sha256"], "lines": sorted(e["lines"])}
                      for p, e in sorted(files.items())},
        }

    def write(self) -> str:
        """Write the reading into the data directory and return its path."""
        os.makedirs(self.data_dir, exist_ok=True)
        path = os.path.join(self.data_dir, f"{os.getpid()}-{secrets.token_hex(4)}.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(self.reading(), f)
        return path

    def stop_and_write(self) -> None:
        """atexit hook: stop first, so writing the reading is not itself recorded."""
        self.stop()
        self.write()


def start(data_dir: str, use_monitoring: bool | None = None) -> Recorder:
    """Start a recorder that writes its reading into data_dir at exit."""
    recorder = Recorder(data_dir)
    recorder.start(use_monitoring)
    return recorder
