---
id: project.new-code-is-covered
version: 1
kind: rule
title: Every line a change adds or modifies is run by a test, file by file and line by line
description: A change ships the tests that run every line it adds or modifies. For the files of the coverage table in tools/gate.sh (COVERAGE_TABLE), the change tier runs cargo llvm-cov over the binaries that build them and fails on any counted line of a file's criterion that no test ran, naming it by file, line and text. A line no test can run is listed in COVERAGE_EXEMPT with its reason, and every exemption is printed on every run.
statement: Ship tests that run every line you add or modify; for a file of COVERAGE_TABLE, let tools/gate.sh change run cargo llvm-cov and refuse any counted line of its criterion no test ran, restructure a line a test cannot reach so its logic is testable, and exempt only what still cannot run, in COVERAGE_EXEMPT, with its written reason.
status: active
class: blocking
depends_on: [project.never-report-unrun-validation-as-pass@1]
tags: [testing, coverage, validation, rust, battlebrain]
---

# Rationale

The owner's order of 2026-11-22: every change ships tests that run every line it adds or
modifies. In a supervised repository this is enforced by a blocking rule and by a coverage gate
that refuses any changed line no test ran, by file and line. An exemption carries a written reason
and is listed in every run, and a check is proven against the build it must refuse.

The gate already held one file to this (I0092: the Rhai host's Prismatic client at 100 % line
coverage). This rule turns that step
into a table, so every file it covers shares one implementation.

# Required behaviour

- **Tests come with the change.** A line a change adds or modifies is run by a test of the same
  change. A test that only compiles a line does not count; it must execute it.
- **The table says what is measured.** `COVERAGE_TABLE` in `tools/gate.sh` has one row per file
  of `tools/research/mcp-rhai`: the file, the test binaries whose tests run it, and its criterion.
  - `all`: every line llvm-cov counts in the file, the file's own tests included.
  - `items:<start>;<start>...`: only the named items of a file the rule covers in part. An item
    starts at the first line containing `<start>` (for example `fn apply_live`) and ends where its
    braces balance again. A `<start>` the file no longer contains fails the step, so a rename
    cannot silently drop a function from the criterion.
- **Covered today**, under `tools/research/mcp-rhai/`:
  `tools/research/mcp-rhai/src/prismatic.rs`; Battlebrain's
  `tools/research/mcp-rhai/src/battlebrain_contract.rs`,
  `tools/research/mcp-rhai/src/battlebrain_policy.rs` and
  `tools/research/mcp-rhai/src/bin/battlebrain.rs` (`all`); and in
  `tools/research/mcp-rhai/src/panel_decision.rs` the live-decision executor:
  `is_freight_slot_setting`, `live_expiry`, `decision_gate`, `apply_live` and the `LiveGame`
  adapter of the panel's game (`items`).
- **A file joins the table when its code must stay covered.** New code in a file of the table is
  covered by construction. A new file that carries a decision, a contract or an executor gets
  a row in the change that adds it.
- **Unreachable is restructured first.** A line a test cannot reach, such as a process entry, a
  real bind failure or a branch the types already exclude, is first restructured so its logic is
  testable: an entry that only hands its arguments to a function the tests call, a pure helper
  for the check, or dead code removed. Only what still cannot run is exempt.
- **An exemption is written down and printed.** `COVERAGE_EXEMPT` in `tools/gate.sh` holds
  `<file>|<item start>|<reason>`. The item is found as a criterion item is, by its text and not
  its line number, so an edit above it cannot move the exemption. One that names nothing fails
  the step. Every exemption is printed with its reason on every run of the change tier.
- **Proven against the build it must refuse.** A change to the step itself, or a new row, shows
  the step failing on a temporarily added line no test runs, then passing once it is removed.
  The report names both runs.

# Validator

`tools/gate.sh change`, step `new_code_coverage` (rule
`project.never-report-unrun-validation-as-pass` applies to its verdicts):

- it runs for every row whose file the change touches, staged or not;
- `cargo llvm-cov --release --no-default-features --bin <each binary of those rows> --lcov` runs
  their tests (the `store` feature gates no line of any row, and the step then needs no system
  libduckdb);
- each row's counted lines are checked against its criterion, and every line no test ran is
  printed as `<file>:<line>  <text>`;
- failing tests, a missed line, a stale criterion item or a stale exemption FAIL the tier;
- without `cargo-llvm-cov` the step is NOT RUN, so the tier is PARTIAL, never PASS;
- a later step's PARTIAL never hides this step's FAIL (`worst` in `tools/gate.sh`).

Majordomus has no validator for this rule. The gate step is its enforcement, and review checks
that a change to a decision, contract or executor file outside the table adds its row.

# Failure behaviour

A missed line blocks the change until a test runs it, it is restructured, or it is exempt with
a reason a reviewer accepts. A report that claims coverage without naming the step's run and its
output is a validation claim without evidence.

# Related

Rule `project.never-report-unrun-validation-as-pass`; I0092 (the Prismatic client at 100 %);
I0117 (the live-decision executor); `research/battlebrain/README.md`.
