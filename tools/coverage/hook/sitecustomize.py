# Starts the line tracer of tools/coverage/changed-lines in this Python process when the gate
# asked for it (CHANGED_LINES_DATA set, this directory on PYTHONPATH); does nothing otherwise.
import os

recorder = None
if os.environ.get("CHANGED_LINES_DATA"):
    import _changed_lines_trace

    recorder = _changed_lines_trace.start(os.environ["CHANGED_LINES_DATA"])
