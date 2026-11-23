# Incident 2026-11-22: four PrismaticAI companies of one map built on the same mines

Post-mortem. In the owner's live 8-company game on `:8800` (seed 2927, master `c5bf32182a`), four
companies ran PrismaticAI: 0, 1, 6 and 7. Two or more of them built rail lines from the same mine or
forest, so each such line carried part of the cargo at full cost. All four stopped growing in 1952,
while AAAHogEx grew to 2.2M with 60 trains. No layer of the tooling noticed, because every test,
duel and battle had put at most one PrismaticAI on a map.

## What happened

Company value in thousands, from the panel's store (`samples` of run `panel-8800`, the last sample
of each month), at 1954-06:

| Company | AI | Value | Trains |
|---|---|---|---|
| 2 | AAAHogEx | 2204 | 60 |
| 6 | PrismaticAI | 836 | 7 |
| 1 | PrismaticAI | 550 | 6 |
| 3 | LuDiAI AfterFix | 501 | 5 |
| 7 | PrismaticAI | 469 | 2 |
| 0 | PrismaticAI | 282 | 1 |
| 5 | AdmiralAI | 200 | 3 |
| 4 | CivilAI | 115 | 5 |

The AI log (`ai.log` events) shows each pair building the same line:

- `COAL Gundhead Coal Mine - Bindingstone Bay Power Station`: companies 0 and 1;
- `IORE Bindingstone Bay Iron Ore Mine - Plunfingfield Steel Mill`: companies 0 and 1;
- `COAL Slarfingway Coal Mine - Senninghead Power Station`: companies 6 and 7;
- `COAL Findingville Coal Mine - Bindingstone Bay Power Station`: companies 6 and 7;
- `WOOD Wruston Forest - Brontford Sawmill`: companies 1 and 6.

The same log has `rail at tile N failed: ERR_OWNED_BY_ANOTHER_COMPANY` where their tracks met. In
the same game the bus side had run out of new routes: 25 or 26 of 30 per company, and about 2,000
retries of `no connected site for stop B`. Every line opened with one steam train on single track,
with "no second track ... would cost about N" and `ERR_NOT_ENOUGH_CASH`.

## Why it happened

- **The bot.** `NextOwnFreightLine` (`tools/research/prismatic-ai/PrismaticAI/rail.nut`) skipped a
  producer only when this company had a rail station near it (`OwnRailStationNear`) or when cargo
  had been transported from it last month. A rival's station was invisible to the first test. The
  second stays 0 for the months a rival lays its track. Four copies of one deterministic score on
  one map therefore picked the same best producers. The planner's lines (`fr_*`) were not checked
  at all.
- **The planner.** `panel_bot.rs` hands one plan to "that AI" (`held_for`: "The plan is one set of
  settings for that AI"), and `replan.rhai` re-plans only "the first running company of AI". In
  this game only company 0 had planner slots. Companies 1, 6 and 7 ran with `fr_from=-1` and chose
  their own lines, so the cause there was the bot's filter, not the plan.
- **The evidence.** All 162 duel records are PrismaticAI with a plan against PrismaticAI without
  one, so a "won" on the scoreboard is a win over a copy of itself. Each battle match puts one
  PrismaticAI against BaNaNaS AIs. No record measured several PrismaticAI companies on one map. The
  arena (`research/live-arena.md`) starts several PrismaticAI variants in one game on purpose.
- **No check.** The panel and the MCP resources serve stations, their owners and the industries.
  No check compared them, so nothing said that two of our companies served one producer.
- **The bigger gap was already on record.** The battle of `battle-20261122T045411-a80cc94a`:
  AAAHogEx won 41 of 41 matches at 104.4M average final value. The best PrismaticAI variant won
  none, at 16.5M. Tuning had optimised duel wins and GA fitness among PrismaticAI variants, while
  the gap to the strongest rival did not close.

## What changed

- **The bot claims a producer by its source station** (`ForeignRailStationAt`, `rail.nut`). A raw
  industry where another company already has a rail station is no candidate for
  `NextOwnFreightLine`. `BuildFreightRailRoute` checks it again before it builds the source station,
  for the planner's, the bot's own and the steward's lines alike. Two companies can also choose one
  producer in the same days, before either station stands. On the fitness seed both siblings picked
  Wontford Coal Mine in their first heartbeat. So after its source station is built, the company of
  the higher id gives the line up and removes the station, before any track is laid. Processing
  industries (sawmill, factory, steel mill) are left out of this check: an AI cannot read another
  company's station ratings, so it cannot tell a station that delivers there from one that picks up.
- **The fitness check plays two PrismaticAI companies on one map.** A fifth lineup
  (`PrismaticAI sibling A`, `PrismaticAI sibling B`, AAAHogEx, LuDiAI AfterFix) is in
  `tools/research/ai-battle/battle.json`. The referee Game Script reports every producer whose
  output rail stations of two or more companies pick up (a station with a rating for one of its
  cargoes) as a `REF S` line. `battle.py fitness` fails with `shared_producer:` when two of those
  owners are PrismaticAI. Tests: `SharedProducers` and `FitnessRun` in
  `tools/research/test_ai_battle.py`.

- **The planner plans per company and leaves out producers another company holds.** `mcp-plan
  --for <ids>` applies the bot's rule from `openttd://map` and `openttd://map/ways`
  (`cargo.rs` `rail_claims`). A freight candidate whose raw producer has a rail station of a
  company outside `<ids>` is no candidate. It is left out before the list is cut and before
  AlgoGlyph prices the best lines, and the plan's `claims` names it. `panel_bot.rs` plans every
  running company of its AI from that one answer. Each producer goes to one company only
  (`assign_producers`): the best to the lower id, the next best to the other, a producer a
  company holds to that company alone. The settings for a new company get only what is left, and
  `bot_settings.companies` carries one plan per company. `replan.rhai` re-plans every running
  company of the AI with its own entry. It keeps its switch model and the owner's approval for
  each company, and sends each company's changes to that company (`build_plan::apply_to`). Tests:
  `rail_stations_near_a_raw_producer_hold_it_for_their_company` (`cargo.rs`),
  `a_plan_for_a_company_leaves_out_the_producers_another_company_holds` (`plan_pipeline.rs`),
  `two_companies_of_one_ai_are_handed_different_producers_best_first` and
  `every_running_company_gets_a_plan_of_its_own` (`panel_bot.rs`), and
  `replan_proposes_each_company_of_its_ai_its_own_plan` (`rhai_shared.rs`).

The check against the tree before and after the change. Both runs used `build/openttd`
`20261119-master-g9750b3f946` and the arena cache at `~/ttd-battle/arena`, and ran
`battle.py fitness --config <copy with fitness.seed set>`. "Siblings" is the sibling lineup's two
PrismaticAI companies together:

| Seed | Before: sibling failures | Before: siblings | After: sibling failures | After: siblings | After: verdict |
|---|---|---|---|---|---|
| 930967890 (the gate's) | 0 | 1.90M | 0 | 1.88M | PASS |
| 31337 | 4 | 3.20M | 0 | 4.83M | PASS |
| 4242 | 1 | 4.49M | 0 | 6.05M | FAIL: self-steward `hoards` |
| 1807087905 | 3 | 4.08M | 0 | 5.01M | PASS |
| 2927 (the live game's) | 3 | 8.39M | 0 | 8.16M | PASS |

These are single matches, one per seed and side. They show that the shared producers are gone.
They are no tuning claim about value (rule `project.a-tuning-claim-needs-held-start-pairs`).

## What is still open

<!-- open-finding kind=failed -->
On seed 4242 the self-steward lineup hoards after the change (`hoards:1964-04..1966-01:716618->1549902@60v`),
where it did not before. Producers that rivals claimed are no longer candidates, and the bot has no
other sink for the money. The gate's seed passes. The sink that is missing is the capital loop:
more trains and second tracks on lines that already earn.

<!-- open-finding kind=not-run -->
The per-company plan is tested offline only. No live game has run a `replan.rhai` round with
several PrismaticAI companies yet, and no fitness or duel record measures it. Some parts of a plan
are still shared by every company of an AI: the passenger rail pair (`rail_a`, `rail_b`) and the
air pair. The plain-language answers of `intent.rs` do not use the claims at all. The planner
infers which industry a tile belongs to and whether an industry is raw, because the game serves
neither (`cargo.rs` `CLAIM_RULE`). The sub-tropical lumber mill, which accepts no cargo, therefore
counts as raw.

<!-- open-finding kind=partial -->
A processing industry served by two PrismaticAI companies is still possible: only the
transported-last-month filter covers it.

<!-- open-finding kind=partial -->
The same-days tie-break settles a race only when both source stations stand when the later check
runs. If the company of the higher id finishes its station before the other's site search ends, it
sees no rival and keeps it, and the lower one then sees a higher rival and keeps its own as well
(found by review, 2026-11-23). PR #240's longer site search widens that window. Waiting 10 days
before the check closed it but cost every line: on the fitness seed 930967890 the six PrismaticAI
lineups summed 7.87M with the wait and 9.36M without.
