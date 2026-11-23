# Incident 2026-11-23: PrismaticAI ran one train for eight years in the owner's live game

Post-mortem. In the owner's live 8-company game on `:8800` (seed 15805, start 1950, binary
`20261122-master-g6d1eda39e1`), company 0 runs PrismaticAI and the owner co-plays it. By the end of
1957 AAAHogEx was worth 7.75M, and in June 1958 it ran 106 trains. PrismaticAI was worth 0.38M and
had one train. The bot tried 15 rail lines. One of them got a train. Four ran out of money: two of
them were finished but got no locomotive, and two were given up half laid. Five found no site for a
station. Four were skipped because AAAHogEx already held the producer, and one ran into another
company's track. After March 1954 the bot planned no more rail lines, because its four own lines
were used up.

The game was saved and loaded at 1952-10-23 with #225 deployed (`rescan_ai`, save, load). At
23:42 on 2026-11-22, at game date 1958-05-26, the panel loaded a save dated 1958-04-13 (`load 2`,
`game.new_world`), with master `f4c02d0aac` deployed. That master has #232 and #234. Unless a
section says otherwise, this post-mortem reads the bot's log up to 1958-04-29 (read before that
load), the year-end samples up to 1957 and the vehicles as of 1958-06-16.

## What happened

Company value in thousands, from the panel's store (`samples` of the latest run of `panel-8800`,
seed 15805, the last sample of each game year):

| Year | 0 PrismaticAI | 1 AAAHogEx | 2 LuDiAI AfterFix | 3 CivilAI | 4 AdmiralAI | 5 NoCAB | 6 DictatorAI | 7 SimpleAI |
|---|---|---|---|---|---|---|---|---|
| 1950 | 101 | 117 | 7 | 0 | 67 | 0 | 4 | 33 |
| 1951 | 180 | 344 | 0 | 0 | 164 | 51 | 64 | 75 |
| 1952 | 127 | 624 | 0 | 0 | 248 | 32 | 113 | 177 |
| 1953 | 149 | 1456 | 5 | 0 | 346 | 69 | 145 | 300 |
| 1954 | 153 | 2747 | 74 | 0 | 457 | 149 | 115 | 380 |
| 1955 | 238 | 4340 | 186 | 0 | 560 | 251 | 187 | 396 |
| 1956 | 317 | 6302 | 296 | 0 | 630 | 278 | 253 | 379 |
| 1957 | 382 | 7750 | 355 | 0 | 726 | 272 | 373 | 364 |

Company 0 was second at the end of 1950 and 1951. AdmiralAI and SimpleAI passed it in 1952. At
the end of 1957 it was third again, behind AAAHogEx and AdmiralAI, with 382k against 7,750k. Its
loan was 300k, the maximum, at the end of 1952 and 1953. From 1954 it paid the loan down: 230k at
the end of 1954, 60k at the end of 1957, 0 at 1958-08-23.

The vehicles that were running on 1958-06-16 (`/api/resources/vehicles`), with their profit of
1957:

| Company | Trains | Profit of trains | Road vehicles | Profit of road vehicles | Aircraft | Profit of aircraft |
|---|---|---|---|---|---|---|
| 0 PrismaticAI | 1 | 33,855 | 15 | 5,562 | 10 | 79,761 |
| 1 AAAHogEx | 106 | 2,868,750 | 375 | 339,913 | 29 | 182,995 |

The sum for company 0, 119,178, is the `income` that `/api/bridge/companies` shows. Of the
3,272,480 a year between the two companies, the trains make 2,834,895 (87%), road vehicles 334,351
(10%) and aircraft 103,234 (3%). PrismaticAI's one train earned 33,855. AAAHogEx's trains earned
27,064 each on average. So a train of ours does not earn less than one of theirs. The gap is in how
many trains each company has. These figures leave out vehicles sold before 1958-06-16.

Company 0 has 12 rail stations (`/api/resources/stations`, 1958-05-14). Its one train calls at two
of them, Minnbridge Heights and Dontford Heights. No train calls at the other ten.

## Why it happened

The bot's log (`ai.log` events of company 0, 4,797 lines from 1950-02-08 to 1958-04-29) records
every rail line it tried. "Rail pieces" counts its `built rail at tile` lines:

| Started | Line | Who chose it | Ended | Rail pieces | Bridges |
|---|---|---|---|---|---|
| 1950-03-18 | COAL Flardingham Coal Mine - Dretford Power Station | planner | `ERR_OWNED_BY_ANOTHER_COMPANY`, 26 tiles short | 152 | 1 |
| 1950-04-22 | COAL Minnbridge Coal Mine - Dontford Power Station | planner | **built, one train** | 129 | 2 |
| 1951-04-14 | OIL_ Sunfingburg Oil Wells - Dontford Oil Refinery | planner | no station site (30 sloped) | 0 | 0 |
| 1951-04-26 | COAL Binfingfield Coal Mine - Minnbridge Power Station | planner | no station site (0 sloped) | 0 | 0 |
| 1952-01-16 | IORE Wruntwood Iron Ore Mine - Sunfingburg Steel Mill | planner | track done, locomotive `ERR_NOT_ENOUGH_CASH` | 180 | 4 |
| 1952-02-10 | Ponninghall - Harnwell (passengers) | planner | `ERR_NOT_ENOUGH_CASH` while laying, given up | 85 | 5 |
| 1952-07-14 | GRAI Sunfingburg Farm - Bredhattan Ridge Factory | planner | `ERR_NOT_ENOUGH_CASH` while laying, given up | 112 | 5 |
| 1953-04-12 | IORE Bredhattan Ridge Iron Ore Mine - Wruntwood Steel Mill | planner | skipped, AAAHogEx holds the producer | 0 | 0 |
| 1953-08-21 | IORE Plondham Iron Ore Mine - Wruntwood Steel Mill | planner | skipped, AAAHogEx holds the producer | 0 | 0 |
| 1953-08-30 | OIL_ Sardington Oil Wells - Brennpool Oil Refinery | planner | skipped, AAAHogEx holds the producer | 0 | 0 |
| 1953-09-15 | COAL Sunfingburg Coal Mine - Dretford Power Station | planner | skipped, AAAHogEx holds the producer | 0 | 0 |
| 1953-09-15 | IORE Wetston Iron Ore Mine - Wruntwood Steel Mill | own 1/4 | track done, locomotive `ERR_NOT_ENOUGH_CASH` | 123 | 7 |
| 1954-02-27 | STEL Sunfingburg Steel Mill - Bredhattan Ridge Factory | own 2/4 | no station site (30 sloped) | 0 | 0 |
| 1954-03-07 | STEL Sunfingburg Steel Mill - Dretford Factory | own 3/4 | no station site (30 sloped) | 0 | 0 |
| 1954-03-15 | GRAI Wruntwood Farm - Bredhattan Ridge Factory | own 4/4 | no station site (0 sloped) | 0 | 0 |

The bot laid 781 rail pieces and 24 bridges in all. A train runs on 129 of those pieces (17%). The
store does not record what each line cost: the bot logs no total for a line, so the money sunk into
the other 652 pieces cannot be measured from these records.

Each hypothesis, checked against this log:

- **1. Failed own lines used up `own_lines=4`. Confirmed.** The heartbeat shows `own_cap=4` from
  start to end. The four own plans ran from 1953-09-15 to 1954-03-15. One line was laid and got no
  train, and three found no station site. Each was counted when it was tried (`own_built++` before
  `BuildFreightRailRoute`). The planner's ten slots were spent by then, and the passenger pair too.
  After `own plan 4/4` on 1954-03-15 the log has no `rail plan` line up to 1958-04-29, so for more
  than four years the bot tried no rail line at all. #234 fixes this. In its `battle.py ablate` on
  this seed (field of four rivals, 6 years, one match per side) master had 1,011,217 and 5 trains,
  and the branch 1,057,742 and 7 trains. That is a single match and not a tuning claim. Its effect
  in this game is not measured, because #234 was never deployed into it.
- **2. Lines failed with "no rail station site". Confirmed, five of 15 lines.** Three of them ended
  with `30 sloped site(s) within 1 height step(s)`: Sunfingburg Oil Wells once and Sunfingburg Steel
  Mill twice. Two ended with `0 sloped site(s)`, at Minnbridge Power Station and Bredhattan Ridge
  Factory. That message means there was no candidate at all, not even one that levelling could fix.
  Two of the five were own lines 2/4 and 3/4, and the third own failure (4/4) was the `0 sloped`
  case at Bredhattan Ridge Factory. Two of these lines had their source station built first, at
  Binfingfield Coal Mine and Wruntwood Farm. In both cases the bot removed that station again
  (`RemoveRailStation`, `rail.nut`). Neither of them is among company 0's stations today.
- **3. After #225, AAAHogEx held the planner's producers. Confirmed, four of the planner's ten
  freight slots.** The four lines from 1953-04-12 to 1953-09-15 are all
  `skipped: company 2 already has a rail station at ...`. The bot prints the company id plus one
  (`holder + 1`, `rail.nut`), and `ForeignRailStationAt` returns the lowest foreign owner. So
  "company 2" is company id 1, AAAHogEx. The guard did what #225 meant it to do: no rail piece was
  laid towards a producer AAAHogEx served. The loss is that the planner chose those producers. It
  still does: the plan of 1958-05-18 (`plan.bot_settings`) puts Iron Ore from industry 12 to 11
  first. That is the Plondham line (157 tiles) that was skipped on 1953-08-21. Before #225, the
  first line (Flardingham) stopped at `rail at tile 37716 failed: ERR_OWNED_BY_ANOTHER_COMPANY`.
  The log does not say whose track it was.
- **4. `ERR_NOT_ENOUGH_CASH` in mid-build. Confirmed, four of the 15 lines and 500 of the 781 rail
  pieces.** There are 6 `rail at tile ... failed: ERR_NOT_ENOUGH_CASH` lines in two lines
  (Ponninghall - Harnwell in 1952-02, Sunfingburg Farm - Bredhattan Ridge Factory in 1952-08).
  Each line was re-planned twice and then given up, with 85 and 112 pieces laid. There are 2
  `locomotive not bought: ERR_NOT_ENOUGH_CASH` lines, and these weigh more. Wruntwood Iron Ore
  Mine - Sunfingburg Steel Mill (1952-02-10, 180 pieces) and Wetston Iron Ore Mine - Wruntwood Steel Mill
  (1953-10-05, 123 pieces) are finished lines with stations and a depot, but they have no train.
  `BuildTrain` returns -1, the line returns 0 before `RAIL_LINES.push`, and so no steward ever sees
  it. After the load the bot restored `1 rail lines (1 freight)`. Their four stations (Wruntwood
  Woods, Sunfingburg Annexe, Wetston North, Ponninghall Sidings) are among the ten that no train
  calls at. The cause is the gate a line starts behind. A line starts when the balance plus the
  loan headroom reach `line_budget` = 30000 (`main.nut`), and then the bot borrows up to the limit.
  The same log puts one train at 30649 (`no second track ... leave less than the train's 30649
  (funds 6390)`, 1953-10-05). The Wruntwood line started with 14848 in the bank and 30000 of
  headroom. Besides these, construction was frozen ("borrowed to stay afloat") for 815 of the 1387
  days from 1950-02-09 to 1953-11-27. Three buses were refused with
  `failed to build vehicle 1: ERR_NOT_ENOUGH_CASH`.
- **5. Buses earn almost nothing. Confirmed, and they are 10% of the gap.** In 1957 the 15 buses
  earned 5,562 together, 371 a bus. AAAHogEx's road vehicles earned 906 each, LuDiAI's 1,067 and
  DictatorAI's 1,193. All 15 run between two stops of one town (their orders, for example
  Sunfingburg South - Sunfingburg North), although `intercity=1`. The intercity attempts ended at 5
  `no road path from` and 3 `road stopped at tile`. The log also has 248 `stops ... are on separate
  road networks; no buses bought` and 233 `no pair of towns ... left to connect`. The fleet sold 21
  buses as unprofitable and 8 more where nobody waited. The air route Lenhill City - Wruntwood
  earned 7,976 a plane in 1957 (`air report 1958`), and a second air route was refused on 1957-11-26
  with `ERR_LOCAL_AUTHORITY_REFUSES`.

Four more findings are not among the hypotheses:

- **The planner's re-plan never reached the bot.** `replan.rhai` re-planned 44 times up to
  1958-05-18 (`plan.bot_settings`). On 1952-05-07 and 1952-05-08 it sent 29
  `openttd.ai.set_setting` calls for company 0 (`panel.mcp`), and each was logged as `pending
  approval`. The bot's running slots in 1958-05 (`/api/bots`: `fr2_from=29`, `rail_b=3`, ...)
  are still those of the first plan of 1950-02-08, not those of 1952-05 (`fr2_from=12`,
  `rail_b=2`). The store does not say whether those approvals
  were denied or ran out.
- **The load of 1958 kept the old count of own lines.** After the load the bot logged
  `restored after a restart: 1 rail lines (1 freight), ... 14 freight lines tried, 4 own`. The
  company has one rail line, but the save carried `own_built=4` from the old code, which had
  counted the failed tries. So #234's slot return could not reach the running game. The
  coordinator measured this in a headless replay of `deploy-ai-2345.sav`, the save after the
  `f4c02d0aac` deploy, with `build-gate-research/openttd` and a diagnostic build of PrismaticAI.
  In the live log the next own plan is `own plan 5/32` on 1959-11-07, with a cap of 32. It came
  after the pressure switch of 1959-10-02 (`pressure high: leader AAAHogEx value 13755694 vs ours
  512265`, then `rich=1`). That line, WOOD Dretford Forest - Flardingham Sawmill, ended at `no rail
  station site at Flardingham Sawmill (0 sloped site(s) ...)`.
- **On the map of 1958 the bot has almost nothing left to claim.** The same headless replay measured
  `NextOwnFreightLine` with the restore fixed, over about one game year after the load. It found no
  candidate. Of 42 producers on land, 5 have a station of ours near them, 26 producer-cargoes are
  held by another company's rail station, 9 to 10 produce under 40 a month, and 5 are transported by
  others. None was left out as already tried. The holder moves 72-91% of every cargo of the
  producers held by a rival (AAAHogEx). Two examples are Plondham Iron Ore Mine at 79-80% of 208-234
  a month and Minnbridge Coal Mine at 72-79% of 189. Sharing such a producer is not worth a line, so
  the coordinator dropped a change that would have shared claims. These figures hold for that save,
  replayed headless, in about one game year after the load. They are not measured in the live game.
- **The planner and the bot disagree on `own_lines`.** The planner logs `the bot may plan up to 8
  more itself (own_lines)`, and its version parameters carry `own_lines: 8`. Yet the bot was
  started with `own_lines=4` (`PRISMATIC_BOT_SETTINGS`) and runs with it.

## What changed

Merged into master (`gh pr list -R korczis/OpenTTD --state all`):

- **#225** (merged 2026-11-22): a raw producer where another company has a rail station is no
  candidate for the bot. This is the guard behind hypothesis 3. It is deployed in this game since
  1952-10-24.
- **#232** (merged 2026-11-22): the second track is no longer refused for a cost that only the
  path search estimated. This game shows one `no second track` refusal (Wetston, funds 6390). The
  line had no train anyway, so this game cannot tell whether #232 would have changed it. It was
  deployed in the load of 1958.
- **#234** (merged 2026-11-22): a failed own line gives its `own_lines` slot back, and a rail,
  bridge or tunnel piece refused for money waits for income (`RailCashWait`, at most 120 days)
  instead of dropping the line. This covers hypothesis 1 and the six mid-build failures of
  hypothesis 4. It was deployed in the load of 1958, but the restored `own_built=4` kept its slot
  return from taking effect.

In flight, not merged, with no pull request open at the time of writing:

- `fix/prismatic-restore-own-built`: uncommitted changes to `restore.nut` and `rail.nut`. A load
  recounts `own_built` from the rail lines it restores instead of taking the saved counter.

- `fix/prismatic-station-sites`: uncommitted changes to `rail.nut` and `main.nut`. A terminus
  tries both axes and the reversed exit, and builds on sloped land with foundations before it
  levels a site. This aims at hypothesis 2.
- `fix/planner-per-company`: uncommitted changes to `mcp-plan`, `panel_bot.rs`, `replan.rhai` and
  `cargo.rs`. The planner leaves out producers that another company holds and plans each company on
  its own. This aims at hypothesis 3 and the Plondham line in the 1958 plan.
- `fix/prismatic-steward-capital` and `fix/prismatic-road-air-yield`: the worktrees exist at
  master `f4c02d0aac`, with no change yet.

## What is still open

<!-- open-finding kind=failed -->
A line whose track is finished but whose locomotive is refused for money is left on the map with
no train, and the bot forgets it: `BuildTrain` returns -1 and the line never enters `RAIL_LINES`.
In this game that happened to two lines with 303 rail pieces and 11 bridges. #234's `RailCashWait`
covers rail, bridge and tunnel pieces only, not the locomotive. Nothing adopts such a line later,
and a load does not either.

<!-- open-finding kind=failed -->
A line that is given up leaves its stations and track behind. Ten of company 0's 12 rail stations
have no train. Two of those lines (Wruntwood, Wetston) have no train because of the finding above.
The other three (Flardingham - Dretford, Ponninghall - Harnwell, Sunfingburg Farm - Bredhattan
Ridge) were half laid and given up. Whether the game charges infrastructure maintenance for them in
this game was not read.

<!-- open-finding kind=partial -->
A line still starts once the balance plus the loan headroom reach `line_budget` (30000 in this
game), which is less than the price of the one train it needs (30649 in this log). #234 makes a line
that runs short of money wait, but the gate still starts lines the company cannot equip.

<!-- open-finding kind=staged -->
A save written by the old code carries `own_built` with the failed tries in it, so #234's slot
return does not reach a game loaded from it. In this game the bot restored `4 own` with one rail
line. A load that recounts `own_built` from the restored lines is on
`fix/prismatic-restore-own-built`, and its pull request has not been opened yet. The only figure
for #234 on this seed is its single ablation match, with four rivals instead of seven.

<!-- open-finding kind=partial -->
On a mature map, more own lines do not help much. In the headless replay of the 1958 save,
`NextOwnFreightLine` found no candidate even with the restore fixed: 26 producer-cargoes were held
by a rival's rail station, and the holder moved 72-91% of their output. The levers left are two.
One is growing the lines that already earn (more trains, second tracks). The other is claiming
producers in the first one or two years, before AAAHogEx takes them. In this game AAAHogEx's log
scored all four producers it later held on 1950-02-15. Neither lever is
implemented.

<!-- open-finding kind=staged -->
The planner still hands company 0 producers that AAAHogEx holds. The first line of the plan of
1958-05-18 is the Plondham Iron Ore line that the bot skipped in 1953. Planning per company with
the held producers left out is on `fix/planner-per-company` and is not merged.

<!-- open-finding kind=staged -->
Five of the 15 lines found no station site. Sites on sloped land, on the other axis and with the
exit reversed are on `fix/prismatic-station-sites` and are not merged. Whether that branch would
have found a site for the two `0 sloped` cases is not known.

<!-- open-finding kind=failed -->
The 29 `set_setting` calls of the re-plan of 1952-05 waited for approval and never reached the bot.
It still runs the slots of 1950-02-08. The store has no record of how those approvals ended.

<!-- open-finding kind=staged -->
From 1954 the company paid its whole 300k loan back (0 by 1958-08-23) and built no rail line,
while its buses earned 371 a year each. The capital sink and the road and air yield are the
subjects of `fix/prismatic-steward-capital` and `fix/prismatic-road-air-yield`, which have no
change yet.

<!-- open-finding kind=not-run -->
Some things were not measured. The records do not give the money each failed line cost, whose
track stopped the Flardingham line at tile 37716, or why line 1 runs one train when it may run two
(`trains_max=2`).
