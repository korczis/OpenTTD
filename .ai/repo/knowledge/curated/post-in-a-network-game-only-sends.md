---
schema: knowledge/v1
id: post-in-a-network-game-only-sends
kind: knowledge
class: constraint
title: "In a network game Command<>::Post returns once the command is sent; it executes frames later, on every peer"
description: "Before reading game state right after a Post to decide whether a command worked (GUI, MCP, bots)."
status: verified
epistemics: observed
date: 2026-11-22
tags: [commands, network]
about: src/command.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/command.cpp
    - file:src/command_func.h
    - file:src/network/network_command.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/commands/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/network/README.md
---

# Statement

`src/command.cpp` sets `only_sending = _networking && !network_command` in `InternalPostBefore`,
and the helper returns after `NetworkSendCommand` (`src/network/network_command.cpp`). The callback
fires only on the issuing client, from `NetworkExecuteLocalCommandQueue`, when the frame arrives.
Follow-up logic belongs in a callback or in a later observation of state, not on the line after
`Post`. `src/prismatic_mcp/prismatic_mcp_server.cpp:ExecuteAction` is the pattern: it posts with
`CcPrismaticMcp` as callback and leaves the verdict to `OnCommandResult`, which concludes the
decision once the command has executed and then checks state through `pr.observe` (for
`game.set_pause`, `_pause_mode`). It no longer reads `_pause_mode` on the line after `Post`.

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote post-in-a-network-game-only-sends` with evidence, or `majordomus knowledge reject post-in-a-network-game-only-sends --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/command.cpp:210 `only_sending = _networking && !network_command`.
- src/command_func.h:434-442 when send_net, NetworkSendCommand and return an empty result; :340 the local callback runs only when not only_sending.
- src/network/network_command.cpp:270-286 NetworkExecuteLocalCommandQueue executes a queued command at its frame and calls its callback.
- Revised today: src/prismatic_mcp/prismatic_mcp_server.cpp:786-802 ExecuteAction sets pr.observe and posts with CcPrismaticMcp; src/prismatic_mcp/prismatic_mcp_commands.cpp:336 forwards to OnCommandResult (prismatic_mcp_server.cpp:1528-1543), which concludes the decision; :739-742 evaluates observe then. The old last sentence (reads _pause_mode at once) was stale.
