---
schema: knowledge/v1
id: prismatic-ai-falls-behind-aaahogex-on-breadth-not-caps
kind: knowledge
class: fact
title: "PrismaticAI trails AAAHogEx on network breadth; raising its caps or growing its stations does not close the gap"
description: "Before tuning PrismaticAI caps, station growth or feeders to catch AAAHogEx; replayed A/Bs on 2026-11-22 show which levers move value and which do not."
status: candidate
epistemics: observed
date: 2026-11-22
tags: [prismatic-ai, ai-battle, ablate, rail, stations, aaahogex]
route: project
provenance:
  origin: authored
  derived_from:
    - file:tools/research/prismatic-ai/PrismaticAI/growth.nut
    - file:tools/research/ai-battle/battle.py
---

# Fact

In the 12 hours before 2026-11-22 06:00, AAAHogEx won all 2,506 arena matches. The default PrismaticAI's
median value was 0.17 of the winner's. Averaged over 120 matches, PrismaticAI stops growing about 1968.
Its stations stay at 65 from 1971. In 1980 it has 59 vehicles against 389, 22 trains against 97 and 17
road vehicles against 240, while 96% of its value is idle cash. A replay log (`07472abe/m-0006`)
shows two causes:

- the own freight planner proposed only 13 lines in 20 years (`NextOwnFreightLine`: production ≥ 40,
  nobody transporting, 99-261 tiles);
- lines kept 2-3 trains on platforms sized at build time (2-3 tiles).

Replayed A/Bs (`battle.py ablate --match`, real lineups, default PrismaticAI):

| Change | Matches | Geometric | Better |
|---|---|---|---|
| `rail_len=4` (4-tile platforms from the start) | 9 | +13.5% | 9 |
| TrainsOf by shared orders (PR #7) vs master | 8 | +7.2% | 5 |
| `own_lines=32, reinvest_budgets=1` | 1 | 0.0% (byte-identical) | - |
| `rail_len=5` / `rail_len=7` | 4 / 1 | mixed / -8% | - |
| growing platforms in place (`rail_grow_max=6`) | 8 | -3.1% | 5 |
| `rail_len=4` plus growth | 8 | -0.5% | 4 |
| joining unserved same-cargo producers (`rail_feed`) | 8 | -1.9% | fired in 1 game |
| `invest_road_cap=20, max_routes=30` | 4 | -0.7% | - |
| `air_routes_max=20, invest_air_cap=12, air_planes=3` | 4 | -12.0% | - |

# How to apply it

- Do not expect caps (own lines, road or air fleet) to close the gap: none of them binds, because the
  bot runs out of profitable things to build first. The lever is breadth: more candidate lines
  (the planner) and sources chosen as clusters.
- Size platforms at 4 tiles when a line is built. Growing them later gave the gain back.
- Post-hoc feeding rarely fires, because a second unserved producer within station spread is rare.
  Multi-source stations must be chosen by the planner, with `JoinSourceToStation` (growth.nut) doing
  the building.
- Two lines that share both stations must tell their trains apart by shared orders. Otherwise each
  sells the other's trains.
- A single replay is chaotic: `07472abe/m-0006` moved by up to 15% with no behavioural change.
  Use 8 or more matches.
