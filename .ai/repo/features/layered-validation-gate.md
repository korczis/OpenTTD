---
schema: feature/v1
id: layered-validation-gate
kind: feature
title: Layered validation gate with a fail-closed report taxonomy
short_title: Validation gate
headline: One script checks a change at the cost its size needs, and a report can only say what actually ran.
summary: tools/gate.sh wraps the CMake/CTest toolchain in smoke, change and full tiers; research/README.md defines PASS, FAIL, PARTIAL, NOT RUN and NOT APPLICABLE.
status: draft
weight: 10
areas: [validation]
audiences: [coding-agent, research-owner]
rules: [project.never-report-unrun-validation-as-pass]
docs: [research/README.md, research/experiment-template.md, research/baseset-prerequisite-macos.md]
adrs: [adr-0002]
related: [research-experiment-cli]
tags: [gate, ctest, reporting]
---

## What it does

`smoke` runs the diff check and an incremental build, `change` adds a configure when needed, the
full ctest suite and an untracked-file review, and `full` rebuilds from scratch in
`build-fullgate/`. It never commits, never pushes and never leaves the repository.

## What it does not do

It does not lint or format, because the codebase has no such tool, and it does not install the
base graphics set its regression cases need on macOS.

## Maturity

`draft`, graded 2026-11-25 against [the levels](README.md#maturity-levels); demoted from `stable`, and no
longer featured, because nothing tests the gate itself.

- **shipped**: the tiers, among them `tools/gate.sh:627:cmd_change`, and the fail-closed exit
  taxonomy (`tools/gate.sh:12:EXIT_PARTIAL`, `tools/gate.sh:237:worst`).
- **missing**: `tested` and `gated`. No test runs a tier of `tools/gate.sh` or its `worst` and
  PARTIAL exit, and no CI job runs the gate. A scratch-repository `test_*.py` that drives a tier
  and proves a FAIL outranks a PARTIAL would supply both: `tools/coverage/changed-lines` runs every
  `test_*.py` (`tools/gate.sh:681:changed`, `.github/workflows/surfaces.yml:89:changed`).
- **documented**: `research/README.md`.
