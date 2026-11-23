# Incident 2026-11-23: finished rail lines without a train, and nothing that saw them

Root-cause analysis of one failure in the owner's live 8-company game on `:8800` (seed 15805,
start 1950, company 0 PrismaticAI co-played by the owner). The game-wide post-mortem up to 1958 is
`research/incident-2026-11-23-live-game-15805.md` (PR #237). This record covers what that one
leaves open: why the bot kept losing money on rail it had already built, how that got into the
code, why no check noticed it, and what now stops it.

## What happened

From the panel's store (`game.month` events of run `panel-8800`, the sample of 1 January):

| 1 January | PrismaticAI value | PrismaticAI bank | AAAHogEx value |
|---|---|---|---|
| 1953 | 125,878 | 4,322 | 619,976 |
| 1956 | 237,802 | 10,147 | 4,340,334 |
| 1959 | 457,649 | 70,916 | 10,560,391 |
| 1963 | 881,262 | 536,928 | 29,382,385 |

On 1962-12-05 (`/api/overview`) company 0 ran 1 train, 15 road vehicles and 10 aircraft. AAAHogEx
ran 138 trains, 401 road vehicles and 33 aircraft. Company 0 owned 12 rail stations
(`/api/resources/stations`, facilities `train`), and its one train called at two of them.

Two of the ten trainless stations pairs were finished lines: track, both termini and a depot, no
train. The bot's log (`ai.log`, company 0):

- `IORE Wruntwood Iron Ore Mine - Sunfingburg Steel Mill`, started 1952-01-16, 180 rail pieces,
  4 bridges, ended at `locomotive not bought: ERR_NOT_ENOUGH_CASH`;
- `IORE Wetston Iron Ore Mine - Wruntwood Steel Mill`, started 1953-09-15, 123 rail pieces, 7
  bridges, ended the same way.

From 1959 the bank grew from 70,916 to 536,928 and the loan fell to 0, while those two lines still
stood without a train. One locomotive cost about 30,649 (`no second track ... train's 30649`,
1953-10-05). The rest of the trainless stations belong to lines given up half laid
(`ERR_NOT_ENOUGH_CASH` while laying) or found broken.

The board's rail monitor (`openttd:panel-8800:rail`) reported `rail: no problems` throughout, for
example at 1962-05-04.

## Why it happened

**1. The builders returned the same "0" for "nothing built" and "everything but the train".**
`BuildRailLineAndTrain` and `BuildDoubleRailLineAndTrain` (`tools/research/prismatic-ai/PrismaticAI/rail.nut`)
are documented as "Returns 1 when a train runs, else 0". Their last step was
`local train = PrismaticAI.BuildTrain(line); if (train == -1) return 0;`. The line table was
pushed into `RAIL_LINES` only after that, so a line whose locomotive was refused was dropped with
everything it had cost still on the map: no slot, no record, no train ever after. The same holds
for three earlier exits that leave the stations standing: `laid == null` (track given up half
laid), `broken != -1` (the walk check failed) and no depot. Only "no rail path" removed the
stations. The single-track return came in with `340209766d` (2026-11-18, the steward and monitor),
and the double-track builder copied it in `1998b05c6d` (2026-11-19).

**2. A leftover station also blocks the bot's own next line.** `NextOwnFreightLine` skips a
producer with a rail station of ours within 8 tiles (`OwnRailStationNear`). A station left by a
failed line therefore removes its producer from the bot's candidates for the rest of the game.
openttd-36's diagnostic for `fix/prismatic-restore-own-built` counts 5 such producers on this map.

**3. Every watcher derived the lines from what the bot reported about itself.**

- The rail monitor (`tools/research/mcp-panel/rhai/rail_lib.rhai`) reads a line from the
  `rail line <slot>: ...` record the bot logs (`LineRecord`), and that record was logged only
  after a train ran. Its `unserved` problem ("a line the bot built has no train") could therefore
  never fire for a line that never had a train. It did not match `locomotive not bought` among
  the rail warnings either.
- The headless fitness check (`tools/research/ai-battle/battle.py fitness`) judges value, fleet
  peak, hoarding and shared producers. None of these sees an asset that earns nothing. The hoard
  rule needs the money to double within 8 quarters to at least 500k, and the live game only got
  there in 1962. The check runs only on the gate's seed and is never applied to the live game.
- The panel's replan (`replan.rhai`) kept proposing new lines. Its 199 `openttd.ai.set_setting`
  calls for company 0 (1952-05-07 to 1962-12-04, `panel.mcp` events) all logged `pending
  approval`, and none was approved. Nothing reminds anyone of approvals that wait for years.

**4. The evidence the bot was tuned on could not show it.** The duel records pit PrismaticAI
against PrismaticAI, and the battle matches ran on maps where the first locomotive was
affordable. A finished line without a train needs money to run out between the last rail piece
and the locomotive. That happens on a crowded map with eight companies, which only the owner's
live game had.

### Hypotheses checked and rejected

- *The guard of #225 (`ForeignRailStationAt`) starves the bot's rich mode.* This session first
  named it as the cause of the 1959-1962 stall. The coordinator's headless replay
  (`research/incident-2026-11-23-live-game-15805.md`, "On the map of 1958 the bot has almost
  nothing left to claim") measured the holders moving 72-91% of every producer AAAHogEx holds. At
  `rich_served_below` 60 the rich mode would skip those producers without the guard too. This is
  not the binding constraint, and #225 stays as it is.

## What changed

- **A line that will run no train leaves no station** (`AbandonLineStations`, `rail.nut`). The
  track given up half laid, a broken line, a line with no depot, and a train that can never run
  (no wagon carries the cargo, orders refused) all remove both termini, as "no rail path" already
  did. The track already laid stays, because the bot keeps no list of the pieces of a partial line.
- **A finished line whose first train was not bought stays a line** (`KeepTrainlessLine`). When
  `BuildTrain` fails for money (`ERR_NOT_ENOUGH_CASH`, including the first wagon) or because there
  is no engine for the cargo yet (`line.train_fail` says which), the line goes into `RAIL_LINES`
  with no train and logs its record. It returns 1, because the line is built.
  `KeepLinesServed` buys its first train once the bank holds the reserve plus the train's price
  (`TrainPrice`). It does not count that train as one of the line's two replacements, and it does
  not try a purchase the bank cannot pay on every heartbeat.
- **Trainless lines already on the map are adopted** (`AdoptTrainlessLines`, `lines.nut`), from
  `Start` after `Restore` and then once a game year. An adopted line is a pair of our rail stations,
  at least one of them with no train of ours calling (the other may be a terminus a served line
  joined), that no line runs between and no train of ours serves both of. The first station's
  catchment holds a producer of a freight cargo and the second's an industry accepting it
  (`CaughtIndustry`, by catchment and not by distance). The pair must be joined by our own track
  (`RailWalk`, the walk the lost-train repair uses) and have a joined rail depot near
  (`RestoreDepot`, `DepotJoined`). This is what brings the two iron ore lines of the live game back
  once the build is deployed there (see Validation).
- **The rail monitor reads the map, not only the bot's records** (`trainless_stations`, `report`,
  `rail_lib.rhai`). Every rail station of the company that no train of the company has orders to
  is a `trainless_station` problem, ranked after `unserved`. `locomotive not bought`, `first train
  was not bought` and `given up (` are rail warnings. Tests:
  `rail_monitor_reports_rail_stations_no_train_calls_at` in `tools/research/mcp-rhai/src/main.rs`.
- **The fitness check fails a PrismaticAI that keeps trainless rail stations.** The referee Game
  Script reports `REF T ... rail_stations=<n> trainless=<n>` per company each quarter. It reads
  the station and vehicle lists in deity mode, so the number comes from the game and not from the
  bot, with the station ids. `battle.py fitness` fails with `trainless_stations:` when a PrismaticAI
  company kept more than `trainless_max` (0) of the same stations trainless through the last
  `trainless_reports` (4) reports, a game year, so the termini of a line still being laid do not
  count. Tests: `TrainlessStations` and `FitnessRun` in `tools/research/test_ai_battle.py`.
- **The fitness check runs the recovery on every change.** The RESEARCH-ONLY setting
  `test_trainless` (default 0) refuses the first train of every new line as if for money (1), and
  also forgets such a line a month later as a restart would (2). The sixth fitness lineup,
  `PrismaticAI trainless test`, runs with `test_trainless=2`. It is in `fitness.must_run_trains`,
  which fails with `no_trains` when it ends without a train. It is in `trainless_skip`, because it
  keeps stations trainless on purpose: each of its lines is refused twice, and a refused line waits
  for the main loop, which does not run while the bot lays a long line in one heartbeat (on the
  fitness seed, 196 and then 138 days of path search for one line).
- **A laid line whose depot was refused for money stays a line too** (`LineDepot`,
  `KeepDepotlessLine`, `BuildPendingDepot`). After `RailCashWait` (#234) the track is laid, and the
  depot can meet the same shortage that the train meets. Such a line is kept with depot -1 and
  `depot_pending`, and `KeepLinesServed` builds the depot, then the train, once the bank holds both.
  openttd-36's review of #234 pointed at this case. The test hook refuses every other new line's
  depot, so the fitness lineup runs it.

## How it could have been prevented

- **Report a partial outcome as a state, not as a failure.** A builder that leaves assets on the
  map must either clean them up or hand them to the bookkeeping. The new code does one or the
  other on every exit of both builders.
- **A watcher reads the ground truth.** All three watchers had trusted the bot's account of its
  own lines. Both new checks count stations and trains from the game itself: the referee in deity
  mode, the monitor through `openttd://stations` and `openttd://vehicles`.
- **An invariant beats a metric.** "Every rail station of ours has a train or belongs to a line
  waiting for its first" can be checked at any moment on any map. A value or fleet threshold only
  fires once the damage is large.

## Validation

All runs used `build/openttd` `20261122-master-gfdc13851cf` (default build, the C++ is unchanged)
and the arena cache at `~/ttd-battle/arena`. "Master" is master's PrismaticAI (`git archive
origin/master`) judged by this branch's `battle.py` and referee, so both sides are counted the
same way.

**The check refuses the build it must refuse.** `battle.py fitness`, gate seed 930967890:

| Lineup | Master: trainless a year | Branch: trainless a year | Branch: trainless at the end |
|---|---|---|---|
| PrismaticAI | 3 | 0 | 0 of 12 |
| PrismaticAI self-steward | 6 | 0 | 2 of 10 |
| PrismaticAI solo-max | 3 | 0 | 0 of 15 |
| PrismaticAI planned | 1 | 0 | 2 of 18 |
| PrismaticAI sibling A | 2 | 0 | 0 of 11 |
| PrismaticAI trainless test | (not on master) | 0 | 0 of 10 |

Master fails with `trainless_stations` in all five of its lineups, and the branch passes (`fitness:
PASS (6 match(es), 6 years, seed 930967890)`). The stations trainless at the end on the branch
belong to lines still being laid: in the self-steward match, the termini of `WOOD Hartburg Forest -
Pronhattan Sawmill` stood three quarters before its first train, which is why the rule counts the
same stations over a year and not a count over two reports (the first version, which master and
branch then both failed).

On the live game's seed 15805, master fails `trainless_stations` in three lineups (3, 3 and 3
stations) and `shared_producer` once. The branch fails neither. Both fail `hoards` in the
`PrismaticAI` lineup with the same final value (3,316,940), so this change neither causes nor fixes
that failure. Final values on the two seeds, one match per side (not a tuning claim, rule
`project.a-tuning-claim-needs-held-start-pairs`): planned 1.26M -> 1.52M and 1.75M -> 2.79M,
solo-max 1.54M -> 1.66M and 2.33M -> 2.64M, PrismaticAI 1.80M -> 1.72M and unchanged.

On seed 31337 master fails `trainless_stations` in five lineups and `shared_producer` once. The
branch fails `trainless_stations` once (the halted line under "What is still open") and `hoards` in
`PrismaticAI planned`, which went from 3.66M on master to 2.61M. The two runs part from their second
line on: on the branch, `COAL Slunbourne Coal Mine - Flintston Power Station` gives up and removes
its stations, and master leaves them standing. From then on each bot plans a different map. A
single seed of a chaotic headless match cannot attribute the difference either way.

**The new paths run.** With the AI log at `-d script=4`, the `PrismaticAI trainless test` lineup on
seeds 930967890 and 15805 logged 13 `first train was not bought`, 13 `test_trainless forgot`,
13 `adopted trainless` and 12 `got its first train`, and no Squirrel error. Two versions of the
adoption failed this test before it passed: one required no train at either station, but a
terminus a served line joined has trains, and one matched a station to its industry within
`RESTORE_INDUSTRY_REACH` (16 tiles from the industry's location), but the Pronhattan Forest station
stands 19 tiles from it. The adoption now reads the station's catchment (`CaughtIndustry`).

**The live game's two lines come back.** The owner's game was stopped and saved at 01:06 on
2026-11-23 (`~/Documents/OpenTTD/save/final-15805-0106.sav`, game date 1963). A copy, loaded
headless with this branch's bot for 45,000 ticks (to 1966-07), logged:

```
PrismaticAI: restored after a restart: 1 rail lines (1 freight), ... (from the save)
PrismaticAI: adopted trainless rail line IORE Wetston Iron Ore Mine - Wruntwood Steel Mill (stations 166 and 171, depot 25914); it waits for its first train
PrismaticAI: adopted trainless rail line IORE Wruntwood Iron Ore Mine - Sunfingburg Steel Mill (stations 106 and 107, depot 21532); it waits for its first train
PrismaticAI: line IORE Wetston Iron Ore Mine - Wruntwood Steel Mill got its first train (Floss '47' (Diesel))
PrismaticAI: line IORE Wruntwood Iron Ore Mine - Sunfingburg Steel Mill got its first train (Floss '47' (Diesel))
```

Neither train was reported lost. The bank still rose, to 940,296 by 1966-07. Two trains do not
spend the money a crowded map leaves idle (see the open finding of the sibling-industries incident).

Offline: `python3 -B tools/research/test_ai_battle.py` (72 tests, `TrainlessStations`,
`FitnessRun`), `cargo test` in `tools/research/mcp-rhai`
(`rail_monitor_reports_rail_stations_no_train_calls_at`), `tools/coverage/changed-lines`,
`tools/surfaces/surfaces check` and `tools/gate.sh change`. Their results are in the pull request.
Squirrel and Rhai lines have no line coverage gate. The headless runs above are their evidence.

## What is still open

<!-- open-finding kind=staged -->
The track of a line given up half laid stays on the map, and so does its upkeep. The builders keep
no list of the pieces they laid before `BuildRailAlong` gave up, so removing it needs that list
first.

<!-- open-finding kind=staged -->
Re-plans that wait for approval are not surfaced. 199 `set_setting` calls for company 0 stayed
pending from 1952 to 1962, and the plan the bot runs is still that of 1950-02-08. Whether the
co-play company's re-plans are approved by the panel or by the owner is the owner's decision.

<!-- open-finding kind=partial -->
The trainless rule counts the same station ids over a year. Two cases can still mislead it, and
neither has been seen in a judged lineup on the gate seed: a line whose laying takes longer than
a year, and a station id the game hands to a new station soon after the old one was removed. The
failure message names the stations so that a reader can tell.

<!-- open-finding kind=failed -->
A line whose trains got lost on a whole track is halted (`lost_trains`, `events.nut`) and keeps its
stations without a train for the rest of the game. On seed 31337 the `PrismaticAI solo-max` lineup
fails `trainless_stations` on such a line (station 98, `WOOD Gonbourne Forest - Wundwood Sawmill`,
`train 470 reached no station in 266 days on its whole track`), and master fails there too. A
halted line should either be repaired or closed, so that its stations go.

<!-- open-finding kind=staged -->
A line waiting for its depot (`depot_pending`) has no depot for the adoption to find. When a restart
from the map loses it, it is not adopted again.

<!-- open-finding kind=partial -->
An adopted line is only as good as the pairing heuristic. Two trainless stations near a producer
and an acceptor that the walk joins are taken as one line even when the bot never planned them as
one. A train that gets lost there is sold within `LOST_SELL_DAYS` and the line is halted
(`events.nut`). This has not been observed in a match, because no fitness seed leaves such a pair.
