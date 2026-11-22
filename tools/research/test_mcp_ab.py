#!/usr/bin/env python3
"""RESEARCH-ONLY. Tests for tools/research/mcp-ab's page and charts, offline: no game, no panel, no
stack.

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
from pathlib import Path

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
        self.assertIn("| 3 | None | 300000000 | 100 | 300000 | 3 | 2 |", md)

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


if __name__ == "__main__":
    unittest.main()
