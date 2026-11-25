"""Feature maturity (tools/surfaces/maturity.py, I0112): a feature record's status is a declared
level and its `## Maturity` section cites the evidence that level requires.

The unit cases call maturity.check on records written into a scratch directory. The end-to-end case
runs `surfaces check` on a scratch repository: the record it must refuse is refused as kind
`feature`, and the same record with its evidence passes. Run by the Surfaces workflow:
python3 -m unittest discover -s tools/surfaces -p 'test_*.py'
"""
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import maturity  # noqa: E402

SURFACES = HERE / "surfaces"
LEVELS = {
    "stable": {"requires": ["shipped", "tested"]},
    "draft": {"requires": ["missing"], "allows": ["shipped", "tested"]},
}
CFG = {"section": "Maturity", "definition": "README.md#maturity-levels", "levels": LEVELS}
FRONT = ("---\nschema: feature/v1\nid: x\nkind: feature\nstatus: {status}\n---\n\n"
         "## What it does\n\nIt works.\n")


class Check(unittest.TestCase):
    def setUp(self):
        self.root = pathlib.Path(tempfile.mkdtemp())
        (self.root / "src").mkdir()
        (self.root / "src/a.py").write_text("def run():\n    return 1\n")
        self.tracked = {"src/a.py", "f.md"}
        self.dirs = {"", "src"}
        self.issues = {"I0007"}

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def problems(self, status, body, cfg=CFG, record=None):
        (self.root / "f.md").write_text(FRONT.format(status=status) + body)
        rec = record or {"path": "f.md", "status": status}
        return [(p.line, p.message) for p in
                maturity.check(self.root, [rec], self.tracked, self.dirs, self.issues, cfg)]

    def test_no_declared_levels_checks_nothing(self):
        self.assertEqual(self.problems("whatever", "", cfg={"levels": {}}), [])

    def test_a_record_with_every_required_line_cited_passes(self):
        body = ("\n## Maturity\n\n`stable`.\n\n- **shipped**: `src/a.py:1:run` and\n"
                "  the rest of `src/`.\n- **tested**: by I0007, PR #12.\n\n"
                "## After\n\n- **bogus**: x\n")
        self.assertEqual(self.problems("stable", body), [])

    def test_a_status_that_is_not_a_level_is_refused(self):
        (line, msg), = self.problems("active", "")
        self.assertEqual(line, 1)
        self.assertIn("status 'active' is not a defined maturity level: stable, draft", msg)
        self.assertIn("README.md#maturity-levels", msg)

    def test_a_record_of_another_kind_is_not_judged(self):
        rec = {"path": "f.md", "status": "active", "kind": "domain"}
        self.assertEqual(self.problems("active", "", record=rec), [])

    def test_a_record_without_the_section_is_refused(self):
        (line, msg), = self.problems("stable", "\n```\n## Maturity\n```\n")
        self.assertEqual(line, 1)
        self.assertIn("no '## Maturity' section: a stable feature cites its evidence there "
                      "(shipped, tested)", msg)

    def test_a_missing_line_an_unknown_label_and_an_uncited_line_are_each_refused(self):
        body = ("\n## Maturity\n\n"
                "- **shipped**: `src/gone.py:1:run`, `tools/gate.sh change`, I9999\n"
                "- **gated**: `src/a.py`\n")
        found = self.problems("stable", body)
        self.assertEqual(len(found), 3, found)
        self.assertIn("'shipped' cites nothing this repository resolves", found[0][1])
        self.assertIn("'gated' is not evidence a stable feature carries (shipped, tested)",
                      found[1][1])
        self.assertIn("a stable feature needs a '- **tested**:' line", found[2][1])
        self.assertEqual([f[0] for f in found], [14, 15, 12])

    def test_a_draft_names_what_is_missing_and_may_carry_what_it_meets(self):
        ok = "\n## Maturity\n\n- **shipped**: `src/a.py`\n- **missing**: `tested`; I0007 adds it.\n"
        self.assertEqual(self.problems("draft", ok), [])
        (_, msg), = self.problems("draft", "\n## Maturity\n\n- **shipped**: `src/a.py`\n")
        self.assertIn("a draft feature needs a '- **missing**:' line", msg)


MANIFEST = "schema: ai-repository/v1\nsections:\n  features: repo/features\n"
TOML = '''\
[features]
definition = "README.md"
levels = { stable = { requires = ["shipped"] } }
'''
RECORD = """---
schema: feature/v1
id: thing
kind: feature
title: Thing
headline: It does a thing.
summary: It does a thing.
status: stable
---

## What it does

A thing.
"""


class SurfacesCheck(unittest.TestCase):
    def setUp(self):
        self.repo = pathlib.Path(tempfile.mkdtemp())
        self.env = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")
        (self.repo / ".ai/repo/features").mkdir(parents=True)
        (self.repo / ".ai/manifest.yaml").write_text(MANIFEST)
        (self.repo / "surfaces.toml").write_text(TOML)
        (self.repo / "README.md").write_text("# Scratch\n")
        (self.repo / "thing.py").write_text("THING = 1\n")
        self.record = self.repo / ".ai/repo/features/thing.md"
        self.record.write_text(RECORD)
        for args in (["init", "-q", "-b", "master"], ["add", "-A"],
                     ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "base"]):
            subprocess.run(["git", *args], cwd=self.repo, env=self.env, check=True,
                           capture_output=True)

    def tearDown(self):
        shutil.rmtree(self.repo, ignore_errors=True)

    def check(self):
        r = subprocess.run([str(SURFACES), "--repo", str(self.repo), "check"],
                           capture_output=True, text=True, env=self.env)
        return r.returncode, [line for line in r.stdout.splitlines() if " feature " in line]

    def test_the_check_refuses_a_record_without_evidence_and_passes_it_with_its_evidence(self):
        code, lines = self.check()
        self.assertEqual(code, 10)
        self.assertEqual(len(lines), 1, lines)
        self.assertIn(".ai/repo/features/thing.md:1", lines[0])
        self.assertIn("no '## Maturity' section", lines[0])
        self.record.write_text(RECORD + "\n## Maturity\n\n- **shipped**: `thing.py:1:THING`\n")
        code, lines = self.check()
        self.assertEqual(lines, [])


if __name__ == "__main__":
    unittest.main()
