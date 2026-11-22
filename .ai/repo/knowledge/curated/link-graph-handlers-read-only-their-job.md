---
schema: knowledge/v1
id: link-graph-handlers-read-only-their-job
kind: knowledge
class: constraint
title: "Link graph component handlers run on worker threads and may read only their LinkGraphJob, never pools, settings or Random()"
description: "Before changing cargo distribution (src/linkgraph/) or anything a link graph job reads."
status: verified
epistemics: observed
date: 2026-11-22
tags: [economy-and-companies]
about: src/linkgraph/linkgraphjob.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/linkgraph/linkgraphjob.cpp
    - file:src/linkgraph/linkgraphschedule.cpp
    - file:src/linkgraph/mcf.cpp
    - file:src/linkgraph/demands.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/economy-and-companies/README.md
---

# Statement

The `LinkGraphJob` constructor (`src/linkgraph/linkgraphjob.cpp`) copies the graph and the link graph
settings and fixes a join date. `src/linkgraph/linkgraphschedule.cpp:LinkGraphSchedule::JoinNext`
joins the thread only at that date, on every peer, which is what keeps the result deterministic. A
handler that reads `_settings_game`, a pool or `Random()` races the game loop and can desync.

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote link-graph-handlers-read-only-their-job` with evidence, or `majordomus knowledge reject link-graph-handlers-read-only-their-job --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/linkgraph/linkgraphjob.cpp:38-45 the constructor copies the link graph and _settings_game.linkgraph and fixes join_date.
- src/linkgraph/linkgraphjob.h:222 IsScheduledToBeJoined is join_date <= TimerGameEconomy::date.
- src/linkgraph/linkgraphschedule.cpp:68-75 JoinNext joins (deletes) the job only when it is scheduled to be joined.
