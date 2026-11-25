#!/usr/bin/env python3
"""RESEARCH-ONLY. Tests for tools/research/mcp-ab's page, charts and company binding, offline: no
game, no panel, no stack (a run is driven against fake panels).

Run: python3 -B tools/research/test_mcp_ab.py
(or python3 -B -m unittest discover -s tools/research -p 'test_*.py', as tools/gate.sh change does).

The run's Python is loaded from the heredoc of tools/research/mcp-ab, so what is tested is what runs
(its run is under __main__, so loading it starts nothing).
"""
import contextlib
import io
import json
import re
import shutil
import subprocess
import sys
import tempfile
import types
import unittest
import urllib.error
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCRIPT = HERE / "mcp-ab"
VENV_PY = ROOT / "tools" / "surfaces" / ".venv" / "bin" / "python"


def load_ab():
    """The Python of mcp-ab's heredoc as a module."""
    m = re.search(r"<<'PY'\n(.*?)\nPY\n", SCRIPT.read_text(), re.S)
    mod = types.ModuleType("mcp_ab")
    exec(compile(m[1], f"{SCRIPT} (heredoc)", "exec"), mod.__dict__)
    return mod


ab = load_ab()

SPEC_PROBE = """
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("surfaces", sys.argv[1])
s = importlib.util.module_from_spec(spec); sys.modules["surfaces"] = s; spec.loader.exec_module(s)
print(json.dumps(s.mermaid_chart_spec(sys.stdin.read())))
"""


def row(month, date, value, road=1, planes=0, **over):
    return dict({"month": month, "date": date, "money": 100, "loan": 300000, "value": value, "trains": 0,
                 "road": road, "planes": planes, "ships": 0}, **over)


class Charts(unittest.TestCase):
    """The page's charts: derived from the record's two series, read back by the site, and rewritten
    in place by --render on every page, old ones included."""
    SERIES = {"A": [row(1, "1950-02-03", 10, planes=2), row(2, "1950-03-03", 20, road=2, planes=2),
                    row(3, None, 300000000, road=3, planes=2)],
              "B": [row(1, "1950-02-03", 50), row(2, "1950-03-03", 6, road=4), row(3, None, 7, road=4)]}

    def rec(self, **over):
        series = over.pop("series", self.SERIES)
        last = {k: (v[-1] if v else {}) for k, v in series.items()}
        return dict({"seed": 4242, "months": 3, "label": "", "plan": {"note": "planner: x"}, "series": series,
                     "final": last, "value_delta": last["A"].get("value", 0) - last["B"].get("value", 0)}, **over)

    def test_value_lead_and_fleet_per_month_link_each_month_to_its_row(self):
        region = ab.charts(self.rec(), "ab-x.md", ROOT)
        self.assertTrue(region.startswith(ab.CHARTS_BEGIN) and region.endswith(ab.CHARTS_END))
        self.assertEqual(region.count("```mermaid"), 3)
        self.assertIn('x-axis ["1950-02", "1950-03", "3"]', region)  # a month without a date keeps its number
        self.assertIn("    bar [10, 20, 300000000]\n    line [50, 6, 7]", region)  # value, A against B
        self.assertIn("    bar [-40, 14, 299999993]", region)  # A's lead, negative while B is ahead
        self.assertIn("    bar [3, 4, 5]\n    line [1, 4, 4]", region)  # trains + road + planes + ships
        self.assertIn("%% series: A (with planner) | B (no planner)", region)
        self.assertIn("%% links: ab-x.md?col=Month&filter=1#a-with-the-planners-air-pair | "
                      "ab-x.md?col=Month&filter=2#a-with-the-planners-air-pair | "
                      "ab-x.md?col=Month&filter=3#a-with-the-planners-air-pair", region)
        self.assertIn("ab-x.md?col=Month&filter=1#b-without-the-planner", region)  # the lead opens B's row
        self.assertIn("-44 -->", region)  # the lead's axis reaches below its lowest value
        self.assertNotIn("e+", region)

    def test_the_anchors_are_the_headings_the_page_writes(self):
        md = ab.markdown(self.rec(), "ab-x.md", ROOT)
        for heading in (ab.A_HEADING, ab.B_HEADING):
            self.assertIn(f"\n## {heading}\n", md)
        self.assertEqual(ab.github_slug(ab.A_HEADING), "a-with-the-planners-air-pair")
        self.assertEqual(ab.github_slug(ab.B_HEADING), "b-without-the-planner")

    def test_only_months_both_games_read_are_charted(self):
        series = {"A": self.SERIES["A"] + [row(4, "1950-05-03", 99)],
                  "B": [self.SERIES["B"][0], {"month": 2, "date": "1950-03-03"}, self.SERIES["B"][2]]}
        region = ab.charts(self.rec(series=series), "ab-x.md", ROOT)
        self.assertIn('x-axis ["1950-02", "3"]', region)
        self.assertIn("    bar [10, 300000000]\n    line [50, 7]", region)

    def test_a_month_without_a_fleet_drops_only_the_fleet_chart(self):
        series = {"A": self.SERIES["A"], "B": [dict(r, ships=None) for r in self.SERIES["B"]]}
        self.assertEqual(ab.charts(self.rec(series=series), "ab-x.md", ROOT).count("```mermaid"), 2)
        self.assertIn("nothing to chart", ab.charts(self.rec(series={"A": [], "B": []}), "ab-x.md", ROOT))

    @unittest.skipUnless(VENV_PY.exists(), "no tools/surfaces/.venv: run tools/surfaces/surfaces check once")
    def test_the_site_reads_each_chart_back_with_its_series_and_links(self):
        region = ab.charts(self.rec(), "ab-x.md", ROOT)
        kinds = []
        for block in region.split("```mermaid\n")[1:]:
            out = subprocess.run([str(VENV_PY), "-B", "-c", SPEC_PROBE, str(ROOT / "tools" / "surfaces" / "surfaces.py")],
                                 input=block.split("```", 1)[0], capture_output=True, text=True, check=True)
            spec = json.loads(out.stdout)
            self.assertEqual(len(spec["links"]), len(spec["labels"]))
            kinds.append([(x["type"], x["name"]) for x in spec["series"]])
        self.assertEqual(kinds, [[("bar", "A (with planner)"), ("line", "B (no planner)")], [("bar", "A minus B")],
                                 [("bar", "A (with planner)"), ("line", "B (no planner)")]])

    def test_a_new_page_carries_its_charts_as_render_derives_them(self):
        md = ab.markdown(self.rec(), "ab-x.md", ROOT)
        self.assertLess(md.index("**Final company value: A 300000000 vs B 7"), md.index(ab.CHARTS_BEGIN))
        self.assertLess(md.index(ab.CHARTS_END), md.index(f"## {ab.A_HEADING}"))
        self.assertEqual(ab.with_charts(md, ab.charts(self.rec(), "ab-x.md", ROOT)), md)
        self.assertIn("| 3 | None |  | 300000000 | 100 | 300000 | 3 | 2 |", md)  # a row naming no company
        self.assertIn("each of the\ncompany the arm's bot holds: the record does not say which.", md)

    def test_render_inserts_the_charts_above_the_a_table_and_then_checks_current(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / ab.STYLE).parent.mkdir(parents=True)
            shutil.copy(ROOT / ab.STYLE, root / ab.STYLE)
            results = root / "research" / "results"
            results.mkdir(parents=True)
            (results / "ab-1-seed4242.json").write_text(json.dumps(self.rec()))
            old = (f"# A/B\n\nThe reading, as written then.\n\n## {ab.A_HEADING}\n\n| Month | Date |\n|---|---|\n"
                   f"| 1 | 1950-02-03 |\n\n## {ab.B_HEADING}\n\n| Month | Date |\n|---|---|\n| 1 | 1950-02-03 |\n")
            page = results / "ab-1-seed4242.md"
            page.write_text(old)
            (results / "ab-2-seed1.md").write_text("no record, left alone\n")
            (results / "duel-1-seed4242.json").write_text(json.dumps(self.rec()))  # not an A/B record
            (results / "duel-1-seed4242.md").write_text("a duel page, left alone\n")
            with contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(ab.render_pages(root, check=True), 11)
            self.assertIn("ab-1-seed4242.md", err.getvalue())
            self.assertEqual(page.read_text(), old)  # --check writes nothing
            with contextlib.redirect_stdout(io.StringIO()) as out:
                self.assertEqual(ab.render_pages(root), 0)
            self.assertIn("1 of 1 page(s) rewritten", out.getvalue())
            new = page.read_text()
            self.assertTrue(new.startswith("# A/B\n\nThe reading, as written then.\n\n" + ab.CHARTS_BEGIN))
            self.assertIn(ab.CHARTS_END + f"\n\n## {ab.A_HEADING}", new)
            self.assertEqual(ab.render_pages(root, check=True), 0)
            # A hand edit inside the region is stale again, and a render puts it back.
            page.write_text(new.replace("bar [10, 20", "bar [11, 20"))
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(ab.render_pages(root, check=True), 11)
            with contextlib.redirect_stdout(io.StringIO()):
                ab.render_pages(root)
            self.assertEqual(page.read_text(), new)
            self.assertEqual((results / "ab-2-seed1.md").read_text(), "no record, left alone\n")
            self.assertEqual((results / "duel-1-seed4242.md").read_text(), "a duel page, left alone\n")

    def test_the_render_flag_starts_no_game_and_refuses_check_alone(self):
        r = subprocess.run([str(SCRIPT), "--check"], capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stderr.strip()), (2, "mcp-ab: --check goes with --render"))
        r = subprocess.run([str(SCRIPT), "--help"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0)
        self.assertIn("tools/research/mcp-ab --render [--check]", r.stdout)
        r = subprocess.run([str(SCRIPT), "--bogus"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)


def line(n, name, value, founded=1950, road=1, planes=0):
    """One company of the console's `companies` dump, as the game prints it."""
    return (f"#:{n}(Red) Company Name: '{name}'  Year Founded: {founded}  Money: 100  Loan: 300000  "
            f"Value: {value}  (T:0, R:{road}, P:{planes}, S:0)")


class FakePanels:
    """Two arms' panels for mcp-ab's main(), keyed by port: each serves GET /api/status, /api/bots,
    the game.month events (one more month on every read after the first), the game date and ai.log,
    and POST /api/console `companies` from its roster of the current month."""

    def __init__(self, arms):
        self.arms = arms  # port: {"ai", "bots", "rosters": [month 0's dump lines, month 1's, ...]}
        self.month = {p: 0 for p in arms}
        self.reads = {p: 0 for p in arms}

    def get(self, port, path):
        arm = self.arms[port]
        if path == "/api/status":
            return {"ai": arm["ai"], "ai_started": True, "events": [{"kind": "ai", "text": "planner: x"}]}
        if path == "/api/bots":
            if isinstance(arm["bots"], Exception):
                raise arm["bots"]
            return {"running": arm["bots"]}
        if path.startswith("/api/events?since=0&kinds=game.month"):
            if self.reads[port]:
                self.month[port] += 1
                assert self.month[port] < len(arm["rosters"]), f"month {self.month[port]} read past the rosters"
            self.reads[port] += 1
            return {"events": [{}] * self.month[port]}
        if path == "/api/resources/game/state":
            return {"game_date": f"1950-{self.month[port] + 1:02d}-01"}
        if path.startswith("/api/events?since=0&kinds=ai.log"):
            return {"events": [{"data": {"text": "built a bus line"}}]}
        raise AssertionError(path)

    def post(self, port, path, body):
        assert (path, body["command"]) == ("/api/console", "companies"), (path, body)
        return {"output": self.arms[port]["rosters"][self.month[port]]}


class TestedCompany(unittest.TestCase):
    """Every observation is of the company the arm's tested agent holds, told by --company or by the
    game, never of whichever company the dump lists first; a run that cannot tell it refuses."""
    RIVAL_FIRST = [line(1, "Rival Transport", 999999), line(2, "Prismatic Transport", 1234, road=3)]

    def fake(self, **arms):
        panels = FakePanels(arms)
        return mock.patch.multiple(ab, get=panels.get, post=panels.post), panels

    def test_the_tested_company_is_observed_when_the_dump_lists_another_first(self):
        rows = ab.companies(self.RIVAL_FIRST)
        self.assertEqual([r["id"] for r in rows], [1, 2])
        got = ab.observe(rows, {"company": 2, "founded": 1950})
        self.assertEqual((got["company"], got["name"], got["value"], got["road"], got["present"]),
                         (2, "Prismatic Transport", 1234, 3, True))

    def test_the_first_row_never_stands_in_for_the_tested_company(self):
        source = re.search(r"<<'PY'\n(.*?)\nPY\n", SCRIPT.read_text(), re.S)[1]
        self.assertNotIn("rows[0]", source)
        gone = ab.observe(ab.companies(self.RIVAL_FIRST[:1]), {"company": 2, "founded": 1950})
        self.assertEqual(gone, {"company": 2, "present": False, "note": "#2 is not among the game's companies"})
        taken = ab.observe(ab.companies([line(2, "Newcomer", 5, founded=1952)]), {"company": 2, "founded": 1950})
        self.assertEqual((taken["present"], "value" in taken), (False, False))
        self.assertIn("#2 is now 'Newcomer', founded 1952: not the tested company (founded 1950)", taken["note"])

    def test_the_game_tells_the_company_of_the_panels_ai(self):
        bots = [{"id": 0, "ai": "RivalAI", "running": True}, {"id": 1, "ai": "PrismaticAI", "running": True},
                {"id": 2, "ai": "PrismaticAI", "running": False}]
        patch, _ = self.fake(**{"7": {"ai": "PrismaticAI", "bots": bots, "rosters": [self.RIVAL_FIRST]}})
        with patch:
            self.assertEqual(ab.bind("7"), {"company": 2, "name": "Prismatic Transport", "founded": 1950,
                                            "from": "GET /api/bots: the one running company of PrismaticAI"})
            self.assertEqual(ab.bind("7", "1")["name"], "Rival Transport")  # --company wins over the game

    def test_a_company_that_cannot_be_told_is_refused(self):
        two = [{"id": 0, "ai": "PrismaticAI"}, {"id": 3, "ai": "PrismaticAI"}]
        cases = [({"ai": "", "bots": [], "rosters": [self.RIVAL_FIRST]}, "", "the panel names no AI"),
                 ({"ai": "PrismaticAI", "bots": two, "rosters": [self.RIVAL_FIRST]}, "",
                  "2 running companies of PrismaticAI (#1, #4): pass --company N"),
                 ({"ai": "PrismaticAI", "bots": urllib.error.URLError("refused"), "rosters": [self.RIVAL_FIRST]}, "",
                  "GET /api/bots did not answer"),
                 ({"ai": "PrismaticAI", "bots": [{"id": 0, "ai": "RivalAI"}], "rosters": [self.RIVAL_FIRST]}, "",
                  "no running company of PrismaticAI in GET /api/bots after 4 s"),
                 ({"ai": "PrismaticAI", "bots": [{"id": 4, "ai": "PrismaticAI"}], "rosters": [self.RIVAL_FIRST]}, "",
                  "#5 is not among the game's companies after 4 s"),
                 ({"ai": "PrismaticAI", "bots": [], "rosters": [self.RIVAL_FIRST]}, "9",
                  "#9 is not among the game's companies after 4 s")]
        for arm, given, why in cases:
            patch, _ = self.fake(**{"7": arm})
            with self.subTest(why=why), patch, mock.patch.object(ab.time, "sleep") as sleep:
                with self.assertRaises(ab.Unbound) as cm:
                    ab.bind("7", given, tries=2, pause=2.0)
                self.assertIn(why, str(cm.exception))
                self.assertLessEqual(sleep.call_count, 1)

    def run_main(self, arms, company=""):
        patch, _ = self.fake(**arms)
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        root = Path(d.name)
        (root / ab.STYLE).parent.mkdir(parents=True)
        shutil.copy(ROOT / ab.STYLE, root / ab.STYLE)
        with patch, mock.patch.object(ab.time, "sleep"), contextlib.redirect_stdout(io.StringIO()) as out, \
                contextlib.redirect_stderr(io.StringIO()) as err:
            try:
                ab.main(["mcp-ab", "1", "2", "3", "4242", "", str(root), company])
                code = 0
            except SystemExit as e:
                code = e.code
        results = root / "research" / "results"
        records = sorted(results.glob("ab-*.json")) if results.is_dir() else []
        return code, out.getvalue(), err.getvalue(), [json.loads(r.read_text()) for r in records], \
            [r.with_suffix(".md").read_text() for r in records]

    def test_a_roster_change_mid_run_keeps_every_month_on_the_tested_company(self):
        # A: a rival is #1 and the bot #2; a company joins as #3 in month 2, the rival leaves in month 3
        # (the bot is then first in the dump). B: the bot is #1; it goes bankrupt in month 2 and a
        # company founded in 1951 takes #1, which is not the bot and is not reported as it.
        a = {"ai": "PrismaticAI", "bots": [{"id": 0, "ai": "RivalAI"}, {"id": 1, "ai": "PrismaticAI"}],
             "rosters": [self.RIVAL_FIRST,
                         [line(1, "Rival Transport", 900), line(2, "Prismatic Transport", 10)],
                         [line(1, "Rival Transport", 800), line(2, "Prismatic Transport", 20), line(3, "Joiner", 7)],
                         [line(2, "Prismatic Transport", 30, planes=2), line(3, "Joiner", 8)]]}
        b = {"ai": "PrismaticAI", "bots": [{"id": 0, "ai": "PrismaticAI"}],
             "rosters": [[line(1, "Plain Transport", 1)], [line(1, "Plain Transport", 5)],
                         [line(1, "Successor", 66666, founded=1951)], [line(1, "Successor", 77777, founded=1951)]]}
        code, out, err, (rec,), (page,) = self.run_main({"1": a, "2": b})
        self.assertEqual((code, err), (0, ""))
        self.assertIn("A observes #2 'Prismatic Transport' (GET /api/bots: the one running company of PrismaticAI)", out)
        self.assertEqual({k: (t["company"], t["name"]) for k, t in rec["tested"].items()},
                         {"A": (2, "Prismatic Transport"), "B": (1, "Plain Transport")})
        self.assertEqual([(r["company"], r["value"]) for r in rec["series"]["A"]], [(2, 10), (2, 20), (2, 30)])
        self.assertEqual([(r["company"], r["present"], r.get("value")) for r in rec["series"]["B"]],
                         [(1, True, 5), (1, False, None), (1, False, None)])
        self.assertTrue(all(r["company"] for side in rec["series"].values() for r in side))  # each names its company
        self.assertEqual(rec["value_delta"], 30)
        self.assertIn("A #2 'Prismatic Transport' (founded 1950, from GET /api/bots: the one running company of "
                      "PrismaticAI); B #1 'Plain Transport'", page)
        self.assertIn("| 3 | 1950-04-01 | #2 Prismatic Transport | 30 |", page)
        self.assertIn("| 2 | 1950-03-01 | #1 (absent) |  |", page)
        self.assertNotIn("66666", page)
        self.assertIn('x-axis ["1950-02"]', page)  # only the month both arms observed their company is charted

    def test_the_given_company_binds_both_arms(self):
        arm = {"ai": "", "bots": [], "rosters": [self.RIVAL_FIRST] * 4}
        code, out, _, (rec,), _ = self.run_main({"1": arm, "2": dict(arm)}, company="2")
        self.assertEqual(code, 0)
        self.assertEqual([(t["company"], t["from"]) for t in rec["tested"].values()], [(2, "--company"), (2, "--company")])
        self.assertEqual([r["value"] for r in rec["series"]["A"] + rec["series"]["B"]], [1234] * 6)

    def test_a_run_that_cannot_tell_its_company_refuses_and_writes_nothing(self):
        arm = {"ai": "PrismaticAI", "bots": [{"id": 0, "ai": "PrismaticAI"}, {"id": 1, "ai": "PrismaticAI"}],
               "rosters": [self.RIVAL_FIRST]}
        code, out, err, records, _ = self.run_main({"1": arm, "2": dict(arm)})
        self.assertEqual((code, records, out), (3, [], ""))
        self.assertIn("mcp-ab: A: refused, the tested company cannot be told (2 running companies of PrismaticAI "
                      "(#1, #2): pass --company N); no other company's figures are reported in its place", err)

    def test_a_company_that_is_not_a_console_number_is_refused_before_any_game(self):
        r = subprocess.run([str(SCRIPT), "--company", "first"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("--company is the console's company number", r.stderr)


if __name__ == "__main__":
    unittest.main()
