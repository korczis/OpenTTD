---
schema: knowledge/v1
id: a-research-build-and-a-default-build-join-the-same-game
kind: knowledge
class: constraint
title: "A research-instrumented build and a default build of the same commit are network-compatible, so they join one game"
description: "Before running a multiplayer experiment that mixes binaries built with different CMake options."
status: verified
epistemics: observed
date: 2026-11-22
tags: [network]
about: src/network/core/network_game_info.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/network/core/network_game_info.cpp
    - file:src/network/network_server.cpp
    - file:cmake/Options.cmake
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/network/README.md
---

# Statement

`src/network/core/network_game_info.cpp:IsNetworkCompatibleVersion` compares revision strings
(and the `dbg_seed`/`dbg_sync` prefixes); CMake options such as `OPTION_RESEARCH_INSTRUMENTATION`
or `OPTION_PRISMATIC_MCP_SERVER` are not part of the revision. A mixed pair therefore joins, and a
gated hunk that ever touched simulation state would desync them with no version error. Keep every
participant of a multiplayer experiment on the same profile, and record it in the manifest.

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote a-research-build-and-a-default-build-join-the-same-game` with evidence, or `majordomus knowledge reject a-research-build-and-a-default-build-join-the-same-game --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/network/core/network_game_info.cpp:50-58 GetNetworkRevisionString starts from _openttd_revision (cmake/scripts/FindVersion.cmake, which reads no CMake option); only the ENABLE_NETWORK_SYNC_EVERY_FRAME debug builds prefix it.
- cmake/Options.cmake:65-66 OPTION_RESEARCH_INSTRUMENTATION and OPTION_PRISMATIC_MCP_SERVER are plain options; :138-147 add definitions, not a revision part.
- src/network/core/network_game_info.cpp:211-231 IsNetworkCompatibleVersion compares revision strings, dbg_seed/dbg_sync prefixes and the git-hash suffix only. The research-only research_net_compat verdict (:213-216, added in 3151fa8d00) can only widen acceptance and defaults to Strict (:205), so it does not make a mixed pair incompatible.
- src/network/network_server.cpp:915-917 (ReceiveClientJoin) is the server's check.
