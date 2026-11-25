#!/usr/bin/env python3
"""RESEARCH-ONLY. Tests for the approver registration of tools/research/mcp-e2e --coplay (#273),
offline: no game, no panel, no stack.

Run: python3 -B tools/research/test_mcp_e2e_approver.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py').

The function `register_owner_approver` is cut out of tools/research/mcp-e2e and run by bash as it
stands there, so what is tested is what runs. A fake panel on 127.0.0.1 records what reaches
POST /api/approvers and answers as mcp-panel does: the game's console lines under `output`, or an
`error`. The tests prove that the secret travels only in the X-Panel-Owner header (never in the
command line, the URL, the body or the script's output), that the body names the owner's client,
that the panel's answer or refusal is shown, and that without the secret file nothing is sent.
"""
import http.server
import json
import os
import re
import shutil
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "research" / "mcp-e2e"
SECRET = "s3cr3t-OwnerToken_0123456789abcdef"


def function_source():
    """The function's text exactly as mcp-e2e holds it, from its first line to its closing brace."""
    text = SCRIPT.read_text()
    match = re.search(r"^register_owner_approver\(\) \{\n.*?^\}\n", text, re.S | re.M)
    if not match:
        raise AssertionError("mcp-e2e holds no register_owner_approver()")
    return match.group(0)


class FakePanel(http.server.ThreadingHTTPServer):
    """Answers POST /api/approvers with `answer` (status, JSON) and keeps every request it got."""

    def __init__(self, answer):
        self.answer = answer
        self.requests = []
        super().__init__(("127.0.0.1", 0), Handler)


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0)).decode()
        self.server.requests.append({"path": self.path, "headers": dict(self.headers), "body": body})
        status, payload = self.server.answer
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_):
        pass


@unittest.skipUnless(shutil.which("curl") and shutil.which("jq"), "needs curl and jq on PATH")
class RegisterOwnerApprover(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.secret_file = Path(self.dir.name) / "panel-owner-secret"
        self.secret_file.write_text(SECRET + "\n")
        os.chmod(self.secret_file, 0o600)

    def tearDown(self):
        self.dir.cleanup()

    def run_function(self, answer, secret_file, client_id="2"):
        panel = FakePanel(answer)
        thread = threading.Thread(target=panel.serve_forever, daemon=True)
        thread.start()
        try:
            url = f"http://127.0.0.1:{panel.server_address[1]}"
            script = function_source() + 'register_owner_approver "$1" "$2" "$3"\n'
            argv = ["bash", "-c", script, "mcp-e2e", url, client_id, str(secret_file)]
            done = subprocess.run(argv, capture_output=True, text=True, timeout=30)
        finally:
            panel.shutdown()
            panel.server_close()
        return argv, done, panel.requests

    def test_secret_goes_only_in_the_owner_header(self):
        answer = (200, {"output": ["[prismatic_mcp] approver added: client #2 (key AbCd1234)"]})
        argv, done, requests = self.run_function(answer, self.secret_file)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(len(requests), 1)
        got = requests[0]
        self.assertEqual(got["path"], "/api/approvers")
        self.assertEqual(got["headers"].get("X-Panel-Owner"), SECRET)
        self.assertEqual(json.loads(got["body"]), {"add": 2})
        self.assertNotIn(SECRET, got["path"] + got["body"])
        self.assertNotIn(SECRET, " ".join(argv))
        self.assertNotIn(SECRET, done.stdout + done.stderr)
        self.assertIn("==> approver: the owner's client #2 ([prismatic_mcp] approver added", done.stdout)

    def test_the_panels_refusal_is_shown(self):
        answer = (401, {"error": "/api/approvers is the owner's"})
        _, done, requests = self.run_function(answer, self.secret_file)
        self.assertEqual(len(requests), 1)
        self.assertIn("(/api/approvers is the owner's)", done.stdout)
        self.assertNotIn(SECRET, done.stdout + done.stderr)

    def test_an_answer_without_an_approver_line_says_no_answer(self):
        _, done, _ = self.run_function((200, {"output": ["something else"]}), self.secret_file)
        self.assertIn("#2 (no answer)", done.stdout)

    def test_without_the_secret_file_nothing_is_sent(self):
        missing = Path(self.dir.name) / "absent"
        _, done, requests = self.run_function((200, {}), missing)
        self.assertEqual(requests, [])
        self.assertIn("no owner secret file", done.stderr)
        self.assertIn("approve in the panel", done.stderr)

    def test_an_empty_secret_file_sends_nothing(self):
        self.secret_file.write_text("")
        _, done, requests = self.run_function((200, {}), self.secret_file)
        self.assertEqual(requests, [])
        self.assertIn("no owner secret file", done.stderr)

    def test_the_call_site_uses_the_function(self):
        text = SCRIPT.read_text()
        self.assertIn('register_owner_approver "$URL" "$OWNER_ID" "$OWNER_SECRET_FILE"', text)
        self.assertNotIn('"command":"approvers', text, "/api/console refuses approvers lines (#258)")


if __name__ == "__main__":
    unittest.main()
