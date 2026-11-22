---
schema: knowledge/v1
id: cheats-are-not-recorded-in-the-gamelog
kind: knowledge
class: fact
title: "Classic cheats leave no gamelog entry; a savegame records them only as been_used flags in the CHTS chunk"
description: "When reconstructing a savegame's history for research provenance, or adding a cheat."
status: verified
epistemics: observed
date: 2026-11-22
tags: [goals-story-news-and-history, saveload]
about: src/gamelog.h
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/gamelog.h
    - file:src/gamelog.cpp
    - file:src/cheat_gui.cpp
    - file:src/saveload/cheat_sl.cpp
    - file:src/settings.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/goals-story-news-and-history/README.md
---

# Statement

`src/gamelog.h:GamelogActionType` declares `Cheat`, but no code calls
`_gamelog.StartAction(GamelogActionType::Cheat)`; only an assert in `src/gamelog.cpp:Gamelog::Mode`
mentions it. `src/cheat_gui.cpp:CheatWindow` sets `*ce->been_used = true` and writes the cheat's
variable directly; `src/saveload/cheat_sl.cpp` saves those `been_used` flags. When, how often and to
what value a cheat was used is not recorded anywhere. Sandbox settings in the same window are
different: they go through `SetSettingValue`, and `src/settings.cpp:IntSettingDesc::ChangeValue`
logs them as a `Setting` change. Provenance that needs cheat use must read `CHTS`, not the gamelog.

# Review

Authored from a read of the source at f8b1d52e80 (issue I0066); awaiting the owner's review.
`majordomus knowledge promote cheats-are-not-recorded-in-the-gamelog` with evidence, or `majordomus knowledge reject cheats-are-not-recorded-in-the-gamelog --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/gamelog.h:22 declares GamelogActionType::Cheat; the only use in src/ is the assert at src/gamelog.cpp:388 (grep for GamelogActionType::Cheat).
- src/cheat_gui.cpp:464 and :625 set `*ce->been_used = true`; src/saveload/cheat_sl.cpp:21-29, :42-43 save the been_used flags in CHTS.
- src/cheat_gui.cpp:572, :619 sandbox settings go through SetSettingValue; src/settings.cpp:1643-1644 IntSettingDesc::ChangeValue logs them as GamelogActionType::Setting.
