"""Offline tests of tools/research/algoglyph-pin (I0227): every path of status, check, install,
runner and bump, against scratch repositories (an "upstream" AlgoGlyph and a superproject that
pins it as a submodule), never the network and never npm (both are stood in)."""
import contextlib
import importlib.machinery
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "algoglyph-pin"


def load():
    loader = importlib.machinery.SourceFileLoader("algoglyph_pin", str(SCRIPT))
    spec = importlib.util.spec_from_loader("algoglyph_pin", loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


pin = load()
# A submodule from a local path needs the file protocol, which git refuses by default.
GIT_ENV = {"GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "protocol.file.allow", "GIT_CONFIG_VALUE_0": "always",
           "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
KERNELS = [{"id": "knapsack-dp", "engine": "grid"}, {"id": "dijkstra", "engine": "graph"}]


def sh(cwd, *args):
    return subprocess.run(["git", "-C", str(cwd), *args], check=True, capture_output=True, text=True).stdout.strip()


def commit(repo, name, text):
    (Path(repo) / name).write_text(text)
    sh(repo, "add", name)
    sh(repo, "commit", "-qm", name)
    return sh(repo, "rev-parse", "HEAD")


class PinTest(unittest.TestCase):
    def setUp(self):
        self.env = mock.patch.dict(os.environ, GIT_ENV)
        self.env.start()
        self.tmp = tempfile.TemporaryDirectory()
        t = Path(self.tmp.name)
        self.home = t / "home"
        self.up = t / "upstream"
        self.up.mkdir()
        sh(self.up, "init", "-q", "-b", "main")
        self.c1 = commit(self.up, "package.json", "{}")
        self.c2 = commit(self.up, "README", "two")
        self.root = t / "fork"
        self.root.mkdir()
        sh(self.root, "init", "-q", "-b", "master")
        commit(self.root, "README.md", "fork")
        sh(self.root, "submodule", "add", "-q", "--name", "algoglyph", str(self.up), pin.PATH)
        sh(self.root / pin.PATH, "checkout", "-q", self.c1)
        sh(self.root, "add", pin.PATH)
        self.sub = self.root / pin.PATH

    def tearDown(self):
        self.tmp.cleanup()
        self.env.stop()

    def record(self, commit_sha, needs=("knapsack-dp",), kernels=KERNELS):
        pin.write_record(self.root, commit_sha, list(kernels), needs, "u")

    def test_a_record_at_the_gitlink_with_every_needed_kernel_passes(self):
        self.record(self.c1)
        self.assertEqual(pin.check(self.root), [])
        self.assertEqual(pin.pinned(self.root), self.c1)
        rec = pin.load_record(self.root)
        self.assertEqual((rec["schema"], rec["source"], rec["url"]), (pin.SCHEMA, pin.SOURCE, "u"))

    def test_check_names_every_way_the_pin_is_wrong(self):
        self.assertIn(f"{pin.RECORD} is missing or not {pin.SCHEMA}", pin.check(self.root))
        self.record(self.c2, needs=("knapsack-dp", "track-planning"))
        problems = " | ".join(pin.check(self.root))
        self.assertIn(f"the gitlink is {self.c1}", problems)
        self.assertIn("lists no track-planning", problems)
        self.record(self.c1, needs=())
        self.assertIn("names no kernel the stack needs", " ".join(pin.check(self.root)))
        self.record(self.c1)
        sh(self.sub, "checkout", "-q", self.c2)
        self.assertIn(f"checked out at {self.c2}", " ".join(pin.check(self.root)))
        (self.sub / "README").write_text("edited")
        self.assertIn("modified, not at the pin", " ".join(pin.check(self.root)))

    def test_check_refuses_a_missing_submodule_declaration_and_gitlink(self):
        sh(self.root, "rm", "-q", "--cached", pin.PATH)
        (self.root / ".gitmodules").write_text("")
        problems = " | ".join(pin.check(self.root))
        self.assertIn("declares no submodule algoglyph", problems)
        self.assertIn("is not a gitlink", problems)
        self.assertIsNone(pin.url(self.root))

    def test_the_runner_is_the_first_installed_checkout_cleanly_at_the_pin(self):
        self.assertIsNone(pin.runner(self.root, self.home))
        main = self.home / "dev/algoglyph-wt/main"
        sh(self.up, "worktree", "add", "-q", "--detach", str(main), self.c1)
        (main / "node_modules").mkdir()
        self.assertEqual(pin.runner(self.root, self.home), main)
        (self.sub / "node_modules").mkdir()
        self.assertEqual(pin.runner(self.root, self.home), self.sub, "the submodule first")
        sh(self.sub, "checkout", "-q", self.c2)
        self.assertEqual(pin.runner(self.root, self.home), main, "a submodule moved off the pin is passed over")
        (main / "package.json").write_text('{"dirty": true}')
        self.assertIsNone(pin.runner(self.root, self.home), "a modified checkout is not the pinned version")
        self.assertIsNone(pin.clean_head(self.home / "nowhere"))

    def test_install_checks_out_the_pin_and_installs_only_when_asked(self):
        sh(self.root, "commit", "-qm", "pin")
        clone = Path(self.tmp.name) / "clone"
        subprocess.run(["git", "clone", "-q", str(self.root), str(clone)], check=True)
        self.assertFalse(pin.checked_out(clone))
        with mock.patch.object(pin, "npm_ci") as npm, contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(pin.install(clone, npm=False), 0)
            npm.assert_not_called()
            self.assertIn("no node_modules", out.getvalue())
            self.assertEqual(pin.install(clone), 0)
            npm.assert_called_once_with(clone / pin.PATH)
        self.assertEqual(pin.clean_head(clone / pin.PATH), self.c1)

    def test_bump_moves_the_gitlink_records_the_catalogue_and_stages_both(self):
        self.record(self.c1)
        (self.sub / "node_modules").mkdir()
        with mock.patch.object(pin, "list_kernels", return_value=KERNELS) as lk, contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(pin.bump(self.root, "origin/main"), 0)
        lk.assert_called_once_with(self.sub)
        self.assertIn(f"pinned {self.c2}", out.getvalue())
        self.assertEqual(pin.pinned(self.root), self.c2)
        rec = pin.load_record(self.root)
        self.assertEqual((rec["commit"], rec["needs"], rec["kernels"]), (self.c2, ["knapsack-dp"], KERNELS))
        staged = sh(self.root, "diff", "--cached", "--name-only")
        self.assertIn(pin.RECORD, staged)
        self.assertEqual(pin.check(self.root), [])

    def test_bump_refuses_a_commit_without_a_needed_kernel_and_leaves_the_pin(self):
        self.record(self.c1, needs=("knapsack-dp", "action-plan"))
        (self.sub / "node_modules").mkdir()
        err = io.StringIO()
        with mock.patch.object(pin, "list_kernels", return_value=KERNELS), contextlib.redirect_stderr(err):
            self.assertEqual(pin.bump(self.root, "origin/main"), 1)
        self.assertIn("lacks action-plan", err.getvalue())
        self.assertEqual(pin.pinned(self.root), self.c1)
        with mock.patch.object(pin, "list_kernels", return_value=KERNELS), contextlib.redirect_stderr(err):
            (self.root / pin.RECORD).unlink()
            self.assertEqual(pin.bump(self.root, "origin/main"), 1)
        self.assertIn("a list of needed kernels (--need)", err.getvalue())

    def test_bump_installs_the_submodule_or_lists_in_a_runner_checkout_at_the_ref(self):
        sh(self.root, "commit", "-qm", "pin")
        clone = Path(self.tmp.name) / "clone"
        subprocess.run(["git", "clone", "-q", str(self.root), str(clone)], check=True)
        with mock.patch.object(pin, "list_kernels", return_value=KERNELS), mock.patch.object(pin, "npm_ci") as npm, \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(pin.bump(clone, self.c2, needs=["dijkstra"]), 0)
        npm.assert_called_once_with(clone / pin.PATH)
        other = self.home / "dev/algoglyph"
        sh(self.up, "worktree", "add", "-q", "--detach", str(other), self.c1)
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            self.assertEqual(pin.bump(self.root, self.c2, runner_dir=other), 1)
            self.assertIn("is not cleanly at", err.getvalue())
            sh(other, "checkout", "-q", self.c2)
            self.assertEqual(pin.bump(self.root, self.c2, runner_dir=other), 1)
            self.assertIn("has no node_modules", err.getvalue())

    def test_the_answer_is_the_output_or_its_last_json_line(self):
        self.assertEqual(pin.parse_answer('[{"id": "a"}]'), [{"id": "a"}])
        self.assertEqual(pin.parse_answer('> flow\nnoise\n3\n[{"id": "a"}]\nbye'), [{"id": "a"}])
        self.assertIsNone(pin.parse_answer("nothing\n4"))

    def test_list_kernels_runs_the_flow_cli_and_refuses_an_answer_without_a_list(self):
        done = subprocess.CompletedProcess([], 0, stdout='> x\n[{"id": "k"}]\n', stderr="")
        with mock.patch.object(pin.subprocess, "run", return_value=done) as run:
            self.assertEqual(pin.list_kernels(self.sub), [{"id": "k"}])
        self.assertEqual(run.call_args.args[0], ["npm", "run", "--silent", "flow", "--", "kernels"])
        bad = subprocess.CompletedProcess([], 1, stdout="", stderr="vite: not found")
        with mock.patch.object(pin.subprocess, "run", return_value=bad), self.assertRaises(SystemExit) as e:
            pin.list_kernels(self.sub)
        self.assertIn("vite: not found", str(e.exception))
        with mock.patch.object(pin.subprocess, "run") as run:
            pin.npm_ci(self.sub)
        self.assertEqual(run.call_args.args[0][:2], ["npm", "ci"])

    def test_the_command_line_reaches_every_subcommand(self):
        self.record(self.c1)
        cwd = os.getcwd()
        os.chdir(self.root)
        try:
            with mock.patch.dict(os.environ, {"HOME": str(self.home)}):
                out, err = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                    self.assertEqual(pin.main(["status"]), 0)
                    self.assertEqual(json.loads(out.getvalue())["pinned"], self.c1)
                    self.assertEqual(pin.main(["check"]), 0)
                    self.assertEqual(pin.main(["runner"]), 1)
                    self.assertIn("no installed checkout", err.getvalue())
                    (self.sub / "node_modules").mkdir()
                    self.assertEqual(pin.main(["runner"]), 0)
                    self.assertTrue(out.getvalue().rstrip().endswith(pin.PATH))
                    self.assertEqual(pin.main(["install", "--no-npm"]), 0)
                    with mock.patch.object(pin, "list_kernels", return_value=KERNELS):
                        self.assertEqual(pin.main(["bump", "origin/main", "--need", "dijkstra"]), 0)
                    self.assertEqual(pin.load_record(self.root)["needs"], ["dijkstra"])
                    sh(self.sub, "checkout", "-q", self.c1)
                    self.assertEqual(pin.main(["check"]), 1)
                    self.assertIn("not at the pin", err.getvalue())
        finally:
            os.chdir(cwd)


    def test_the_script_runs_as_a_command(self):
        self.record(self.c1)
        r = subprocess.run([sys.executable, "-B", str(SCRIPT), "check"], cwd=self.root, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("record current", r.stdout)


class RepositoryPinTest(unittest.TestCase):
    """The fork's own pin, as committed."""

    def test_the_committed_pin_is_sound(self):
        root = HERE.parents[1]
        if pin.pinned(root) is None:
            self.skipTest("NOT RUN: this checkout has no AlgoGlyph gitlink")
        self.assertEqual(pin.check(root), [])


if __name__ == "__main__":
    unittest.main()
