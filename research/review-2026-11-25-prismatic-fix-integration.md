# Review 2026-11-25: how the merged PrismaticAI fixes interact

RESEARCH-ONLY. Six PrismaticAI pull requests were merged within about a day by different agents.
Each was tested mostly on its own: #271 (handed `fr_` slots counted only when whole and stable),
#272 (growth funnel counters), #276 (capital allocator, also inside rail searches), #277 (claims by
fresh stations, sibling rule, first-seen table, funnel year roll inside `FindRailPath`), #279
(autopilot floors under any plan, `PairDue`/`TryRailPair`) and #280 (the steward's monthly round
inside `FindRailPath`, `JudgeAge`, the close rule). This review reads merged master `8f35e35995`
(the PrismaticAI tree is the same as at `42e3d37ae7`) for defects that come from two of them
together. Each finding is listed below as confirmed, suspected or cleared.

**Method.** The code was read. Every suspected defect was given a concrete scenario. Codex
(`codex exec --sandbox read-only`, asked about the behaviour, with no source in the prompt) gave a
second opinion. It confirmed items 1 to 6 below and found no state leak. Item 1 is also confirmed
in match logs that the #277 branch left in session scratch: fitness lineups on seed 31337 at
`-d script=4`, run from code whose `FindRailPath` matches master. Line numbers are master's.

## Confirmed

### 1. A January round's trains are counted in the previous FUNNEL year (fixed here)

`FindRailPath` called `MaintenanceYield()` and then `FunnelRoll()` every `MAINT_YIELD_NODES` nodes
(`rail.nut:183-186`). The round (#280, `steward.nut:153-163`) buys trains: `ReconcileLines` with
`inside`, `KeepLinesServed`, and `AllocateCapitalInside` (#276). The roll (#277) that runs after it
reports the old year with the trains standing at that moment (`funnel.nut:88-100`, `125-159`).
Whenever a search spans 1 January and that month's first round buys a train, the train lands in
the old year's `trains=` and `trains_added=`, and the new year's `trains_added` misses it.
Rounds fall on 1 or 2 January in most years of a match. Two examples:

- Seed 31337, lineup 1, company 0: `steward round 25 on 1962-1-2 (survey)`, then `rail line 2:
  added train 315 (2 of 2) during a rail path search`, then `FUNNEL year=1961 mode=rail ...
  trains=5 trains_added=2`.
- Lineup 6, company 1: `steward round 31 on 1963-1-1 (survey)`, `added train 494 (3 of 3)`, then
  `FUNNEL year=1962 ... trains=12 trains_added=7`.
- Master `8f35e35995` itself (this review's run of lineup 1): on 1964-01-02 train 795 and a plane
  that the allocator bought inside the search, then `FUNNEL year=1963 ... trains=16
  trains_added=4`. The same thing happened on 1966-01-02 (train 1271, then `FUNNEL year=1965`).
  The allocator's road, air and water purchases land in the old year's `vehicles=` the same way.

`trains_added` is the number the analyses of why PrismaticAI loses read for the steward's growth
(`analysis-2026-11-23-why-prismatic-loses.md`). **Fix:** roll the year first, then run the round.
`FunnelRoll` only reads and logs, so the bot decides nothing differently. The heartbeat can still
put a vehicle bought before its first funnel event into the old year. Examples are
`KeepLinesServed`'s first train and `ManageFleet`'s vehicles, which come before `FunnelTick`
(`main.nut:802-827`). That is rarer and not changed here.

### 2. Purchases inside a round skip the capital quarter's bookkeeping

`AllocateCapitalInside` (`capital.nut:273-280`) calls `CapitalBuy`. That increments
`CAPITAL.bought`/`spent` and can add `CAPITAL.skip` entries (`capital.nut:230`). But
`CapitalQuarter`, which reports a quarter and clears `skip`, runs only in the heartbeat's
`AllocateCapital` (`capital.nut:291`). The heartbeats of a company that lays its own lines come
183 to 366 days apart. So purchases inside a round are reported under the last heartbeat's quarter,
not under the quarters they fell in. A route "skipped this quarter" also stays skipped until the
next heartbeat. The `frozen=`/`days=` fields that fitness reads are not affected. Impact: the
report and the skip lifetime only. No fix here.

### 3. No capital purchase in any rail search of the first heartbeat after a start or a load

`CAPITAL.b` is set only at `capital.nut:297`, near the end of the heartbeat.
`AllocateCapitalInside` returns while it is null (`capital.nut:274-275`). In the first heartbeat
after the AI starts, and after every savegame load (a deploy into a running game is a save and a
load), the searches buy nothing through the allocator. The steward's part of the round still runs.
At game start no money is idle, so this costs nothing. After a load of a rich company it costs up
to one heartbeat of idle money. The `CAPITAL` comment (`capital.nut:57-58`) does not mention this.
A fix would set `CAPITAL.b` at the top of each heartbeat. No fix here.

## Confirmed in code, impact not measured

### 4. A steward shrink can be undone by the allocator in the same heartbeat

The heartbeat runs `SelfSteward` (`main.nut:832`), then `ReconcileLines` (`main.nut:837`), which
sells the newest train of a line above its target (`lines.nut:282-286`), then `AllocateCapital`
(`main.nut:853`). The steward shrinks a line when its newest train lost money last year and less
than a quarter of all its trains' capacity waits (`steward.nut:357`). The steward's own rating rule
is skipped for that line, because `LineEarns` fails on the losing newest train
(`steward.nut:350-351`). `CapitalBest` judges the line by the average profit of its judged trains
instead (`capital.nut:113-139`). It wants a train when half a trainload waits, or when the source
rating is below `steward_rating` (default 55). The other conditions are a free source platform and
a remaining newest train at least 90 days old. `CapitalBuy` then raises the in-bot target to the
remaining trains plus one (`capital.nut:237-238`). That undoes the shrink: the company sells an old
train and buys a new one at full price. The waiting-cargo window is narrow: at least half a
trainload of the remaining trains, but under a quarter of all trains' capacity. That needs three or
more trains. The rating branch has no such limit. Each cycle can repeat once the new train is
judged, after at least 400 days.

The capital branch's pre-merge logs in session scratch have no case of both in one heartbeat. A
fix would let `CapitalBest` respect a target the steward lowered this month, or apply the newest
train test of `LineEarns`.

### 5. The double track's money test comes before a search in which a round can spend

`BuildDoubleRailLineAndTrain` sets `afford_b` from the funds against track A's cost plus the train
(`rail.nut:1506`). It then searches track B (`rail.nut:1514`). A round during that search may spend
everything above the loan, the cash reserve, one `line_budget` and `MaintenanceHold`
(`steward.nut:158-162`). The hold does not include track B's estimated cost. When track A cost more
than the loan, the round can leave track B or the train unaffordable after `afford_b` said yes.
Track A costs that much when it was paid partly from the company's own money, on a long double
track. Track B would then wait in `RailCashWait`, or the line would be kept trainless. Before #276 a
round spent at most `MAINT_YIELD_TRAINS` trains here. `CapitalBuy` loops until the money above the
hold runs out. No case was found in the logs read.

### 6. An externally targeted rail line ends the round's capital loop

Inside a round, `CapitalBuy` runs with `may_hold` false. When the best candidate is a train an
external steward's target asks for, `buys` is false, so the candidate is not bought. The loop then
breaks (`capital.nut:246`), and no road, air or water candidate is bought in that round. This only
matters with the panel's steward setting targets. No fix here.

## Suspected, not confirmed

- **Opcode cost of a round.** `CapitalBest` walks every vehicle again after each purchase
  (`capital.nut:87-108`), and `MaintenanceHold` runs `PickRailEngines` once per line. This is
  suspendable work once a game month, so it slows the search but never fails it.
- **`KeepLinesServed` serves a line the in-bot steward closed.** It tests the external target only
  (`rail.nut:1717`, `LineTarget`, not `EffectiveTarget`). A line whose self target is 0 and whose
  last train is lost gets a replacement before the heartbeat's `ReconcileLines` closes it. This
  was so before these pull requests.

## Cleared

- **Exec mode, accounting and rail type around a round.** `MaintenanceRound` holds `AIExecMode`
  and `AIAccounting` as function locals (`steward.nut:154-155`), and `MaintenanceYield` restores
  the rail type (`steward.nut:145-147`). The probes' `AITestMode` blocks inside `FindRailPath`
  (`rail.nut:239-241`, `266-267`) end at their closing brace, because the vendored Squirrel emits
  `_OP_SCOPE_END` (`src/3rdparty/squirrel/squirrel/sqcompiler.cpp:265`). No mode object outlives
  its frame, so the mode-stack fatal error cannot fire. Track A's `acc_a` is read before track B's
  search, and no round runs while track is laid (`MAINT_LAYING`, `rail.nut:1081-1085`).
- **Re-entrancy.** Nothing a round calls (`SelfSteward`, `ReconcileLines`, `KeepLinesServed`,
  `AllocateCapitalInside`) builds a line or searches a path. `MAINT_TICKING` guards the rest, and
  no round pushes to `RAIL_LINES`, so `RAIL_LINES.top()` after a build is still the new line.
- **Double spending.** Every purchase reads the live bank balance. Inside a round the allocator's
  `keep` is at least the round's hold (`capital.nut:278`). The heartbeat's `held` money is only kept
  from repayment, never counted as spent.
- **Floors (#279), allocator (#276) and `KeepLinesServed` (#280).** All of them read the same
  `b.cash_reserve`, capped at 5000 by `b.grows` (`main.nut:409-416`). `MAINT_RESERVE` is set from it
  in `MaintenanceAttach` and the heartbeat's `ReconcileLines` (`lines.nut:211`). `line_budget` is
  the raw setting everywhere.
- **Close rule (#280) against the allocator.** Close needs the judged trains to have lost money
  last year, and from 1 April on (`STEWARD_SETTLE_DAYS`) `JudgeAge` equals `CapitalFullYear`. Both
  then judge the same trains, so the allocator never buys for a line the steward closes.
- **Claims after a load (#277).** `CLAIM_SEEN` is saved and restored (`restore.nut:82`, `186`), with
  integer tile keys, which the script save accepts (`script_instance.cpp` `SaveObject`, `OT_TABLE`).
  `ClaimsAfterRestart` takes unknown foreign tiles as old. Rounds never call
  `ForeignRailStationAt`. A long search only delays a first sight, which lengthens a claim and
  never shortens it.
- **Squirrel definitions.** The PrismaticAI tree defines no function twice and no global or
  constant twice. No constant is used in a file compiled before the file that defines it (checked
  in `require` order from `main.nut:113-132`).
- **Known, not re-reported.** The pair's wait (`pair_due_since`) is not saved. `restore.nut:321`
  marks the pair done whenever any rail line exists after a restore from the map. `CAPITAL` holds
  are not saved.

## Validation

Lineups 1 and 6 of `battle.json` `fitness` were run on seed 31337 for master `8f35e35995` and for
the fix. Each run was 6 years against the arena field at `-d script=4`, one process at a time,
and each started only while the load average was under 24.

| Run | January rounds that bought before their year's FUNNEL line | Bought after it | Fitness verdict |
|---|---|---|---|
| master, lineup 1 | 3 (1963, 1964, 1965) | 0 | PASS |
| master, lineup 6 | 2 (1961; 1962, company 1) | 0 | FAIL `trainless_stations:1` (claim race A) |
| fix, lineup 1 | 0 | 2 | FAIL `trainless_stations:1` (self-steward) |
| fix, lineup 6 | 0 | 3 | PASS |

The counts come from the match logs: a January survey round followed by a purchase, set against
where the old year's `FUNNEL ... mode=rail` line falls. With the fix, every such line comes before
the round's purchases. The only fitness failure is `trainless_stations:1`, a rail station without
a train from 1965-04 to the end of the match. Master shows the same failure in lineup 6, so by the
owner's fitness rule the fix adds no new kind of failure. The two failures fall on different
lineups because the matches diverge: the same seed ends at different values on the two trees.
The fix does not change what the bot decides, but it changes when the script uses its opcodes.
**PARTIAL:** `tools/gate.sh change` was not run, because it means a full C++ build in this
worktree and the change touches only Squirrel. `tools/surfaces/surfaces check`,
`tools/knowledge/refs check` and `python3 -B -m unittest test_prismatic_ai_docs` passed.
