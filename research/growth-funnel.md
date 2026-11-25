# PrismaticAI's growth funnel: where a line stops before it pays

PrismaticAI logs one `FUNNEL` line per transport mode once a game year. Each line counts the stages
a new line passes: candidate discovery, affordability, construction, and operation that pays. The
code is `tools/research/prismatic-ai/PrismaticAI/funnel.nut` (RESEARCH-ONLY observability: it
counts and logs, and never acts). `battle.py fitness` prints every line and names, for each
population, the stage that lost the largest share.

## Why it exists

The external review of pass 2 of openttd-36 named the conversion funnel as the missing
measurement. The plan's next step is a wider candidate search, and without a count per stage
that step may only add failures. The evidence of where growth stops was scattered over logs
and incidents:

- 19 to 21 producers skipped as claimed, and "0 pairs tried" (`no own freight line`, rail.nut);
- 5 path searches that ran out of nodes, and 17 single-track fallbacks;
- a locomotive refused for money, which left a line idle for 99 days in live game 10487;
- 117 identical bus failures;
- 165M in the bank of a company running 40 trains (live game 10487). The investor
  (`fleet.nut` `InvestSurplus`) buys only road, air and water vehicles, so idle money never buys
  a train.

No log line said how many candidates entered each stage and how many left it.

## The line

```
FUNNEL year=<y> mode=rail hb=<n> short_hb=<n> idle_hb=<n> searches=<n> found=<n> producers=<n>
  ours=<n> claimed=<n> small=<n> served=<n> no_acceptor=<n> pairs_tried=<n> tried=<n> stations=<n>
  path_fail=<n> laid=<n> built=<n> single_track=<n> trainless=<n> loco_refused_cash=<n> trained=<n>
  lines=<n> paying=<n> trains=<n> trains_added=<n>
FUNNEL year=<y> mode=road tried=<n> stop_fail=<n> network_fail=<n> depot_fail=<n> vehicle_fail=<n>
  built=<n> invested=<n> vehicles=<n> paying=<n>
FUNNEL year=<y> mode=air short=<n> no_pair=<n> tried=<n> built=<n> invested=<n> vehicles=<n> paying=<n>
FUNNEL year=<y> mode=ship invested=<n> vehicles=<n> paying=<n>
```

Each is one log line in the game. A mode with nothing counted and no vehicle is left out. A year
whose count began after January, after a save was loaded, ends with `partial=1`. The bot logs
the line with `AILog.Error`, as the referee logs its `REF` lines, because a headless match runs
at `-d script=1` and that level shows only errors. In the panel's log view the line therefore
shows at error level.

## The stages and their counters

### Rail

| Stage | Counter | Counted where |
|---|---|---|
| heartbeats | `hb` | `FunnelTick`, once a heartbeat from `main.nut` |
| affordability | `short_hb` | a heartbeat whose bank balance plus loan headroom was under `line_budget`, the money a new own line needs |
| idle money | `idle_hb` | a heartbeat whose bank balance alone reached the investor's threshold (`max(invest_min, INVEST_SURPLUS_RESERVES * cash_reserve)`). That money buys no train: `InvestSurplus` skips rail (fleet.nut) |
| discovery | `searches`, `found` | each `NextOwnFreightLine` call, and the calls that returned a pair |
| discovery census | `producers`, `ours`, `claimed`, `small`, `served`, `no_acceptor`, `pairs_tried` | the last search's skip counts (its `why` table, the same counts as the `no own freight line` message): producers on land; skipped as ours, or as claimed by another company's rail station; cargoes under 40 a month or transported by others; cargoes with no acceptor in the distance window; pairs already tried. A key the search adds later, such as `released`, is printed as well |
| construction | `tried` | entry of `BuildFreightRailRoute` and `BuildRailRoute`. A double track retried as single track counts a second time, because it builds new stations and searches again |
| construction | `stations` | both termini stand: entry of `BuildRailLineAndTrain` or `BuildDoubleRailLineAndTrain`. `tried - stations` were lost to a station site or to a claim |
| construction | `path_fail` | no rail path between the two stations |
| construction | `laid` | the track was laid and walked. `stations - path_fail - laid` broke while it was laid |
| construction | `single_track` | a double-track line with no double path (retried single), or whose second track was not built |
| construction | `built` | the line is kept, with a train or waiting for one (`KeepTrainlessLine`). `laid - built` were given up for no depot or for a train that can never run |
| affordability | `trainless`, `loco_refused_cash` | kept without a first train. `loco_refused_cash` counts each locomotive refused for `ERR_NOT_ENOUGH_CASH`, including later retries and the `test_trainless` refusal |
| operation | `trained` | a line's first train was bought, when the line was built or later (`KeepLinesServed`) |
| operation | `lines`, `paying`, `trains`, `trains_added` | at the report: open rail lines, the lines whose trains together earned money last year, the company's trains, and the change in trains since the year began |

### Road, air and water

| Mode | Counter | Counted where |
|---|---|---|
| road | `tried`, `stop_fail`, `network_fail`, `depot_fail`, `vehicle_fail`, `built` | `BuildRouteInTown` (main.nut), one outcome per try. `vehicle_fail` is no engine or no bus bought. Intercity routes are not counted yet |
| air | `short`, `no_pair`, `tried`, `built` | `AutopilotAir` (autopilot.nut). Each call that passes the route caps ends `short` (money plus headroom under the route's price), with `no_pair` (no fresh town pair), or `tried` |
| road, air, water | `invested` | a vehicle that `InvestSurplus` added |
| road, air, water | `vehicles`, `paying` | at the report: the mode's vehicles, and those at least a year old that earned money last year |

## Reading it

`battle.py fitness` prints every row, and the post-mortem of a match carries each mode's last
row. The row ends with `most lost:` and, for each population, the stage that lost the largest
share of what reached it (`FUNNEL_DROPS` in battle.py):

- **discovery** counts producers of the last census;
- **build** counts tries;
- **run** counts lines or vehicles;
- for air, **discovery** counts calls past the caps.

Losses in different units are never compared. `paying` means different things by mode. A rail
line's trains pay together, so a train that loses money inside a paying line stays hidden. A
road, air or water vehicle pays on its own.

`fitness.must_funnel` in battle.json lists the contestants that owe a rail line for every game
year the match played whole (`no_funnel`). A match of N years from `start_year` owes
`start_year` to `start_year + N - 2`, because year Y's line comes at the first count of year
Y+1. This puts the funnel code itself under the fitness check on every change.

## First measurement (2026-11-24)

The run was `battle.py fitness` with lineup 0 only (PrismaticAI with default settings against
AAAHogEx, LuDiAI AfterFix and AdmiralAI), on seed 930967890 for 6 years. It ran on lundra from a
tmpfs copy of this branch's bot, with the binary of lundra's master checkout (`20261122-master-
g2a6a616221`). It was one bounded match, deterministic for that tree and binary, and it passed.
PrismaticAI ended at 2.52M against AAAHogEx's 7.31M. The rail lines it logged, abridged:

```
rail 1960: hb=1 idle_hb=1 searches=2 found=2 producers=41 claimed=0 small=17 served=3 tried=2 stations=2 laid=1 built=1 single_track=1 trained=1 lines=1 paying=1 trains=1
rail 1962: hb=1 idle_hb=1 searches=1 found=1 producers=41 claimed=8 small=15 served=5 tried=2 stations=1 path_fail=2 laid=0 built=0 lines=2 paying=2 trains=4 trains_added=1
rail 1964: hb=5 idle_hb=5 searches=5 found=5 producers=41 ours=2 claimed=12 served=10 pairs_tried=3 tried=5 stations=1 laid=1 built=1 trained=1 lines=3 paying=2 trains=5 trains_added=1
rail 1965: hb=67 short_hb=0 idle_hb=67 searches=66 found=3 producers=41 claimed=13 small=9 served=10 no_acceptor=2 pairs_tried=6 tried=3 stations=1 laid=0 lines=3 paying=3 trains=7 trains_added=2
air 1965: no_pair=66 tried=0 invested=0 vehicles=20 paying=19
road 1961: tried=4 network_fail=3 built=1
```

What it shows, for this seed and lineup only:

- **Discovery loses the most candidates, and the loss grows each year.** Producers claimed by
  another company's rail station rose from 0 of 41 in 1960 to 13 of 41 in 1965. Cargoes
  transported by others rose from 3 to 10. In 1965, 3 of 66 own-line searches found a pair, and
  every one of the 66 air calls found no fresh town pair.
- **Construction loses most of the rest before a track is laid.** In 1964, 4 of 5 tries never
  had both stations standing (a station site, or a claim at build time). In 1962, 2 path
  searches failed for 1 try that reached its stations. Over six years, 15 tries gave 3 kept
  lines.
- **Money is never the limit.** `short_hb` is 0 in every year, and `idle_hb` equals `hb`: at the
  end of every heartbeat the bank held the investor's threshold. Trains grew by 0 to 2 a year,
  to 7.
- **Until 1963, one heartbeat a year.** `hb=1` from 1960 to 1963 means that the main loop ended
  one heartbeat a game year. Building, and waiting while building, took the rest of the year,
  so everything that runs once a heartbeat ran once a year: fleet management, the steward's
  turn and the loan. That matches the steward rounds up to 548 days apart in the pass-2 evidence.
- Road: in 1961, 3 of 4 bus tries ended with stops on separate road networks.

Widening the search addresses the largest loss (discovery). A wider window alone would feed
more tries into a construction stage that already loses 4 in 5 at the stations.

## Limits and observer effects

- The counters are globals and are not saved. After a load, the first line covers the rest of
  the year only (`partial=1`).
- A stage can show more losses than entries (`path 2/1` in 1962): an attempt can reach its
  stations in December and fail its path search in January.
- An event counts in the game year in which it is counted. A heartbeat that builds can run for
  game months, so an attempt that starts in December can be counted in January.
- The census is the last search's, not a sum, because each search walks every producer again.
  The event counts sum over the year.
- The parser keeps the last line for each company, year and mode. A restarted bot that repeats a
  year replaces the earlier line.
- Cost: one date read per counted event, a few balance reads per heartbeat, and once a year a
  pass over the vehicle list and each rail line's trains. The funnel writes no game state and
  uses no random numbers. The yearly pass costs script time only.
- Not counted yet, follow-ups for the branches that rewrite those lines: the budget gate of the
  own-line block in `main.nut`, which is rewritten by `fix/prismatic-planned-and-early-rail`
  (`short_hb` measures the same condition from outside); a claim found at build time, and the
  rival race in `BuildFreightRailRoute`, which is rewritten by `fix/prismatic-claims-clean` (both
  are in `tried - stations`); station-site failures by cause, in `BuildRailStationAt`, which is
  rewritten by `fix/prismatic-sites-levelling`; intercity road routes; the steward's rounds,
  owned by `fix/prismatic-steward-cadence`.
