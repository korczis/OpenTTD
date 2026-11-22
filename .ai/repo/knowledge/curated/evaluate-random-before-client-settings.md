---
schema: knowledge/v1
id: evaluate-random-before-client-settings
kind: knowledge
class: lesson
title: "In a condition chain, the Random() draw comes before any _settings_client test, or a local option changes the shared random stream"
description: "When combining randomness with a client-side setting (sound, GUI) in simulation code."
status: verified
epistemics: observed
date: 2026-11-22
tags: [towns-industries-stations]
about: src/industry_cmd.cpp
route: platform
provenance:
  origin: authored
  derived_from:
    - file:src/industry_cmd.cpp
relations:
  - type: relates_to
    target: file:.ai/repo/workflows/substrate/towns-industries-stations/README.md
---

# Statement

`src/industry_cmd.cpp:ProduceIndustryGoods` writes `Chance16R(...) && ... && _settings_client.sound.ambient`,
so the draw happens on every client whatever its sound option. Short-circuiting on a client setting
first would make the number of draws differ between peers.

# Review

Authored from a read of the source at f069e8f0f5 (issue I0055); awaiting the owner's review.
`majordomus knowledge promote evaluate-random-before-client-settings` with evidence, or `majordomus knowledge reject evaluate-random-before-client-settings --reason ...`.

# Evidence

Re-verified against origin/master at 1a1297fce2 (owner-delegated review, task t-20261122093452-9b02, 2026-11-22), read only:
- src/industry_cmd.cpp:1169 (ProduceIndustryGoods, :1163) `if (Chance16R(1, 14, r) && !indsp->random_sounds.empty() && _settings_client.sound.ambient)`: the draw comes first.
