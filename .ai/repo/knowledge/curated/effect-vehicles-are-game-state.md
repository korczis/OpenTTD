---
schema: knowledge/v1
id: effect-vehicles-are-game-state
kind: knowledge
class: constraint
title: "Effect vehicles (smoke, sparks, explosions, bubbles) are lockstep game state, not client-side decoration"
description: "When adding, removing or gating the creation of an effect vehicle, or treating smoke as purely visual."
status: verified
epistemics: observed
date: 2026-11-22
tags: [vehicles, consists-groups-and-timetables, determinism]
about: src/effectvehicle.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/effectvehicle.cpp
    - file:src/vehicle.cpp
    - file:src/saveload/vehicle_sl.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/consists-groups-and-timetables/README.md
---

# Statement

`src/effectvehicle.cpp:CreateEffectVehicle` takes a slot in the shared `VehiclePool`, and several
init/tick procs draw the game `Random()` (`ChimneySmokeInit`, `BubbleTick`; callers such as
`src/vehicle.cpp:Vehicle::ShowVisualEffect` use `Chance16`). Effect vehicles are saved in `VEHS`
(`animation_state`, `animation_substate` in `src/saveload/vehicle_sl.cpp`). Creating or skipping one
on a client-only condition (a client setting, transparency, a dedicated server) shifts the random
stream and later `VehicleID`s and desyncs. Only drawing (`EffectVehicle::GetTransparencyOption`) and
ambient sound may depend on client state. They are never put in the tile hash
(`src/vehicle.cpp:Vehicle::UpdatePositionAndViewport`), so they never block construction.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote effect-vehicles-are-game-state` with evidence, or `majordomus knowledge reject effect-vehicles-are-game-state --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/effectvehicle.cpp:593-597 CreateEffectVehicle checks Vehicle::CanAllocateItem and creates in the vehicle pool.
- src/effectvehicle.cpp:41 ChimneySmokeInit and :499, :516 BubbleTick draw Random().
- src/vehicle.cpp:2934, :2948 Vehicle::ShowVisualEffect (:2836) uses Chance16 with the game setting smoke_amount.
- src/saveload/vehicle_sl.cpp:1024-1025 save animation_state and animation_substate.
- src/effectvehicle.cpp:656 GetTransparencyOption is the drawing hook.
- src/vehicle.cpp:1784-1786 UpdatePositionAndViewport skips UpdatePosition (:1701-1703, the tile hash) for VehicleType::Effect.
