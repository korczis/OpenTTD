---
id: project.new-code-tested
version: 1
kind: rule
title: Every line a change adds or modifies is run by a test, and a Python line no test runs fails the gate
description: A change ships tests that run every line it adds or modifies. For Python this is enforced line by line. tools/coverage/changed-lines runs every test_*.py of the repository under line tracing, subprocesses included, and refuses each changed executable line no test ran, by file and line, unless an entry of tools/coverage/exemptions.toml says why it cannot run. It runs in tools/gate.sh change and in CI (surfaces.yml). C++, Rust, JavaScript, Squirrel and shell are not measured yet; every run names their changed files and never refuses them.
statement: Ship every change with tests that run every line it adds or modifies; keep tools/coverage/changed-lines passing, which refuses any changed Python line no test runs (file:line) and accepts an untestable one only through an exemption with a written reason that every run prints; until the other languages are measured, cover their changed lines with tests anyway and say in the report that no gate measured them.
status: active
class: blocking
depends_on: [project.never-report-unrun-validation-as-pass@1, project.web-ui-is-covered-end-to-end@2]
tags: [testing, coverage, python, quality, gate]
---

# Rationale

The owner, 2026-11-22: every change ships tests that run every line it adds or modifies. This
is to be enforced, not advised: a blocking project rule plus a coverage gate in verify and CI
that refuses any changed line no test ran, by file and line. An exemption carries a written
reason and is listed in every run, and the check is proven against a change it must refuse.

A green suite says nothing about a line it never reached. This fork's tools grow quickly. Their
tests mostly drive them as subprocesses, through scratch repositories, or by loading a script
without an extension. A per-file percentage would hide exactly the new branch that no test
reaches. The gate therefore judges only the lines the change touched, and each one by name.

# Required behaviour

- **The change** is the diff from a base to the working tree. The base is `--base`, else
  `COVERAGE_BASE` (CI: the pull request's base, or the commit before the push), else the merge
  base with `origin/HEAD` (or `origin/master`). Where that is HEAD itself, the base is HEAD when
  the working tree holds the change, else HEAD's first parent. Untracked files count whole.
- **Judged**: the added or modified lines of every Python file. That means `*.py`, and a file
  without an extension whose first line is a python shebang. The tests themselves (`test_*.py`)
  and vendored code (`src/3rdparty/`) are not judged. Only executable lines count: those a
  compiled code object maps an instruction to. Blank lines, comments and folded constants never
  count.
- **The tests are derived**: every tracked or untracked `test_*.py`, one
  `python3 -m unittest discover` per directory that holds one. No list is kept by hand. A failing
  suite, or one that runs no test, fails the gate.
- **Tracing reaches subprocesses.** `tools/coverage/hook/` goes on `PYTHONPATH` with
  `CHANGED_LINES_DATA`, and its `sitecustomize.py` starts the tracer in every Python process a
  test starts. The tracer uses `sys.monitoring` on 3.12+ and falls back to `sys.settrace`; it is
  stdlib only. A file run from a scratch copy counts for the repository file whose path it ends
  with, when the bytes are identical.
- **Exemptions** live in `tools/coverage/exemptions.toml`, as `[[exempt]]` entries. Each has a
  `path` glob, a non-empty `reason`, and optionally a `function` (dotted qualname, decorators
  included) or a `match` (a substring of the exempted lines). Every entry is printed with its
  reason and its hit count on every run. An entry without a reason, with an unknown key, or one
  that matches nothing in the repository fails the gate. An exemption is for a line that cannot
  run under a test, such as an interpreter-version fallback or a call into Blender. It is never
  for a line that merely has no test yet.
- **Not yet measured**: C++, Rust, JavaScript, Squirrel, Rhai and shell. Every run names their
  changed files under "not judged" and never refuses them. The rule still asks for their tests.
  A report must say that no gate measured those lines, and must not imply that one did.
- **Enforced**: `tools/gate.sh change` runs it, in place of the old discovery of the research
  tool tests, and so does CI's `Check and build` job (`.github/workflows/surfaces.yml`) on
  every pull request and every push to master.

# Failure behaviour

`tools/coverage/changed-lines` exits 1 and prints one line per refusal:
`missing <file>:<line>: no test runs this line: <code>`, or `problem ...` for a failing suite, a
file that does not compile, or a bad or stale exemption. It exits 2 when there is no base to
measure from. A repository is never judged against nothing.

Some processes are not seen: one started with `-I` or `-E`, one whose environment drops
`PYTHONPATH`, and one that ends through `os._exit()` or a signal. A line that only such a
process runs needs a test that runs it in-process, or an exemption that says so.

# Verification

```sh
python3 -B -m unittest discover -s tools/coverage -p 'test_*.py'   # the gate's own proof
tools/coverage/changed-lines                                        # judge the current change
tools/coverage/changed-lines --base <ref>                           # judge any range
```

`test_refuses_a_changed_line_no_test_runs` is the change the gate must refuse: a function added
to a scratch repository without a test, refused at its file and line. The neighbouring cases
accept the same change once a test runs it, and show that subprocesses and scratch copies count.
They also cover exemptions with and without a reason, stale entries, failing suites, other
languages and the base resolution. The suite runs the gate's own files at 100 % of their lines
with no exemption.

# Related

Rules `project.never-report-unrun-validation-as-pass` and `project.web-ui-is-covered-end-to-end`
(the browser half of the same demand); `tools/coverage/README.md`.
