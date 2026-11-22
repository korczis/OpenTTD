#!/usr/bin/env python3
"""Tests for plan_github.py: every line runs, against a fake gh and a fake majordomus.

Run directly (`python3 tools/plan-github/test_plan_github.py`); with --coverage it also measures,
through sys.monitoring, that every executable line of plan_github.py ran and fails naming the
lines that did not (owner order 2026-11-22: every changed line is run by a test).
"""

import contextlib
import io
import json
import re
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import plan_github as pg  # noqa: E402

ROADMAP = {"milestones": [
    {"id": "b-second", "order": 20, "title": "Second", "status": "PLANNED"},
    {"id": "a-first", "order": 10, "title": "First", "status": "DONE"},
]}
ISSUES = {"issues": [
    {"id": "I0001", "milestone": "a-first", "status": "DONE", "priority": "p1", "title": "One"},
    {"id": "I0002", "milestone": "b-second", "status": "READY", "priority": "p2", "title": "Two"},
    {"id": "I0003", "milestone": "b-second", "status": "BLOCKED", "priority": "p3", "title": "Three"},
]}


class FakeWorld:
    """An in-memory GitHub repository plus canned majordomus answers, served through pg.run."""

    def __init__(self, has_issues=True):
        self.has_issues = has_issues
        self.labels = {"plan": {"name": "plan", "color": "000000", "description": "old"}}
        self.milestones = []
        self.issues = []
        self.calls = []

    def run(self, cmd, stdin=None):
        self.calls.append(cmd)
        if cmd[0] == "majordomus":
            return self.majordomus(cmd[1:])
        if cmd[0] == "git":
            return "/tmp/repo\n"
        assert cmd[:2] == ["gh", "api"], cmd
        args = [a for a in cmd[2:] if a != "--paginate"]
        method, path = args[1], args[2].split("repos/o/r", 1)[1].lstrip("/")
        payload = json.loads(stdin) if stdin else None
        out = self.api(method, path, payload)
        return json.dumps(out) if out is not None else ""

    def majordomus(self, args):
        if args[:2] == ["plan", "roadmap"]:
            return json.dumps(ROADMAP)
        if args[:2] == ["plan", "list"]:
            return json.dumps(ISSUES)
        if args[:2] == ["plan", "body"]:
            return f"# {args[2]}\n\nsee `.ai/repo/project/issues/{args[2]}.yaml`\n"
        if args[:2] == ["plan", "show"]:
            return f"# {args[2]}\n\n## Outcome\n\nOutcome of {args[2]}.\n\n## Next\n"
        raise AssertionError(args)

    def api(self, method, path, payload):
        if path == "":
            return {"has_issues": self.has_issues}
        if path.startswith("labels") and method == "GET":
            return list(self.labels.values())
        if path == "labels" and method == "POST":
            self.labels[payload["name"]] = payload
            return payload
        if path.startswith("labels/") and method == "PATCH":
            name = path.split("/", 1)[1].replace("%2F", "/")
            self.labels[name].update(payload)
            return self.labels[name]
        if path.startswith("milestones") and method == "GET":
            return self.milestones
        if path == "milestones" and method == "POST":
            m = dict(payload, number=len(self.milestones) + 1)
            self.milestones.append(m)
            return m
        if path.startswith("milestones/") and method == "PATCH":
            m = next(x for x in self.milestones if x["number"] == int(path.split("/")[1]))
            m.update(payload)
            return m
        if path.startswith("issues") and method == "GET":
            return self.issues
        if path == "issues" and method == "POST":
            i = {"number": 100 + len(self.issues), "state": "open", "title": payload["title"],
                 "body": payload["body"], "labels": [{"name": n} for n in payload["labels"]],
                 "milestone": {"number": payload["milestone"]} if payload["milestone"] else None}
            self.issues.append(i)
            return i
        if path.startswith("issues/") and method == "PATCH":
            i = next(x for x in self.issues if x["number"] == int(path.split("/")[1]))
            for k, v in payload.items():
                if k == "labels":
                    i["labels"] = [{"name": n} for n in v]
                elif k == "milestone":
                    i["milestone"] = {"number": v} if v else None
                elif k != "state_reason":
                    i[k] = v
            return i
        raise AssertionError((method, path))


def run_main(world, argv):
    out = io.StringIO()
    with mock.patch.object(pg, "run", world.run), \
            mock.patch.object(pg, "repo_root", lambda: Path("/tmp/repo")), \
            mock.patch.object(pg, "repository_from_project", lambda root: "o/r"), \
            mock.patch.object(pg, "default_branch", lambda root: "master"), \
            mock.patch.object(sys, "argv", ["plan_github.py", *argv]), \
            contextlib.redirect_stdout(out):
        pg.main()
    return out.getvalue()


class PureFunctions(unittest.TestCase):
    def test_marker_round_trips(self):
        path = ".ai/repo/project/issues/I0110.yaml"
        self.assertEqual(pg.MARKER_RE.search("x\n" + pg.marker(path)).group(1), path)

    def test_milestone_number_follows_plan_order(self):
        roadmap = ROADMAP["milestones"]
        self.assertEqual(pg.milestone_number(roadmap, "a-first"), 1)
        self.assertEqual(pg.milestone_number(roadmap, "b-second"), 2)
        with self.assertRaises(SystemExit):
            pg.milestone_number(roadmap, "nope")

    def test_titles_are_bounded(self):
        self.assertEqual(pg.issue_title("I1", "short"), "I1: short")
        self.assertLessEqual(len(pg.issue_title("I1", "x" * 400)), 250)
        self.assertEqual(pg.milestone_title(3, "short"), "M3: short")
        self.assertLessEqual(len(pg.milestone_title(3, "y" * 400)), 120)

    def test_linkify_only_touches_canonical_records(self):
        out = pg.linkify("`.ai/repo/project/issues/I1.yaml` and `tools/gate.sh`", "o/r", "master")
        self.assertIn("(https://github.com/o/r/blob/master/.ai/repo/project/issues/I1.yaml)", out)
        self.assertIn("`tools/gate.sh`", out)

    def test_status_labels_cover_every_status(self):
        self.assertEqual(set(pg.STATUS_LABELS), {"READY", "BLOCKED", "ACTIVE", "VERIFY", "DONE", "CANCELLED"})


class ProcessAndProject(unittest.TestCase):
    def test_run_returns_stdout_and_fails_loudly(self):
        self.assertEqual(pg.run([sys.executable, "-c", "print('hi')"]).strip(), "hi")
        with self.assertRaises(SystemExit) as e:
            pg.run([sys.executable, "-c", "import sys; sys.stderr.write('bad'); sys.exit(3)"])
        self.assertIn("failed (3): bad", str(e.exception))

    def test_majordomus_and_repo_root_go_through_run(self):
        with mock.patch.object(pg, "run", lambda cmd, stdin=None: " ".join(cmd) + "\n"):
            self.assertEqual(pg.majordomus("plan", "list"), "majordomus plan list\n")
            self.assertEqual(pg.repo_root(), Path("git rev-parse --show-toplevel"))

    def test_project_yaml_parsing(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / ".ai/repo/project").mkdir(parents=True)
            project = root / ".ai/repo/project/project.yaml"
            project.write_text("name: x\nrepository: 'owner/name'\ndefault_branch: trunk\n")
            self.assertEqual(pg.repository_from_project(root), "owner/name")
            self.assertEqual(pg.default_branch(root), "trunk")
            project.write_text("name: x\n")
            with self.assertRaises(SystemExit):
                pg.repository_from_project(root)
            self.assertEqual(pg.default_branch(root), "main")


class Bodies(unittest.TestCase):
    def test_issue_body_links_marks_and_truncates(self):
        with mock.patch.object(pg, "majordomus", lambda *a: "see `.ai/repo/project/issues/I1.yaml`"):
            body = pg.issue_body("I1", ".ai/repo/project/issues/I1.yaml", "o/r", "master")
        self.assertIn(pg.marker(".ai/repo/project/issues/I1.yaml"), body)
        self.assertIn("blob/master/.ai/repo/project/issues/I1.yaml", body)
        with mock.patch.object(pg, "majordomus", lambda *a: "z" * (pg.BODY_LIMIT + 10)):
            body = pg.issue_body("I1", "p.yaml", "o/r", "master")
        self.assertIn("_(truncated; see the record)_", body)
        self.assertLessEqual(len(body), pg.BODY_LIMIT)

    def test_milestone_description_short_long_and_missing_outcome(self):
        with mock.patch.object(pg, "majordomus", lambda *a: "## Outcome\n\nDone well.\n\n## Next\n"):
            self.assertTrue(pg.milestone_description("m", "p.yaml").startswith("Done well."))
        with mock.patch.object(pg, "majordomus", lambda *a: "## Outcome\n\n" + "w" * 2000):
            text = pg.milestone_description("m", "p.yaml")
        self.assertLessEqual(len(text), 1000)
        self.assertIn(pg.marker("p.yaml"), text)
        with mock.patch.object(pg, "majordomus", lambda *a: "no outcome section"):
            self.assertTrue(pg.milestone_description("m", "p.yaml").startswith("\n\nCanonical: p.yaml"))


class Sync(unittest.TestCase):
    def test_paginated_api_joins_pages(self):
        gh = pg.GitHub("o/r", dry_run=False)
        with mock.patch.object(pg, "run", lambda cmd, stdin=None: '[{"a": 1}]\n[{"a": 2}]'):
            self.assertEqual(gh.api("GET", "labels", paginate=True), [{"a": 1}, {"a": 2}])
        with mock.patch.object(pg, "run", lambda cmd, stdin=None: ""):
            self.assertIsNone(gh.api("DELETE", "labels/x"))

    def test_dry_run_writes_nothing(self):
        world = FakeWorld()
        out = run_main(world, ["--milestone", "b-second", "--dry-run"])
        self.assertIn("checked (dry run)", out)
        self.assertEqual(world.issues, [])
        self.assertEqual(world.milestones, [])
        self.assertIn("DRY POST  issue I0002: Two", out)

    def test_first_run_creates_then_rerun_is_a_no_op(self):
        world = FakeWorld()
        run_main(world, ["--all"])
        self.assertEqual([m["title"] for m in world.milestones], ["M2: Second", "M1: First"])
        self.assertEqual(world.milestones[1]["state"], "closed")
        by_title = {i["title"]: i for i in world.issues}
        self.assertEqual(by_title["I0001: One"]["state"], "closed")
        self.assertEqual(by_title["I0002: Two"]["state"], "open")
        self.assertEqual(sorted(l["name"] for l in by_title["I0003: Three"]["labels"]),
                         ["plan", "plan/blocked", "priority/p3"])
        self.assertEqual(world.labels["plan"]["color"], "1d76db")
        before = len(world.calls)
        out = run_main(world, ["--all"])
        writes = [c for c in world.calls[before:] if c[:4] != ["gh", "api", "-X", "GET"] and c[:5] != ["gh", "api", "--paginate", "-X", "GET"] and c[0] == "gh"]
        self.assertEqual(writes, [], out)

    def test_changes_on_github_are_reconciled(self):
        world = FakeWorld()
        run_main(world, ["--all"])
        world.milestones[0]["title"] = "renamed by hand"
        two = next(i for i in world.issues if i["title"] == "I0002: Two")
        two["labels"] = [{"name": "plan"}]
        two["state"] = "closed"
        two["body"] = "edited"
        world.labels["plan/ready"]["color"] = "ffffff"
        world.issues.append({"number": 999, "pull_request": {}, "body": pg.marker(".ai/repo/project/issues/I0002.yaml")})
        out = run_main(world, ["--milestone", "b-second"])
        self.assertEqual(world.milestones[0]["title"], "M2: Second")
        self.assertEqual(two["state"], "open")
        self.assertEqual(sorted(l["name"] for l in two["labels"]), ["plan", "plan/ready", "priority/p2"])
        self.assertIn("PATCH #", out)
        self.assertEqual(world.labels["plan/ready"]["color"], "0e8a16")
        self.assertEqual(len([i for i in world.issues if i.get("title") == "I0002: Two"]), 1, "no duplicate")
        self.assertIn(pg.marker(".ai/repo/project/issues/I0002.yaml"), two["body"])

    def test_a_milestone_that_lost_its_marker_is_found_by_title(self):
        world = FakeWorld()
        run_main(world, ["--milestone", "b-second"])
        world.milestones[0]["description"] = "edited by hand"
        run_main(world, ["--milestone", "b-second"])
        self.assertEqual(len(world.milestones), 1)
        self.assertIn(pg.marker(".ai/repo/project/milestones/b-second.yaml"), world.milestones[0]["description"])

    def test_argument_and_repository_errors(self):
        with self.assertRaises(SystemExit):
            with contextlib.redirect_stderr(io.StringIO()):
                run_main(FakeWorld(), [])
        with self.assertRaises(SystemExit) as e:
            run_main(FakeWorld(), ["--milestone", "missing"])
        self.assertIn("unknown milestone(s): missing", str(e.exception))
        with self.assertRaises(SystemExit) as e:
            run_main(FakeWorld(has_issues=False), ["--all"])
        self.assertIn("has issues disabled", str(e.exception))

    def test_explicit_repo_flag_wins(self):
        world = FakeWorld()
        with mock.patch.object(pg, "repository_from_project", side_effect=AssertionError("not called")):
            out = io.StringIO()
            with mock.patch.object(pg, "run", world.run), \
                    mock.patch.object(pg, "repo_root", lambda: Path("/tmp/repo")), \
                    mock.patch.object(pg, "default_branch", lambda root: "master"), \
                    mock.patch.object(sys, "argv", ["x", "--all", "--repo", "o/r", "--dry-run"]), \
                    contextlib.redirect_stdout(out):
                pg.main()
        self.assertIn("o/r:", out.getvalue())


def executable_lines(module):
    lines = set()

    def walk(code):
        for _, _, line in code.co_lines():
            if line is not None:
                lines.add(line)
        for const in code.co_consts:
            if isinstance(const, types.CodeType):
                walk(const)
    walk(compile(Path(module.__file__).read_text(), module.__file__, "exec"))
    return lines


def main():
    if "--coverage" not in sys.argv:
        unittest.main()
        return
    sys.argv.remove("--coverage")
    target = str(Path(pg.__file__).resolve())
    hit = set()
    tool = sys.monitoring.COVERAGE_ID
    sys.monitoring.use_tool_id(tool, "plan-github-tests")

    def on_line(code, line):
        if code.co_filename == target:
            hit.add(line)
        return sys.monitoring.DISABLE
    sys.monitoring.register_callback(tool, sys.monitoring.events.LINE, on_line)
    sys.monitoring.set_events(tool, sys.monitoring.events.LINE)
    import importlib
    importlib.reload(pg)
    result = unittest.main(exit=False).result
    sys.monitoring.set_events(tool, 0)
    source = Path(target).read_text().splitlines()
    guard = {i + 1 for i, text in enumerate(source) if re.match(r"\s*(if __name__ ==|sys\.exit\(main\(\)\))", text)}
    missed = sorted(executable_lines(pg) - hit - guard - {0})  # line 0: the module's own entry
    if missed:
        print(f"UNCOVERED {target}: lines {missed}")
        sys.exit(1)
    print(f"coverage: every executable line of {Path(target).name} ran ({len(hit)} lines)")
    sys.exit(0 if result.wasSuccessful() else 1)


if __name__ == "__main__":
    main()
