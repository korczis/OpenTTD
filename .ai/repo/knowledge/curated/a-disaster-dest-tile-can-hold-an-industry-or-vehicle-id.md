---
schema: knowledge/v1
id: a-disaster-dest-tile-can-hold-an-industry-or-vehicle-id
kind: knowledge
class: fact
title: "A disaster vehicle's dest_tile holds an IndustryID (airplane, helicopter) or a VehicleID (small UFO) once it has a target"
description: "When reading, drawing, exporting or validating dest_tile of a DisasterVehicle."
status: verified
epistemics: observed
date: 2026-11-22
tags: [vehicles, consists-groups-and-timetables]
about: src/disaster_vehicle.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/disaster_vehicle.cpp
    - file:src/saveload/vehicle_sl.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/consists-groups-and-timetables/README.md
---

# Statement

`src/disaster_vehicle.cpp:DisasterTick_Aircraft` reads `Industry::Get(v->dest_tile.base())` and
`src/disaster_vehicle.cpp:DisasterTick_Ufo` stores `TileIndex{u->index.base()}` of the targeted road
vehicle in `dest_tile`; `src/disaster_vehicle.cpp:ReleaseDisastersTargetingIndustry` compares it to
an `IndustryID`, and `src/saveload/vehicle_sl.cpp:AfterLoadVehiclesPhase2` turns it back into a
`RoadVehicle`. Code that treats every vehicle's `dest_tile` as a map tile (a viewer, an exporter, an
invariant check) gets a meaningless or out-of-map tile for these states. Check `state` and
`subtype` first.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote a-disaster-dest-tile-can-hold-an-industry-or-vehicle-id` with evidence, or `majordomus knowledge reject a-disaster-dest-tile-can-hold-an-industry-or-vehicle-id --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/disaster_vehicle.cpp:369 (DisasterTick_Ufo, from :328) stores `TileIndex{u->index.base()}`; :379 reads it back as RoadVehicle::Get.
- src/disaster_vehicle.cpp:462 and :480 (DisasterTick_Aircraft, from :447) read `Industry::Get(v->dest_tile.base())`; :496 stores `TileIndex{ind.base()}`.
- src/disaster_vehicle.cpp:999-1000 (ReleaseDisastersTargetingIndustry, :994) compares dest_tile with the IndustryID ("yes, dest_tile is IndustryID here").
- src/saveload/vehicle_sl.cpp:535 (AfterLoadVehiclesPhase2, :435) `RoadVehicle::GetIfValid(v->dest_tile.base())`.
