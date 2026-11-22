# Changed-line coverage

`changed-lines` is the gate of rule
[`project.new-code-tested`](../../.ai/repo/rules/project/new-code-tested.v1.md). Every Python
line a change adds or modifies must run under a test. A line that cannot run carries an
exemption with a written reason.

```sh
tools/coverage/changed-lines                 # the current change, against origin/master
tools/coverage/changed-lines --base <ref>    # any range: <ref> to the working tree
tools/coverage/changed-lines --list-tests    # the test directories it discovers
python3 -B -m unittest discover -s tools/coverage -p 'test_*.py'   # its own tests
```

## How it works

1. **The change.** `git diff -U0 -M <base>` gives the added and modified lines. Untracked
   files count whole. The base is `--base`, else `COVERAGE_BASE`, else the merge base with
   `origin/HEAD`. On the trunk itself it is HEAD (a dirty tree) or HEAD's first parent. A
   given base counts from its merge base with HEAD, or from the newest `origin/HEAD` commit
   that HEAD already holds when that is newer. A branch that merged a newer master than its
   pull request's `base.sha` is judged by its own lines only.
2. **What is judged.** Python files: `*.py`, and extensionless scripts with a python shebang.
   The tests (`test_*.py`) and `src/3rdparty/` are not judged. Only executable lines count,
   those a compiled code object maps an instruction to.
3. **The tests.** Every `test_*.py` git knows of. The gate runs one
   `python3 -m unittest discover` per directory that holds one, and skips a directory an
   ancestor's discovery already reaches. A failing suite fails the gate.
4. **Tracing.** [`hook/`](hook/) goes first on `PYTHONPATH`, with `CHANGED_LINES_DATA` set.
   Its `sitecustomize.py` starts [`_changed_lines_trace.py`](hook/_changed_lines_trace.py) in
   every Python process, so a tool a test runs as a subprocess is traced too. The tracer uses
   `sys.monitoring` on 3.12+, which reports each line once at little cost, and falls back to
   `sys.settrace` on older interpreters. At exit each process writes the files it ran, with
   their SHA-256 and their lines. A file run from a scratch copy counts for the repository file
   whose path it ends with, when the bytes match.
5. **The verdict.** Each changed executable line that no process ran is
   `missing <file>:<line>: no test runs this line: <code>`, and the gate exits 1. Otherwise it
   prints `changed-lines: PASS` and exits 0. Exit 2 is a usage error or no base.

## Exemptions

[`exemptions.toml`](exemptions.toml) holds `[[exempt]]` entries. Each has a `path` glob and a
`reason`, and optionally a `function` (a dotted qualname) or a `match` (a substring). The
gate prints every entry with its reason and hit count on every run. An entry without a reason,
with an unknown key, or that matches nothing fails the gate. Exempt a line no test can run,
such as a fallback for an older interpreter or a call into Blender. A line that merely lacks a
test is not exempt.

## Not measured yet

C++, Rust, JavaScript, Squirrel, Rhai and shell files are named under `not judged` in every
run and never refused. The rule still asks for their tests, and a report says that no gate
measured them. The Rhai host's Prismatic client is the one exception: `tools/gate.sh change`
requires 100 % line coverage of it through `cargo llvm-cov`.

## Blind spots

The tracer does not see:

- a process started with `-I` or `-E`, which ignores `PYTHONPATH`;
- a process whose environment drops `PYTHONPATH` or `CHANGED_LINES_DATA`;
- a process that ends through `os._exit()` or a signal, which writes no reading.

A line that only such a process runs needs an in-process test, or an exemption that says so.

## Where it runs

`tools/gate.sh change` runs it in place of the old discovery of the research tool tests. CI's
`Check and build` job in `.github/workflows/surfaces.yml` runs it on every pull request,
judged from the pull request's base, and on every push to master, judged from the commit
before the push.
