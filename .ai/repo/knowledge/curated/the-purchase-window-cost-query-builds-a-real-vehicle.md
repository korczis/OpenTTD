---
schema: knowledge/v1
id: the-purchase-window-cost-query-builds-a-real-vehicle
kind: knowledge
class: fact
title: "Selecting an engine in the purchase window with a cargo filter builds, refits and sells a real vehicle on the local client"
description: "When changing BuildVehicleWindow::SelectEngine, CmdBuildVehicle's refit path, or anything a vehicle build triggers (pool, group statistics, events)."
status: verified
epistemics: observed
date: 2026-11-22
tags: [vehicles, consists-groups-and-timetables, commands]
about: src/build_vehicle_gui.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/build_vehicle_gui.cpp
    - file:src/vehicle_cmd.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/consists-groups-and-timetables/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/vehicles-and-pathfinding/README.md
---

# Statement

`src/build_vehicle_gui.cpp:BuildVehicleWindow::SelectEngine` calls
`Command<Commands::BuildVehicle>::Do(DoCommandFlag::QueryCost, ...)` directly from GUI code, outside
the network path. When the chosen cargo differs from the engine's default,
`src/vehicle_cmd.cpp:CmdBuildVehicle` upgrades its sub-flags to `Execute`+`AutoReplace`, really
builds and refits the vehicle, sells it again with `Commands::SellVehicle`, and restores the random
seeds (`SaveRandomSeeds`/`RestoreRandomSeeds`). The temporary vehicle passes through the shared
pool and the group statistics on that client only. Any side effect added to a build or sell path
that is not undone by the sell (an event, news, a NOSAVE counter, a pool-order assumption) leaks
into one client's state.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote the-purchase-window-cost-query-builds-a-real-vehicle` with evidence, or `majordomus knowledge reject the-purchase-window-cost-query-builds-a-real-vehicle --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/build_vehicle_gui.cpp:1379 (BuildVehicleWindow::SelectEngine, :1365) `Command<Commands::BuildVehicle>::Do(DoCommandFlag::QueryCost, ...)` directly.
- src/vehicle_cmd.cpp:134 refitting when the cargo differs from the default; :156 subflags gain Execute and AutoReplace; :160-161 SaveRandomSeeds.
- src/vehicle_cmd.cpp:230-236 sells it with Commands::SellVehicle and RestoreRandomSeeds.
