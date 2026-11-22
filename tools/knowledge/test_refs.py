#!/usr/bin/env python3
"""Tests for refs.py against a throwaway git repository: python3 -m unittest tools/knowledge/test_refs.py"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("refs", HERE / "refs.py")
refs = importlib.util.module_from_spec(spec)
sys.modules["refs"] = refs
spec.loader.exec_module(refs)

RECORD = """---
schema: context/v1
id: probe
status: active
tracks:
  - 'src/net/**'
---

# Probe

- `src/net/server.cpp:Server::Accept` accepts.
- `src/core/pool.hpp:Pool` and the `src/core/` directory.
- A branch `research/topic` and a method `tools/list` are not paths.
- A local path `.ai/local/state` is ignored.

```
`src/gone.cpp` inside a fence is not read
```
"""


def sh(root: Path, *args: str) -> str:
    return subprocess.run(args, cwd=root, check=True, capture_output=True, text=True).stdout


class RefsTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        sh(self.root, "git", "init", "-q", "-b", "main")
        sh(self.root, "git", "config", "user.email", "t@example.invalid")
        sh(self.root, "git", "config", "user.name", "t")
        self.write("src/net/server.cpp", "void Server::Accept() {}\n")
        self.write("src/core/pool.hpp", "template <class T> struct Pool {};\n")
        self.write("research/notes.md", "notes\n")
        self.write("tools/x.sh", "true\n")
        self.write(".gitignore", ".ai/local/\n")
        self.write(".ai/repo/map/README.md", RECORD)
        self.commit("base")

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def write(self, path: str, text: str) -> None:
        p = self.root / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")

    def commit(self, msg: str) -> str:
        sh(self.root, "git", "add", "-A")
        sh(self.root, "git", "commit", "-q", "-m", msg)
        return sh(self.root, "git", "rev-parse", "HEAD").strip()

    def run_cli(self, *argv: str) -> tuple[int, str]:
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = refs.main(["--repo", str(self.root), *argv])
        return code, buf.getvalue()

    def test_check_passes_on_resolving_references(self) -> None:
        code, out = self.run_cli("check")
        self.assertEqual(code, 0, out)
        self.assertIn("0 failure(s)", out)

    def test_fence_branch_method_and_ignored_paths_are_not_references(self) -> None:
        repo = refs.Repo.load(self.root)
        labels = {r.label() for r in refs.scan(repo, ".ai/repo/map/README.md")}
        self.assertEqual(labels, {"tracks 'src/net/**'", "src/net/server.cpp:Server::Accept",
                                  "src/core/pool.hpp:Pool", "src/core"})

    def test_missing_symbol_file_and_empty_tracks_fail(self) -> None:
        self.write(".ai/repo/bad/README.md", "---\nid: bad\ntracks:\n  - 'lib/**'\n---\n\n"
                   "`src/net/server.cpp:Reject` and `src/net/client.cpp` and `src/nowhere/`\n")
        code, out = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn("does not contain 'Reject'", out)
        self.assertIn("src/net/client.cpp is not a tracked file", out)
        self.assertIn("src/nowhere is not a directory", out)
        self.assertIn("tracks 'lib/**' matches no tracked file", out)

    def test_untracked_new_record_is_checked_before_git_add(self) -> None:
        self.write(".ai/repo/new/README.md", "`src/net/server.cpp:Missing`\n")
        code, out = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn(".ai/repo/new/README.md", out)

    def test_inside_a_hook_a_file_written_but_not_added_does_not_resolve(self) -> None:
        # A record citing an unadded file would pass a pre-commit check and fail in CI, whose
        # checkout does not have it: inside a hook (GIT_INDEX_FILE set) only the index counts.
        self.write("src/net/client.cpp", "void Client::Join() {}\n")
        self.write(".ai/repo/new/README.md", "`src/net/client.cpp:Client::Join`\n")
        sh(self.root, "git", "add", ".ai/repo/new/README.md")
        code, out = self.run_cli("check")
        self.assertEqual(code, 0, out)
        old = os.environ.get("GIT_INDEX_FILE")
        os.environ["GIT_INDEX_FILE"] = str(self.root / ".git" / "index")
        try:
            code, out = self.run_cli("check")
        finally:
            if old is None:
                os.environ.pop("GIT_INDEX_FILE")
            else:
                os.environ["GIT_INDEX_FILE"] = old
        self.assertEqual(code, 1, out)
        self.assertIn("src/net/client.cpp:Client::Join is not a tracked file", out)

    def test_superseded_record_is_history_and_not_judged(self) -> None:
        self.write(".ai/repo/old/README.md", "---\nid: old\nstatus: superseded\n---\n\n`src/gone.cpp`\n")
        code, _ = self.run_cli("check")
        self.assertEqual(code, 0)

    def test_index_maps_files_to_records_with_symbols(self) -> None:
        code, out = self.run_cli("index")
        self.assertEqual(code, 0)
        files = json.loads(out)["files"]
        self.assertEqual(files["src/net/server.cpp"][".ai/repo/map/README.md"], ["Server::Accept"])
        self.assertIn(".ai/repo/map/README.md", files["src/core/pool.hpp"])

    def test_impact_names_records_and_symbols_to_reread(self) -> None:
        base = sh(self.root, "git", "rev-parse", "HEAD").strip()
        self.write("src/net/server.cpp", "void Server::Accept() { /* changed */ }\n")
        self.write("src/core/new.hpp", "struct New {};\n")
        self.write("tools/x.sh", "false\n")
        self.commit("change")
        code, out = self.run_cli("impact", f"{base}..HEAD", "--json")
        self.assertEqual(code, 0)
        recs = json.loads(out)["records"]
        hit = recs[".ai/repo/map/README.md"]
        self.assertEqual(hit["src/net/server.cpp"]["symbols"], ["Server::Accept"])
        self.assertEqual(hit["src/net/server.cpp"]["via"], "reference")
        self.assertIn("src/core/new.hpp", hit)  # through the src/core/ directory reference
        self.assertNotIn("tools/x.sh", hit)

    def test_impact_skips_a_record_changed_in_the_same_range(self) -> None:
        base = sh(self.root, "git", "rev-parse", "HEAD").strip()
        self.write("src/net/server.cpp", "void Server::Accept() { /* changed */ }\n")
        self.write(".ai/repo/map/README.md", RECORD + "\nReviewed.\n")
        self.commit("change with review")
        _, out = self.run_cli("impact", f"{base}..HEAD", "--json")
        self.assertEqual(json.loads(out)["records"], {})

    def test_impact_reports_a_deleted_file_through_tracks(self) -> None:
        base = sh(self.root, "git", "rev-parse", "HEAD").strip()
        sh(self.root, "git", "rm", "-q", "src/net/server.cpp")
        self.commit("delete")
        _, out = self.run_cli("impact", f"{base}..HEAD", "--json")
        entry = json.loads(out)["records"][".ai/repo/map/README.md"]["src/net/server.cpp"]
        self.assertEqual(entry["status"], "D")

    def test_an_ignored_directory_that_does_not_exist_here_is_still_ignored(self) -> None:
        # `.ai/local/` is a directory pattern, and a fresh checkout has no `.ai/local` at all.
        self.assertFalse((self.root / ".ai/local").exists())
        self.write(".ai/repo/local/README.md", "`.ai/local` and `.ai/local/state` are this checkout's own\n")
        code, out = self.run_cli("check")
        self.assertEqual(code, 0, out)

    def test_line_citation_checks_the_file_only(self) -> None:
        self.write(".ai/repo/lines/README.md", "`src/net/server.cpp:999` and `src/net/gone.cpp:12-14`\n")
        code, out = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn("src/net/gone.cpp:12-14 is not a tracked file", out)
        self.assertNotIn("server.cpp", out)

    MATRIX = ("| row | code |\n|---|---|\n"
              "| accept | `src/net/server.cpp:2:Accept` |\n"
              "| range | `src/net/server.cpp:1-3:Server::Reject` |\n")

    def write_server(self) -> None:
        self.write("src/net/server.cpp", "// header\nvoid Server::Accept() {}\nvoid Server::Reject() {}\n")

    def test_an_anchored_line_must_hold_its_symbol(self) -> None:
        self.write_server()
        self.write("research/matrix.md", self.MATRIX)
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md", "--strict-lines")
        self.assertEqual(code, 0, out)
        self.assertIn("2 reference(s) in 1 record(s), 0 failure(s)", out)
        # The symbol moved down a line: named with where it is now.
        self.write("src/net/server.cpp", "// header\n// moved\nvoid Server::Accept() {}\nvoid Server::Reject() {}\n")
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md")
        self.assertEqual(code, 1, out)
        self.assertIn("src/net/server.cpp:2:Accept does not hold 'Accept' there; it is on line 3", out)
        self.assertIn("src/net/server.cpp:1-3:Server::Reject does not hold 'Reject' there; it is on line 4", out)
        self.assertIn("2 failure(s)", out)
        # Gone altogether: no line to move to.
        self.write("src/net/server.cpp", "// header\nvoid Server::Join() {}\n")
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md")
        self.assertEqual(code, 1)
        self.assertIn("src/net/server.cpp:2:Accept does not contain 'Accept'", out)
        self.assertIn("src/net/server.cpp:1-3:Server::Reject does not contain 'Reject'", out)

    def test_an_anchored_line_past_the_end_is_named_with_its_symbol(self) -> None:
        self.write_server()
        self.write("research/matrix.md", "`src/net/server.cpp:40:Accept` and `src/net/server.cpp:9:Gone`\n")
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md")
        self.assertEqual(code, 1)
        self.assertIn("src/net/server.cpp:40:Accept does not hold 'Accept' there; it is on line 2", out)
        self.assertIn("src/net/server.cpp:9:Gone does not contain 'Gone'", out)

    def test_many_occurrences_are_listed_five_at_most(self) -> None:
        self.write("src/net/server.cpp", "x\n" + "Accept\n" * 7)
        self.write("research/matrix.md", "`src/net/server.cpp:1:Accept`\n")
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md")
        self.assertEqual(code, 1)
        self.assertIn("it is on line 2, 3, 4, 5, 6 ...", out)

    def test_strict_lines_refuses_a_bare_line_number(self) -> None:
        self.write_server()
        self.write("research/matrix.md", "`src/net/server.cpp:2` and `src/net/server.cpp:7-9`\n")
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md")
        self.assertEqual(code, 0, out)  # without the flag only the file is checked, as before
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md", "--strict-lines")
        self.assertEqual(code, 1)
        self.assertIn("src/net/server.cpp:2 cites a line with no anchor", out)
        self.assertIn("src/net/server.cpp:7-9 has 3 line(s), so line 7-9 does not exist", out)
        self.write("research/matrix.md", "`src/net/server.cpp:5`\n")
        _, out = self.run_cli("check", "--pathspec", "research/matrix.md", "--strict-lines")
        self.assertIn("so line 5 does not exist", out)

    def test_fix_lines_moves_each_citation_to_its_symbol_and_keeps_the_rest(self) -> None:
        self.write_server()
        self.write("research/matrix.md", self.MATRIX + "`src/net/server.cpp:2:Accept` again, `src/net/x.cpp:1:Nope`\n")
        self.write("src/net/server.cpp", "// a\n// b\n// c\nvoid Server::Accept() {}\n// d\nvoid Server::Reject() {}\n")
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md", "--strict-lines", "--fix-lines")
        self.assertEqual(code, 1, out)  # x.cpp is not tracked: nothing can move it
        self.assertIn("MOVED research/matrix.md:3: src/net/server.cpp:2:Accept -> src/net/server.cpp:4:Accept", out)
        # The range keeps its length but is cut at the file's end.
        self.assertIn("src/net/server.cpp:1-3:Server::Reject -> src/net/server.cpp:6:Server::Reject", out)
        self.assertIn("MOVED research/matrix.md:5: src/net/server.cpp:2:Accept -> src/net/server.cpp:4:Accept", out)
        self.assertIn("src/net/x.cpp:1:Nope is not a tracked file", out)
        self.assertIn("1 failure(s)", out)
        text = (self.root / "research/matrix.md").read_text(encoding="utf-8")
        self.assertIn("| accept | `src/net/server.cpp:4:Accept` |", text)
        self.assertIn("`src/net/server.cpp:6:Server::Reject`", text)
        self.assertTrue(text.endswith("`src/net/x.cpp:1:Nope`\n"), "the rest of the record is kept")
        # Checked again, only the untracked file is left.
        code, out = self.run_cli("check", "--pathspec", "research/matrix.md", "--strict-lines")
        self.assertEqual(code, 1)
        self.assertIn("4 reference(s) in 1 record(s), 1 failure(s)", out)
        # A range whose symbol moved inside it stays as it is.
        self.write("research/matrix.md", "`src/net/server.cpp:3-6:Reject`\n")
        _, out = self.run_cli("check", "--pathspec", "research/matrix.md", "--fix-lines")
        self.assertNotIn("MOVED", out)
        # Nearest occurrence wins, and an earlier line breaks a tie.
        self.write("src/net/server.cpp", "Accept\nx\nx\nAccept\nx\nx\nAccept\n")
        self.write("research/matrix.md", "`src/net/server.cpp:5:Accept` and `src/net/server.cpp:2:Accept`\n")
        self.run_cli("check", "--pathspec", "research/matrix.md", "--fix-lines")
        self.assertEqual((self.root / "research/matrix.md").read_text(encoding="utf-8"),
                         "`src/net/server.cpp:4:Accept` and `src/net/server.cpp:1:Accept`\n")

    def test_fix_lines_needs_records_not_files(self) -> None:
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.run_cli("check", "--file", "research/notes.md", "--fix-lines")

    def test_anchored_label_and_moved_edge_cases(self) -> None:
        repo = refs.Repo.load(self.root)
        plain = refs.Ref("r.md", 1, "src/net/server.cpp", None, "file", (3, 3), None)
        self.assertEqual(plain.label(), "src/net/server.cpp:3")
        self.assertIsNone(refs.moved(repo, plain), "a bare line has nothing to follow")
        gone = refs.Ref("r.md", 1, "src/net/gone.cpp", None, "file", (1, 1), "Accept")
        self.assertIsNone(refs.moved(repo, gone))
        self.assertIsNone(refs.line_range(None))
        self.assertEqual(refs.line_range("4-9"), (4, 9))

    def test_coverage_names_files_no_record_tracks_and_ignores_umbrellas(self) -> None:
        self.write(".ai/repo/umbrella/README.md", "---\nid: u\ntracks:\n  - 'src/**'\n---\n")
        code, out = self.run_cli("coverage", "src/", "--json")
        self.assertEqual(code, 0)
        data = json.loads(out)
        self.assertEqual(data["files"], 2)
        self.assertEqual(data["uncovered"], ["src/core/pool.hpp"])
        self.assertEqual(data["records"]["src/net/server.cpp"], [".ai/repo/map/README.md"])
        code, out = self.run_cli("coverage", "src", "--strict")
        self.assertEqual(code, 1)
        self.assertIn("UNCOVERED src/core/ 1: pool.hpp", out)
        self.assertIn("1/2 file(s)", out)

    def test_glob_subset(self) -> None:
        rx = refs.glob_regex(":(glob)src/*_cmd.cpp")
        self.assertTrue(rx.match("src/road_cmd.cpp"))
        self.assertFalse(rx.match("src/sub/road_cmd.cpp"))
        self.assertTrue(refs.glob_regex("src/**").match("src/a/b/c.h"))
        self.assertTrue(refs.glob_regex("src/**/x.h").match("src/x.h"))


if __name__ == "__main__":
    unittest.main()
