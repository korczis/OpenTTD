---
schema: knowledge/v1
id: desync-debug-level-two-changes-what-it-measures
kind: knowledge
class: lesson
title: "-d desync=2 recomputes caches every tick and runs the rail pathfinder twice, so never time an experiment with it on"
description: "When choosing debug flags for a measured run; record them as an observer effect."
status: verified
epistemics: observed
date: 2026-11-22
tags: [network, vehicles-and-pathfinding]
about: src/cachecheck.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/cachecheck.cpp
    - file:src/pathfinder/yapf/yapf_rail.cpp
    - file:src/command.cpp
    - file:src/debug.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/vehicles-and-pathfinding/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/network/README.md
---

# Statement

A numeric `-d` level is the index of `Severity` (`src/debug.cpp:SetDebugString`, `src/debug_type.h`):
1 is `Severity::Error`, 2 is `Severity::Warning`. `src/cachecheck.cpp:CheckCaches` returns early
unless `Facility::Desync` is visible at `Severity::Warning` (`-d desync=2`); from there it rebuilds and
compares caches each tick, and the rail pathfinder (`src/pathfinder/yapf/yapf_rail.cpp`) repeats each
choice with caching disabled. `Severity::Error` (`-d desync=1`) already logs every executed command to
`commands-out.log` (`src/command.cpp`, `src/debug.cpp:DebugPrint`) and writes `dmp_cmds` saves
(`src/openttd.cpp:StateGameLoop`), which `docs/desync.md` places at levels 2 and 3.

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote desync-debug-level-two-changes-what-it-measures` with evidence, or `majordomus knowledge reject desync-debug-level-two-changes-what-it-measures --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/debug_type.h:14-25 Severity: Critical 0, Error 1, Warning 2; src/debug.cpp:131, :159 SetDebugString casts the numeric level to Severity.
- src/cachecheck.cpp:41 CheckCaches returns unless Facility::Desync is visible at Severity::Warning (level 2).
- src/pathfinder/yapf/yapf_rail.cpp:439-449 at that level ChooseRailTrack runs a second pathfinder with DisableCache(true) (also :283, :360, :510).
- src/command.cpp:261 logs every command at Severity::Error; src/debug.cpp:89-90 writes Facility::Desync lines to commands-out.log; src/openttd.cpp:1246-1249 (StateGameLoop, :1209) writes dmp_cmds saves at Severity::Error (level 1).
- docs/desync.md:160-172 describes recording at '-d desync=2' and '-d desync=3'.
- Revised today: the statement names the Severity values instead of bare levels.
