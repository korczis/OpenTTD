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
| planner scores source clusters and joins them at once (`rail_feed`, after 8b8e121e8a) | 8 | -8.8% | 3 joins in 8 games; 0.47 to 1.50 |
| `invest_road_cap=20, max_routes=30` | 4 | -0.7% | - |
| `air_routes_max=20, invest_air_cap=12, air_planes=3` | 4 | -12.0% | - |
| rich growth with its air lifts (#46, `rich_growth=1`, as merged) vs `fced659668` | 8 | -29.2% | 1 |
| `rich_air=0`: rich growth and pressure leave the air caps alone, vs `rich_air=1` | 6 | +35.5% | 6 |
| `rich_growth=0` (hoards; fails `fitness.must_not_hoard`) vs `rich_air=0` | 6 | +6.6% | 4 |
| `rich_air=1` with `air_add_waiting=150` or `300` vs `rich_air=0` (both thresholds byte-identical: the planes come from new air routes, not from more planes per route) | 6 | -10.2% | 2 |
| yield gates (#243, `load_add_min=50`, `road_yield=20`, as merged) vs `dbd39147ee` | 24 | -22.6% | 4 (219 planes against 488) |
| `load_add_min=0` vs 50, on `11411cdc4d` | 24 | +29.6% | 22 (0.88 to 2.03; 332 more planes) |
| `load_add_min=0, road_yield=0` vs both on, on `11411cdc4d` | 24 | +21.3% | 20 |
| `air_yield_routes=0` or `16` vs 8, with `load_add_min=0` | 24 | 0.0% (byte-identical: never fires) | - |
| master `c5bf32182a` (#46 with #68) vs `c86fbd1ccb` (before #46) | 24 | -1.8% | 14 |
| master `5cc9cdbf8d` (#213 rail search fail-fast, #163 ships off, #217 depot guard) vs `c5bf32182a` | 24 | +11.8% | 19 |
| #213 alone (`3e4ec154ff`) vs `c5bf32182a` | 24 | +14.4% | 17 |
| #163 (off) and #217 depot guard (`5cc9cdbf8d`) vs `3e4ec154ff` | 24 | -2.3% | 14 |
| greedier rail A* estimate `H_TILE=8` vs 7, on `5cc9cdbf8d` | 24 | +9.4% | 18 |
| `H_TILE=9` / `H_TILE=11` vs 7, on `5cc9cdbf8d` | 24 / 12 | +8.8% / +8.7% | 15 / 4 |
| master `6333347512` (#232 second track, sibling claims, air yield log) vs `5cc9cdbf8d` | 24 | -1.0% | 12 |
| `H_TILE=8` vs 7, on `6333347512` | 24 | +0.9% | 12 |
| master `fdc13851cf` (a failed own line no longer uses up `own_lines`) vs `6333347512` | 24 | -0.6% | 0 (0.87 to 1.00: mostly unchanged) |
| own freight planner production floor 25 instead of 40, on `fdc13851cf` | 12 | +0.7% | 8 (249 trains against 232) |
| `air_full_load=1` (planes wait for a full load, as AAAHogEx does), on `dbd39147ee` | 12 | -35.4% | 0 (0.26 to 0.98; 42 fewer planes) |
| `rail_len=6` vs 4, on `dbd39147ee` | 12 | +1.8% | 7 (0.63 to 1.33; 58 fewer trains) |
| compete for producers a rival's rail station claims (`ForeignRailStationAt` off), on `dbd39147ee` | 12 | +9.6% | 7 (251 trains against 208) |
| the same plus producers others serve, discounted by the served share | 12 / 24 | +10.2% / -1.4% | 7 / 11 (the second 12: -11.8%, 4 better) |

# How to apply it

- Do not hold planes back by load: `load_add_min=50` (no plane more on a route under half full)
  cost 29.6% over 24 matches. A plane that flies part empty still earns on these maps.
- Do not expect caps (own lines, road or air fleet) to close the gap: none of them binds, because the
  bot runs out of profitable things to build first. The lever is breadth: more candidate lines
  (the planner) and sources chosen as clusters.
- Do not spend idle money on aircraft: more planes and air routes lowered company value in every
  replay. Rich growth keeps its rail, road and own-line lifts (`rich_air=0`).
- Do not make planes wait for a full load: copying AAAHogEx's `air_full_load` cost 35% over 12
  matches. AAAHogEx's higher profit per plane comes from which pairs it picks, not from loading.
- Size platforms at 4 tiles when a line is built. Growing them later gave the gain back.
- Post-hoc feeding rarely fires, because a second unserved producer within station spread is rare.
  Scoring clusters in the planner did not help either (-8.8%, 3 joins in 8 games): on these 256x256
  maps, same-cargo producers within one station's spread are rare. Multi-source needs feeders
  (road or rail) from producers further away, not catchment joins.
- Two lines that share both stations must tell their trains apart by shared orders. Otherwise each
  sells the other's trains.
- Keep every search bounded: a rail search that ran most of a game year before giving up cost 14.4%
  of value (#213). Time the bot spends searching is time its money sits idle.
- A gain measured on one master does not carry to the next: `H_TILE=8` replayed at +9.4% over 24
  matches on `5cc9cdbf8d` and at +0.9% on `6333347512`, three bot changes later. Re-measure on the
  current master before opening a PR.
- Decide on 24 matches, not 12. Three changes that led by 10-15% on matches 6, 18 and 30 of the four
  editions fell to +6%, +3% and -12% on matches 12, 24 and 36 (`H_TILE=8`, `H_TILE=9`, competing
  for producers). Twelve paired replays spread by about 10%.
- In a 20-year replay the own freight planner runs out of producers: of 42-53 on land it holds
  8-14, rivals' rail stations claim 15-19, 5-12 are transported by others and 5-10 make under 40
  a month (`55e2546b/m-0018`, `07472abe/m-0006`). Taking claimed or served producers did not add
  value (above), so the money it cannot place stays idle.
- A single replay is chaotic: `07472abe/m-0006` moved by up to 15% with no behavioural change.
  Use 8 or more matches.
