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

## What changed next: the claim made exact and narrow (2026-11-23)

Three items stayed open after the change above: the same-days race, a claim that was too broad,
and processing industries. Before any of it was written, the design was reviewed by Codex and by
an independent peer agent. The peer agent stood in for Gemini, whose client was refused. Their
critique changed the design in the ways listed below. The coordinator then found the defect that
mattered most in the owner's live game. From 1952-10 the bot reported 19 to 21 producers
"claimed" and 0 pairs tried, and still did so in rich mode from 1954-03. `ForeignRailStationAt`
ran before the rich share test of `NextOwnFreightLine`, so a rival's working line held its
producer for good.

- **A claim is a fresh station, nothing more** (`ForeignRailStationAt`, `ClaimFresh`, `rail.nut`).
  A foreign rail station tile catching a producer claims it while this company first saw it less
  than `CLAIM_FRESH_DAYS` (365) ago. Such a station belongs to a line still being built, or to
  one waiting for its first train. From the choice of a line to its first train, PrismaticAI's
  lines took 0 to 18 months, most within 12 (quarter resolution).

  Once a rival's station is older, the producer's transported amount decides, through the filters
  `NextOwnFreightLine` already has. Outside rich mode, a producer that moved cargo last month is
  skipped. In rich mode it is scored on the share left below `rich_served_below`. A station that
  only catches a producer no longer holds it for good, for example a rival's passenger station in a
  town beside a mine, or a station delivering to a power station next to one. The old comment's
  premise was wrong: a station that catches a raw industry need not pick up from it.

  Nothing else can tell those stations apart:
  - an AI is refused a foreign station's ratings, waiting cargo, vehicles, name and construction
    date (`AIStation.IsValidStation` is false for it);
  - the cargo monitor is for Game Scripts only;
  - a pickup station at a mine inside a town accepts passengers too, so acceptance proves nothing;
  - company names are the game's own unless `disclose_name` is set.

  A sibling's older station still claims while cargo moves from the producer. The bot knows a
  sibling by its auto-renew settings (`IsSibling`): every company's are readable, and PrismaticAI
  sets them at every start to a pair that differs from the game's defaults (-6 months and 30000,
  against 6 months and 100000). Without this rule, on seed 31337 rich mode built a second coal
  line at Pruston Coal Mine, beside the sibling's line, whose station rated the coal 49%. The
  fitness check failed that run with `shared_producer` (cargo 1). Test: `SiblingMark` in
  `test_ai_battle.py` keeps `main.nut` and `rail.nut` in step.

  The first sight is kept per tile, with the owner and the station id. A platform joined to an old
  station, or a station rebuilt on the same tile, is therefore fresh again.

  The table is saved and restored (`restore.nut`). After a restart, a station of unknown age
  counts as old (`ClaimsAfterRestart`): a load restarts every AI, so no line is being built across
  it. Without that, the first deploy into a running game would block every such producer for a
  year. A neutral station (an oil rig's) never claims.
- **Processing industries are claimed the same way.** The raw-only test is gone. During the race,
  a sibling's fresh goods pickup is a claim like any other. A station delivering to a sawmill holds
  the sawmill's output for a year at most.
- **The same-days race is settled by two checks** (`BuildFreightRailRoute`, `ClaimBlocks`). Both
  count only fresh tiles, so an old station whose producer starts to move cargo is not mistaken
  for a new claim.
  - Check 1, after the source station: a lower company id wins, as before. This settles the
    common race, where both companies start in one heartbeat.
  - Check 2, after the destination station: a lower id wins at once. A higher id wins as well if
    its station still stands after `CLAIM_SETTLE_DAYS` (2). A company that saw us at its own
    check 1 has removed its station by then. One whose station still stands kept it, because its
    check 1 came before ours existed.

  Of two companies that both kept a station, the one whose check 2 came later saw the other's
  source station standing and yielded. So two lines are never both kept, and the check costs no
  wait without contention. Both companies yield only when one's check 2 falls within the days the
  other takes to remove a station it gave up. Then nobody builds there. The first attempt's
  10-day wait, which cost every line, is not used.
- **The fitness check plays the race.** A seventh lineup, `PrismaticAI claim race A/B`, sets
  `test_claim_race=20` (RESEARCH-ONLY) on company 0. Company 0 waits 20 days before its first
  source station, so company 1 builds and checks first, and only check 2 can settle the race. On
  the fitness seed company 0 yields at check 2 (`given up at its second station`), and only
  company 1 and AAAHogEx pick up at Wontford Coal Mine. With check 2 removed, the same lineup fails
  `shared_producer:Wontford Coal Mine` (companies 0 and 1). Test:
  `SharedProducers.test_the_fitness_config_plays_the_claim_race`.
- **A shared producer is judged per cargo** (referee `REF S ... cargo=<id>`, `battle.py`
  `parse_shared`, `sibling_producers`). The narrower claim let company 0 take Punthill Farm's
  livestock. Company 1's grain station at the neighbouring Dindinghall Farm also caught Punthill
  Farm's grain. The referee counted any rated cargo, so it flagged the farm as shared, although
  the two companies split nothing. Tests: `test_parse_keeps_each_cargo_apart` and
  `test_two_prismatic_companies_on_different_cargoes_of_one_farm_do_not_fail`.

How it was checked (PR #277). By the owner's rule, a change passes the fitness check when it shows
no failure that master does not also show, over repeated runs.

- **Master and the branch, after master's #279 and #280 were merged.** Both played all seven
  fitness lineups on seeds 930967890, 31337 and 4242: master at `1fd84fa0b9`, the branch at
  `4b3add84eb`. Both used the same per-cargo referee and `fitness_verdict`.
  - Over 21 matches each, master failed 3: `hoards` twice on 31337, and `trainless_stations` once
    on 4242.
  - The branch failed 4, all `trainless_stations`: lineup 4 on the gate's seed, and lineups 1, 4
    and 6 on 31337.
  - Neither showed `shared_producer` or `no_funnel`. Summed PrismaticAI value: master 60.55M,
    branch 60.81M.
  - Each of the branch's trainless stations is the terminus of a rich-mode line started in the
    last game year and still being laid when the match ended. Path searches there took 100 to 210
    days each. The branch spends money that master's 31337 lineups hoarded.
  - An earlier pair of trees, master `e95a0e37aa` against branch `7d0a1c1a0e`, gave master 6
    failures (`hoards` 2, `trainless_stations` 4) and the branch 4 (`hoards` 1,
    `trainless_stations` 3).
- **The growth funnel.** After the merge, the gate's seed failed `no_funnel` once. One heartbeat
  had searched a 258-tile double track from 1962 to 1964, and the funnel logs a year only at a
  heartbeat or an event. `FindRailPath` now rolls the funnel's year at its yield point.
- **A replay of the owner's live save `final-15805-0106.sav`.** In its first yearly report the
  branch counted 0 producers claimed and 30 released, where master counted 21 claimed. Of the 30
  released, 27 move cargo and go to the share test, and 3 move none. Rivals move 60% or more of
  every one of them, so on that map the rich share test, not the claim, is what now limits the bot.

These are single matches per seed and side, so they make no tuning claim about value.

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
A source station can catch a second producer of its cargo besides the one it was built for, and
pick up from it too. When another PrismaticAI company already serves that neighbour, the two share
it. The claim checks look only at the producer the line was built for. A site that avoids a claimed
neighbour is the station-site functions' choice (`BuildRailStationAt` and its helpers). Found by
review, 2026-11-23. Seen once in the claim-race lineup, with different cargoes (Punthill Farm
above), which the per-cargo check no longer counts.

<!-- open-finding kind=partial -->
The sibling mark is the pair of auto-renew settings that `main.nut` sets at every start. If a
person changes those settings in a co-played company, that company stops counting as a sibling,
and a rival that uses the same pair is taken for one. The planner's copy of the claim (`cargo.rs`
`CLAIM_RULE`, `mcp-plan --for`) still applies the old rule: any rail station near a raw producer
holds it, and a processing industry is never held.

<!-- open-finding kind=staged -->
A line that yields to a claim counts as a failed own line (`own_failed` in `main.nut`), and its
pair stays in `freight_tried`. If the company that kept the producer later fails to build its
line, neither company returns to that producer.

<!-- open-finding kind=partial -->
With more producers free, rich mode starts lines in the last game year of a fitness match. Their
path searches take 100 to 210 days each, so they are still being laid when the match ends. The
fitness check's `trainless_stations` (four quarterly reports) counts their termini as stations
without a train: four of the branch's 21 matches on PR #277 failed this way. Master failed it once.
Either the construction is too slow or the rule's window is too short for it; neither is decided
here.

<!-- open-finding kind=staged -->
`FeedNewLine` (`growth.nut`, setting `rail_feed`, default 0) joins feeder platforms at other
producers and checks only the transported amount, not `ForeignRailStationAt`. With `rail_feed`
on, it can join a producer that a sibling has just claimed.
