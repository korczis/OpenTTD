---
schema: knowledge/v1
id: the-revision-check-is-one-function-both-roles-run
kind: knowledge
class: fact
title: "The network revision check is one function both roles run, so a local override is local: a stock server still rejects a mismatched client"
description: "When reasoning about connecting a research build across a revision mismatch, or about what the research_net_compat override can and cannot do."
status: verified
epistemics: observed
date: 2026-11-22
tags: [network, research, determinism]
about: src/network/core/network_game_info.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/network/core/network_game_info.cpp
    - file:src/network/network_server.cpp
    - file:src/network/network_client.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/adrs/0029-a-research-gated-local-only-override-accepts-a-mismatched-pee.md
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/network/README.md
  - type: supports
    target: file:.ai/repo/rules/project/research-net-compat-is-local-and-non-spoofing.v1.md
---

# Statement

`src/network/core/network_game_info.cpp:IsNetworkCompatibleVersion` is the one function both roles
run: the server calls it on the client's revision at join
(`src/network/network_server.cpp:ServerNetworkGameSocketHandler`, kicking with
`NetworkErrorCode::WrongRevision`), and the client marks a server incompatible through
`CheckGameCompatibility` before connecting. The client sends its own true revision
(`src/network/network_client.cpp` sends `GetNetworkRevisionString()`); it never sends what the
server reports back.

Two consequences. A single process acting as both the loopback server and the client is covered by
widening that one function (the fork's `research_net_compat` override, ADR 0029). And because the
server runs the check independently on the client's sent revision, a client-side override is
**local only**: a stock, non-consenting server still rejects a mismatched client. Making a stock
server accept a mismatched client would require changing the revision the client *sends* — spoofing
— which this fork forbids (`project.research-net-compat-is-local-and-non-spoofing`).

# Review

Authored from a read of the source at the I0070 head (issue I0070); awaiting the owner's review.
`majordomus knowledge promote the-revision-check-is-one-function-both-roles-run` with evidence, or
`majordomus knowledge reject the-revision-check-is-one-function-both-roles-run --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/network/core/network_game_info.cpp:211-231 IsNetworkCompatibleVersion is the one check; the research_net_compat verdict (:213-216, :196-208, from 3151fa8d00) sits inside it and never reads our own revision.
- Server role: src/network/network_server.cpp:915-917 (ReceiveClientJoin, :897) rejects with NetworkErrorCode::WrongRevision.
- Client role: src/network/core/network_game_info.cpp:238-241 CheckGameCompatibility, called from src/network/network_coordinator.cpp:260 and src/network/network_query.cpp:128.
- The client sends its own revision: src/network/network_client.cpp:298 (SendJoin, :287) Send_string(GetNetworkRevisionString()); GetNetworkRevisionString (network_game_info.cpp:50-87) has no research override (comment :116-118).
