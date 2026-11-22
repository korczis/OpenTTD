---
schema: knowledge/v1
id: scripts-run-only-on-the-server-and-the-game-script-runs-while-paused
kind: knowledge
class: fact
title: "AIs and the Game Script run only on the server, and the Game Script keeps ticking while the game is paused"
description: "Before designing a bridge, MCP or experiment flow that pauses the game or expects script activity on clients."
status: verified
epistemics: observed
date: 2026-11-22
tags: [core-loop, scripting]
about: src/openttd.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/ai/ai_core.cpp
    - file:src/game/game_core.cpp
    - file:src/openttd.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/scripting/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/core-loop/README.md
---

# Statement

`src/ai/ai_core.cpp:AI::GameLoop` returns on clients (and when AIs in multiplayer are off), as does
`src/game/game_core.cpp:Game::GameLoop`. `src/openttd.cpp:StateGameLoop` still calls the Game Script in
its paused branch but not the AIs. A paused game therefore still has Game Script activity and no
AI activity.

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote scripts-run-only-on-the-server-and-the-game-script-runs-while-paused` with evidence, or `majordomus knowledge reject scripts-run-only-on-the-server-and-the-game-script-runs-while-paused --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/ai/ai_core.cpp:85 AI::GameLoop returns when _networking and (!_network_server or !ai_in_multiplayer); :41 clients do not start AIs.
- src/game/game_core.cpp:33 Game::GameLoop returns on a network client (`if (_networking && !_network_server)`).
- src/openttd.cpp:1216-1229 StateGameLoop's paused branch calls Game::GameLoop (:1227) and no AI code; :1273-1274 the running branch calls AI::GameLoop and Game::GameLoop.
