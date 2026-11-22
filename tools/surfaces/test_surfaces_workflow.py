""".github/workflows/surfaces.yml never pushes to a pull request's branch, and cancels only off master.

A `refresh` job once merged master into every open auto-merge branch as github-actions[bot] and
dispatched Surfaces on it; a bot-pushed head never merged (its pull_request workflows end
`action_required`, and the pull request stayed BLOCKED until a human pushed), so the job only
stranded pull requests and filled the runner queue. These checks keep it from coming back
unnoticed: only the `wiki` job may write contents (it pushes to the wiki repository), nothing
dispatches a workflow or runs refresh-branch with --push, and superseded runs are cancelled on
every ref except the default branch. The workflow is read as text, no YAML library needed. Run by
the Surfaces workflow: python3 -m unittest tools/surfaces/test_surfaces_workflow.py
"""
import pathlib
import re
import unittest

WORKFLOW = pathlib.Path(__file__).resolve().parents[2] / ".github/workflows/surfaces.yml"
CANCEL = ("cancel-in-progress: ${{ github.ref != format('refs/heads/{0}', "
          "github.event.repository.default_branch) }}")


def jobs(text):
    """Each job's name and its lines, from the two-space keys under `jobs:`."""
    body = text.split("\njobs:\n", 1)[1]
    found, name = {}, None
    for line in body.splitlines():
        m = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if m:
            name = m[1]
            found[name] = []
        elif name:
            found[name].append(line)
    return found


class SurfacesWorkflow(unittest.TestCase):
    def setUp(self):
        self.text = WORKFLOW.read_text()
        self.jobs = jobs(self.text)

    def code(self):
        """The workflow without its comments: a note may name what is gone."""
        return "\n".join(line for line in self.text.splitlines()
                         if not line.lstrip().startswith("#"))

    def test_the_jobs_are_check_pages_and_wiki(self):
        self.assertEqual(sorted(self.jobs), ["check", "pages", "wiki"])

    def test_only_the_wiki_job_writes_contents(self):
        writers = sorted(n for n, lines in self.jobs.items()
                         if any(line.strip() == "contents: write" for line in lines))
        self.assertEqual(writers, ["wiki"])

    def test_nothing_dispatches_a_workflow_or_pushes_a_branch(self):
        code = self.code()
        self.assertNotIn("gh workflow run", code)
        self.assertNotIn("refresh-branch --push", code)
        self.assertNotIn("actions: write", code)

    def test_superseded_runs_are_cancelled_except_on_the_default_branch(self):
        top = self.text.split("\njobs:\n", 1)[0]
        block = top.split("\nconcurrency:\n", 1)[1].split("\n\n", 1)[0]
        self.assertIn("group: surfaces-${{ github.ref }}", block)
        self.assertIn(CANCEL, block)


if __name__ == "__main__":
    unittest.main()
