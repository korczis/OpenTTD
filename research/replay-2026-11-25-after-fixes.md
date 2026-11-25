# Replay 2026-11-25: live saves before and after the PrismaticAI fixes

Verification note (openttd-36, agent "replay"). Copies of the owner's live saves were loaded
headless with `battle.py replay` (#274), once with master's PrismaticAI and once with the pre-fix
`eb0d839ce0`, for four game years each. The question was whether the fixes from
[the analysis of 2026-11-23](analysis-2026-11-23-why-prismatic-loses.md) (#271, #272, #276, #277,
#279, #280) change what a loaded live game does. It also asks whether a bot that the save left
"planned" with `own_lines=4` keeps the floors of #279 after the load (`own_lines` 10,
`max_routes` 30). That decides whether deploying master into the running live game helps.

**Answer.** After the load, master keeps the floors in every replayed save: every heartbeat reads
`own_lines=10 max_routes=30 cash_reserve=5000 autopilot=growth-on-plan`. Keeping the floors did not
make the bot grow. In the two late, rich saves the `rich` layer already lifts the bot's own-line
cap to 32 under both codes, so the floor never binds there. On the early 15805 save the bot is
short of money on 74 of 76 heartbeats of 1953. The stages that bind are candidate discovery,
station and path building, and a capital allocator that finds nothing to buy. Over four years the
two codes end within 7% of each other's value on every save. Deploying master into the live game
21647 is therefore not expected to change its course measurably. The live game was restarted from
`final-21647-1992.sav` with master's PrismaticAI around 04:10 CET, while these replays ran. #285,
merged later, does not end the live game's hoard either. On that save it buys nothing in two game
years, the same as the live tree, because every line that earns already runs its `trains_max=2`
trains.

## Setup

- **Replay.** `battle.py replay --config <scratch> --save <copy> --years 4 [--rev eb0d839ce0]
  --binary <wrapper> --log-dir <dir>`, run from the `doc/replay-after-fixes` worktree at
  `8f35e35995`. Its PrismaticAI tree is identical to `42e3d37ae7`, and `src/` is identical to the
  binary's. "master" below means that tree. The note was committed on a later master, where #285
  had changed PrismaticAI again.
- **Binary.** `20261124-master-g53cd7b467d` (`build/openttd` of the primary checkout, copied with
  its `lang/`, `ai/`, `game/` and `baseset/` so that no rebuild could change it mid-run). A
  wrapper swaps `-d script=1` for `-d script=4`, so the heartbeat, capital and claim lines
  (`AILog.Info`/`Warning`) reach the log. The FUNNEL lines are `AILog.Error` and show at either
  level.
- **Rivals.** The scratch config points `cache` at a copy of the live AI tars
  (`~/Documents/OpenTTD/ai/*.tar` and `library/`): AAAHogEx 115, LuDiAI AfterFix v27, CivilAI 37,
  AdmiralAI 25, NoCAB 2.1.4, DictatorAI 183 and SimpleAI 14. The arena cache lacks the last three.
  No replay reported a substituted AI. The config also raises `timeout_s` to 10800. The rules
  are battle.json's: at least 2 trains, more than one train in some month of the last 3 years,
  and at least 0.05 of the leader's value.
- **Saves.** All were read-only copies. Nothing was written into the live game or its save
  directory.

  | Save | Game | Loaded at | SHA-1 |
  |---|---|---|---|
  | `deploy-225.sav` | 15805, its earliest save | 1952-10-24 | `daeb8132d7` |
  | `final-15805-0106.sav` | 15805 with its continuation | 1964-11-09 | `38b9e44652` |
  | `autoapprove-incident-0429.sav` | 12609 | not loadable | `283332c3c0` |
  | live `autosave0.sav` of 2026-11-25 03:23 | 21647 | 1990-12-23 | `0504ddabec` |
  | `final-21647-1992.sav` (on lundra, 2 years) | 21647, the live game's restart | 1992-03-15 | `5107e93b5d` |

- **Limits.** Two game processes at most, each pair started only while the 1-minute load was
  under 24 and at least 10 GiB was free. The pairs started at loads of 17-23; in between, the
  machine's load rose to 40-70.

## Results

Company 0 is PrismaticAI in every save. "Ratio" is its value over the leader's (AAAHogEx in
every row), taken from the console's monthly `companies` report.

| Save | Code | Trains, end of each year | Value at the end | Ratio | Verdict |
|---|---|---|---|---|---|
| deploy-225 | master | 1, 1, 1, 3, 7 (1952-12 … 1956-10) | 300,681 | 0.047 | **FAIL**: value under 0.05 |
| deploy-225 | `eb0d839ce0` | 1, 2, 2, 4, 6 | 313,078 | 0.055 | PASS |
| final-15805-0106 | master | 4, 5, 5, 6, 6 (1964-12 … 1968-11) | 2,013,268 | 0.030 | **FAIL**: value under 0.05 |
| final-15805-0106 | `eb0d839ce0` | 3, 5, 5, 5, 6 | 2,164,046 | 0.033 | **FAIL**: value under 0.05 |
| 21647 autosave | master | 11, 11, 11, 11, 11 (1990-12 … 1994-12) | 37,431,856 | 0.143 | PASS |
| 21647 autosave | `eb0d839ce0` | 11, 11, 11, 11, 11 | 37,826,807 | 0.144 | PASS |

Each replay took 220-480 s (deploy-225, final-15805) or about 800 s (21647). The leader ended at
6.34M / 5.73M (deploy-225), 67.7M / 66.0M (final-15805) and 261M / 263M (21647).

### Autopilot state after the load (heartbeat lines)

| Save | master | `eb0d839ce0` |
|---|---|---|
| deploy-225 | 340 of 340 heartbeats: `own_lines=10 max_routes=30`, `own_cap=10` | 311 of 311: `own_lines=4 max_routes=30`, `own_cap=4` |
| final-15805-0106 | 360 of 360: `own_lines=10`, `rich=1`, `own_cap=32` | 362 of 362: `own_lines=4`, `rich=1`, `own_cap=32` |
| 21647 autosave | 349 of 349: `own_lines=10 cash_reserve=5000`, `rich=1`, `own_cap=32` | 362 of 362: `own_lines=8 cash_reserve=25000`, `rich=1`, `own_cap=32` |

Every heartbeat of both codes reads `autopilot=growth-on-plan`, never `engaged`. All three saves
are planned bots. Before #279 the floors applied only to an unplanned bot (`b.autopiloted`), so
the pre-fix code keeps the save's `own_lines` of 4 (or 8). #279 applies them to every growing bot,
and that holds after a load. `max_routes=30` is the saves' own setting under both codes.

### What bound growth (master only)

The pre-fix code logs no FUNNEL lines (#272), no claim releases (#277) and no capital reports
(#276), so these rows exist for master alone. The FUNNEL rows cover the whole years after the
load. A year's line is logged in January of the next one, so the last replayed year has none.

- **deploy-225 (1952-10 → 1956-10): money.**
  - Rail heartbeats short of money for a line: 18 of 18 (1952), 74 of 76 (1953), 23 of 81 (1954)
    and 0 of 90 (1955). In 1953 two finished lines got no locomotive (`loco_refused_cash=2`), and
    air was short of budget on every heartbeat of every year.
  - Producer claims: 21 claimed and 5 released in 1954, then 5 claimed and 22 released in 1955.
    27 `no claim on` lines. Once released, the producers fall to the next filter: in 1955, 24 of
    41 were served by others, and 0 pairs were tried.
  - Capital (`capital.nut`): 16 quarterly reports. It bought 3 road vehicles for 14,763 in total
    (1952Q4 and 1953Q1), held money twice for a train of line 1, and was never idle. Money stayed
    at 7-29k with the loan at 300k until 1954; construction was frozen on 44, 8, 39 and 77 days
    of 1952Q4, 1953Q1, 1954Q1 and 1954Q2.
- **final-15805-0106 (1964-11 → 1968-11): discovery.**
  - Every year: 43-44 producers, 0 claimed, 29-30 released, 32-34 "transported by others (rich:
    past RICH_SERVED_BELOW)", 0-1 pairs tried, 0 lines built. Air: `no_pair` on every heartbeat
    (89-95 a year).
  - Capital: idle in 16 of 16 quarters. It bought one train (1967Q2, 28,429 for line 2) while its
    money rose from 736,883 to 1,193,395 with no loan. Twelve times it logged "money idle a
    quarter and the air fleet proven; trying air route 2", and no route was built.
- **21647 autosave (1990-12 → 1994-12): station and path building.**
  - Every year: 49-51 producers, 0-3 claimed, 31-32 released, 30-34 served by others. The search
    found 6 candidates a year and tried 6-7 of them, but built 0 lines. In 1991, 6 of 6 tries got
    no station; in 1992-93, 2 of 2 laid paths failed (`no rail path ... every reachable tile
    tried`). Air: `no_pair` on 90-92 heartbeats a year.
  - Capital: idle in 16 of 16 quarters, 0 bought. Money rose from 32,282,964 to 36,384,755.
- No replay logged a script error for company 0.

## Variance

<!-- open-finding kind=failed -->
One replay of one code does not separate the two codes on these saves. The same pre-fix code
`eb0d839ce0` on `deploy-225.sav` was worth 151,321 (0.029) on lundra on 2026-11-24
([AI battle README](ai-battle/README.md#the-live-like-arena-and-the-15805-replay)). Here it was
worth 313,078 (0.055). The two runs differ in binary build and host, so the verdict flips across
the 0.05 line on noise alone. The `replay` rules judge one run against a fixed threshold, and a
code comparison on a live save needs several runs per code.

## Deploying master into the live game

Measured: master keeps the floors after a load, and it runs without script errors on all three
loaded saves. Not measured: any gain. On the 21647 save the two codes end 1% apart after four
years (37.43M against 37.83M), with the same 11 trains and no new line. The live game was saved as
`final-21647-1992.sav` at 03:39 and restarted from it at about 04:11, while these replays ran.
`~/Documents/OpenTTD/ai/PrismaticAI` is a symlink into the primary checkout. Its reflog moved
from `53cd7b467d` (the pre-fix PrismaticAI tree) to `8f35e35995` at 03:39:13 and to
`4f40a1233f` at 04:05:57. Both carry the PrismaticAI tree replayed here, so the live game now
runs the "master" arm, and the autosave of 03:23 is the pre-fix arm as it ran live. These replays
predict no measurable change in its course from that deploy.

What would change it is in the funnel: producers that others already serve, stations and paths
that fail for the few candidates left, air pairs that are never found, and the capital
allocator, which holds tens of millions without a purchase.

### #285 on the live game's own save

#285 (merged at 05:28, after the restart) makes the capital allocator's purchases inside rail
searches run from the first heartbeat after a load. The live game runs `8f35e35995`, which does
not include it. In the live game the panel's capital watch flags hoarding: 30.6 years of income in
the bank, reinvestment at most 10% for four quarters, `bought=0` and 42.8M idle. The hypothesis
was that a load stops the purchases and #285 restarts them. To test it, `final-21647-1992.sav`
(loaded at 1992-03-15) was replayed for two game years on lundra. One run used the PrismaticAI
of origin/master `f50322d30a` (with #285), the other `8f35e35995`. Lundra's binary was
`20261122-master-g2a6a616221`, which has the same `src/saveload` and script API as master, with
the same `-d script=4` wrapper and the live rival tars.

| PrismaticAI | Capital reports 1992Q1-1993Q4 | Bought / spent | Money 1992Q1 → 1993Q4 | Trains | Value 1994-03 | Ratio |
|---|---|---|---|---|---|---|
| `f50322d30a` (with #285) | 8, `idle=1` in all 8 | 0 / 0 | 33,666,399 → 35,760,553 | 11 | 36,843,280 | 0.146 |
| `8f35e35995` (live) | 8, `idle=1` in all 8 | 0 / 0 | 33,666,399 → 35,759,906 | 11 | 36,902,919 | 0.146 |

#285 changes nothing on this save. Neither code buys anything in two years, and the two runs end
0.2% apart. The allocator's rail candidate (`capital.nut`) has to be a line that earns and runs
under its `trains_max`. Every line that earns already has `trains_max=2` trains. At the last
steward state of the replay with #285, lines 1, 2 and 8 run 2 of 2 trains with 545, 522 and 479
units waiting and source ratings of 64%, 14% and 14%. Lines 3 and 4 run 2 of 2 with nothing
waiting, and line 11 runs 1 of 2 and has no cargo. Each of the 8 quarters logs "trying air
route 4", and no route is built. The hoard comes from the per-line train cap and from the
missing new lines and routes, not from the load.

<!-- open-finding kind=failed -->
On both rich saves, master's capital allocator (#276) was idle in all 16 quarters. On
final-15805 it bought one train while money rose from 0.74M to 1.19M; on 21647 it bought nothing
while money rose from 32.3M to 36.4M. With #285, `final-21647-1992.sav` is the same: 0 purchases in
8 quarters, because every earning line is at `trains_max=2` while 479-545 units wait at three of
them. Its fallback "trying air route" never built a route.

## Not run

- **12609** (`autoapprove-incident-0429.sav`): the file is truncated. It is 245,760 bytes
  (60 × 4 KiB), and its lzma stream stops without an end-of-stream marker after 2,444,970
  unpacked bytes, so `replay_copy` raises `LZMAError` and no game can load it. No other 12609
  save exists.
- **10487**: no save of that game exists in `~/Documents/OpenTTD/save/`, in the live panel's
  autosaves or under `$TMPDIR`.
- **A second run of each code on `deploy-225.sav`.** It was queued at 04:41 under the same
  limits and never started: the Mac's 1-minute load stayed between 40 and 130 until the queue
  was stopped at 06:25. `final-21647-1992.sav` was run later on lundra, for two years only
  (see above). It was not run with the pre-fix `eb0d839ce0`.
- Six-year horizons were not done. Every run is one per code and save.
