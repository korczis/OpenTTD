"""tools/surfaces/refresh-next and the Surfaces refresh step: a push to master refreshes one branch.

Every path of the script runs here against fixture `gh pr list` JSON: the candidates and their
order (--list), the first candidate refreshed, branches passed over as current, conflicting or
moved, no candidate needing a refresh, an empty list, input that is not JSON, and the usage paths.
The `run:` block of the workflow's "Merge master into the branch and regenerate" step runs too,
read from .github/workflows/surfaces.yml and executed as GitHub does (bash -eo pipefail) with a
fake `gh` on PATH: the pull-request path, the push path, and that neither dispatches Research Rust.

tools/surfaces/refresh-branch is replaced by a stub that logs its arguments and exits with the
code a table gives each branch, so the test exercises the selection, not the merge (that is
test_refresh_branch.py). Both scripts run under bash -x with the line number in PS4, and once
every test has run the module fails naming any line of either that none of them ran (owner order
2026-11-22: every changed line is run by a test). Run by the Surfaces workflow:
python3 -m unittest tools/surfaces/test_refresh_next.py
"""
import json
import os
import pathlib
import re
import shutil
import subprocess
import tempfile
import textwrap
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "refresh-next"
WORKFLOW = HERE.parent.parent / ".github/workflows/surfaces.yml"
STEP_NAME = "Merge master into the branch and regenerate"
STUB = """#!/usr/bin/env bash
# Stand-in for refresh-branch: log the call, exit with the code the table gives the branch.
echo "$*" >> "$(dirname "$0")/calls"
echo "refresh-branch: stub for ${@: -1}"
code=$(awk -F= -v b="${@: -1}" '$1 == b { print $2 }' "$(dirname "$0")/codes")
exit "${code:-0}"
"""
FAKE_GH = """#!/usr/bin/env bash
# Stand-in for gh: log the call; `pr list` prints the fixture, or fails when gh-fails exists.
echo "$*" >> "$FAKE_DIR/gh-calls"
if [ "$1 $2" = "pr list" ]; then
  [ ! -e "$FAKE_DIR/gh-fails" ] || exit 1
  cat "$FAKE_DIR/prs.json"
fi
"""
STRUCTURAL = re.compile(r"^\s*(esac|done|fi|else)\b")
HIT = {"refresh-next": set(), "step": set()}
RAN = set()


def pr(number, branch, auto=True, cross=False):
    return {"number": number, "headRefName": branch, "isCrossRepository": cross,
            "autoMergeRequest": {"mergeMethod": "MERGE"} if auto else None}


# Unordered on purpose; one fork's pull request and one without auto-merge are never candidates.
PRS = [pr(75, "ci/c"), pr(62, "plan/a"), pr(70, "fix/manual", auto=False),
       pr(64, "fork/x", cross=True), pr(69, "feature/b")]


def step_script():
    """The step's `run: |` block, dedented, and the workflow line its first line sits on."""
    lines = WORKFLOW.read_text().splitlines()
    at = next(i for i, text in enumerate(lines) if text.strip() == f"- name: {STEP_NAME}")
    run = next(i for i in range(at, len(lines)) if lines[i].strip() == "run: |")
    body = []
    for text in lines[run + 1:]:
        if text.strip() and not text.startswith(" " * 10):
            break
        body.append(text)
    while body and not body[-1].strip():
        body.pop()
    return textwrap.dedent("\n".join(body)) + "\n", run + 2


def executable(source):
    """Lines a trace can name: no blank, comment or closing keyword, no continued line (bash
    credits a statement continued with a backslash to its last line)."""
    return {i for i, text in enumerate(source.splitlines(), 1)
            if text.strip() and not text.lstrip().startswith("#")
            and not STRUCTURAL.match(text) and not text.rstrip().endswith("\\")}


class Scratch(unittest.TestCase):
    def setUp(self):
        RAN.add(f"{type(self).__name__}.{self._testMethodName}")
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.dir = self.tmp / "tools/surfaces"
        self.dir.mkdir(parents=True)
        shutil.copy(SCRIPT, self.dir / "refresh-next")
        stub = self.dir / "refresh-branch"
        stub.write_text(STUB)
        stub.chmod(0o755)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def traced(self, name, argv, stdin="", codes=None, env=None):
        (self.dir / "codes").write_text("".join(f"{b}={c}\n" for b, c in (codes or {}).items()))
        full = dict(os.environ, PS4=f"+{name}:${{LINENO}}: ", **(env or {}))
        r = subprocess.run(argv, cwd=self.tmp, env=full, input=stdin, capture_output=True,
                           text=True)
        HIT[name].update(int(n) for n in re.findall(rf"^\++{name}:(\d+): ", r.stderr, re.M))
        return r

    def calls(self, name="calls"):
        path = (self.dir if name == "calls" else self.tmp) / name
        return path.read_text().splitlines() if path.exists() else []


class RefreshNext(Scratch):
    def run_script(self, *args, stdin="[]", codes=None):
        return self.traced("refresh-next", ["bash", "-x", str(self.dir / "refresh-next"), *args],
                           stdin=stdin, codes=codes)

    def test_list_orders_same_repository_auto_merge_pull_requests_by_number(self):
        r = self.run_script("--list", stdin=json.dumps(PRS))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout, "plan/a\nfeature/b\nci/c\n")
        self.assertEqual(self.calls(), [], "--list refreshes nothing")

    def test_list_of_no_candidates_prints_nothing(self):
        r = self.run_script("--list", stdin=json.dumps([pr(1, "x", auto=False)]))
        self.assertEqual((r.returncode, r.stdout), (0, ""), r.stderr)

    def test_the_lowest_numbered_candidate_is_refreshed_and_no_other(self):
        r = self.run_script("--push", stdin=json.dumps(PRS), codes={"plan/a": 3, "feature/b": 3})
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertEqual(r.stdout, "plan/a\n", "the branch name is the only line on stdout")
        self.assertIn("refresh-branch: stub for plan/a", r.stderr)
        self.assertEqual(self.calls(), ["--push plan/a"])

    def test_current_conflicting_and_moved_branches_are_passed_over(self):
        prs = PRS + [pr(80, "fix/d")]
        codes = {"plan/a": 0, "feature/b": 10, "ci/c": 1, "fix/d": 3}
        r = self.run_script(stdin=json.dumps(prs), codes=codes)
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertEqual(r.stdout, "fix/d\n")
        self.assertEqual(self.calls(), ["plan/a", "feature/b", "ci/c", "fix/d"],
                         "lowest number first, and without --push none is handed on")

    def test_no_candidate_needing_a_refresh_exits_zero(self):
        r = self.run_script("--push", stdin=json.dumps(PRS))
        self.assertEqual((r.returncode, r.stdout), (0, ""), r.stderr)
        self.assertIn("no pull request in line needed a refresh", r.stderr)
        self.assertEqual(len(self.calls()), 3)

    def test_an_empty_list_refreshes_nothing(self):
        r = self.run_script("--push", stdin="[]")
        self.assertEqual((r.returncode, r.stdout), (0, ""), r.stderr)
        self.assertEqual(self.calls(), [])

    def test_input_that_is_not_gh_json_fails(self):
        r = self.run_script("--push", stdin="not json")
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn("stdin is not gh pr list JSON", r.stderr)
        self.assertEqual(self.calls(), [])

    def test_an_unknown_argument_is_a_usage_error(self):
        r = self.run_script("fix/a")
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn("unknown argument fix/a", r.stderr)

    def test_help_prints_the_header(self):
        r = self.run_script("-h")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("refresh the next pull request in line", r.stdout)
        self.assertIn("Exit: 3 a branch was refreshed", r.stdout)


class RefreshStep(Scratch):
    """The workflow step itself, in a scratch git repository with a fake gh."""

    def setUp(self):
        super().setUp()
        subprocess.run(["git", "init", "-q", str(self.tmp)], check=True)
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        (self.tmp / "prs.json").write_text(json.dumps(PRS))
        source, _ = step_script()
        (self.tmp / "step.sh").write_text(source)
        self.path = f"{bin_dir}{os.pathsep}{os.environ['PATH']}"

    def run_step(self, head_ref="", codes=None):
        env = {"PATH": self.path, "FAKE_DIR": str(self.tmp), "HEAD_REF": head_ref,
               "GITHUB_REPOSITORY": "o/r", "GIT_CONFIG_GLOBAL": "/dev/null"}
        argv = ["bash", "--noprofile", "--norc", "-eo", "pipefail", "-x", "step.sh"]
        return self.traced("step", argv, codes=codes, env=env)

    def dispatches(self):
        return [c for c in self.calls("gh-calls") if c.startswith("workflow run")]

    def test_a_push_to_master_refreshes_and_dispatches_one_branch(self):
        r = self.run_step(codes={"plan/a": 3, "feature/b": 3, "ci/c": 3})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.calls(), ["--push plan/a"])
        self.assertEqual(self.dispatches(), ["workflow run surfaces.yml -R o/r --ref plan/a"],
                         "one Surfaces dispatch and no Research Rust one")
        self.assertIn("number,headRefName,isCrossRepository,autoMergeRequest",
                      self.calls("gh-calls")[0])

    def test_a_push_to_master_with_nothing_to_refresh_dispatches_nothing(self):
        r = self.run_step()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(len(self.calls()), 3)
        self.assertEqual(self.dispatches(), [])

    def test_a_failing_gh_pr_list_fails_the_step(self):
        (self.tmp / "gh-fails").write_text("")
        r = self.run_step()
        self.assertNotEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.calls(), [])
        self.assertEqual(self.dispatches(), [])

    def test_a_pull_request_refreshes_its_own_branch(self):
        r = self.run_step(head_ref="feature/b", codes={"feature/b": 3})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.calls(), ["--push feature/b"])
        self.assertEqual(self.calls("gh-calls"),
                         ["workflow run surfaces.yml -R o/r --ref feature/b"],
                         "no pr list, one Surfaces dispatch and no Research Rust one")

    def test_a_pull_request_left_to_its_owner_dispatches_nothing(self):
        r = self.run_step(head_ref="feature/b", codes={"feature/b": 10})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.calls("gh-calls"), [])


def tearDownModule():
    """Once every test above ran, every executable line of both scripts must have run."""
    tests = {f"{c.__name__}.{n}" for c in (RefreshNext, RefreshStep)
             for n in dir(c) if n.startswith("test_")}
    if RAN != tests:
        return  # a filtered run (-k, one test) proves nothing about coverage
    step, first = step_script()
    missed = [f"{SCRIPT.name}: lines {m}" for m in
              [sorted(executable(SCRIPT.read_text()) - HIT["refresh-next"])] if m]
    missed += [f"{WORKFLOW.name} step {STEP_NAME!r}: lines {[first + n - 1 for n in m]}" for m in
               [sorted(executable(step) - HIT["step"])] if m]
    if missed:
        raise AssertionError("UNCOVERED " + "; ".join(missed))


if __name__ == "__main__":
    unittest.main()
