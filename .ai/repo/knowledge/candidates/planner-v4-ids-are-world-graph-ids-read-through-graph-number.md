---
schema: knowledge/v1
id: planner-v4-ids-are-world-graph-ids-read-through-graph-number
kind: knowledge
class: fact
title: "Planner v4 names industries by world-graph id (\"industry:41\"), not by number; battlebrain_contract::graph_number is the one normalisation"
description: "Before comparing, storing or passing on a producer or acceptor id taken from mcp-plan, the world graph or a Battlebrain record."
status: candidate
epistemics: observed
date: 2026-11-22
tags: [battlebrain, planner, world-graph, ids]
about: tools/research/mcp-rhai/src/battlebrain_contract.rs
route: project
provenance:
  origin: authored
  derived_from:
    - file:tools/research/mcp-rhai/src/battlebrain_contract.rs
    - file:tools/research/mcp-rhai/src/panel_bot.rs
    - file:tools/research/mcp-rhai/src/battlebrain_policy.rs
    - commit:194552a5fa011bfc1c221f3e4c4e77979cb6e69c
---

# Fact

The planner's freight candidates carry `producer.id` and `acceptor.id` as world-graph node ids
(`"industry:41"`), as `openttd://world/graph` does; the game's own number is the part after the
last colon. At 194552a5fa, `battlebrain_contract.rs:166-176` (`graph_number`) is the crate's one
normalisation: a number, a number in text or a graph id gives the number, anything else `None`
(tests at :1410-1414). `panel_bot.rs:428-432` (`num`) delegates to it, and
`battlebrain_policy.rs:91` reads candidate ids through it; the panel bot's fixtures
(`panel_bot.rs:1008`) and the policy's (`battlebrain_policy.rs:213`) use the `industry:N` form.

# How to apply

Read an id through `graph_number`; never parse it as an integer directly (a bare parse turns
every v4 id into nothing) and never write a second normaliser.

# Review

Authored 2026-11-22 from a read of origin/master at 194552a5fa; awaiting the owner's review.
`majordomus knowledge promote planner-v4-ids-are-world-graph-ids-read-through-graph-number` with
evidence, or `majordomus knowledge reject planner-v4-ids-are-world-graph-ids-read-through-graph-number --reason ...`.
