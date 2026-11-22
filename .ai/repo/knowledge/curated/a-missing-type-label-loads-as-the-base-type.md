---
schema: knowledge/v1
id: a-missing-type-label-loads-as-the-base-type
kind: knowledge
class: fact
title: "A savegame whose rail or road type label no loaded NewGRF provides loads that track as plain rail, road or tram, without an error"
description: "When loading a savegame with a different NewGRF set, or reasoning about rail/road types read from a loaded map."
status: verified
epistemics: observed
date: 2026-11-22
tags: [rail, road, newgrf, saveload]
about: src/newgrf_railtype.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/newgrf_railtype.cpp
    - file:src/newgrf_roadtype.cpp
    - file:src/saveload/labelmaps_sl.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/rail-road-and-infrastructure/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/newgrf/README.md
---

# Statement

Tiles store a rail/road type index; the `RAIL` and `ROTT` chunks
(`src/saveload/labelmaps_sl.cpp:RAILChunkHandler`) save the label of every index.
`src/saveload/labelmaps_sl.cpp:AfterLoadLabelMaps` calls `src/newgrf_railtype.cpp:ConvertRailTypes`
and `src/newgrf_roadtype.cpp:ConvertRoadTypes`, which look each saved label up and, when no loaded
type has it (or a road label now names a tram type), map it to `RAILTYPE_RAIL`, `ROADTYPE_ROAD` or
`ROADTYPE_TRAM`, then rewrite every railway, crossing and road tile. No message is raised there, so a
loaded network can silently change gauge, power and speed limits.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote a-missing-type-label-loads-as-the-base-type` with evidence, or `majordomus knowledge reject a-missing-type-label-loads-as-the-base-type --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/saveload/labelmaps_sl.cpp:27-30 AfterLoadLabelMaps calls ConvertRailTypes and ConvertRoadTypes; RAIL :36-37 and ROTT :72-73 are table chunks of the labels.
- src/newgrf_railtype.cpp:200-203 an unknown label becomes RAILTYPE_RAIL; :213-233 rewrites the map tiles.
- src/newgrf_roadtype.cpp:233-236 an unknown label, or one whose road/tram type differs, becomes ROADTYPE_TRAM or ROADTYPE_ROAD.
- No Debug, GrfMsg or ShowErrorMessage in src/newgrf_railtype.cpp, src/newgrf_roadtype.cpp or src/saveload/labelmaps_sl.cpp (grep).
