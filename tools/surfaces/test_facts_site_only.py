"""A fact marked `site_only` stays out of every Markdown region (tools/surfaces/surfaces.py gen_facts).

The document count changes with nearly every pull request; written into README.md it made every
two open pull requests conflict on one line. A scratch repository with two facts, one of them
site_only, is synced, and only the other reaches the region. Run by the Surfaces workflow:
python3 -m unittest discover -s tools/surfaces -p 'test_*.py'
"""
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

SURFACES = pathlib.Path(__file__).resolve().parent / "surfaces"

TOML = '''\
[[facts.items]]
id = "notes"
label = "notes counted"
files = "notes/*.txt"

[[facts.items]]
id = "documents"
label = "published documents"
docs = ""
site_only = true
'''
README = "# Scratch\n\n<!-- surfaces:begin facts -->\n<!-- surfaces:end facts -->\n"


class SiteOnlyFacts(unittest.TestCase):
    def setUp(self):
        self.repo = pathlib.Path(tempfile.mkdtemp())
        env = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")
        (self.repo / "notes").mkdir()
        (self.repo / "notes/a.txt").write_text("a\n")
        (self.repo / "notes/b.txt").write_text("b\n")
        (self.repo / "surfaces.toml").write_text(TOML)
        (self.repo / "README.md").write_text(README)
        for args in (["init", "-q", "-b", "master"], ["add", "-A"],
                     ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "base"]):
            subprocess.run(["git", *args], cwd=self.repo, env=env, check=True, capture_output=True)

    def tearDown(self):
        shutil.rmtree(self.repo, ignore_errors=True)

    def test_a_site_only_fact_is_not_written_into_the_region(self):
        r = subprocess.run([str(SURFACES), "--repo", str(self.repo), "sync"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        text = (self.repo / "README.md").read_text()
        self.assertIn("notes counted", text)
        self.assertIn("**2**", text)
        self.assertNotIn("published documents", text)


if __name__ == "__main__":
    unittest.main()
