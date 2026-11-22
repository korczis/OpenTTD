---
schema: knowledge/v1
id: a-planner-capture-is-coherent-only-through-read-many
kind: knowledge
class: fact
title: "Resource reads of a running game span ticks; a planner capture is at one tick only through openttd.resources.read_many"
description: "Before treating a planner capture or a snapshot of a running game as one state, or deciding live from one."
status: candidate
epistemics: observed
date: 2026-11-22
tags: [battlebrain, planner, prismatic-mcp, snapshot, coherence]
about: tools/research/mcp-rhai/src/bin/mcp-plan.rs
route: project
provenance:
  origin: authored
  derived_from:
    - file:research/battlebrain/README.md
    - file:tools/research/mcp-rhai/src/bin/mcp-plan.rs
    - file:src/prismatic_mcp/prismatic_mcp_server.cpp
    - file:src/prismatic_mcp/prismatic_mcp_detail.h
    - commit:13ac8a413b0330148a3b77b66b91665de51d9580
    - commit:5a22e26255dd2e85dac71b958085fd65b4f4d207
---

# Fact

Each `resources/read` is answered at whatever tick the game has reached, so a capture made of
one read per resource while the game runs is not one state. The three shadow captures of the
co-play game spanned 28 to 43 ticks (`research/battlebrain/README.md:242-244` at 194552a5fa).

`openttd.resources.read_many` (PR #39, merge 13ac8a413b) builds several resources inside one
main-thread poll (`prismatic_mcp_server.cpp:363`, :920; at most the cap in
`prismatic_mcp_detail.h:275`, `openttd://` only, :277). `mcp-plan --capture` (PR #44, merge
5a22e26255) uses it when the server's tool list offers it read-only (`mcp-plan.rs:13`, :53-82),
derives the set of resources from the plan's own reads, and otherwise falls back to one read
per resource; `capture.json`'s `coherence` says which happened and why.

# How to apply

Check `window.coherent` / `coherence` before using a capture as one state; a live decision needs
a one-tick window (`battlebrain live --max-window-ticks` defaults to 0) or a paused game.

# Review

Authored 2026-11-22 from origin/master at 194552a5fa and PRs #39 and #44; no live coherent
capture had been made when this was written (PR #44: NOT RUN). Awaiting the owner's review:
`majordomus knowledge promote a-planner-capture-is-coherent-only-through-read-many`, or
`majordomus knowledge reject a-planner-capture-is-coherent-only-through-read-many --reason ...`.
