"""Offline tests of tools/research/agent-loop-deps (I0171): met, missing and NOT RUN for each of
the three dependencies, against scratch git repositories (a stand-in prismatic-platform and
openttd-prismatic-bridge) and a fixture HTTP server on loopback, never the real repositories or
services. An endpoint that does not answer is NOT RUN, never met."""
import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "agent-loop-deps"
GIT_ENV = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t",
           "GIT_COMMITTER_EMAIL": "t@t", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}


def load():
    loader = importlib.machinery.SourceFileLoader("agent_loop_deps", str(SCRIPT))
    spec = importlib.util.spec_from_loader("agent_loop_deps", loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


deps = load()

CONTROLLER_NO_ECHO = """defmodule PrismaticApi.Controllers.OpenttdController do
  def decision(conn, params) do
      json(conn, %{
        ai: "PrismaticAI",
        settings: "aggressiveness=50"
      })
  end
end
"""
CONTROLLER_ECHO = CONTROLLER_NO_ECHO.replace('settings: "aggressiveness=50"',
                                             'settings: "aggressiveness=50",\n        observed_digest: digest')
BRIDGE_MAIN = """from fastapi import FastAPI
app = FastAPI()


@app.get("/companies", response_model=list[CompanyOut], tags=["state"])
async def get_companies():
    return [CompanyOut.from_company(c) for c in client.state.companies.values()]


@app.post("/decision/pull")
async def decision_pull():
    return {}
"""
BRIDGE_MAIN_TICK = BRIDGE_MAIN.replace("    return [CompanyOut",
                                       "    tick, digest = client.state.tick, identity_digest()\n    return [CompanyOut")
BRIDGE_RESUME = BRIDGE_MAIN + """

@app.post("/decision/resume")
async def decision_resume():
    return {}
"""
DECISION_SELF = """async def apply_settings(rcon, approval):
    approved = await rcon(f"prismatic_mcp approve {approval}")
    return approved
"""
DECISION_OPERATOR = """async def apply_settings(rcon, approval):
    return ["pending"]
"""


def git(repo, *args):
    env = dict(os.environ, **GIT_ENV)
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True,
                          text=True, env=env).stdout.strip()


def commit(repo, files, message):
    for name, text in files.items():
        path = Path(repo) / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        git(repo, "add", name)
    git(repo, "commit", "-qm", message)
    return git(repo, "rev-parse", "HEAD")


def new_repo(path):
    path.mkdir(parents=True)
    git(path, "init", "-q", "-b", "main")
    return path


class Server:
    """A loopback HTTP server answering {path: (status, body bytes)}; anything else 404."""

    def __init__(self, routes):
        routes = dict(routes)

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                status, body = routes.get(self.path, (404, b'{"error": "not found"}'))
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}"
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()


def closed_url():
    """A loopback URL nothing listens on."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    return f"http://127.0.0.1:{port}"


def as_json(obj):
    return json.dumps(obj).encode()


class Fixture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        """The scratch repositories, built once per class and copied for each test."""
        cls.template = Path(tempfile.mkdtemp(prefix="agent-loop-deps-template-"))
        cls.addClassCleanup(shutil.rmtree, cls.template)
        prismatic = new_repo(cls.template / "prismatic-platform")
        commit(prismatic, {deps.CONTROLLER: CONTROLLER_NO_ECHO}, "decision")
        bridge = new_repo(cls.template / "openttd-prismatic-bridge")
        cls.base = commit(bridge, {deps.BRIDGE_MAIN: BRIDGE_MAIN, "bridge/decision.py": DECISION_SELF},
                          "bridge")
        git(bridge, "checkout", "-qb", "feature/operator-approval")
        cls.approval = commit(bridge, {deps.BRIDGE_MAIN: BRIDGE_RESUME,
                                       "bridge/decision.py": DECISION_OPERATOR}, "operator approval")
        git(bridge, "checkout", "-q", "main")

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="agent-loop-deps-"))
        self.addCleanup(shutil.rmtree, self.tmp)
        self.prismatic = self.tmp / "prismatic-platform"
        self.bridge = self.tmp / "openttd-prismatic-bridge"
        for repo in (self.prismatic, self.bridge):
            shutil.copytree(self.template / repo.name, repo, symlinks=True)

    def args(self, *extra, prismatic_url=None, bridge_url=None):
        return ["--prismatic-repo", str(self.prismatic), "--bridge-repo", str(self.bridge),
                "--prismatic-url", prismatic_url or closed_url(), "--bridge-url", bridge_url or closed_url(),
                "--approval-commit", self.approval, "--timeout", "2", *extra]

    def run_main(self, *argv):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = deps.main(list(argv))
        return code, out.getvalue()

    def result(self, *argv):
        code, out = self.run_main(*argv, "--json")
        result = json.loads(out)
        self.assertEqual(code, result["exit"])
        return {d["id"]: d for d in result["dependencies"]}, result

    def serve(self, routes):
        server = Server(routes)
        self.addCleanup(server.close)
        return server.url


class EchoTest(Fixture):
    def test_echoed_digest_is_met(self):
        url = self.serve({"/api/openttd/decision": (200, as_json({"ai": "PrismaticAI", "observed_digest": "ab12"}))})
        d, _ = self.result(*self.args(prismatic_url=url))
        self.assertEqual(d["echo"]["status"], deps.MET)
        self.assertIn("observed_digest", d["echo"]["evidence"][0])
        self.assertIsNone(d["echo"]["request"])
        self.assertIn("no observed_digest in " + deps.CONTROLLER, d["echo"]["main"])
        self.assertIn(deps.CONTROLLER + ":3", d["echo"]["main"])

    def test_answer_without_echo_is_missing(self):
        for body in ({"ai": "PrismaticAI"}, {"observed_digest": ""}, ["not", "an", "object"]):
            url = self.serve({"/api/openttd/decision": (200, as_json(body))})
            d, result = self.result(*self.args(prismatic_url=url))
            self.assertEqual(d["echo"]["status"], deps.MISSING, body)
            self.assertIn("prismatic-platform: echo `observed_digest`", d["echo"]["request"])
            self.assertEqual(result["exit"], 1)

    def test_unreachable_or_undecided_is_not_run(self):
        cases = {"unreachable": None,
                 "answered 502": {"/api/openttd/decision": (502, as_json({"error": {"code": "BRIDGE_UNREACHABLE"}}))},
                 "not JSON": {"/api/openttd/decision": (200, b"<html>")}}
        for why, routes in cases.items():
            url = self.serve(routes) if routes else closed_url()
            d, _ = self.result(*self.args(prismatic_url=url))
            self.assertEqual(d["echo"]["status"], deps.NOT_RUN, why)
            self.assertNotEqual(d["echo"]["status"], deps.MET)
            self.assertIn("the echo was not observed", d["echo"]["evidence"][0])
            self.assertIsNotNone(d["echo"]["request"])

    def test_main_that_echoes_is_named_with_its_line(self):
        commit(self.prismatic, {deps.CONTROLLER: CONTROLLER_ECHO}, "echo")
        d, _ = self.result(*self.args())
        self.assertIn(f"echoes it at {deps.CONTROLLER}:6", d["echo"]["main"])
        self.assertEqual(d["echo"]["status"], deps.NOT_RUN, "main's code is a hint, never the verdict")

    def test_main_without_the_controller_and_without_a_repository(self):
        git(self.prismatic, "rm", "-q", deps.CONTROLLER)
        git(self.prismatic, "commit", "-qm", "gone")
        d, _ = self.result(*self.args())
        self.assertTrue(d["echo"]["main"].endswith(f"no {deps.CONTROLLER}"))
        d, result = self.result(*self.args("--prismatic-repo", str(self.tmp / "nowhere")))
        self.assertIn("no git repository at", d["echo"]["main"])
        self.assertIn("no git repository", result["repositories"]["prismatic-platform"]["error"])


class ApprovalTest(Fixture):
    def test_main_without_the_branch_is_missing_with_the_self_approval_line(self):
        d, result = self.result(*self.args())
        a = d["approval"]
        self.assertEqual(a["status"], deps.MISSING)
        self.assertIn(f"{self.approval[:7]} is not an ancestor of openttd-prismatic-bridge main", a["evidence"][0])
        self.assertIn(f"no POST /decision/resume in {deps.BRIDGE_MAIN}", a["evidence"])
        self.assertTrue(any("bridge/decision.py:2:" in e for e in a["evidence"]), a["evidence"])
        self.assertIn("merge feature/operator-approval", a["request"])
        self.assertEqual(a["note"], "checkout main " + self.base[:7] + ": 0 ahead of, 0 behind main")
        self.assertEqual(result["exit"], 1)

    def test_merged_branch_is_met(self):
        git(self.bridge, "merge", "-q", "--ff-only", "feature/operator-approval")
        d, _ = self.result(*self.args())
        a = d["approval"]
        self.assertEqual(a["status"], deps.MET)
        self.assertIn("is an ancestor of", a["evidence"][0])
        self.assertIn(f"POST /decision/resume at {deps.BRIDGE_MAIN}:15", a["evidence"])
        self.assertIsNone(a["request"])

    def test_squashed_branch_is_met_by_its_content(self):
        commit(self.bridge, {deps.BRIDGE_MAIN: BRIDGE_RESUME, "bridge/decision.py": DECISION_OPERATOR}, "squash")
        d, _ = self.result(*self.args())
        self.assertEqual(d["approval"]["status"], deps.MET)
        self.assertIn("is not an ancestor of", d["approval"]["evidence"][0])

    def test_unknown_commit_with_self_approval_is_missing(self):
        d, _ = self.result(*self.args("--approval-commit", "0" * 40))
        self.assertIn("0000000 is unknown to", d["approval"]["evidence"][0])
        self.assertEqual(d["approval"]["status"], deps.MISSING)

    def test_no_repository_or_no_main_is_not_run(self):
        d, _ = self.result(*self.args("--bridge-repo", str(self.tmp / "nowhere")))
        self.assertEqual(d["approval"]["status"], deps.NOT_RUN)
        self.assertIn("no git repository at", d["approval"]["evidence"][0])
        d, _ = self.result(*self.args("--bridge-ref", "no-such-branch"))
        self.assertEqual(d["approval"]["status"], deps.NOT_RUN)
        self.assertIn("has no no-such-branch", d["approval"]["evidence"][0])
        self.assertIn("has no no-such-branch", d["companies"]["main"])

    def test_default_ref_prefers_origin_main(self):
        self.assertEqual(deps.Repo("b", self.bridge).ref, "main")
        git(self.bridge, "update-ref", "refs/remotes/origin/main", self.approval)
        repo = deps.Repo("b", self.bridge)
        self.assertEqual((repo.ref, repo.sha), ("origin/main", self.approval))
        d, _ = self.result(*self.args())
        self.assertEqual(d["approval"]["status"], deps.MET)
        self.assertIn("1 behind origin/main", d["approval"]["note"])
        empty = new_repo(self.tmp / "empty")
        self.assertEqual(deps.Repo("e", empty).error, f"e at {empty} has no origin/main or main")

    def test_checkout_without_a_head_has_no_note(self):
        repo = deps.Repo("b", self.bridge)
        repo.path = self.tmp / "nowhere"
        self.assertIsNone(repo.checkout())


class CompaniesTest(Fixture):
    def test_tick_and_digest_are_met(self):
        for body in ({"tick": 7, "digest": "d1", "companies": []}, {"game_tick": 9, "observed_digest": "d2"}):
            url = self.serve({"/companies": (200, as_json(body))})
            d, _ = self.result(*self.args(bridge_url=url))
            self.assertEqual(d["companies"]["status"], deps.MET, body)
            self.assertIsNone(d["companies"]["request"])
        self.assertIn("tick 9, digest d2", d["companies"]["evidence"][0])
        self.assertIn(f"GET /companies at {deps.BRIDGE_MAIN}:5; its handler names no tick and no digest",
                      d["companies"]["main"])

    def test_list_or_partial_answer_is_missing(self):
        cases = {"a JSON list of 2 companies": [{"id": 0}, {"id": 1}],
                 "no digest": {"tick": 7},
                 "no tick and no digest": {"tick": True, "digest": ""},
                 "not a JSON object": 5}
        for why, body in cases.items():
            url = self.serve({"/companies": (200, as_json(body))})
            d, result = self.result(*self.args(bridge_url=url))
            self.assertEqual(d["companies"]["status"], deps.MISSING, why)
            self.assertIn(why, d["companies"]["evidence"][0])
            self.assertIn("carry the game tick", d["companies"]["request"])

    def test_unreachable_or_undecided_is_not_run(self):
        for routes in (None, {"/companies": (503, b'{"detail": "no game"}')}, {"/companies": (200, b"not json")}):
            url = self.serve(routes) if routes else closed_url()
            d, _ = self.result(*self.args(bridge_url=url))
            self.assertEqual(d["companies"]["status"], deps.NOT_RUN, routes)
            self.assertIn("tick and digest were not observed", d["companies"]["evidence"][0])

    def test_long_answer_is_cut(self):
        url = self.serve({"/companies": (503, as_json({"detail": "x" * 300}))})
        d, _ = self.result(*self.args(bridge_url=url))
        evidence = d["companies"]["evidence"][0]
        self.assertIn("...; tick and digest were not observed", evidence)
        self.assertLess(len(evidence), 300)

    def test_main_handler_naming_tick_and_digest_and_main_without_the_route(self):
        commit(self.bridge, {deps.BRIDGE_MAIN: BRIDGE_MAIN_TICK}, "tick")
        d, _ = self.result(*self.args())
        self.assertTrue(d["companies"]["main"].endswith("its handler names tick and digest"))
        commit(self.bridge, {deps.BRIDGE_MAIN: "app = None\n"}, "no route")
        d, _ = self.result(*self.args())
        self.assertTrue(d["companies"]["main"].endswith(f"no GET /companies in {deps.BRIDGE_MAIN}"))

    def test_bad_url_is_not_run(self):
        d, _ = self.result(*self.args(bridge_url="nota-url"))
        self.assertEqual(d["companies"]["status"], deps.NOT_RUN)


class RunTest(Fixture):
    def met_everywhere(self):
        git(self.bridge, "merge", "-q", "--ff-only", "feature/operator-approval")
        return (self.serve({"/api/openttd/decision": (200, as_json({"observed_digest": "ab"}))}),
                self.serve({"/companies": (200, as_json({"tick": 1, "digest": "ab"}))}))

    def test_all_met_exits_0_and_lists_no_request(self):
        p, b = self.met_everywhere()
        code, out = self.run_main(*self.args(prismatic_url=p, bridge_url=b))
        self.assertEqual(code, 0)
        self.assertNotIn("request", out)
        self.assertIn("agent-loop-deps: 3 met, 0 missing, 0 NOT RUN (met, exit 0)", out)
        self.assertEqual(out.count("OBSERVED "), 3)

    def test_unreachable_with_the_rest_met_is_partial_never_met(self):
        git(self.bridge, "merge", "-q", "--ff-only", "feature/operator-approval")
        code, out = self.run_main(*self.args())
        self.assertEqual(code, 3)
        self.assertIn("1 met, 0 missing, 2 NOT RUN (partial, exit 3)", out)
        self.assertEqual(out.count("  request   "), 2)

    def test_nothing_readable_is_all_not_run(self):
        code, out = self.run_main(*self.args("--prismatic-repo", str(self.tmp / "a"),
                                             "--bridge-repo", str(self.tmp / "b")))
        self.assertEqual(code, 4)
        self.assertIn("0 met, 0 missing, 3 NOT RUN (not-run, exit 4)", out)
        self.assertNotIn(" met  ", out)

    def test_missing_beats_not_run(self):
        code, out = self.run_main(*self.args())
        self.assertEqual(code, 1)
        self.assertIn("0 met, 1 missing, 2 NOT RUN (missing, exit 1)", out)
        self.assertIn("  note      checkout main", out)

    def test_record_writes_json_and_markdown(self):
        target = self.tmp / "results"
        code, out = self.run_main(*self.args("--record", str(target)))
        files = sorted(p.name for p in target.iterdir())
        self.assertEqual(len(files), 2)
        self.assertRegex(files[0], r"^agent-loop-deps-\d{8}T\d{6}Z\.json$")
        stored = json.loads((target / files[0]).read_text())
        self.assertEqual(stored["schema"], deps.SCHEMA)
        self.assertEqual(stored["exit"], code)
        md = (target / files[1]).read_text()
        self.assertIn("OBSERVED approval  missing", md)
        self.assertTrue(md.endswith("```\n"))

    def test_defaults_read_the_environment(self):
        with mock.patch.dict(os.environ, {"PRISMATIC_URL": "http://p", "BRIDGE_URL": "http://b"}):
            args = deps.parse([])
        self.assertEqual((args.prismatic_url, args.bridge_url), ("http://p", "http://b"))
        with mock.patch.dict(os.environ, {"PRISMATIC_URL": "", "BRIDGE_URL": ""}):
            args = deps.parse([])
        self.assertEqual((args.prismatic_url, args.bridge_url), ("http://127.0.0.1:4004", "http://127.0.0.1:8730"))
        self.assertTrue(args.prismatic_repo.endswith("dev/prismatic-platform"))
        self.assertEqual(args.approval_commit, deps.APPROVAL_COMMIT)

    def test_script_runs_as_a_command(self):
        p = subprocess.run([sys.executable, "-B", str(SCRIPT), *self.args()], capture_output=True, text=True)
        self.assertEqual(p.returncode, 1, p.stderr)
        self.assertIn("OBSERVED echo      NOT RUN", p.stdout)
        with mock.patch.object(sys, "argv", ["agent-loop-deps", *self.args()]):
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(deps.main(), 1)


if __name__ == "__main__":
    unittest.main()
