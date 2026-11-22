---
schema: knowledge/v1
id: rcon-and-admin-console-commands-pass-server-only-hooks
kind: knowledge
class: constraint
title: "A console command reached over rcon or the admin port passes server-only hooks, so a new command restricts remote callers itself"
description: "Before adding a console command (research_status, prismatic_mcp, ...) that must not be reachable remotely."
status: verified
epistemics: observed
date: 2026-11-22
tags: [gui-and-console, network]
about: src/network/network_server.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/network/network_server.cpp
    - file:src/network/network_admin.cpp
    - file:src/console_cmds.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/network/README.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/gui-and-console/README.md
---

# Statement

`src/network/network_server.cpp:ServerNetworkGameSocketHandler::ReceiveClientRemoteConsoleCommand`
and the admin equivalent execute the line in the server process, where hooks such as
`src/console_cmds.cpp:ConHookServerOnly` test only `_network_server`, and the output is redirected to
the remote caller (`src/console.cpp:IConsolePrint`). Rcon sets `_redirect_console_to_client`
(`src/network/network_server.cpp:ReceiveClientRemoteConsoleCommand`); the admin port sets
`_redirect_console_to_admin` instead (`src/network/network_admin.cpp:ReceiveAdminRemoteConsoleCommand`).
A command that prints a secret checks both itself. `prismatic_mcp token` and the bearer line of
`prismatic_mcp start` (I0061) check only `_redirect_console_to_client`, so an authenticated admin-port
caller would still receive the token (inferred from the source, not reproduced).

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote rcon-and-admin-console-commands-pass-server-only-hooks` with evidence, or `majordomus knowledge reject rcon-and-admin-console-commands-pass-server-only-hooks --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/network/network_server.cpp:1501, :1521-1523 ReceiveClientRemoteConsoleCommand sets _redirect_console_to_client and runs IConsoleCmdExec in the server.
- src/network/network_admin.cpp:515-526 ReceiveAdminRemoteConsoleCommand sets _redirect_console_to_admin and runs IConsoleCmdExec.
- src/console_cmds.cpp:164-172 ConHookServerOnly tests only _network_server.
- src/console.cpp:95-103 IConsolePrint redirects to the rcon client or the admin.
- Superseded on branch fix/mcp-token-never-printed (red-team finding 8): no console, local or remote, receives the token; `start` writes it to a 0600 file and prints the path, `token` prints the path and a fingerprint and still refuses a remote caller (PrismaticMcpConsoleIsRemote tests both redirects since PR #34).
- Revised today: src/console_cmds.cpp:1949-1952 (prismatic_mcp start) and :1971-1972 (prismatic_mcp token) test only _redirect_console_to_client; _redirect_console_to_admin is not tested there (grep), so the token would reach an admin-port caller. Inferred from the source, not reproduced.
