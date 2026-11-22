"""Rule project.docs-are-derived-and-current: a chart is generated, and drawn by one layer.

tools/surfaces/surfaces.py check_charts flags a Mermaid chart (pie, xychart, ...) outside a
generated region; mermaid_chart_spec reads a generated chart back for the site's ECharts;
gen_toc lists the headings after its region; check_libraries refuses a second chart library.
Run by tools/gate.sh change (python3 -m unittest discover -s tools/research -p 'test_*.py').
"""
import json
import pathlib
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SURFACES = ROOT / "tools" / "surfaces"
PYTHON = SURFACES / ".venv" / "bin" / "python"

PROBE = """
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("surfaces", sys.argv[1])
s = importlib.util.module_from_spec(spec); sys.modules["surfaces"] = s; spec.loader.exec_module(s)
what, body = sys.argv[2], sys.stdin.read()
doc = s.Doc(path="x.md", text=body, fm=None, body=body, offset=0)
if what == "charts":
    print(json.dumps([[f.kind, f.line] for f in s.check_charts(doc, s.markdown().parse(body))]))
elif what == "spec":
    print(json.dumps(s.mermaid_chart_spec(body)))
elif what == "num":
    print(json.dumps(s.mermaid_num(float(body))))
elif what == "docsindex":
    spec = json.loads(body)
    docs = {}
    for path, (fm, text) in spec["docs"].items():
        d = s.Doc(path=path, text=text, fm=fm, body=text, offset=0)
        d.title, d.summary = path, ""
        docs[path] = d
    g = s.Regions(None, docs, None)
    print(json.dumps(g.render("docs-index", spec["args"], docs[spec["in"]])))
elif what == "toc":
    g = s.Regions(None, {"x.md": doc}, None)
    print(json.dumps(g.render("toc", {"depth": "3"}, doc, body.index("-->") + 4)))
"""


def run(what, body):
    out = subprocess.run([str(PYTHON), "-B", "-c", PROBE, str(SURFACES / "surfaces.py"), what],
                         input=body, capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


PIE = '```mermaid\npie showData title T\n    "a" : 2\n    "b" : 1\n```\n'


class ChartsAreGenerated(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not PYTHON.exists():
            raise unittest.SkipTest("no tools/surfaces/.venv yet: run tools/surfaces/surfaces check once")

    def test_a_hand_typed_chart_is_a_finding(self):
        self.assertEqual(run("charts", "# T\n\n" + PIE), [["chart", 3]])

    def test_a_chart_in_a_region_is_not(self):
        body = "<!-- surfaces:begin facts as=pie -->\n" + PIE + "<!-- surfaces:end facts -->\n"
        self.assertEqual(run("charts", body), [])
        body = "<!-- scoreboard:begin -->\n" + PIE + "<!-- scoreboard:end -->\n"
        self.assertEqual(run("charts", body), [])

    def test_a_flowchart_is_not_a_chart(self):
        self.assertEqual(run("charts", "```mermaid\nflowchart LR\n  a --> b\n```\n"), [])

    def test_a_chart_reads_back_for_echarts(self):
        self.assertEqual(run("spec", PIE.split("\n", 1)[1].rsplit("```", 1)[0]),
                         {"kind": "pie", "title": "T", "items": [["a", 2.0], ["b", 1.0]]})
        xy = ('---\nconfig:\n  x: 1\n---\nxychart-beta\n    title "V"\n    x-axis ["r1", "r2"]\n'
              '    y-axis "Value" 0 --> 10\n    bar [1, 2]\n    line [3, 4]\n')
        spec = run("spec", xy)
        self.assertEqual((spec["labels"], spec["y"], [x["type"] for x in spec["series"]]),
                         (["r1", "r2"], "Value", ["bar", "line"]))
        self.assertIsNone(run("spec", "flowchart LR\n  a --> b\n"))

    def test_series_names_and_links_read_back(self):
        xy = ('xychart-beta\n    title "V"\n    x-axis ["r1", "r2"]\n    %% series: Planned | Unplanned\n'
              '    %% links: a.md | b.md\n    y-axis "Value" 0 --> 10\n    bar [1, 2]\n    line [3, 4]\n')
        spec = run("spec", xy)
        self.assertEqual([x["name"] for x in spec["series"]], ["Planned", "Unplanned"])
        self.assertEqual(spec["links"], ["a.md", "b.md"])

    def test_large_numbers_are_plain_digits(self):
        self.assertEqual(run("num", "104400000.0"), "104400000")
        self.assertEqual(run("num", "2.857"), "2.857")

    def test_docs_index_shows_fields_and_pies_link_to_the_filtered_table(self):
        page = ("## Records\n\n<!-- surfaces:begin docs-index dir=k fields=status -->\n"
                "<!-- surfaces:end docs-index -->\n")
        docs = {"x.md": [None, page], "k/a.md": [{"status": "verified"}, ""],
                "k/b.md": [{"status": "candidate"}, ""], "k/c.md": [{"status": "verified"}, ""]}
        table = run("docsindex", json.dumps({"docs": docs, "in": "x.md", "args": {"dir": "k", "fields": "status"}}))
        self.assertIn("| [k/a.md](k/a.md) | verified |  |", table)
        pie = run("docsindex", json.dumps({"docs": docs, "in": "x.md", "args": {"dir": "k", "as": "pie", "by": "status"}}))
        self.assertIn('"verified" : 2', pie)
        self.assertIn("%% links: x.md?col=Status&filter=verified#records | x.md?col=Status&filter=candidate#records", pie)

    def test_the_toc_lists_the_headings_after_it(self):
        body = ("## Before\n\n<!-- surfaces:begin toc -->\n<!-- surfaces:end toc -->\n\n"
                "## One\n\n### Two\n\n```\n## not a heading\n```\n\n## One\n")
        self.assertEqual(run("toc", body), "- [One](#one)\n    - [Two](#two)\n- [One](#one-1)")


if __name__ == "__main__":
    unittest.main()
