"""tools/surfaces/findings.py: every source, every refusal, the CLI, and a surfaces sync/check
round trip, each against a scratch repository; and every line of findings.py runs.

Run by the Surfaces workflow (python3 -B -m unittest discover -s tools/surfaces -p 'test_*.py').
findings.py needs PyYAML, which the generator's environment holds (tools/surfaces/surfaces creates
tools/surfaces/.venv; CI runs surfaces check first). An interpreter without it runs this file
again under that environment, so the suite never passes by skipping.
"""
import contextlib
import importlib.util
import io
import json
import os
import pathlib
import runpy
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = HERE / "findings.py"
VENV_PY = HERE / ".venv" / "bin" / "python"

try:
    import yaml  # noqa: F401 - only probed: findings.py imports it
except ImportError:
    yaml = None

# Every line of findings.py runs: LINE events from the moment it is imported (sys.monitoring,
# Python 3.12+), compared at the end of the module with the lines its code objects hold.
# The first free tool id, the coverage slot preferred: the changed-lines tracer
# (tools/coverage) may already hold it in this very process.
TOOL = next(t for t in (sys.monitoring.COVERAGE_ID, 0, 2, 3, 4, 5)
            if sys.monitoring.get_tool(t) is None)
HITS: set[int] = set()


def _line(code, line):
    if code.co_filename == str(SOURCE):
        HITS.add(line)
    return sys.monitoring.DISABLE


if yaml is not None:
    sys.monitoring.use_tool_id(TOOL, "test_findings")
    sys.monitoring.register_callback(TOOL, sys.monitoring.events.LINE, _line)
    sys.monitoring.set_events(TOOL, sys.monitoring.events.LINE)
    spec = importlib.util.spec_from_file_location("findings_under_test", SOURCE)
    F = importlib.util.module_from_spec(spec)
    sys.modules["findings_under_test"] = F
    spec.loader.exec_module(F)


def executable_lines(path: pathlib.Path) -> set[int]:
    lines, todo = set(), [compile(path.read_text(), str(path), "exec")]
    while todo:
        code = todo.pop()
        lines.update(ln for _, _, ln in code.co_lines() if ln)  # 0: the module's RESUME
        todo.extend(c for c in code.co_consts if hasattr(c, "co_lines"))
    return lines


def tearDownModule():
    if yaml is None:
        return
    sys.monitoring.set_events(TOOL, 0)
    sys.monitoring.free_tool_id(TOOL)
    missed = sorted(executable_lines(SOURCE) - HITS)
    if missed:
        raise AssertionError(f"lines of findings.py no test ran: {missed}")
    print("\ntest_findings: every line of findings.py ran (exemptions: none)", file=sys.stderr)


KINDS = {
    "blocked": "waits", "out-of-scope": "left out", "not-run": "not run", "partial": "partly",
    "failed": "failed", "handoff": "asked elsewhere", "unreviewed": "awaits review", "drift": "live",
}
CONFIG = {
    "kinds": KINDS,
    "statuses": {"NOT RUN": "not-run", "PARTIAL": "partial"},
    "issues": {"in": "plan/*.yaml", "id": r"I\d{4}", "non_scope_of": ["done"]},
    "tables": {"in": ["docs/**/*.md"], "columns": ["Status"], "exclude": ["docs/vocabulary.md"]},
    "front_matter": [{"in": "know/*.md", "field": "status", "values": ["candidate"],
                      "kind": "unreviewed"}],
    "markers": {"in": ["docs/**/*.md"], "exclude": ["docs/vocabulary.md"]},
    "max_chars": 120,
}
LONG = "word " * 40

FILES = {
    "plan/I0001.yaml": """\
        id: I0001
        title: "The first, done"
        completed_at: 2026-11-22T10:00:00Z
        non_scope:
          - "Left out entirely"
          - "Taken over by I0002"
          - "Taken over by I0003, which is done"
          - "An outside tracker's I9999"
        evidence:
          - result: "Ran the suite. NOT RUN: the live check; it needs a panel. PARTIAL again. NOT RUN twice"
          - "a bare string"
        """,
    "plan/I0002.yaml": f"""\
        id: I0002
        title: "Second, open"
        depends_on: [I0001, I0003, I0004, I0099]
        evidence:
          - result: "NOT RUN while open"
          - result: "{LONG}then PARTIAL at the very end of a long sentence"
        non_scope:
          - "Open issues leave their non_scope alone"
        """,
    "plan/I0003.yaml": "id: I0003\ntitle: Third\ncompleted_at: 2026-11-22T10:00:00Z\n",
    "plan/I0004.yaml": "id: I0004\ntitle: Fourth\nverified_at: 2026-11-22T10:00:00Z\n",
    "plan/I0005.yaml": "id: I0005\ntitle: Fifth\nstarted_at: 2026-11-22T10:00:00Z\ndepends_on: [I0004]\n",
    "plan/I0006.yaml": "",
    "docs/table.md": """\
        # Table

        | Thing | Status |
        |---|---|
        | Live run | **NOT RUN**: needs I0002 |
        | Fine | PASS |
        | Short |
        | | PARTIAL |

        ```text
        | Thing | Status |
        |---|---|
        | Fenced | NOT RUN |
        ```

        | not a table header
        plain text
        """,
    "docs/vocabulary.md": "| Status | Meaning |\n|---|---|\n| NOT RUN | not run |\n",
    "docs/marked.md": """\
        # Marked

        Before. <!-- open-finding kind=failed -->None of 21 lines found a route.<!-- /open-finding --> After.

        <!-- open-finding kind=handoff follow-up=I0002 -->
        A paragraph asked of another repository,
        across two lines.

        <!-- open-finding kind=not-run -->Runs to the end of its paragraph, naming I0005
        and I0005 again. <!-- open-finding kind=partial -->Second marker in the paragraph.

        <!-- open-finding kind=failed follow-up=I0001 -->Owned by a done issue.

        `<!-- open-finding kind=failed -->` in backticks is an example.

        ```text
        <!-- open-finding kind=failed -->fenced example
        ```
        """,
    "docs/bad.md": """\
        <!-- open-finding kind=failed color=red -->unknown attribute

        <!-- open-finding follow-up=I0002 -->no kind

        <!-- open-finding kind=ghost -->undeclared kind

        <!-- open-finding kind=failed follow-up=I0777 -->no such issue

        stray <!-- /open-finding --> closing marker
        """,
    "docs/plain.md": "# Plain\n\nNothing marked here.\n",
    "know/c1.md": "---\nid: c-one\nstatus: candidate\ntitle: \"Reads I0002\"\n---\n# C1\n",
    "know/c2.md": "---\nstatus: verified\ntitle: Settled\n---\n",
    "know/c3.md": "# No front matter\n",
    "know/c4.md": "---\nstatus: candidate\nnever closed\n",
    "know/c5.md": "---\nstatus: [unclosed\n---\n",
    "know/c6.md": "---\n- a list\n---\n",
    "know/c7.md": "---\nstatus: candidate\ndescription: Only a description\n---\n",
}


def scratch(files: dict[str, str], config: dict | None = None) -> pathlib.Path:
    root = pathlib.Path(tempfile.mkdtemp(prefix="findings-"))
    for path, text in files.items():
        (root / path).parent.mkdir(parents=True, exist_ok=True)
        (root / path).write_text(textwrap.dedent(text))
    if config is not None:
        (root / "surfaces.toml").write_text(toml(config))
    subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True)
    subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
    subprocess.run(["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t", "commit",
                    "-qm", "scratch"], check=True)
    return root


def toml_value(v) -> str:
    if isinstance(v, dict):
        return "{ " + ", ".join(f'"{k}" = {toml_value(x)}' for k, x in v.items()) + " }"
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    if isinstance(v, int):
        return str(v)
    return json.dumps(v)


def toml(config: dict) -> str:
    return "[findings]\n" + "".join(f"{k} = {toml_value(v)}\n" for k, v in config.items())


def tracked(root: pathlib.Path) -> list[str]:
    out = subprocess.run(["git", "-C", str(root), "ls-files"], capture_output=True, text=True)
    return out.stdout.split()


@unittest.skipIf(yaml is not None, "PyYAML is here: the suite runs in this interpreter")
class InTheGeneratorsEnvironment(unittest.TestCase):
    def test_the_suite_passes_under_the_generators_python(self):
        self.assertTrue(VENV_PY.exists(), f"{VENV_PY} is missing: run tools/surfaces/surfaces check first")
        r = subprocess.run([str(VENV_PY), "-B", "-m", "unittest", "-v", "test_findings"], cwd=HERE,
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


@unittest.skipIf(yaml is None, "PyYAML is missing: InTheGeneratorsEnvironment runs the suite")
class Sources(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = scratch(FILES)
        files = dict(FILES)
        del files["docs/bad.md"]
        cls.good = scratch(files)
        cls.result = F.collect(cls.good, tracked(cls.good), CONFIG)
        cls.by = {}
        for f in cls.result.findings:
            cls.by.setdefault((f.kind, f.record or f.path), []).append(f)

    def test_a_clean_tree_has_no_declaration_errors(self):
        self.assertEqual(self.result.errors, [])
        self.assertEqual(self.result.issues["I0001"], "plan/I0001.yaml")

    def test_blocked_waits_on_what_is_not_done(self):
        (two,) = self.by[("blocked", "I0002")]
        self.assertEqual(two.follow_up, ["I0004"])
        self.assertIn("waits on I0004 (verified)", two.text)
        self.assertEqual((two.label, two.line), ("DERIVED", 3))
        (five,) = self.by[("blocked", "I0005")]
        self.assertEqual(five.follow_up, ["I0004"])
        self.assertNotIn(("blocked", "I0001"), self.by)

    def test_non_scope_of_a_done_issue_without_an_open_owner(self):
        texts = {f.text: f for f in self.by[("out-of-scope", "I0001")]}
        self.assertEqual(sorted(texts), ["An outside tracker's I9999", "Left out entirely",
                                         "Taken over by I0002"])
        self.assertEqual(texts["Taken over by I0002"].follow_up, ["I0002"])
        self.assertEqual(texts["Left out entirely"].line, 5)
        self.assertNotIn(("out-of-scope", "I0002"), self.by)

    def test_evidence_reports_each_open_status_once_with_its_sentence(self):
        (nr,) = self.by[("not-run", "I0001")]
        self.assertEqual(nr.text, "NOT RUN: the live check;")
        self.assertEqual((nr.follow_up, nr.label), ([], "OBSERVED"))
        (part,) = self.by[("partial", "I0001")]
        self.assertEqual(part.text, "PARTIAL again.")
        (own,) = self.by[("not-run", "I0002")]
        self.assertEqual(own.follow_up, ["I0002"])
        (late,) = self.by[("partial", "I0002")]
        self.assertTrue(late.text.startswith("PARTIAL at the very end"))

    def test_status_cells_of_declared_columns(self):
        (row,) = self.by[("not-run", "docs/table.md")]
        self.assertEqual((row.text, row.line, row.follow_up), ("Live run: NOT RUN: needs I0002", 5, ["I0002"]))
        (empty_name,) = self.by[("partial", "docs/table.md")]
        self.assertEqual(empty_name.text, ": PARTIAL")
        self.assertNotIn(("not-run", "docs/vocabulary.md"), self.by)

    def test_front_matter_values(self):
        (c1,) = self.by[("unreviewed", "c-one")]
        self.assertEqual((c1.text, c1.line, c1.follow_up), ("candidate: Reads I0002", 3, ["I0002"]))
        (c7,) = self.by[("unreviewed", "c7")]
        self.assertEqual(c7.text, "candidate: Only a description")

    def test_marked_passages(self):
        marked = [f for f in self.result.findings if f.path == "docs/marked.md"]
        got = [(f.kind, f.line, f.text, f.follow_up) for f in marked]
        self.assertIn(("failed", 3, "None of 21 lines found a route.", []), got)
        self.assertIn(("handoff", 5, "A paragraph asked of another repository, across two lines.",
                       ["I0002"]), got)
        self.assertIn(("not-run", 9, "Runs to the end of its paragraph, naming I0005 and I0005 again.",
                       ["I0005"]), got)
        self.assertIn(("partial", 10, "Second marker in the paragraph.", []), got)
        self.assertIn(("failed", 12, "Owned by a done issue.", ["I0001"]), got)
        self.assertEqual(len(marked), 5)

    def test_each_malformed_marker_is_named(self):
        errors = F.collect(self.root, tracked(self.root), CONFIG).errors
        self.assertEqual(errors, [
            "docs/bad.md:1: open-finding takes kind=K and follow-up=ID, not 'color=red'",
            "docs/bad.md:3: open-finding needs kind=K",
            "docs/bad.md:5: kind 'ghost' is not declared in [findings] kinds",
            "docs/bad.md:7: follow-up 'I0777' names no issue record",
            "docs/bad.md:9: /open-finding closes no open-finding in its paragraph",
        ])

    def test_an_undeclared_kind_from_a_status_is_an_error(self):
        cfg = {**CONFIG, "statuses": {"NOT RUN": "ghost"}}
        result = F.collect(self.good, tracked(self.good), cfg)
        self.assertIn("plan/I0001.yaml:10: kind 'ghost' is not declared in [findings] kinds", result.errors)
        self.assertEqual(result.findings[-1].kind, "ghost")

    def test_a_bare_configuration_reads_nothing(self):
        result = F.collect(self.good, tracked(self.good), {"issues": {"in": "plan/*.yaml"}})
        self.assertEqual({f.kind for f in result.findings}, {"blocked", "out-of-scope"})
        self.assertEqual(F.collect(self.good, [], {}).findings, [])
        self.assertIsNone(F.Sources(self.good, [], {}).status_kind("NOT RUN"))
        self.assertEqual(F.Sources(self.good, [], {}).open_ids("I0001"), [])


@unittest.skipIf(yaml is None, "PyYAML is missing: InTheGeneratorsEnvironment runs the suite")
class Live(unittest.TestCase):
    def live(self, *decls):
        root = scratch(FILES)
        return F.collect(root, tracked(root), {**CONFIG, "live": list(decls)}, live=True)

    def test_items_are_mapped_and_filtered(self):
        data = {"out": {"items": [
            {"issue": "I0002", "class": "merged_not_recorded", "remedy": "stamp it"},
            {"issue": "I0001", "class": "merged_not_recorded", "remedy": "names I0005"},
            {"issue": "X-1", "class": "merged_not_recorded", "remedy": "elsewhere"},
            {"issue": "I0003", "class": "advisory", "remedy": "dropped"},
        ]}}
        cmd = [sys.executable, "-c", f"print({json.dumps(json.dumps(data))})"]
        result = self.live({"id": "drift", "kind": "drift", "command": cmd, "items": "out.items",
                            "keep": {"class": ["merged_not_recorded"]}, "record": "issue",
                            "text": ["class", "remedy"]})
        live = [f for f in result.findings if f.kind == "drift"]
        got = [(f.record, f.path, f.line, f.follow_up, f.text) for f in live]
        self.assertEqual(got, [
            ("X-1", "drift", 0, [], "merged_not_recorded — elsewhere"),
            ("I0001", "plan/I0001.yaml", 1, ["I0005"], "merged_not_recorded — names I0005"),
            ("I0002", "plan/I0002.yaml", 1, ["I0002"], "merged_not_recorded — stamp it"),
        ])
        self.assertEqual(result.not_run, [])

    def test_a_source_that_cannot_answer_is_not_run(self):
        result = self.live(
            {"id": "missing", "kind": "drift", "command": ["no-such-command-findings-test"]},
            {"id": "fails", "kind": "drift", "command": [sys.executable, "-c", "import sys; sys.exit(3)"]},
            {"id": "garbled", "kind": "drift", "command": [sys.executable, "-c", "print('not json')"]},
            {"id": "slow", "kind": "drift", "timeout": 1,
             "command": [sys.executable, "-c", "import time; time.sleep(5)"]},
            {"id": "shapeless", "kind": "drift", "items": "a.b",
             "command": [sys.executable, "-c", "print('{\"a\": [1]}')"]},
        )
        ids = [line.split(":")[1].strip() for line in result.not_run]
        self.assertEqual(ids, ["missing", "fails", "garbled", "slow"])
        self.assertTrue(all(line.startswith("NOT RUN: ") for line in result.not_run))
        self.assertIn("exited 3", result.not_run[1])


@unittest.skipIf(yaml is None, "PyYAML is missing: InTheGeneratorsEnvironment runs the suite")
class Rendering(unittest.TestCase):
    def test_table_and_legend(self):
        result = F.Result([
            F.Finding("blocked", "a | b", "plan/I0002.yaml", 3, "I0002", ["I0004", "X-9"], "DERIVED"),
            F.Finding("failed", "line\nbreak", "docs/a.md", 7),
            F.Finding("drift", "live", "drift", 0, "X-1"),
            F.Finding("drift", "live", "drift", 0),
        ], [], [], {"I0002": "plan/I0002.yaml", "I0004": "plan/I0004.yaml"})
        out = F.render(result, {"kinds": {"blocked": "waits", "failed": "failed", "drift": "live"}},
                       lambda p: "../" + p)
        self.assertIn("| blocked | 1 | waits |", out)
        self.assertIn("| drift | 2 | live |", out)
        self.assertIn("| blocked | a \\| b | [I0002](../plan/I0002.yaml):3 | "
                      "[I0004](../plan/I0004.yaml), X-9 | DERIVED |", out)
        self.assertIn("| failed | line break | [docs/a.md](../docs/a.md):7 | none — needs an issue |", out)
        self.assertIn("| drift | live | `drift` (X-1) |", out)
        self.assertIn("| drift | live | `drift` | none — needs an issue | OBSERVED |", out)

    def test_helpers(self):
        self.assertTrue(F.glob_re("a/**/b?.md").match("a/x/y/bc.md"))
        self.assertTrue(F.glob_re("a/**").match("a/x/y"))
        self.assertFalse(F.glob_re("a/*.md").match("a/x/y.md"))
        self.assertTrue(F.matches("a/b.md", "a/*.md"))
        self.assertEqual(F.clip("one two three", 9), "one two…")
        self.assertEqual(F.clip("short", 9), "short")
        self.assertEqual(F.plain("[x](y) `c` **b** <i>t</i> <!-- n --> _e_"), "x c b t e")
        self.assertEqual(F.yaml_lines(""), {})
        self.assertEqual(F.fenced_lines(["a", "```", "b", "```", "c", "~~~"]), {1, 2, 3, 5})
        self.assertEqual(F.front_matter("no"), (None, 0))
        self.assertEqual(F.dig([1], "a"), {})
        self.assertEqual(F.dig({"a": 1}, ""), {"a": 1})


@unittest.skipIf(yaml is None, "PyYAML is missing: InTheGeneratorsEnvironment runs the suite")
class CommandLine(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        files = dict(FILES)
        del files["docs/bad.md"]
        cls.good = scratch(files, {**CONFIG, "live": [
            {"id": "gone", "kind": "drift", "command": ["no-such-command-findings-test"]}]})
        cls.bad = scratch(FILES, CONFIG)

    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = F.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def test_markdown_json_and_live(self):
        code, out, _ = self.run_main("--repo", str(self.good))
        self.assertEqual(code, 0)
        self.assertIn("| blocked | Second, open — waits on I0004 (verified) | [I0002](plan/I0002.yaml):3 |", out)
        code, out, _ = self.run_main("--repo", str(self.good), "--json")
        self.assertEqual((code, json.loads(out)["errors"]), (0, []))
        code, out, _ = self.run_main("--repo", str(self.good), "--live")
        self.assertIn("NOT RUN: gone:", out)

    def test_a_malformed_marker_fails(self):
        code, _, err = self.run_main("--repo", str(self.bad))
        self.assertEqual(code, 10)
        self.assertIn("FAIL findings docs/bad.md:3: open-finding needs kind=K", err)

    def test_run_as_a_script(self):
        argv, cwd = sys.argv, os.getcwd()
        sys.argv = [str(SOURCE), "--repo", str(self.good), "--json"]
        try:
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as done:
                runpy.run_path(str(SOURCE), run_name="__main__")
        finally:
            sys.argv = argv
            os.chdir(cwd)
        self.assertEqual(done.exception.code, 0)


@unittest.skipIf(yaml is None, "PyYAML is missing: InTheGeneratorsEnvironment runs the suite")
class SurfacesRoundTrip(unittest.TestCase):
    """The open-findings region through surfaces itself: sync writes it, check passes, a record
    change makes it stale, a malformed marker fails it."""

    PAGE = textwrap.dedent("""\
        # Findings page

        ## By kind

        <!-- surfaces:begin open-findings as=pie -->
        <!-- surfaces:end open-findings -->

        ## Findings

        <!-- surfaces:begin open-findings -->
        <!-- surfaces:end open-findings -->
        """)

    def surfaces(self, root, *argv):
        self.assertTrue(VENV_PY.exists(), f"{VENV_PY} is missing: run tools/surfaces/surfaces check first")
        return subprocess.run([str(VENV_PY), "-B", str(HERE / "surfaces.py"), "--repo", str(root), *argv],
                              capture_output=True, text=True, env={**os.environ, "SURFACES_GENERATING": "1"})

    def test_sync_check_stale_and_malformed(self):
        # know/c5.md's front matter does not parse, which surfaces check reports on its own.
        files = {k: v for k, v in FILES.items() if k.startswith(("plan/", "know/")) and k != "know/c5.md"}
        files["docs/findings.md"] = self.PAGE
        files["docs/marked.md"] = FILES["docs/marked.md"]
        root = scratch(files, CONFIG)
        r = self.surfaces(root, "sync")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        page = (root / "docs/findings.md").read_text()
        self.assertIn("pie showData title Open findings by kind", page)
        self.assertIn("findings.md?col=Kind&filter=blocked#findings", page)
        self.assertIn("| out-of-scope | Left out entirely | [I0001](../plan/I0001.yaml):5 | "
                      "none — needs an issue | DERIVED |", page)
        r = self.surfaces(root, "check")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

        four = root / "plan/I0004.yaml"
        four.write_text(four.read_text() + "completed_at: 2026-11-22T11:00:00Z\n")
        r = self.surfaces(root, "check")
        self.assertEqual(r.returncode, 10)
        self.assertIn("region 'open-findings' is stale: run tools/surfaces/surfaces sync", r.stdout)

        (root / "docs/marked.md").write_text("<!-- open-finding kind=ghost -->x\n")
        r = self.surfaces(root, "check")
        self.assertEqual(r.returncode, 10)
        self.assertIn("open findings: docs/marked.md:1: kind 'ghost' is not declared", r.stdout)


if __name__ == "__main__":
    unittest.main()
