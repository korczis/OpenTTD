---
schema: knowledge/v1
id: game-state-is-owned-by-the-game-thread-not-the-main-thread
kind: knowledge
class: fact
title: "On threaded GUI drivers the game loop runs on the ottd:game thread under game_state_mutex; 'main thread' in fork records means that thread"
description: "When reasoning about which thread may read pools (MCP handlers, hooks, screenshots)."
status: verified
epistemics: observed
date: 2026-11-22
tags: [core-loop, prismatic-mcp]
about: src/video/video_driver.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/video/video_driver.cpp
    - file:src/video/dedicated_v.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/core-loop/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/prismatic-mcp/README.md
---

# Statement

`src/video/video_driver.cpp:VideoDriver::GameThread` starts `ottd:game` when the driver is
game-threaded and runs `::GameLoop` there holding `game_state_mutex`; `src/video/video_driver.cpp:VideoDriver::Tick`
takes the same mutex to draw and handle input. The dedicated driver is not game-threaded. ADR 0008 and
rule `project.game-state-mutates-only-through-commands` say "main thread": the safety argument holds
(one thread owns game state at a time), the wording does not.

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote game-state-is-owned-by-the-game-thread-not-the-main-thread` with evidence, or `majordomus knowledge reject game-state-is-owned-by-the-game-thread-not-the-main-thread --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/video/video_driver.cpp:90-91 starts the thread "ottd:game" when is_game_threaded; :45 GameThread loops over VideoDriver::GameLoop, which at :39-41 takes game_state_mutex and calls ::GameLoop.
- src/video/video_driver.cpp:104, :129 Tick takes game_state_mutex for drawing and input.
- src/video/dedicated_v.cpp:215 sets is_game_threaded = false.
