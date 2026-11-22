"""tools/surfaces/refresh-branch: a branch is brought up to master and its derived regions regenerated.

Every path of the script runs here against scratch repositories (a bare origin and a clone):
nothing to do, a two-branch count race resolved by regeneration, --push and its rerun, a branch
checked out in another worktree, a prose conflict left to its owner, a refused push, and the
usage errors. It works in a temporary worktree of its own: the checkout it is run from keeps its
branch, HEAD and files, the worktree is gone after every exit (a failure and a signal included),
and the result stays in refs/refresh/<branch>. `tools/surfaces/surfaces` is replaced by a stub that writes the number of documents
into a region, so the test exercises the script, not the generator. Run by the Surfaces workflow:
python3 -m unittest tools/surfaces/test_refresh_branch.py
"""
import os
import pathlib
import shutil
import signal
import subprocess
import tempfile
import textwrap
import time
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parent / "refresh-branch"
TITLE = "Doc: [Surfaces] Regenerate derived regions after merging master"

STUB = textwrap.dedent("""\
    #!/usr/bin/env bash
    # Stand-in for surfaces sync: the facts region holds the number of documents under docs/.
    [ "$1" = sync ] || exit 2
    [ -z "${STUB_ENV_OUT:-}" ] || printf '%s\\n' "${SURFACES_VENV:-}" > "$STUB_ENV_OUT"
    if [ -n "${STUB_HOLD:-}" ]; then : > "$STUB_HOLD"; sleep 30; fi
    n=$(ls docs/*.md 2>/dev/null | wc -l | tr -d ' ')
    python3 - "$n" <<'EOF'
    import re, sys
    from pathlib import Path
    p = Path("README.md")
    p.write_text(re.sub(r"(<!-- surfaces:begin facts -->\\n).*?(<!-- surfaces:end facts -->)",
                        lambda m: m[1] + sys.argv[1] + "\\n" + m[2], p.read_text(), flags=re.S))
    EOF
    """)
README = "# Repo\n\nProse line.\n\n<!-- surfaces:begin facts -->\n0\n<!-- surfaces:end facts -->\n"


def git(cwd, *args, check=True):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t",
               GIT_COMMITTER_EMAIL="t@t", GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")
    return subprocess.run(["git", *args], cwd=cwd, env=env, check=check, capture_output=True, text=True)


class RefreshBranch(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.origin = self.tmp / "origin.git"
        self.work = self.tmp / "work"
        git(self.tmp, "init", "-q", "--bare", "-b", "master", str(self.origin))
        git(self.tmp, "clone", "-q", str(self.origin), str(self.work))
        (self.work / "docs").mkdir()
        (self.work / "tools/surfaces").mkdir(parents=True)
        stub = self.work / "tools/surfaces/surfaces"
        stub.write_text(STUB)
        stub.chmod(0o755)
        (self.work / "README.md").write_text(README)
        git(self.work, "checkout", "-q", "-b", "master")
        self.commit("Add: base", ["README.md", "tools/surfaces/surfaces"])
        git(self.work, "push", "-q", "origin", "master")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def commit(self, msg, paths):
        git(self.work, "add", *paths)
        git(self.work, "commit", "-q", "-m", msg)

    def add_doc(self, name):
        (self.work / "docs").mkdir(exist_ok=True)  # git drops the empty directory on checkout
        (self.work / f"docs/{name}.md").write_text(f"# {name}\n")
        subprocess.run([str(self.work / "tools/surfaces/surfaces"), "sync"], cwd=self.work, check=True)
        self.commit(f"Doc: add {name}", [f"docs/{name}.md", "README.md"])

    def env(self, **extra):
        # TMPDIR inside the scratch area, so a temporary worktree left behind would be seen.
        self.scratch = self.tmp / "scratch"
        self.scratch.mkdir(exist_ok=True)
        return dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t",
                    GIT_COMMITTER_EMAIL="t@t", GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null",
                    TMPDIR=str(self.scratch), **extra)

    def run_script(self, *args, **extra):
        return subprocess.run([str(SCRIPT), *args], cwd=self.work, env=self.env(**extra),
                              capture_output=True, text=True)

    def caller_state(self):
        """What the checkout the script runs from must keep: its branch, its HEAD, its files."""
        return (git(self.work, "symbolic-ref", "-q", "HEAD", check=False).stdout.strip(),
                git(self.work, "rev-parse", "HEAD").stdout.strip(),
                git(self.work, "status", "--porcelain").stdout)

    def assert_no_temporary_worktree(self):
        listed = git(self.work, "worktree", "list", "--porcelain").stdout
        self.assertNotIn(str(self.scratch), listed)
        self.assertEqual(list(self.scratch.iterdir()), [], "the temporary directory is removed")

    def race(self):
        """A branch and master each add a document; both write the count 1."""
        git(self.work, "checkout", "-q", "-b", "t")
        self.add_doc("branch")
        git(self.work, "push", "-q", "origin", "t")
        git(self.work, "checkout", "-q", "master")
        self.add_doc("master")
        git(self.work, "push", "-q", "origin", "master")

    def facts(self, ref):
        text = git(self.work, "show", f"{ref}:README.md").stdout
        return text.split("<!-- surfaces:begin facts -->\n")[1].split("\n")[0]

    def test_a_current_branch_has_nothing_to_do(self):
        git(self.work, "push", "-q", "origin", "master:t")
        r = self.run_script("t")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("is current", r.stdout)

    def test_a_count_race_is_resolved_by_regeneration(self):
        self.race()
        self.assertEqual(self.facts("origin/t"), "1")
        r = self.run_script("t")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertIn("refreshed locally", r.stdout)
        self.assertIn("refs/refresh/t", r.stdout)
        self.assertEqual(self.facts("refs/refresh/t"), "2")
        # One merge commit, already holding the regenerated region.
        self.assertEqual(git(self.work, "log", "-1", "--format=%s", "refs/refresh/t").stdout.strip(),
                         "Merge origin/master into t")
        self.assertEqual(git(self.work, "rev-list", "--count", "origin/t..refs/refresh/t",
                             "^origin/master").stdout.strip(), "1")
        self.assertEqual(self.facts("origin/t"), "1", "without --push nothing reaches origin")

    def test_a_local_pre_commit_hook_that_checks_regions_accepts_the_merge(self):
        self.race()
        hook = self.work / ".git/hooks/pre-commit"
        hook.write_text(textwrap.dedent("""\
            #!/usr/bin/env bash
            # Stand-in for surfaces check: the facts region must hold the number of documents.
            n=$(ls docs/*.md 2>/dev/null | wc -l | tr -d ' ')
            grep -A1 'surfaces:begin facts' README.md | tail -1 | grep -qx "$n" || { echo stale >&2; exit 10; }
            """))
        hook.chmod(0o755)
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        git(self.work, "fetch", "-q", "origin")
        self.assertEqual(self.facts("origin/t"), "2")

    def test_a_branch_with_master_but_stale_regions_gets_one_sync_commit(self):
        git(self.work, "checkout", "-q", "-b", "t")
        (self.work / "docs").mkdir(exist_ok=True)
        (self.work / "docs/unsynced.md").write_text("# unsynced\n")
        self.commit("Doc: a page without sync", ["docs/unsynced.md"])
        git(self.work, "push", "-q", "origin", "t")
        r = self.run_script("t")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual(git(self.work, "log", "-1", "--format=%s", "refs/refresh/t").stdout.strip(), TITLE)
        self.assertEqual(self.facts("refs/refresh/t"), "1")

    def test_push_then_rerun_is_idempotent(self):
        self.race()
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        git(self.work, "fetch", "-q", "origin")
        self.assertEqual(self.facts("origin/t"), "2")
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_branch_checked_out_in_another_worktree(self):
        self.race()
        git(self.work, "worktree", "add", "-q", str(self.tmp / "wt"), "t")
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)

    def test_a_prose_conflict_is_left_to_its_owner(self):
        git(self.work, "checkout", "-q", "-b", "t")
        (self.work / "README.md").write_text(README.replace("Prose line.", "Branch prose."))
        self.commit("Doc: branch prose", ["README.md"])
        git(self.work, "push", "-q", "origin", "t")
        git(self.work, "checkout", "-q", "master")
        (self.work / "README.md").write_text(README.replace("Prose line.", "Master prose."))
        self.commit("Doc: master prose", ["README.md"])
        git(self.work, "push", "-q", "origin", "master")
        before = self.caller_state()
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 10, r.stdout + r.stderr)
        self.assertIn("left to its owner", r.stdout)
        self.assertEqual(self.caller_state(), before)
        self.assert_no_temporary_worktree()

    def test_a_refused_push_is_not_forced(self):
        self.race()
        hook = self.origin / "hooks/pre-receive"
        hook.write_text("#!/bin/sh\necho refused >&2\nexit 1\n")
        hook.chmod(0o755)
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("not pushed", r.stdout)
        self.assert_no_temporary_worktree()

    def test_the_checkout_it_runs_from_keeps_its_branch(self):
        """Run from the primary checkout on master, it never detaches master (or touches a file)."""
        self.race()
        self.assertEqual(git(self.work, "symbolic-ref", "HEAD").stdout.strip(), "refs/heads/master")
        before = self.caller_state()
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual(self.caller_state(), before)
        self.assert_no_temporary_worktree()
        git(self.work, "fetch", "-q", "origin")
        self.assertEqual(self.facts("origin/t"), "2")

    def test_the_generator_reuses_the_callers_environment(self):
        self.race()
        probe = self.tmp / "venv.txt"
        r = self.run_script("t", STUB_ENV_OUT=str(probe))
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual(probe.read_text().strip(), str(self.work.resolve() / "tools/surfaces/.venv"))
        r = self.run_script("t", STUB_ENV_OUT=str(probe), SURFACES_VENV="/elsewhere")
        self.assertEqual(probe.read_text().strip(), "/elsewhere")

    def test_a_signal_still_removes_the_temporary_worktree(self):
        for signum, code in ((signal.SIGTERM, 143), (signal.SIGINT, 130), (signal.SIGHUP, 129)):
            with self.subTest(signal=signum.name):
                self.race() if signum == signal.SIGTERM else None
                before = self.caller_state()
                hold = self.tmp / f"hold-{signum.name}"
                proc = subprocess.Popen([str(SCRIPT), "t"], cwd=self.work, env=self.env(STUB_HOLD=str(hold)),
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
                deadline = time.monotonic() + 30
                while not hold.exists() and time.monotonic() < deadline:
                    time.sleep(0.05)
                self.assertTrue(hold.exists(), "the script reached the generator")
                # The whole group, as a terminal's Ctrl-C or a runner's cancel delivers it.
                os.killpg(proc.pid, signum)
                proc.wait(timeout=30)
                self.assertEqual(proc.returncode, code, proc.stderr.read().decode())
                proc.stdout.close(); proc.stderr.close()
                self.assertEqual(self.caller_state(), before)
                self.assert_no_temporary_worktree()

    def test_usage(self):
        self.assertEqual(self.run_script("--help").returncode, 0)
        self.assertIn("Usage:", self.run_script("-h").stdout)
        self.assertEqual(self.run_script("--bogus").returncode, 2)
        self.assertEqual(self.run_script().returncode, 2)
        self.assertEqual(self.run_script("a", "b").returncode, 2)


if __name__ == "__main__":
    unittest.main()
