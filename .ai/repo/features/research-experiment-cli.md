---
schema: feature/v1
id: research-experiment-cli
kind: feature
title: Research experiment CLI with manifest and evidence bundle
short_title: Research CLI
headline: An experiment records its own build profile, toolchain, commands and results, and its report is generated from that record.
summary: tools/research/research drives init, configure, build, run, validate, report and clean over four build profiles (research-debug, baseline-release, research-asan, research-mcp) and a versioned JSON manifest.
status: stable
weight: 30
featured: true
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

**stable** (definition: `README.md#what-a-status-means`).

- Complete: `tools/research/research` is on master with its four build profiles
  (`research/tooling.md`).
- Own check: `bash tools/research/test/research_test.sh`. It is run by hand, and neither CI nor a
  gate tier runs it.
- Last recorded pass: 51 passed and 0 failed at 99e98b8adc (master, 2026-11-25, I0112).
