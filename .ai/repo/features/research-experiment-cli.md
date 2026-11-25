---
schema: feature/v1
id: research-experiment-cli
kind: feature
title: Research experiment CLI with manifest and evidence bundle
short_title: Research CLI
headline: An experiment records its own build profile, toolchain, commands and results, and its report is generated from that record.
summary: tools/research/research drives init, configure, build, run, validate, report and clean over four build profiles (research-debug, baseline-release, research-asan, research-mcp) and a versioned JSON manifest.
status: draft
weight: 30
areas: [observability, validation]
audiences: [research-owner, coding-agent]
rules: [project.never-report-unrun-validation-as-pass]
docs: [research/tooling.md]
adrs: [adr-0005, adr-0006]
related: [research-instrumentation, layered-validation-gate]
tags: [reproducibility, manifest, evidence]
---

## What it does

It captures `research_status` from a headless dedicated server, refuses to silently reconfigure a
mismatched build directory, and keeps the system compiler whenever it supports
`<source_location>`.

## What it does not do

It does not make the stdin start-up race deterministic, and the `research-asan` profile is
configure-only.

## Maturity

`draft`, graded 2026-11-25 against [the levels](README.md#maturity-levels); demoted from `stable`, and no
longer featured, because its tests run only by hand.

- **shipped**: `tools/research/research:194:cmd_init`, `tools/research/research:433:cmd_validate`
  and `tools/research/research:481:cmd_report`.
- **tested**: `tools/research/test/research_test.sh` (assertion suite, `tools/research/test/research_test.sh:16:assert_eq`).
- **missing**: `gated`. No gate tier or CI job runs `tools/research/test/research_test.sh`; it is
  run by hand (`.ai/repo/workflows/upstream-sync.md:73:research_test`). A step of
  `tools/gate.sh:627:cmd_change` that runs it would promote it.
- **documented**: `research/tooling.md`.
