---
schema: knowledge/v1
id: a-heightmap-is-scaled-into-the-configured-map-and-sets-freeform-edges
kind: knowledge
class: fact
title: "A heightmap is scaled and padded into the map size from the settings, and its borders decide freeform_edges"
description: "When starting a game from a heightmap headlessly (-g <file>) and expecting the map size or edge behaviour to follow the image."
status: verified
epistemics: observed
date: 2026-11-22
tags: [heightmap, world-generation, cli]
about: src/heightmap.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/heightmap.cpp
    - file:src/openttd.cpp
    - file:src/genworld_gui.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/world-generation/README.md
---

# Statement

`src/openttd.cpp:MakeNewGame` generates a heightmap game at `1 << map_x` by `1 << map_y` from the
game settings; only the GUI path reads the image size (`src/heightmap.cpp:GetHeightmapDimensions`
in `src/genworld_gui.cpp`) to offer it. `GreyscaleToMapHeights` in `src/heightmap.cpp` then scales
the image to fit (honouring `heightmap_rotation`) and centres it with padding.
`src/heightmap.cpp:LoadHeightmap` finally sets `construction.freeform_edges` to
`!IsMapSurroundedByWater()`, overriding the configured value. A headless `-g <file>` run therefore
needs `map_x`/`map_y` (and `heightmap_height`) in its config to get the intended map.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote a-heightmap-is-scaled-into-the-configured-map-and-sets-freeform-edges` with evidence, or `majordomus knowledge reject a-heightmap-is-scaled-into-the-configured-map-and-sets-freeform-edges --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/openttd.cpp:908 (MakeNewGame, :897) calls GenerateWorld(GWM_HEIGHTMAP or GWM_NEWGAME, 1 << map_x, 1 << map_y); :1130 the same for SwitchMode::LoadHeightmap.
- src/heightmap.cpp:524 GetHeightmapDimensions is called only from src/genworld_gui.cpp:1043.
- src/heightmap.cpp:326-380 GreyscaleToMapHeights: scale and row_pad/col_pad per heightmap_rotation (:341, :356, :360).
- src/heightmap.cpp:551 (LoadHeightmap, :537) `_settings_game.construction.freeform_edges = !IsMapSurroundedByWater();`.
