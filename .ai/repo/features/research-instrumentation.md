---
schema: feature/v1
id: research-instrumentation
kind: feature
title: Compile-gated research instrumentation
short_title: Instrumentation
headline: Internal state becomes observable in research builds and provably absent from every other build.
summary: OPTION_RESEARCH_INSTRUMENTATION (default OFF) compiles RESEARCH-ONLY hooks such as the read-only research_status console command.
status: stable
weight: 20
featured: true
areas: [observability]
audiences: [coding-agent, research-owner]
rules: [project.research-code-is-gated-and-labelled, project.game-state-mutates-only-through-commands]
docs: [research/tooling.md]
adrs: [adr-0004]
related: [research-experiment-cli, embedded-mcp-server]
tags: [instrumentation, cmake, console]
---

## What it does

`research_status` prints one line: instrumentation state, tick, date, vehicle and company
counts. It mutates nothing.

## What it does not do

It is not a stable API, and an instrumented result says nothing about release behaviour without
separate evidence.

## Maturity

`stable`, graded 2026-11-25 against [the levels](README.md#maturity-levels).

- **shipped**: `cmake/Options.cmake:65:OPTION_RESEARCH_INSTRUMENTATION`, which defines the macro
  `OTTD_RESEARCH_INSTRUMENTATION` (`cmake/Options.cmake:139:add_definitions`); the command
  `src/console_cmds.cpp:1780:ConResearchStatus` inside its guard
  (`src/console_cmds.cpp:1770:OTTD_RESEARCH_INSTRUMENTATION`).
- **tested**: the gate's checks of both halves of the headline: a research build carries
  `research_status` (`tools/gate.sh:739:research_status`), a default build carries no string naming
  research (`tools/gate.sh:561:default_build_free_of_research`), and every source line naming
  research is inside a guard (`tools/gate.sh:499:research_code_is_guarded`).
- **gated**: `tools/gate.sh change` runs the second and third (`tools/gate.sh:666:research_code_is_guarded`,
  `tools/gate.sh:700:default_build_free_of_research`); the research tier
  (`tools/gate.sh:715:cmd_research`) runs the first.
- **documented**: `research/tooling.md`.
