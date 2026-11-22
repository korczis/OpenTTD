---
schema: knowledge/v1
id: a-dragged-build-succeeds-when-any-piece-was-built
kind: knowledge
class: lesson
title: "A dragged rail or road build reports success when any piece was built; with the GUI's arguments it stops silently at the first obstacle after that"
description: "When a caller (GUI, the fork's mcp-human player, research code) builds track or road with BuildRailLong or BuildRoadLong and treats success as 'the whole run exists'."
status: verified
epistemics: observed
date: 2026-11-22
tags: [rail, road, commands, mcp-human]
about: src/rail_cmd.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/rail_cmd.cpp
    - file:src/road_cmd.cpp
    - file:src/rail_gui.cpp
    - file:src/road_gui.cpp
    - file:src/script/api/script_rail.cpp
    - file:src/script/api/script_road.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/rail-road-and-infrastructure/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/prismatic-mcp/README.md
---

# Statement

`src/rail_cmd.cpp:CmdRailTrackHelper` (behind `BuildRailLong`) and
`src/road_cmd.cpp:CmdBuildLongRoad` (behind `BuildRoadLong`) loop over the dragged tiles. With
`fail_on_obstacle` false (rail) or `is_ai` false (road), an error other than "already built" before
any success is skipped, and the first such error after a success ends the loop: the command
returns the cost of the pieces built so far and succeeds. "Already built" pieces count as success.
The GUI passes false (`src/rail_gui.cpp:DoRailroadTrack`, `src/road_gui.cpp:BuildRoadToolbarWindow`);
the script API passes true (`src/script/api/script_rail.cpp:ScriptRail::BuildRail`,
`src/script/api/script_road.cpp:ScriptRoad::_BuildRoadInternal`) and then fails on the first
obstacle. A caller that drives the GUI (the fork's `tools/research/mcp-human/src/play.rs`) must
verify the built run on the map (e.g. `openttd://map/ways`) rather than trust the result.

Also: the Doxygen of `src/rail_cmd.cpp:CmdBuildRailroadTrack` swaps the descriptions of
`auto_remove_signals` and `fail_on_obstacle`.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote a-dragged-build-succeeds-when-any-piece-was-built` with evidence, or `majordomus knowledge reject a-dragged-build-succeeds-when-any-piece-was-built --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/rail_cmd.cpp:897-926 CmdRailTrackHelper: ALREADY_BUILT sets had_success (:901, :913); another error returns at once only with fail_on_obstacle (:906), otherwise ends the loop if had_success (:907); :925 returns total_cost when had_success.
- src/road_cmd.cpp:1000-1048 CmdBuildLongRoad: the same with is_ai (:1021, :1025, :1026, :1029).
- GUI passes false: src/rail_gui.cpp:396-397 (DoRailroadTrack, :390), src/road_gui.cpp:819-820. Script API passes true: src/script/api/script_rail.cpp:376 (ScriptRail::BuildRail, :361), src/script/api/script_road.cpp:521 (_BuildRoadInternal, :510).
- src/rail_cmd.cpp:937-938: the Doxygen of CmdBuildRailroadTrack describes auto_remove_signals as "build up to an obstacle / fail if an obstacle is found" and fail_on_obstacle as the signal option: swapped.
- tools/research/mcp-human/src/play.rs:68, :395, :489 read openttd://map/ways to check what was built.
