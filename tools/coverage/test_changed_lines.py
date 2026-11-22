"""Tests of tools/coverage/changed-lines, the changed-line coverage gate (rule
project.new-code-tested): its diff parsing, base, discovery, exemptions and tracer, and, in
scratch repositories, the change it must refuse and the ones it must let through.

    python3 -B -m unittest discover -s tools/coverage -p 'test_*.py'
"""

from __future__ import annotations

import atexit
import importlib.machinery
import importlib.util
import json
import os
import runpy
import shutil
import subprocess
import sys
import tempfile
import textwrap
import threading
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "changed-lines"
HOOK = HERE / "hook"

_loader = importlib.machinery.SourceFileLoader("changed_lines", str(SCRIPT))
_spec = importlib.util.spec_from_loader("changed_lines", _loader)
cl = importlib.util.module_from_spec(_spec)
_loader.exec_module(cl)

if str(HOOK) not in sys.path:
    sys.path.insert(0, str(HOOK))
import _changed_lines_trace as trace  # noqa: E402

# The gate's own run in CI sets COVERAGE_BASE to the pull request's base, a commit no scratch
# repository here holds: every test chooses its base itself.
os.environ.pop("COVERAGE_BASE", None)

GIT_ENV = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t",
           "GIT_COMMITTER_EMAIL": "t@t", "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}


def git(root: Path, *args: str) -> str:
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update(GIT_ENV)
    return subprocess.run(["git", *args], cwd=root, env=env, check=True, capture_output=True,
                          text=True).stdout


FOO = '''\
def add(a, b):
    return a + b
'''
TEST_FOO = '''\
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("foo", Path(__file__).parent / "foo.py")
foo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(foo)


class FooTest(unittest.TestCase):
    def test_add(self):
        self.assertEqual(foo.add(1, 2), 3)
'''


class Scratch(unittest.TestCase):
    """A scratch repository: tools/x/foo.py and its test, committed as the base."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="changed-lines-test-")
        self.root = Path(self.tmp.name).resolve()
        git(self.root, "init", "-q", "-b", "master")
        self.write("tools/x/foo.py", FOO)
        self.write("tools/x/test_foo.py", TEST_FOO)
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "base")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def write(self, rel: str, text: str) -> Path:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(text), encoding="utf-8")
        return path

    def gate(self, *args: str) -> subprocess.CompletedProcess:
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        env.pop("COVERAGE_BASE", None)
        return subprocess.run([sys.executable, "-B", str(SCRIPT), "--repo", str(self.root), *args],
                              env=env, capture_output=True, text=True)


class RefusesAndAccepts(Scratch):
    def test_refuses_a_changed_line_no_test_runs(self) -> None:
        # The proof: a function added without a test is refused, by file and line.
        self.write("tools/x/foo.py", FOO + "\n\ndef sub(a, b):\n    return a - b\n")
        out = self.gate()
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("missing tools/x/foo.py:6: no test runs this line: return a - b", out.stdout)
        self.assertIn("changed-lines: FAIL", out.stdout)
        self.assertNotIn("foo.py:5:", out.stdout)  # the def line ran: the module was imported

    def test_accepts_once_a_test_runs_it(self) -> None:
        self.write("tools/x/foo.py", FOO + "\n\ndef sub(a, b):\n    return a - b\n")
        self.write("tools/x/test_foo.py", TEST_FOO + "\n    def test_sub(self):\n"
                   "        self.assertEqual(foo.sub(3, 2), 1)\n")
        out = self.gate()
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
        self.assertIn("changed-lines: PASS", out.stdout)
        self.assertIn("tests changed (the tests themselves are not judged): tools/x/test_foo.py",
                      out.stdout)

    def test_a_line_run_in_a_subprocess_counts(self) -> None:
        self.write("tools/x/foo.py", FOO + "\n\nif __name__ == '__main__':\n    print(add(1, 1))\n")
        self.write("tools/x/test_foo.py", TEST_FOO + textwrap.indent(textwrap.dedent('''
            def test_main(self):
                import subprocess, sys
                out = subprocess.run([sys.executable, str(Path(__file__).parent / "foo.py")],
                                     capture_output=True, text=True, check=True)
                self.assertEqual(out.stdout, "2\\n")
                # No run leaves __pycache__/ behind, the hook's own sitecustomize.pyc included.
                import os
                self.assertEqual(os.environ.get("PYTHONDONTWRITEBYTECODE"), "1")
            '''), "    "))
        out = self.gate()
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)

    def test_a_scratch_copy_with_the_same_bytes_counts(self) -> None:
        self.write("tools/x/foo.py", FOO + "\n\nif __name__ == '__main__':\n    print(add(2, 2))\n")
        self.write("tools/x/test_foo.py", TEST_FOO + textwrap.indent(textwrap.dedent('''
            def test_copy(self):
                import shutil, subprocess, sys, tempfile
                with tempfile.TemporaryDirectory() as tmp:
                    dst = Path(tmp) / "tools" / "x"
                    dst.mkdir(parents=True)
                    shutil.copy(Path(__file__).parent / "foo.py", dst / "foo.py")
                    out = subprocess.run([sys.executable, str(dst / "foo.py")],
                                         capture_output=True, text=True, check=True)
                self.assertEqual(out.stdout, "4\\n")
            '''), "    "))
        out = self.gate()
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)

    def test_a_failing_suite_fails_the_gate(self) -> None:
        self.write("tools/x/test_foo.py", TEST_FOO + "\n    def test_broken(self):\n"
                   "        self.fail('broken')\n")
        out = self.gate()
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("tests   tools/x: FAILED", out.stdout)
        self.assertIn("problem tools/x: the test suite failed", out.stdout)

    def test_a_shebang_tool_without_extension_is_judged(self) -> None:
        tool = self.write("tools/x/tool", "#!/usr/bin/env python3\nprint('never run')\n")
        tool.chmod(0o755)
        out = self.gate()
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("missing tools/x/tool:2: no test runs this line: print('never run')",
                      out.stdout)

    def test_other_languages_are_named_and_never_refused(self) -> None:
        self.write("src/a.cpp", "int f() { return 1; }\n")
        self.write("tools/x/b.rs", "fn main() {}\n")
        self.write("tools/x/c.mjs", "export const c = 1;\n")
        self.write("tools/x/run", "#!/bin/sh\necho hi\n")
        self.write("docs/d.md", "# doc\n")
        out = self.gate()
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
        self.assertIn("not judged (C++: no changed-line coverage for it yet, never refused): "
                      "1 file(s): src/a.cpp", out.stdout)
        self.assertIn("not judged (Rust:", out.stdout)
        self.assertIn("not judged (JavaScript:", out.stdout)
        self.assertIn("not judged (shell:", out.stdout)
        self.assertNotIn("d.md", out.stdout)

    def test_a_file_that_does_not_compile_is_refused(self) -> None:
        self.write("tools/x/foo.py", FOO + "def broken(:\n")
        out = self.gate("--no-run", "--data", str(self.root / "none"))
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("problem tools/x/foo.py:3: does not compile", out.stdout)

    def test_explicit_base_judges_committed_lines(self) -> None:
        base = git(self.root, "rev-parse", "HEAD").strip()
        self.write("tools/x/foo.py", FOO + "\n\ndef sub(a, b):\n    return a - b\n")
        git(self.root, "commit", "-qam", "sub")
        out = self.gate("--base", base)
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("tools/x/foo.py:6:", out.stdout)
        # Clean tree on HEAD, no remote: the base is HEAD's parent, the same commit.
        out = self.gate()
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn(f"since {base[:12]}", out.stdout)

    def test_no_base_is_a_usage_error(self) -> None:
        out = self.gate("--base", "no-such-ref")
        self.assertEqual(out.returncode, 2)
        self.assertIn("is not a commit", out.stderr)
        out = self.gate("--no-run")
        self.assertEqual(out.returncode, 2)
        self.assertIn("--no-run judges existing readings", out.stderr)

    def test_list_tests(self) -> None:
        out = self.gate("--list-tests")
        self.assertEqual(out.stdout.split(), ["tools/x"])


class Exemptions(Scratch):
    UNTESTED = FOO + "\n\ndef sub(a, b):\n    return a - b\n"

    def test_an_exemption_with_a_reason_is_printed_and_lets_the_line_pass(self) -> None:
        self.write("tools/x/foo.py", self.UNTESTED)
        self.write("tools/coverage/exemptions.toml", '''\
            [[exempt]]
            path = "tools/x/foo.py"
            function = "sub"
            reason = "a fixture's exemption"
            ''')
        out = self.gate()
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
        self.assertIn("exempt  tools/x/foo.py function sub: a fixture's exemption "
                      "(1 changed unrun line(s) here)", out.stdout)
        self.assertIn("1 exempt with a reason", out.stdout)

    def test_an_exemption_without_a_reason_is_refused(self) -> None:
        self.write("tools/x/foo.py", self.UNTESTED)
        self.write("tools/coverage/exemptions.toml", '[[exempt]]\npath = "tools/x/foo.py"\n'
                   'match = "a - b"\nreason = "  "\n')
        out = self.gate()
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("no reason: every exemption says why", out.stdout)
        self.assertIn("tools/x/foo.py:6: no test runs this line", out.stdout)

    def test_an_exemption_that_matches_nothing_is_refused(self) -> None:
        self.write("tools/coverage/exemptions.toml", '[[exempt]]\npath = "tools/gone.py"\n'
                   'reason = "gone"\n')
        out = self.gate()
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("matches nothing in the repository: remove it", out.stdout)

    def test_a_malformed_file_is_refused(self) -> None:
        self.write("tools/coverage/exemptions.toml", "[[exempt]\n")
        out = self.gate()
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("problem exemptions.toml:", out.stdout)

    def test_entry_problems(self) -> None:
        e = cl.Exemption({"path": "a.py", "reason": "r", "function": "f", "match": "m",
                          "lines": [1]}, 1)
        self.assertEqual(e.problems(), ["unknown key(s) lines", "both function and match: name one"])
        self.assertEqual(cl.Exemption({"reason": 3}, 2).problems(), [
            "no path", "no reason: every exemption says why its lines cannot run under a test"])
        self.assertEqual(cl.Exemption({"path": "a.py", "match": "x", "reason": "r"}, 3).label(),
                         "a.py match 'x'")

    def test_entry_lines(self) -> None:
        src = "import os\n\nclass A:\n    @staticmethod\n    def f():\n        return 1\n"
        self.assertEqual(cl.Exemption({"path": "*.py", "reason": "r"}, 1).lines("b.py", src),
                         set(range(1, 8)))
        self.assertEqual(cl.Exemption({"path": "a.py", "reason": "r"}, 1).lines("b.py", src), set())
        self.assertEqual(cl.Exemption({"path": "*.py", "function": "A.f", "reason": "r"}, 1)
                         .lines("b.py", src), {4, 5, 6})
        self.assertEqual(cl.Exemption({"path": "*.py", "function": "f", "reason": "r"}, 1)
                         .lines("b.py", src), set())
        self.assertEqual(cl.Exemption({"path": "*.py", "match": "return", "reason": "r"}, 1)
                         .lines("b.py", src), {6})

    def test_without_tomllib_the_file_is_refused(self) -> None:
        self.write("tools/coverage/exemptions.toml", "")
        with mock.patch.dict(sys.modules, {"tomllib": None}):
            self.assertEqual(cl.load_exemptions(self.root / "tools/coverage/exemptions.toml"),
                             ([], ["exemptions.toml: reading it needs Python 3.11+ (tomllib)"]))

    def test_a_missing_file_is_no_exemption(self) -> None:
        self.assertEqual(cl.load_exemptions(self.root / "none.toml"), ([], []))

    def test_stale_skips_bad_entries_and_unreadable_files(self) -> None:
        (self.root / "bin.py").write_bytes(b"\xff\xfe")
        self.write("bad.py", "def (:\n")
        entries = [cl.Exemption({"path": "bin.py", "reason": "r"}, 1),
                   cl.Exemption({"path": "bad.py", "function": "f", "reason": "r"}, 2),
                   cl.Exemption({"path": "x.py"}, 3)]
        self.assertEqual(cl.stale_exemptions(self.root, entries), [
            "exemption 1 (bin.py) matches nothing in the repository: remove it",
            "exemption 2 (bad.py function f) matches nothing in the repository: remove it"])


class Base(Scratch):
    def test_explicit_and_environment(self) -> None:
        head = git(self.root, "rev-parse", "HEAD").strip()
        self.assertEqual(cl.resolve_base(self.root, "HEAD"), head)
        with mock.patch.dict(os.environ, {"COVERAGE_BASE": "HEAD"}):
            self.assertEqual(cl.resolve_base(self.root, None), head)
        with self.assertRaises(cl.Refusal):
            cl.resolve_base(self.root, "nope")

    def test_merge_base_with_the_remote(self) -> None:
        head = git(self.root, "rev-parse", "HEAD").strip()
        git(self.root, "update-ref", "refs/remotes/origin/master", head)
        self.write("tools/x/foo.py", FOO + "# more\n")
        git(self.root, "commit", "-qam", "more")
        self.assertEqual(cl.default_remote_branch(self.root), "origin/master")
        self.assertEqual(cl.resolve_base(self.root, None), head)
        git(self.root, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/master")
        self.assertEqual(cl.default_remote_branch(self.root), "origin/master")

    def test_on_the_trunk(self) -> None:
        head = git(self.root, "rev-parse", "HEAD").strip()
        with self.assertRaises(cl.Refusal):  # a root commit, clean tree: nothing to measure from
            cl.resolve_base(self.root, None)
        self.write("tools/x/new.py", "x = 1\n")
        self.assertEqual(cl.resolve_base(self.root, None), head)  # the working tree is the change

    def test_an_untracked_binary_file_has_no_lines(self) -> None:
        (self.root / "blob.bin").write_bytes(b"\xff\xfe")
        self.write("tools/x/new.py", "a = 1\nb = 2\n")
        head = git(self.root, "rev-parse", "HEAD").strip()
        lines, files = cl.changed_lines(self.root, head)
        self.assertEqual(lines, {"tools/x/new.py": {1, 2, 3}})
        self.assertEqual(files, ["blob.bin", "tools/x/new.py"])

    def test_a_tracked_name_with_a_space(self) -> None:
        self.write("tools/x/my tool.py", "a = 1\n")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-qm", "space")
        head = git(self.root, "rev-parse", "HEAD").strip()
        self.write("tools/x/my tool.py", "a = 2\n")
        self.assertEqual(cl.changed_lines(self.root, head), ({"tools/x/my tool.py": {1}},
                                                             ["tools/x/my tool.py"]))

    def test_no_head(self) -> None:
        empty = self.root / "empty"
        empty.mkdir()
        git(empty, "init", "-q")
        self.assertIsNone(cl.default_remote_branch(empty))
        with self.assertRaises(cl.Refusal):
            cl.resolve_base(empty, None)
        with self.assertRaises(cl.Refusal):
            cl.git(empty, "rev-parse", "HEAD")


class Parsing(unittest.TestCase):
    def test_parse_diff(self) -> None:
        diff = textwrap.dedent("""\
            diff --git a/a.py b/a.py
            --- a/a.py
            +++ b/a.py
            @@ -1,0 +2,3 @@ x
            +a
            @@ -9 +12 @@
            @@ -20,2 +22,0 @@
            diff --git a/gone.py b/gone.py
            --- a/gone.py
            +++ /dev/null
            @@ -1,2 +0,0 @@
            """)
        self.assertEqual(cl.parse_diff(diff), {"a.py": {2, 3, 4, 12}})

    def test_language(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "py").write_text("#!/usr/bin/env python3.12\n")
            (root / "sh").write_text("#!/usr/bin/env bash\n")
            (root / "data").write_text("plain\n")
            (root / "bin").write_bytes(b"\xff\xfe")
            self.assertEqual(cl.language(root, "py"), "Python")
            self.assertEqual(cl.language(root, "sh"), "shell")
            self.assertIsNone(cl.language(root, "data"))
            self.assertIsNone(cl.language(root, "bin"))
            self.assertIsNone(cl.language(root, "missing"))
            self.assertEqual(cl.language(root, "a.py"), "Python")
            self.assertEqual(cl.language(root, "a.hpp"), "C++")
            self.assertIsNone(cl.language(root, "src/3rdparty/x.py"))
            self.assertIsNone(cl.language(root, "a.md"))

    def test_executable(self) -> None:
        src = '"""doc."""\nx = (1 +\n     2)\n\n\ndef f():\n    # c\n    return x\n'
        # (1 +\n 2) folds into one constant on line 2; comments and blank lines are never judged.
        self.assertEqual(cl.executable(src, "s.py"), {1, 2, 6, 8})

    def test_test_dirs_skips_what_an_ancestor_discovers(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git(root, "init", "-q")
            for rel in ("a/test_a.py", "a/pkg/__init__.py", "a/pkg/test_b.py", "c/test_c.py",
                        "c/d/test_d.py", "src/3rdparty/test_v.py", "e/__init__.py", "e/test_e.py"):
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                (root / rel).write_text("")
            self.assertEqual(cl.test_dirs(root), ["a", "c", "c/d", "e"])

    def test_load_readings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "repo"
            (root / "t").mkdir(parents=True)
            (root / "t" / "a.py").write_text("x = 1\n")
            (root / "t" / "b.py").write_text("y = 1\n")
            sha = cl.sha256(root / "t" / "a.py")
            data = Path(tmp) / "data"
            data.mkdir()
            real = os.path.realpath(root)
            (data / "1.json").write_text(json.dumps({"files": {
                f"{real}/t/a.py": {"sha256": sha, "lines": [1]},
                f"{real}/t/other.py": {"sha256": "x", "lines": [4]},
                f"{real}/t/b.py": {"sha256": "edited since", "lines": [1]},
                "/scratch/t/a.py": {"sha256": sha, "lines": [2]},
                "/scratch2/t/a.py": {"sha256": "changed", "lines": [3]}}}))
            (data / "2.json").write_text("{trunc")
            ran, stale = cl.load_readings(root, data, {"t/a.py", "t/b.py"})
            self.assertEqual(ran, {"t/a.py": {1, 2}, "t/other.py": {4}})
            self.assertEqual(stale, {"t/b.py"})


class Tracer(unittest.TestCase):
    SAMPLE = "def f(n):\n    if n:\n        return 1\n    return 2\n"

    def run_sample(self, use_monitoring: bool) -> tuple[dict, str]:
        with tempfile.TemporaryDirectory() as tmp:
            sample = Path(tmp) / "sample.py"
            sample.write_text(self.SAMPLE)
            rec = trace.Recorder(str(Path(tmp) / "data"))
            rec.start(use_monitoring)
            atexit.unregister(rec.stop_and_write)
            try:
                ns = runpy.run_path(str(sample))
                ns["f"](1)
                worker = threading.Thread(target=ns["f"], args=(0,))
                worker.start()
                worker.join()
            finally:
                rec.stop()
            rec.stop()  # a second stop does nothing
            path = rec.write()
            reading = json.loads(Path(path).read_text())
        return reading, os.path.realpath(sample)

    @unittest.skipUnless(hasattr(sys, "monitoring"), "sys.monitoring needs Python 3.12+")
    def test_monitoring(self) -> None:
        reading, sample = self.run_sample(True)
        self.assertEqual(reading["mode"], "monitoring")
        self.assertEqual(reading["files"][sample]["lines"], [1, 2, 3, 4])
        self.assertFalse(any(p.startswith(trace._LIBRARY) for p in reading["files"]))

    def test_settrace(self) -> None:
        before = sys.gettrace()
        reading, sample = self.run_sample(False)
        self.assertEqual(reading["mode"], "settrace")
        self.assertEqual(reading["files"][sample]["lines"], [1, 2, 3, 4])
        self.assertIs(sys.gettrace(), before)  # the outer tracer, if any, is back

    def test_default_mode_and_removed_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            rec = trace.start(str(Path(tmp) / "data"))
            atexit.unregister(rec.stop_and_write)
            rec.stop()
            self.assertEqual(rec.mode, "monitoring" if hasattr(sys, "monitoring") else "settrace")
            rec.ran = {"/no/such/file.py": {1}, "<string>": {1}, os.__file__: {1}}
            self.assertEqual(rec.reading()["files"], {})
            rec.stop_and_write()
            self.assertEqual(len(list((Path(tmp) / "data").glob("*.json"))), 1)

    def test_the_module_body(self) -> None:
        # sitecustomize imports the tracer before any tracer exists: run its body again here.
        ns = runpy.run_path(str(HOOK / "_changed_lines_trace.py"))
        self.assertEqual(ns["_LIBRARY"], trace._LIBRARY)

    def test_the_callbacks(self) -> None:
        # The interpreter never traces its own tracing callbacks: call them as it would.
        rec = trace.Recorder("unused")
        frame = mock.Mock(f_code=mock.Mock(co_filename=__file__), f_lineno=12)
        if hasattr(sys, "monitoring"):
            self.assertIs(rec.on_line(frame.f_code, 7), sys.monitoring.DISABLE)
        else:
            rec.ran[__file__] = {7}
        self.assertEqual(rec.tracer(frame, "call", None), rec.local)
        self.assertEqual(rec.local(frame, "line", None), rec.local)
        self.assertEqual(rec.local(mock.Mock(f_code=frame.f_code, f_lineno=13), "return", None),
                         rec.local)
        self.assertEqual(rec.ran[__file__], {7, 12})
        os_frame = mock.Mock(f_code=mock.Mock(co_filename=os.__file__))
        self.assertIsNone(rec.tracer(os_frame, "call", None))

    def test_wanted(self) -> None:
        self.assertFalse(trace.wanted(""))
        self.assertFalse(trace.wanted("<frozen os>"))
        self.assertFalse(trace.wanted(os.__file__))
        self.assertTrue(trace.wanted(__file__))

    @unittest.skipUnless(hasattr(sys, "monitoring"), "sys.monitoring needs Python 3.12+")
    def test_every_tool_id_taken(self) -> None:
        rec = trace.Recorder("unused")
        with mock.patch.object(sys.monitoring, "get_tool", return_value="other"):
            with self.assertRaises(RuntimeError):
                rec.start(True)

    def test_sitecustomize_starts_the_tracer_only_when_asked(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.dict(os.environ, {"CHANGED_LINES_DATA": ""}):
                self.assertIsNone(runpy.run_path(str(HOOK / "sitecustomize.py"))["recorder"])
            with mock.patch.dict(os.environ, {"CHANGED_LINES_DATA": tmp}):
                rec = runpy.run_path(str(HOOK / "sitecustomize.py"))["recorder"]
            atexit.unregister(rec.stop_and_write)
            rec.stop()
            self.assertEqual(rec.data_dir, tmp)


class OwnRepository(unittest.TestCase):
    def test_discovers_its_own_suite(self) -> None:
        out = subprocess.run([sys.executable, "-B", str(SCRIPT), "--repo", str(HERE),
                              "--list-tests"], capture_output=True, text=True, check=True)
        self.assertIn("tools/coverage", out.stdout.split())

    def test_main_judges_existing_readings(self) -> None:
        # In-process, so the argument handling runs under the tracer of an outer gate run too.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git(root, "init", "-q", "-b", "master")
            (root / "a.py").write_text("x = 1\n")
            git(root, "add", "-A")
            git(root, "commit", "-qm", "a")
            (root / "a.py").write_text("x = 2\n")
            data = root / "data"
            data.mkdir()
            (data / "r.json").write_text(json.dumps({"files": {
                f"{os.path.realpath(root)}/a.py": {"sha256": cl.sha256(root / "a.py"),
                                                   "lines": [1]}}}))
            with mock.patch("sys.stdout"), mock.patch("sys.stderr"):
                self.assertEqual(cl.main(["--repo", str(root), "--no-run", "--data", str(data)]), 0)
                self.assertEqual(cl.main(["--repo", str(root), "--base", "nope"]), 2)
                self.assertEqual(cl.main(["--repo", str(root), "--list-tests"]), 0)
                (root / "test_a.py").write_text("import unittest\n\n\nclass T(unittest.TestCase):\n"
                                                "    def test_t(self):\n        pass\n")
                self.assertEqual(cl.main(["--repo", str(root), "--data", str(data)]), 0)
                (data / "s.json").write_text(json.dumps({"files": {
                    f"{os.path.realpath(root)}/a.py": {"sha256": "older bytes", "lines": [1]}}}))
                self.assertEqual(cl.main(["--repo", str(root), "--no-run", "--data", str(data)]), 1)
                (data / "s.json").unlink()
                with mock.patch.object(cl, "test_dirs", return_value=[]):
                    # A temporary readings directory and no test: a.py:1 ran nowhere.
                    self.assertEqual(cl.main(["--repo", str(root)]), 1)


if __name__ == "__main__":
    unittest.main()
