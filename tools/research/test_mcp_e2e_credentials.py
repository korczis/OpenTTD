#!/usr/bin/env python3
"""RESEARCH-ONLY. Tests for the stack's own credentials in tools/research/mcp-e2e, offline: no
game, no panel, no stack.

Run: python3 -B tools/research/test_mcp_e2e_credentials.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py').

The functions `make_run_token`, `stack_post`, `seal_client_home` and `open_client_secrets` are cut
out of tools/research/mcp-e2e and run by bash as they stand there, so what is tested is what runs.
The tests prove that the run token lives in a 0600 file and travels only in the X-Panel-Token
header (never on a command line, in a URL, a body or the output), that a client's secrets.cfg (the
owner client's approver key) is closed to everyone but the owner, and that the stack hands the
token to the panel and to every helper it starts.
"""
import http.server
import json
import os
import re
import stat
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools" / "research" / "mcp-e2e"
DUEL = ROOT / "tools" / "research" / "mcp-duel"


def function_source(name):
    """The function's text exactly as mcp-e2e holds it, from its first line to its closing brace."""
    match = re.search(rf"^{name}\(\) \{{\n.*?^\}}\n", SCRIPT.read_text(), re.S | re.M)
    if not match:
        raise AssertionError(f"mcp-e2e holds no {name}()")
    return match.group(0)


def run(script, env=None):
    """`script` in bash after the four functions, with `env` added."""
    lib = "".join(function_source(n) for n in ("make_run_token", "stack_post", "seal_client_home", "open_client_secrets", "open_owner_page"))
    return subprocess.run(["bash", "-c", "set -euo pipefail\n" + lib + script], capture_output=True, text=True,
                          env={**os.environ, **(env or {})}, timeout=60)


class FakePanel(http.server.ThreadingHTTPServer):
    """Answers every POST with `answer` ({"ok": true}) and keeps what it got."""

    def __init__(self, answer=None):
        self.requests = []
        self.answer = answer or {"ok": True}
        super().__init__(("127.0.0.1", 0), Handler)


class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0)).decode()
        self.server.requests.append({"path": self.path, "headers": dict(self.headers), "body": body})
        data = json.dumps(self.server.answer).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


def mode(path):
    return stat.S_IMODE(os.stat(path).st_mode)


class RunToken(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())

    def test_the_run_token_is_a_0600_file_of_random_hex_printed_nowhere(self):
        token = self.dir / "run-token"
        out = run(f'make_run_token "{token}"; make_run_token "{self.dir}/second"\n')
        self.assertEqual(out.returncode, 0, out.stderr)
        text = token.read_text()
        self.assertRegex(text, r"^[0-9a-f]{64}$")
        self.assertEqual(mode(token), 0o600)
        self.assertNotEqual(text, (self.dir / "second").read_text(), "a new token every run")
        self.assertNotIn(text, out.stdout + out.stderr)

    def test_making_the_token_fails_where_no_file_can_be_written(self):
        out = run(f'make_run_token "{self.dir}/missing/dir/run-token" 2>/dev/null || echo refused\n')
        self.assertEqual(out.stdout.strip(), "refused")

    def test_the_stacks_post_carries_the_token_only_in_its_header(self):
        token = self.dir / "run-token"
        token.write_text("t0ken-0123456789abcdef\n")
        os.chmod(token, 0o600)
        panel = FakePanel()
        threading.Thread(target=panel.serve_forever, daemon=True).start()
        try:
            url = f"http://127.0.0.1:{panel.server_address[1]}"
            out = run(f'RUN_TOKEN="{token}"; stack_post "{url}/api/console" \'{{"command":"clients"}}\'\n')
        finally:
            panel.shutdown()
            panel.server_close()
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(json.loads(out.stdout), {"ok": True})
        [req] = panel.requests
        self.assertEqual(req["path"], "/api/console")
        self.assertEqual(req["headers"].get("X-Panel-Token"), "t0ken-0123456789abcdef")
        self.assertEqual(json.loads(req["body"]), {"command": "clients"})
        self.assertNotIn("t0ken", req["path"] + req["body"] + out.stdout + out.stderr)


class ClientHome(unittest.TestCase):
    def setUp(self):
        self.home = Path(tempfile.mkdtemp()) / "client-home"
        self.secrets = self.home / "Documents" / "OpenTTD" / "secrets.cfg"
        self.secrets.parent.mkdir(parents=True)
        self.secrets.write_text("[network]\nclient_secret_key = abc\n")
        os.chmod(self.home, 0o755)
        os.chmod(self.secrets, 0o644)

    def test_the_client_home_is_sealed_and_its_secrets_read_by_the_owner_only(self):
        before = run(f'open_client_secrets "{self.home}"\n')
        self.assertEqual(before.stdout.strip(), str(self.secrets), "a 0644 secrets.cfg is reported")
        out = run(f'seal_client_home "{self.home}"; open_client_secrets "{self.home}"\n')
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(out.stdout.strip(), "", "nothing readable beyond the owner")
        self.assertEqual(mode(self.home), 0o700)
        self.assertEqual(mode(self.secrets), 0o600)

    def test_every_group_or_other_bit_counts_as_open(self):
        for m in (0o640, 0o620, 0o610, 0o604, 0o602, 0o601):
            os.chmod(self.secrets, m)
            self.assertEqual(run(f'open_client_secrets "{self.home}"\n').stdout.strip(), str(self.secrets), oct(m))
        os.chmod(self.secrets, 0o400)
        self.assertEqual(run(f'open_client_secrets "{self.home}"\n').stdout.strip(), "")

    def test_a_home_not_made_yet_is_no_error(self):
        out = run(f'seal_client_home "{self.home}/none"; open_client_secrets "{self.home}/none"; echo done\n')
        self.assertEqual((out.returncode, out.stdout.strip()), (0, "done"), out.stderr)


class Wiring(unittest.TestCase):
    """What the stack hands on, read from the scripts as they stand."""

    def test_the_panel_and_every_helper_get_the_run_token_file(self):
        text = SCRIPT.read_text()
        self.assertIn('--run-token-file "$RUN_TOKEN"', text)
        helpers = re.findall(r'"\$RHAI" --panel "\$URL"([^\n]*)', text)
        self.assertGreaterEqual(len(helpers), 4, helpers)
        for line in helpers:
            self.assertIn('--panel-token-file "$RUN_TOKEN"', line)

    def test_every_post_of_the_stack_carries_its_token_and_clients_run_under_umask_077(self):
        text = SCRIPT.read_text()
        posts = [l for l in text.splitlines() if "curl -s -X POST" in l]
        self.assertTrue(posts and all("-H @-" in l for l in posts), posts)
        self.assertEqual(text.count("    umask 077\n"), 2, "the GUI and the windowless client")
        self.assertIn('MCP_E2E_RUN_TOKEN_FILE="" "${BASH_SOURCE[0]}"', text, "another instance makes its own token")
        duel = DUEL.read_text()
        self.assertIn('export MCP_E2E_RUN_TOKEN_FILE="$TMP/run-token"', duel)
        self.assertTrue(all("-H @-" in l for l in duel.splitlines() if "curl -s -X POST" in l))


class OwnerPage(unittest.TestCase):
    """open_owner_page: the owner's browser opens through a one-time link, never with the secret."""

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.opened = self.dir / "opened"
        self.opener = self.dir / "open"
        self.opener.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" >> "{self.opened}"\n')
        os.chmod(self.opener, 0o700)
        self.secret = self.dir / "secret"
        self.secret.write_text("s3cr3t-0wner\n")
        os.chmod(self.secret, 0o600)

    def open_with(self, answer, secret_file):
        panel = FakePanel(answer)
        threading.Thread(target=panel.serve_forever, daemon=True).start()
        try:
            url = f"http://127.0.0.1:{panel.server_address[1]}"
            out = run(f'open_owner_page "{url}" "{secret_file}"\n', {"OPEN_CMD": str(self.opener)})
        finally:
            panel.shutdown()
            panel.server_close()
        return url, out, panel.requests, self.opened.read_text().strip() if self.opened.exists() else ""

    def test_the_page_opens_through_a_one_time_link_and_the_secret_goes_only_in_the_header(self):
        url, out, reqs, opened = self.open_with({"once": "n0nce123"}, self.secret)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(opened, f"{url}/?once=n0nce123")
        [req] = reqs
        self.assertEqual((req["path"], req["headers"].get("X-Panel-Owner")), ("/api/owner/once", "s3cr3t-0wner"))
        self.assertNotIn("s3cr3t", opened + out.stdout + out.stderr)

    def test_without_the_secret_file_or_a_link_the_plain_page_opens(self):
        url, out, reqs, opened = self.open_with({"once": "x"}, self.dir / "none")
        self.assertEqual((opened, reqs), (url, []), "no file: nothing asked, the plain URL")
        self.opened.unlink()
        url, out, reqs, opened = self.open_with({"error": "refused"}, self.secret)
        self.assertEqual(opened, url, "a panel that makes no link: the plain URL")
        self.assertNotIn("s3cr3t", opened + out.stdout + out.stderr)

    def test_the_script_never_puts_the_secret_into_a_url(self):
        self.assertNotIn('?owner=$(', SCRIPT.read_text())


if __name__ == "__main__":
    unittest.main()
