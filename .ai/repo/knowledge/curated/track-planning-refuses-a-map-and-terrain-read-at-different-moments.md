---
schema: knowledge/v1
id: track-planning-refuses-a-map-and-terrain-read-at-different-moments
kind: knowledge
class: constraint
title: "AlgoGlyph track-planning refuses openttd://map/terrain read on other land than openttd://map; one terraform between the two reads leaves every rail line of a plan without a price"
description: "When building a track-planning input from a running game (mcp-plan --track, yards, algoglyph.track_plan), or reading why a planner round priced none of its rail lines."
status: verified
epistemics: observed
date: 2026-11-22
tags: [planner, track-planning, algoglyph, rail, mcp]
about: tools/research/mcp-rhai/src/linecost.rs
route: project
provenance:
  origin: authored
  derived_from:
    - file:tools/research/mcp-rhai/src/linecost.rs
    - file:tools/research/mcp-rhai/src/hub_algoglyph.rs
    - file:tools/research/mcp-rhai/src/bin/mcp-plan.rs
    - file:tools/research/mcp-rhai/src/panel_bot.rs
    - file:research/mechanisms/planner-and-algoglyph.md
relations:
  - type: relates_to
    target: file:.ai/repo/knowledge/curated/the-bots-rail-pathfinder-lays-plain-track-only.md
---

# Constraint

AlgoGlyph's `track-planning` (checkout 1b8e009: `applyTerrain` in the track-planning plate's data
module, in the AlgoGlyph repository) checks the slope that openttd://map/terrain gives for each
tile against the slope that openttd://map's heights give. If one tile differs, it refuses the whole
input and prices no line: `terrain.slopes[y] says tile (x, y) has Slope a, its heights give b: read
map and terrain at the same moment`. The fork reads the input's resources one after another while
the game runs. hub_algoglyph.rs `track_input` reads openttd://map, map/ways, map/terrain,
economy/prices and bridges in that order. A terraform between the map read and the terrain read, by
a town's road or by any company, makes the input refuse.

mcp-plan reads that input once and shares it with every candidate, so a refusal is not one line's
failure. Every line priced in that round gets `error: track-planning refused: ...` and loses its
rail option.

Evidence (2026-11-22):

- OBSERVED, the outcome: the :8800 store of the seed 20261122 co-play game (8 companies) records
  30 planner rounds of that game's run that priced lines on the map. One priced 0 of 21: the
  replan round that began on 1954-01-16 (panel.ai seq 28283, plan.portfolio seq 28287). Each of
  the other 29 priced 10 to 20. The later run from the same world's save (1977 to 1981) recorded
  14 more rounds, each pricing 13 or 14. In the failing round, the call to AlgoGlyph's
  `knapsack-dp` succeeded (solver `algoglyph knapsack-dp`), so the runner worked.
- OBSERVED, the mechanism, on a copy of the same game's save (coplay-deploy-1626.sav, 1981) with
  its 8 AIs running: in 400 back-to-back reads of the five resources from the panel, 3 read
  terrain that did not match the map (4 to 8 tiles around (200, 16)). The flow CLI refused one of
  them with the message above, exit 1. The heights changed between 6 of the 399 pairs of
  consecutive reads.
- NOT OBSERVED: the refusal text of the January 1954 round itself. The panel logged only "0 with a
  route", so that this race caused it is INFERRED. It fits the evidence: all lines failed together,
  the runner worked, and no other round of that game failed. On the copy, 3 reads in 400 did not
  match, within a factor of five of 1 round in 30.

# How to apply it

- Build the input with linecost.rs `coherent_input`. It reads openttd://map once more after the
  rest. When the heights are the same, the terrain was read on that land. Otherwise it reads the
  whole input again, up to `READ_TRIES` times. If the land changes during every read, it leaves
  the terrain out instead of sending an input that will be refused. mcp-plan's answer records how
  it read the input in `line_costs.input`.
- A source that reads at one tick (read_many, a capture) always passes on the first read.
- The panel now names the reason for each line without a route (`planner: ... N without a route
  dropped their rail option (21x error: track-planning refused: ...)`). Read that before guessing
  at a map, a site or a limit.
- The hub's `algoglyph.track_plan` still reads through `track_input` with no second map read, so
  a call to it from a running game can be refused the same way.
