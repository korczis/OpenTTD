---
schema: knowledge/v1
id: a-server-only-command-from-a-network-client-kicks-it
kind: knowledge
class: constraint
title: "A command flagged CommandFlag::Server, posted from a network client, gets that client kicked, and nothing on the client stops it being sent"
description: "Before an MCP tool, a bot or injected input posts a command from a GUI client's process."
status: verified
epistemics: inferred
date: 2026-11-22
tags: [network, commands, mcp]
about: src/network/network_server.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/network/network_server.cpp
    - file:src/command.cpp
    - file:src/misc_cmd.h
    - file:src/prismatic_mcp/prismatic_mcp_server.cpp
    - commit:0cd5925bd2bc229e991f75c4cc876c250bdbc882
relations:
  - type: relates_to
    target: file:.ai/repo/adrs/0013-every-openttd-process-may-run-its-own-mcp-server-and-its-vie.md
  - type: supports
    target: file:.ai/repo/rules/project/game-state-mutates-only-through-commands.v1.md
---

# Statement

`ServerNetworkGameSocketHandler::ReceiveClientCommand` (`src/network/network_server.cpp`) answers
a client that sends a command whose flags include `CommandFlag::Server` with
`NetworkErrorCode::Kicked`. The client side does not filter: in `src/command.cpp`,
`InternalPostBefore` only marks the command `only_sending` when `_networking`, and no check of
`CommandFlag::Server` precedes the send.

`Commands::Pause` is such a command (`src/misc_cmd.h`, `DEF_CMD_TRAIT(Commands::Pause, CmdPause,
CommandFlags({CommandFlag::Server, CommandFlag::NoEst}), ...)`). Since commit 0cd5925bd2 a GUI
client can run its own MCP server. When this record was written, `ExecuteAction` in
`src/prismatic_mcp/prismatic_mcp_server.cpp` posted `Command<Commands::Pause>::Post` with no
`_network_server` guard, so an approved `openttd.game.set_pause` served by a network client's MCP
server would have got the owner's client kicked. On master it now calls `ClientMayPost` first and
refuses the action on a network client before anything is sent.

# Evidence status

Inferred from the source at f069e8f0f5; not reproduced in a running game. To verify: join a GUI
client with its MCP server to a dedicated server, approve a `set_pause` on the client's server, and
watch the server console for "Kicking client ... due to calling a server only command".

# What to do

A tool that posts a command from a process that may be a network client checks the command's
`CommandFlag::Server` against `_networking` and `_network_server` before posting, and refuses with an
explanation instead (`src/prismatic_mcp/prismatic_mcp_server.cpp:ClientMayPost` does this for the
MCP actions: `game.set_pause` and every `command.*` action).

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/network/network_server.cpp:1137-1139 (ReceiveClientCommand, :1111) kicks a client that sends a CommandFlag::Server command with NetworkErrorCode::Kicked.
- src/command.cpp:210 `only_sending = _networking && !network_command`; nothing on that path tests CommandFlag::Server.
- src/misc_cmd.h:36 Commands::Pause carries CommandFlag::Server.
- Revised today: src/prismatic_mcp/prismatic_mcp_server.cpp:669-676 ClientMayPost refuses a CommandFlag::Server command on a network client; ExecuteAction calls it for game.set_pause (:799) and command.* (:825). The kick itself was not reproduced in a running game.
