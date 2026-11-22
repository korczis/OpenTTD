#!/usr/bin/env python3
"""Unit tests for the pure parts of plan_github.py (no network, no majordomus)."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import plan_github as pg  # noqa: E402


class PlanGithubTest(unittest.TestCase):
    def test_marker_round_trips(self):
        path = ".ai/repo/project/issues/I0110.yaml"
        found = pg.MARKER_RE.search("body\n" + pg.marker(path) + "\n")
        self.assertEqual(found.group(1), path)

    def test_milestone_number_follows_plan_order_not_file_order(self):
        roadmap = [{"id": "b", "order": 20}, {"id": "a", "order": 10}, {"id": "c", "order": 100}]
        self.assertEqual(pg.milestone_number(roadmap, "a"), 1)
        self.assertEqual(pg.milestone_number(roadmap, "c"), 3)

    def test_titles_are_bounded(self):
        self.assertTrue(pg.issue_title("I0001", "x" * 400).startswith("I0001: "))
        self.assertLessEqual(len(pg.issue_title("I0001", "x" * 400)), 250)
        self.assertLessEqual(len(pg.milestone_title(10, "y" * 400)), 120)

    def test_linkify_only_touches_canonical_records(self):
        body = "see `.ai/repo/project/issues/I0096.yaml` and `tools/gate.sh`"
        out = pg.linkify(body, "owner/repo", "master")
        self.assertIn("(https://github.com/owner/repo/blob/master/.ai/repo/project/issues/I0096.yaml)", out)
        self.assertIn("`tools/gate.sh`", out)
        self.assertNotIn("blob/master/tools", out)

    def test_status_labels_cover_every_derived_status(self):
        for status in ("READY", "BLOCKED", "ACTIVE", "VERIFY", "DONE", "CANCELLED"):
            self.assertIn(status, pg.STATUS_LABELS)
        self.assertEqual(pg.CLOSED_STATES, {"DONE", "CANCELLED"})


if __name__ == "__main__":
    unittest.main()
