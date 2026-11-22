---
schema: knowledge/v1
id: gfxloadsprites-loads-every-newgrf
kind: knowledge
class: fact
title: "GfxLoadSprites is the NewGRF load path, not only graphics: it runs LoadNewGRF on every peer, dedicated servers included"
description: "When moving, skipping or guarding a GfxLoadSprites call (headless runs, null blitter, research hooks), or when looking for where NewGRF game data is (re)built."
status: verified
epistemics: observed
date: 2026-11-22
tags: [graphics-and-sprites, newgrf]
about: src/gfxinit.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/gfxinit.cpp
    - file:src/genworld.cpp
    - file:src/saveload/afterload.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/graphics-and-sprites/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/newgrf/README.md
---

# Statement

`src/gfxinit.cpp:GfxLoadSprites` resets the sprite cache and calls `LoadSpriteTables`, which loads the
base set GRFs and then `LoadNewGRF(SPR_NEWGRFS_BASE, 2)`: every enabled NewGRF, with the engines,
industries and other features it defines. Nothing in that path checks the blitter, so a dedicated
server or a `-v null` run executes it too. It is called from world generation
(`src/genworld.cpp`), `src/saveload/afterload.cpp:AfterLoadGame` and
`src/saveload/afterload.cpp:ReloadNewGRFData`. Skipping it to save time in a headless run
changes the game's content, not just its pictures.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote gfxloadsprites-loads-every-newgrf` with evidence, or `majordomus knowledge reject gfxloadsprites-loads-every-newgrf --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/gfxinit.cpp:335-345 GfxLoadSprites clears caches and calls LoadSpriteTables; :172-215 LoadSpriteTables ends with LoadNewGRF(SPR_NEWGRFS_BASE, 2). No blitter check on that path (the blitter checks are in RealChangeBlitter/SwitchNewGRFBlitter, :231-263).
- Callers: src/genworld.cpp:327 (GenerateWorld, :291), src/saveload/afterload.cpp:799 (AfterLoadGame, :558), :3474 (ReloadNewGRFData, :3471).
