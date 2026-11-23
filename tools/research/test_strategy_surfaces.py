#!/usr/bin/env python3
"""RESEARCH-ONLY. Tests for tools/research/strategy-surfaces and the ablation `standings` and
`imported` fields of battle.py, offline: no game, no binary, temporary git repositories only.

Run: python3 -B tools/research/test_strategy_surfaces.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py', as tools/gate.sh change does).
"""
import argparse
import contextlib
import importlib.machinery
import importlib.util
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
loader = importlib.machinery.SourceFileLoader("strategy_surfaces", str(HERE / "strategy-surfaces"))
spec = importlib.util.spec_from_loader("strategy_surfaces", loader)
ss = importlib.util.module_from_spec(spec)
loader.exec_module(ss)
battle = ss.battle

FIELD = ["Hog", "Admiral"]
MC = {"years": 2, "start_year": 1950, "landscape": "temperate", "map_x": 8, "map_y": 8}


def ref(cid, value, t="1951-1", rail=1, road=2, air=3, name="X"):
    return (f"[2026-11-23 02:04:37] dbg: [script:1] [18] [E] REF Q date={t} cid={cid} value={value} "
            f"money=7 vehicles={rail + road + air} rail={rail} road={road} air={air} name={name}")


LOG = "\n".join([ref(0, 1, "1950-1"), ref(1, 1, "1950-1"), ref(2, 1, "1950-1"),
                 ref(0, 100), ref(1, 400, rail=9), ref(2, 50),
                 "[2026-11-23 02:04:38] dbg: [script:1] [0] [S] boom"])


def record(rid="ablation-20261123T000000-seed7", standings=True, **over):
    row = battle.ablation_row("pai", "x=1", 0, LOG, FIELD, 0)
    if not standings:
        del row["standings"]
    row["vs_first"] = 0.0
    rec = {"schema": battle.ABLATION_SCHEMA, "id": rid, "generated_at": "2026-11-23T00:00:00+00:00",
           "seed": 7, "years": 2, "field": list(FIELD), "slot": 0, "match_cfg": MC, "rev": "a" * 40,
           "tree_dirty": False, "rows": [row]}
    rec.update(over)
    return rec


def zero_value(rec):
    """The record's PrismaticAI ends at 0 (no finite gap), its ratio to the baseline kept derived."""
    row = rec["rows"][0]
    row["standings"][0]["value"] = row["value"] = 0
    row["vs_first"] = battle.ablation_vs_first([row])[0]
    return rec


def run_quiet(fn, *args, **kw):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = fn(*args, **kw)
    return code, out.getvalue(), err.getvalue()


class Standings(unittest.TestCase):
    def test_a_row_from_a_log_keys_every_company_as_the_lineup_names_it(self):
        row = battle.ablation_row("pai", "x=1", None, LOG, FIELD, 0)
        self.assertEqual(row["exit_code"], None)
        self.assertEqual((row["value"], row["field_best"], row["rail"], row["errors"]), (100, 400, 1, 1))
        self.assertEqual([(e["key"], e["value"]) for e in row["standings"]],
                         [("pai", 100), ("Hog", 400), ("Admiral", 50)])
        first = row["standings"][1]
        self.assertEqual((first["company"], first["vehicles"], first["rail"], first["water"], first["errors"]),
                         (1, 14, 9, 0, 0))
        self.assertEqual(row["standings"][0]["errors"], 1)
        later = battle.ablation_row("pai", "", 0, LOG, FIELD, 1)   # the variant started second
        self.assertEqual([e["key"] for e in later["standings"]], ["Hog", "pai", "Admiral"])
        self.assertEqual(later["value"], 400)

    def test_valid_and_legacy_records_pass(self):
        self.assertEqual(battle.validate_ablation(record(), "ablation-20261123T000000-seed7.json"), [])
        self.assertEqual(battle.validate_ablation(record(standings=False),
                                                  "ablation-20261123T000000-seed7.json"), [])

    def test_each_broken_standings_is_refused(self):
        name = "ablation-20261123T000000-seed7.json"

        def errs(mutate):
            rec = record()
            mutate(rec["rows"][0])
            return "\n".join(battle.validate_ablation(rec, name))
        self.assertIn("not a list", errs(lambda r: r.update(standings="x")))
        self.assertIn("integer value", errs(lambda r: r["standings"][0].update(value=True)))
        self.assertIn("named twice", errs(lambda r: r["standings"][1].update(key="pai")))
        self.assertIn("the match held", errs(lambda r: r["standings"][1].update(key="Other")))
        self.assertIn("is not the row's 99", errs(lambda r: r.update(value=99)))
        self.assertIn("field_best", errs(lambda r: r.update(field_best=1)))

    def test_an_imported_record_says_where_it_came_from(self):
        imp = {"log": "a.log", "log_sha256": "f" * 64, "by": "tool", "binary": "v1", "note": "Why."}
        line = battle.imported_line(imp)
        self.assertIn("`a.log` (sha256 `ffffffffffffffff`), binary `v1`, by `tool`", line)
        self.assertTrue(line.endswith("Why."))
        self.assertNotIn("binary", battle.imported_line({"log": "a", "log_sha256": "0" * 64, "by": "t"}))
        self.assertIn(line, battle.render_ablation_md(record(imported=imp)))


class AblateRun(unittest.TestCase):
    """cmd_ablate end to end against a fake game binary that prints the referee lines of LOG: each
    variant's row is built by ablation_row, standings included."""

    def test_rows_of_a_fake_match_carry_standings(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            repo = tmp / "repo"
            (repo / "build").mkdir(parents=True)
            (repo / "ai" / "PrismaticAI").mkdir(parents=True)
            fake = repo / "build" / "openttd"
            fake.write_text("#!/bin/sh\ncat <<'EOF'\n" + LOG + "\nEOF\n")
            fake.chmod(0o755)
            arena = tmp / "cache" / "arena"
            for d in ("baseset", "game", "ai/library"):
                (arena / d).mkdir(parents=True)
            cfg = {"cache": tmp / "cache", "prismatic": {"path": "ai/PrismaticAI"}, "match": MC}
            old = battle.REPO
            battle.REPO = repo
            try:
                code, out, _ = run_quiet(battle.cmd_ablate, cfg, 7, 2, ["base=x=1"], FIELD, False,
                                         str(tmp / "logs"))
            finally:
                battle.REPO = old
            self.assertEqual(code, 0)
            self.assertIn("ablate: base         value          100", out)
            self.assertIn("REF Q", (tmp / "logs" / "base.log").read_text())


class Script(unittest.TestCase):
    def test_the_script_runs_as_a_command(self):
        done = subprocess.run([sys.executable, "-B", str(HERE / "strategy-surfaces"), "--help"],
                              capture_output=True, text=True)
        self.assertEqual(done.returncode, 0)
        self.assertIn("strategy-evals", done.stdout)


class Evaluations(unittest.TestCase):
    def test_rows_rank_rival_and_gap_are_derived(self):
        rows = ss.eval_rows([("a.json", record(rid="a")), ("b.json", record(rid="b", standings=False))])
        self.assertEqual(len(rows), 1)
        r = rows[0]
        self.assertEqual((r["rank"], r["of"], r["rival"], r["rival_value"], r["gap"], r["mix"]),
                         (2, 3, "Hog", 400, 4.0, (1, 2, 3)))

    def test_an_invalid_record_is_unreadable(self):
        with self.assertRaises(ss.Unreadable):
            ss.eval_rows([("wrong.json", record())])

    def test_gap_and_its_format(self):
        self.assertIsNone(ss.gap(0, 5))
        self.assertIsNone(ss.gap(5, 0))
        self.assertEqual(ss.fmt_gap(None), "n/a")
        self.assertEqual(ss.fmt_gap(2.25), "2.2x")

    def test_cohorts_summarise_comparable_runs(self):
        a, b = record(rid="a"), record(rid="b", seed=8)
        c = record(rid="c", rev="b" * 40)
        zero_value(b)   # no ratio on this seed
        rows = ss.eval_rows([("a.json", a), ("b.json", b), ("c.json", c)])
        groups = ss.cohorts(rows)
        self.assertEqual([g["id"] for g in groups], ["C1", "C2"])
        self.assertEqual([len(g["rows"]) for g in groups], [2, 1])
        self.assertEqual((groups[0]["geo"], groups[0]["excluded"]), (4.0, 1))
        bad = record(rid="d")
        bad["rows"][0]["standings"] = [e for e in bad["rows"][0]["standings"] if e["key"] == "pai"]
        bad["field"], bad["rows"][0]["field_best"] = [], 0
        self.assertIsNone(ss.cohorts(ss.eval_rows([("d.json", bad)]))[0]["geo"])

    def test_render_evals(self):
        self.assertIn("No ablation record", ss.render_evals([("a.json", record(rid="a", standings=False))]))
        md = ss.render_evals([("a.json", record(rid="a")), ("b.json", record(rid="b", seed=8))],
                             "https://github.com/o/r/commit/")
        self.assertIn("| C1 | pai | [`aaaaaaaaaa`](https://github.com/o/r/commit/" + "a" * 40 + ")", md)
        self.assertIn("[a](../results/a.md)", md)
        self.assertIn("- C1: field Hog, Admiral; PrismaticAI in slot 0; settings `x=1`", md)
        self.assertIn("xychart-beta", md)
        plain = ss.render_evals([("a.json", record(rid="a"))])
        self.assertIn("| `aaaaaaaaaa` |", plain)
        zero = record(rid="z")
        zero["rows"][0]["settings"] = ""
        zero_value(zero)
        md = ss.render_evals([("z.json", zero)])
        self.assertIn("(1 without a ratio)", md)
        self.assertIn("settings `(defaults)`", md)
        self.assertNotIn("xychart-beta", md)


def git_repo(root: Path, origin: str | None = None) -> None:
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    if origin:
        subprocess.run(["git", "-C", str(root), "remote", "add", "origin", origin], check=True)


class Incidents(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "research" / "strategy").mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def doc(self, name, text):
        f = self.root / "research" / name
        f.write_text(text)
        return f

    def test_lead_and_label(self):
        self.assertEqual(ss.lead("no heading"), ("", ""))
        self.assertEqual(ss.lead("# T\n"), ("T", ""))
        self.assertEqual(ss.lead("# T\n\n**Post-mortem.** A\nB\n\nC"), ("T", "**Post-mortem.** A\nB"))
        self.assertEqual(ss.label_of("Root-cause analysis of one failure"), "Root-cause analysis")
        self.assertIsNone(ss.label_of("**RESEARCH-ONLY.** A tool."))

    def test_documents_are_selected_dated_and_interlinked(self):
        a = self.doc("incident-2026-11-22-a.md", "# Incident 2026-11-22: mines\n\nPost-mortem. On seed 2927.\n")
        b = self.doc("incident-2026-11-23-b.md",
                     "# Incident 2026-11-23: trains\n\nRoot-cause analysis of seeds 15805 and seed 77.\n\n"
                     "See [a](incident-2026-11-22-a.md#x) and `research/strategy-postmortem.md`.\n")
        c = self.doc("strategy-postmortem.md", "# Strategy post-mortem\n\nPost-mortem, 2026-11-24. "
                     "Also `incident-2026-11-23-b.md` and `missing.md`.\n")
        tool = self.doc("battle-postmortem.md", "# Battle post-mortem\n\n**RESEARCH-ONLY.** A tool.\n")
        docs = ss.incidents([a, b, c, tool], self.root)
        self.assertEqual([d["rel"] for d in docs], ["research/strategy-postmortem.md",
                                                    "research/incident-2026-11-23-b.md",
                                                    "research/incident-2026-11-22-a.md"])
        self.assertEqual([d["date"] for d in docs], ["2026-11-24", "2026-11-23", "2026-11-22"])
        self.assertEqual(docs[1]["title"], "Trains")
        self.assertEqual(docs[1]["seeds"], ["15805"])
        self.assertEqual(docs[2]["seeds"], ["2927"])
        self.assertEqual([p.name for p in docs[1]["refers"]], ["incident-2026-11-22-a.md", "strategy-postmortem.md"])
        self.assertEqual([p.name for p in docs[1]["referred"]], ["strategy-postmortem.md"])
        page = self.root / "research" / "strategy" / "README.md"
        md = ss.render_incidents([a, b, c], self.root, page, {2927})
        self.assertIn("[Trains](../incident-2026-11-23-b.md)", md)
        self.assertIn("[2927](#evaluations-against-the-field)", md)
        self.assertIn("| 15805 |", md)
        self.assertIn("[incident-2026-11-22-a](../incident-2026-11-22-a.md)", md)
        self.assertIn("No research document", ss.render_incidents([tool], self.root, page))

    def test_a_post_mortem_without_a_date_is_unreadable(self):
        f = self.doc("undated.md", "# U\n\nPost-mortem. Sometime.\n")
        with self.assertRaises(ss.Unreadable):
            ss.incidents([f], self.root)


class Pages(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        git_repo(self.root, "git@github.com:o/r.git")
        self.results = self.root / "research" / "results"
        self.results.mkdir(parents=True)
        (self.root / "research" / "strategy").mkdir()
        self.page = self.root / "research" / "strategy" / "README.md"
        self.page.write_text("# S\n\n<!-- strategy-evals:begin -->\n<!-- strategy-evals:end -->\n\n"
                             "<!-- strategy-incidents:begin -->\nold\n<!-- strategy-incidents:end -->\n")
        rec = record(seed=7777)
        (self.results / f"{rec['id']}.json").write_text(json.dumps(rec))
        (self.root / "research" / "incident-2026-11-22-a.md").write_text("# A\n\nPost-mortem. seed 7777.\n")

    def tearDown(self):
        self.tmp.cleanup()

    def test_replace_needs_its_markers(self):
        with self.assertRaises(ss.Unreadable):
            ss.replace("no markers", "strategy-evals", "x")

    def test_commit_url(self):
        self.assertEqual(ss.commit_url(self.root), "https://github.com/o/r/commit/")
        other = self.root / "other"
        git_repo(other, "https://gitlab.com/o/r.git")
        self.assertIsNone(ss.commit_url(other))
        bare = self.root / "bare"
        git_repo(bare)
        self.assertIsNone(ss.commit_url(bare))

    def test_tracked_lists_markdown_git_would_commit(self):
        (self.root / "research" / "x.txt").write_text("x")
        self.assertIn(self.root / "research" / "incident-2026-11-22-a.md", ss.tracked("research", self.root))
        self.assertNotIn(self.root / "research" / "x.txt", ss.tracked("research", self.root))

    def test_render_check_refuses_drift_and_render_rewrites(self):
        kw = dict(page=self.page, root=self.root, results=self.results)
        code, _, err = run_quiet(ss.render, True, **kw)
        self.assertEqual(code, 11)
        self.assertIn("stale", err)
        self.assertEqual(run_quiet(ss.render, False, **kw)[0], 0)
        text = self.page.read_text()
        self.assertIn("https://github.com/o/r/commit/", text)
        self.assertIn("[7777](#evaluations-against-the-field)", text)
        code, out, _ = run_quiet(ss.render, True, **kw)
        self.assertEqual((code, "current" in out), (0, True))
        self.page.write_text("# no markers\n")
        code, _, err = run_quiet(ss.render, False, **kw)
        self.assertEqual(code, 10)
        self.assertIn("markers missing", err)


class Import(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.tree = self.root / "tree"
        git_repo(self.tree)
        ai = self.tree / "ai" / "PrismaticAI"
        ai.mkdir(parents=True)
        (ai / "main.nut").write_text("x")
        subprocess.run(["git", "-C", str(self.tree), "add", "."], check=True)
        subprocess.run(["git", "-C", str(self.tree), "-c", "user.name=t", "-c", "user.email=t@t",
                        "commit", "-qm", "x"], check=True)
        self.results = self.root / "results"
        self.results.mkdir()
        self.log = self.root / "run.log"
        self.log.write_text(LOG)
        self.config = self.root / "cfg.json"
        self.config.write_text(json.dumps({"match": MC, "prismatic": {"path": "ai/PrismaticAI"}}))
        self.binary = self.root / "openttd"
        self.binary.write_bytes(b"\0junk 20261122-master-gfdc13851cf\0")

    def tearDown(self):
        self.tmp.cleanup()

    def args(self, **over):
        a = dict(log=self.log, tree=self.tree, variant="now=x=1", seed=7, years=2, field="Hog,Admiral",
                 slot=0, config=self.config, binary=self.binary, note="Why.")
        a.update(over)
        return argparse.Namespace(**a)

    def test_a_run_is_recorded_from_its_log(self):
        rec = run_quiet(ss.import_record, self.args(), self.results)[0]
        self.assertTrue(rec["id"].startswith("ablation-") and rec["id"].endswith("-seed7-now"))
        self.assertEqual(rec["rev"], subprocess.run(["git", "-C", str(self.tree), "rev-parse", "HEAD"],
                                                    capture_output=True, text=True).stdout.strip())
        self.assertFalse(rec["tree_dirty"])
        self.assertEqual(rec["imported"]["binary"], "20261122-master-gfdc13851cf")
        self.assertEqual(rec["imported"]["note"], "Why.")
        self.assertEqual(rec["rows"][0]["exit_code"], None)
        self.assertEqual(rec["match_cfg"]["years"], 2)
        self.assertTrue((self.results / f"{rec['id']}.md").is_file())
        self.assertEqual(battle.validate_ablation(json.loads((self.results / f"{rec['id']}.json").read_text()),
                                                  f"{rec['id']}.json"), [])
        with self.assertRaises(ss.Unreadable):   # the same run twice
            run_quiet(ss.import_record, self.args(), self.results)

    def test_dirty_tree_unnamed_binary_and_no_note(self):
        (self.tree / "ai" / "PrismaticAI" / "main.nut").write_text("changed")
        self.binary.write_bytes(b"no version here")
        rec = run_quiet(ss.import_record, self.args(note=None, variant="P R=x"), self.results)[0]
        self.assertTrue(rec["tree_dirty"])
        self.assertEqual(rec["imported"]["binary"], "openttd")
        self.assertNotIn("note", rec["imported"])
        self.assertTrue(rec["id"].endswith("-p-r"))
        rec = run_quiet(ss.import_record, self.args(binary=None, seed=8), self.results)[0]
        self.assertNotIn("binary", rec["imported"])

    def test_unreadable_runs_are_refused(self):
        self.log.write_text("no timestamps\n")
        with self.assertRaises(ss.Unreadable):
            ss.import_record(self.args(), self.results)
        self.log.write_text(LOG)
        with self.assertRaises(ss.Unreadable):   # the field names one AI twice
            ss.import_record(self.args(field="Hog,Hog"), self.results)

    def test_main_dispatches(self):
        battle_results = battle.RESULTS_DIR
        ss.battle.RESULTS_DIR = self.results
        try:
            argv = ["import", "--log", str(self.log), "--tree", str(self.tree), "--variant", "now=x=1",
                    "--seed", "9", "--years", "2", "--field", "Hog,Admiral", "--config", str(self.config)]
            self.assertEqual(run_quiet(ss.main, argv)[0], 0)
            code, _, err = run_quiet(ss.main, argv)
            self.assertEqual(code, 10)
            self.assertIn("exists already", err)
        finally:
            ss.battle.RESULTS_DIR = battle_results
        original = ss.render
        ss.render = lambda check: 42 if check else 43
        try:
            self.assertEqual(ss.main(["render", "--check"]), 42)
            self.assertEqual(ss.main(["render"]), 43)
        finally:
            ss.render = original


if __name__ == "__main__":
    unittest.main()
