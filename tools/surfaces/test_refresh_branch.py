"""tools/surfaces/refresh-branch: a branch is brought up to master and its derived regions regenerated.

Every path of the script runs here against scratch repositories (a bare origin and a clone):
nothing to do, a two-branch count race resolved by regeneration, --push and its rerun, a branch
checked out in another worktree, a prose conflict left to its owner, a refused push, and the
usage errors. `tools/surfaces/surfaces` is replaced by a stub that writes the number of documents
into a region, so the test exercises the script, not the generator. Run by the Surfaces workflow:
python3 -m unittest tools/surfaces/test_refresh_branch.py
"""
import os
import pathlib
import shutil
import subprocess
import tempfile
import textwrap
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parent / "refresh-branch"
TITLE = "Doc: [Surfaces] Regenerate derived regions after merging master"

STUB = textwrap.dedent("""\
    #!/usr/bin/env bash
    # Stand-in for surfaces sync: the facts region holds the number of documents under docs/.
    [ "$1" = sync ] || exit 2
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

    def run_script(self, *args):
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t",
                   GIT_COMMITTER_EMAIL="t@t", GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")
        return subprocess.run([str(SCRIPT), *args], cwd=self.work, env=env, capture_output=True, text=True)

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
        self.assertEqual(self.facts("HEAD"), "2")
        self.assertEqual(git(self.work, "log", "-1", "--format=%s").stdout.strip(), TITLE)
        self.assertEqual(git(self.work, "log", "-1", "--format=%s", "HEAD^").stdout.strip(),
                         "Merge origin/master into t")
        self.assertEqual(self.facts("origin/t"), "1", "without --push nothing reaches origin")

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
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 10, r.stdout + r.stderr)
        self.assertIn("left to its owner", r.stdout)
        self.assertEqual(git(self.work, "status", "--porcelain").stdout, "")

    def test_a_refused_push_is_not_forced(self):
        self.race()
        hook = self.origin / "hooks/pre-receive"
        hook.write_text("#!/bin/sh\necho refused >&2\nexit 1\n")
        hook.chmod(0o755)
        r = self.run_script("--push", "t")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("not pushed", r.stdout)

    def test_usage(self):
        self.assertEqual(self.run_script("--help").returncode, 0)
        self.assertIn("Usage:", self.run_script("-h").stdout)
        self.assertEqual(self.run_script("--bogus").returncode, 2)
        self.assertEqual(self.run_script().returncode, 2)
        self.assertEqual(self.run_script("a", "b").returncode, 2)


if __name__ == "__main__":
    unittest.main()
