"""tools/check-prismatic-docs.sh refuses the false MCP capability claims wherever they reappear.

The check (I0111) greps every tracked file for the phrases an external review found false on
2026-11-22: "does NOT expose any mutation" and a "planned" entry for openttd.ai.*. Only the plan
records under .ai/repo/project/ (the history of the fix), the check and this test may name them.
Each case runs the real script in a throwaway git repository whose generator is a stub, so the
second half of the script (the generated-docs diff) passes and only the claim check decides.
Run by tools/gate.sh change (python3 -m unittest discover -s tools/research -p 'test_*.py').
"""
import pathlib
import shutil
import subprocess
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE.parent / "check-prismatic-docs.sh"
MUTATION = "It deliberately " + "does NOT expose any mutation yet."
PLANNED = "Planned, not yet in the game: `openttd.ai." + "*` tools, client mode."


class CheckPrismaticDocs(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        (self.root / "tools" / "research").mkdir(parents=True)
        shutil.copy(SCRIPT, self.root / "tools" / "check-prismatic-docs.sh")
        (self.root / "tools" / "research" / "generate_prismatic_docs.py").write_text("")
        (self.root / "research" / "prismatic-mcp").mkdir(parents=True)
        (self.root / "README.md").write_text("# README\n")
        (self.root / "research" / "prismatic-mcp" / "README.md").write_text("# MCP\n")
        self.git("init", "-q")
        self.commit_all()

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, *args):
        subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                       cwd=self.root, check=True, capture_output=True)

    def commit_all(self):
        self.git("add", "-A")
        self.git("commit", "-q", "--allow-empty", "-m", "fixture")

    def write(self, rel, text, track=True):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        if track:
            self.git("add", rel)

    def run_check(self):
        return subprocess.run(["bash", "tools/check-prismatic-docs.sh"], cwd=self.root,
                              capture_output=True, text=True)

    def test_a_tree_without_the_claims_passes(self):
        r = self.run_check()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("check-prismatic-docs: PASS", r.stdout)

    def test_the_no_mutation_claim_fails_and_names_the_file(self):
        self.write("src/prismatic_mcp/server.cpp", " * " + MUTATION + "\n")
        r = self.run_check()
        self.assertEqual(r.returncode, 1)
        self.assertIn("src/prismatic_mcp/server.cpp:1:", r.stdout)
        self.assertIn("STALE CLAIM", r.stderr)
        self.assertIn("FAIL: describe what master implements", r.stderr)

    def test_a_planned_entry_for_the_ai_tools_fails(self):
        self.write("research/mcp-control.md", "Intro.\n\n" + PLANNED + "\n")
        r = self.run_check()
        self.assertEqual(r.returncode, 1)
        self.assertIn("research/mcp-control.md:3:", r.stdout)

    def test_the_plan_records_may_quote_the_history_and_untracked_files_are_not_read(self):
        self.write(".ai/repo/project/issues/I0111.yaml", "why: '" + MUTATION + " " + PLANNED + "'\n")
        self.write("notes.md", MUTATION + "\n", track=False)
        r = self.run_check()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
