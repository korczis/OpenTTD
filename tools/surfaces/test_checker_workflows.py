"""Pull-request workflows queue a runner only when they can say something new.

Every pull request started seven workflows, so a burst of rebases (one per open pull request
after a master merge) queued forty and more runs on the account's shared hosted runners, and
the required "Check and build" of the pull request that fixes master waited behind checks of
files no pull request had touched. These checks keep the queue proportional to the change:
- the upstream checkers that read only `src/` and their own scripts run only when those change;
- the workflow that carries the required check (surfaces.yml) has no path filter, because a
  check that never reports blocks the merge;
- every pull-request workflow cancels its superseded runs off the default branch.
The workflows are read as text, no YAML library needed. Run by the Surfaces workflow:
python3 -m unittest tools/surfaces/test_checker_workflows.py
"""
import pathlib
import re
import unittest

WORKFLOWS = pathlib.Path(__file__).resolve().parents[2] / ".github/workflows"

# Each path-filtered checker, and the script that must stay inside its own filter.
FILTERED = {
    "docs-checker": ".github/sort_doxygen_warnings.py",
    "unused-strings": ".github/unused-strings.py",
    "script-missing-mode-enforcement": ".github/script-missing-mode-enforcement.py",
    "file-descriptions": ".github/file-descriptions.py",
}


def trigger(text):
    """The `on:` block, up to the next top-level key."""
    return re.search(r"^on:\n((?:[ #].*\n|\n)*)", text, re.M)[1]


def pr_paths(text):
    """The paths listed under `pull_request: paths:`, in order."""
    block = trigger(text).split("  pull_request:\n", 1)[1]
    listed = re.match(r"    paths:\n((?:    - .*\n)+)", block)[1]
    return re.findall(r"^    - '([^']+)'$", listed, re.M)


def alternatives(regex):
    """The literal paths a `^(a|b/(c|d)/|e$)` pattern names: `a`, `b/c/`, `b/d/`, `e`."""
    body = regex[2:-1]
    tops, depth, start = [], 0, 0
    for i, ch in enumerate(body + "|"):
        depth += (ch == "(") - (ch == ")")
        if ch == "|" and depth == 0:
            tops.append(body[start:i])
            start = i + 1
    out = []
    for top in tops:
        m = re.fullmatch(r"(.*?)\(([^()]*)\)(.*)", top) or re.fullmatch(r"(.*)()()", top)
        for name in m[2].split("|"):
            out.append((m[1] + name + m[3]).replace("\\.", ".").rstrip("$"))
    return out


class CheckerWorkflows(unittest.TestCase):
    def read(self, name):
        return (WORKFLOWS / f"{name}.yml").read_text()

    def test_each_checker_runs_only_when_its_inputs_change(self):
        for name, script in FILTERED.items():
            paths = pr_paths(self.read(name))
            self.assertIn(f".github/workflows/{name}.yml", paths, name)
            self.assertIn(script, paths, name)
            self.assertTrue(any(p.startswith("src/") for p in paths), name)

    def test_research_rust_filters_on_exactly_the_paths_its_job_checks(self):
        text = self.read("research-rust")
        regex = "".join(re.findall(r"pattern\+?='([^']*)'", text))
        named = sorted(n.rstrip("/") for n in alternatives(regex))
        filtered = sorted(p[:-3] if p.endswith("/**") else p for p in pr_paths(text))
        self.assertEqual(filtered, named)
        self.assertIn("tools/research/mcp-rhai", filtered)

    def test_the_required_check_is_never_path_filtered(self):
        on = trigger(self.read("surfaces"))
        self.assertIn("  pull_request:\n", on)
        self.assertNotIn("paths:", on)
        self.assertNotIn("paths-ignore:", on)
        self.assertIn("name: Check and build", self.read("surfaces"))

    def test_every_pull_request_workflow_cancels_superseded_runs(self):
        for path in sorted(WORKFLOWS.glob("*.yml")):
            text = path.read_text()
            on = trigger(text)
            if "pull_request:" not in on or "pull_request_target" in on:
                continue
            top = text.split("\njobs:\n", 1)[0]
            self.assertIn("\nconcurrency:\n", top, path.name)
            self.assertIn("cancel-in-progress:", top, path.name)

