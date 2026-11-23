# Control loops audit: why nothing reacted while we fell behind

RESEARCH-ONLY. A read-only audit of the owner's live game of 2026-11-22/23 (panel :8800, server
:3979, MCP :8731), made on 2026-11-23. Company 0 (company 1 in the game) runs PrismaticAI; company 1
runs AAAHogEx, the leader. Every number below was read from the panel's DuckDB store
(`POST /api/db/query`, restricted to the newest run, `runs.started_ms = 1795400722667`, game
1950-01 to 1953-12) or from a read-only GET of the panel, unless the row says otherwise. The reads
are listed at the end. The fix for the main missing link is
[`gap_watch.rhai`](../tools/research/mcp-panel/rhai/gap_watch.rhai)
([The leader-gap watch](mechanisms/rhai-scripting.md#the-leader-gap-watch)).

## The answer in one paragraph

Every loop in the fork measures something, and none of them is wired to react to the leader. replan
records each month where the leader wins (aircraft: 29,871 a plane against our 16,900), and nothing
reads that record. Its own output is 719 single-setting approvals, of which 25 were executed (3.5 %).
The other 694 were retired at the 30-day deadline with the verdict `keep`, so its plans almost never
take effect. The few rows that did land came one at a time and did damage. `air_routes_max=2` (the
default is 15) and `air_grow_money=0` (0 disables "idle money buys more air routes") capped our
aircraft in the mode where the leader won. Freight triples were applied in part (`fr10_to` alone,
later `fr10_cargo` and `fr10_from` without `fr10_to`). The bot's own steward, meanwhile, repaid
1,190,000 of loan until the loan reached 0, with `invest_borrow=0`, while the leader kept 290,000
borrowed and grew from 35 to 67 vehicles. Ours stayed between 28 and 34. watchdog flags only a
company that built or bought nothing. Its threshold tunes itself up to 365 days, so slow growth is
never a stall, and it does not escalate. Battlebrain is not running, and the fitness rules (hoarding,
fleet drop) run only in headless battles. So no loop compared us with the leader, noticed idle
capital, or saw its own collateral damage. The trajectory stayed the bot's defaults plus a few
restrictive settings approved row by row.

## The trajectory (store `samples`, last sample of the quarter's first month)

| Month | Our value | Leader's value | Share | Our vehicles | Leader's vehicles | Our money | Our loan | Leader's loan |
|---|---|---|---|---|---|---|---|---|
| 1951-07 | 208,718 | 611,875 | 34 % | 12 | 21 | 64,221 | 300,000 | 300,000 |
| 1952-01 | 391,925 | 740,510 | 53 % | 23 | 28 | 13,211 | 280,000 | 300,000 |
| 1952-07 | 595,081 | 921,415 | 65 % | 28 | 35 | 11,714 | 190,000 | 300,000 |
| 1953-01 | 647,274 | 1,282,936 | 50 % | 28 | 47 | 16,349 | 130,000 | 260,000 |
| 1953-07 | 873,802 | 1,593,779 | 55 % | 31 | 57 | 30,840 | 0 | 260,000 |
| 1953-10 | 1,002,578 | 1,809,965 | 55 % | 33 | 66 | 99,090 | 0 | 280,000 |
| 1953-12 (`GET /api/overview`) | 1,090,839 | 1,821,760 | 60 % | 34 | 67 | 155,522 | 0 | 290,000 |

The brief quoted "372k cash idle with a 300k loan" in 1953-06. The store's samples for this run
do not show that state. They show cash of 30-155k with the loan already repaid to 0. The idle
capital is real either way: 155k of cash plus 300k of unused credit, against a leader that borrows
to the limit.

## The loops

| Loop | Inputs | Decision | Action | Measured effect in this game | Missing link |
|---|---|---|---|---|---|
| **replan.rhai** (service, running) | the company's 4 quarters (ScriptCompany), its rank and 6-month trend (store), every company's last quarter, per-type profit a vehicle (`openttd://vehicles`), the planner's portfolio (`/api/bots/planned`, AlgoGlyph) | adopt when bayes-update's lower bound ≥ 0.5 and pert-monte-carlo's p80 payback ≤ 24 months; hold while any of its approvals is pending | one `openttd.ai.set_setting` per differing setting; `replan.<c>` on the blackboard; a mesh `plan` record | 23 rounds read, `adopt=true` in 14. 719 rows filed, 25 executed (3.5 %), all approved by the owner in chat; the rest retired by the deadline (`keep`). The mesh write is refused every round (`out_of_scope: ... info.nut ... claim them first`). | It never reads its own `competition` block; the planner chooses without it. It files about 40 rows a round, which the owner cannot answer one by one, so whatever lands is a random subset. It cannot see whether a round landed. Nothing rereads its record. |
| **The approval path** (game server, #222 #231 #251) | pending approvals | the owner approves a row, or at 30 game days the standing verdict settles it | execute, or retire | 25 executed, 694 retired. The executed rows include `air_routes_max=2` (#222), `air_grow_money=0` (#60), `fr10_to=5` alone (#227), and `fr10_cargo`/`fr10_from` without `fr10_to` (#460, #461, #507, #594-596) | Row-by-row approval of a multi-setting plan applies it in part. `openttd.ai.set_settings` (one atomic approval) exists and is used by nobody live. The deadline settles; it does not inform whoever filed. |
| **watchdog.rhai** (service, running) | fleet by type and stations (`openttd://companies`, `openttd://stations`), the tick | stalled when no change for longer than the threshold (1.5 × the 90th percentile of quiet times, 7..365 days) | `watchdog.<c>`; one chat line per stall for company 0 | company 0 stalled 14 rounds (1950-02 .. 1953-04, max 155 quiet days), with reasons "N approval(s) wait for the owner" and "money below the line budget" | It measures only "built nothing". A company that buys a vehicle every 100 days is never stalled, and there is no escalation, no leader comparison and no money test. The supervisor killed and restarted it 14 times: it prints nothing between rounds, and its stall limit is 180 s (`== stalled: printed nothing for 180 s`). |
| **rail_monitor.rhai** (service, running) | the bot's rail lines, the game's advice news | per-line state, advice tallies | `advice.<c>` (6 writes), chat | observation only | It is about rail lines; no money or competition input. |
| **build_audit.rhai** (service, running) | stations, depots, ways, map | defects by rule | `audit.<c>` monthly | 42 audits of company 0 | It audits buildings, not the money they cost or whether they were removed. |
| **Battlebrain** (not running) | snapshot, planner candidates, AlgoGlyph | one freight line or HOLD above an ROI margin | a decision record → executor → `set_settings` at the console | none: no `planner.<c>` key was written in this run, no `decision` | Freight lines only, by design (ADR 0037); it does not cover aircraft, borrowing or idle cash. |
| **PrismaticAI in-bot** (steward, fleet, autopilot, RepaySurplus) | its own bank, loan, routes | `RepaySurplus(max(2·cash_reserve, afloat))` every heartbeat; `InvestSurplus` with bank money only (`invest_borrow=0`); `AutopilotAir` up to `air_routes_max` (+1 per `air_grow_money` of idle money, 0 disables) | repay, add vehicles, build routes | "repaid N of the loan" 42 times, 1,190,000 in total; loan 300k → 0 by 1953-04; "no freight route built; loan back to N" 6×; "construction frozen" 6× | It optimises its own balance sheet, not its share. Its comment on `AutopilotAir` names this exact failure ("held 15M idle by 1980 ... while AAAHogEx grew to 434 vehicles"), and the setting that fixes it was set to 0 by an approved replan row. |
| **Fitness rules** (`battle.py` `hoarding`, fleet drop) | quarterly series of a headless battle | hoarding: fleet flat over 8 quarters while money − loan rises 2× to ≥ 500k | fail a contestant offline | none live | They are headless only. Even live they would not fire here, because net money stayed under the 500k floor: the money went into repaying the loan. |

## Collateral damage, and who noticed it

"Collateral damage" here is the cost of the loops' own actions and of the bot's failed attempts.
Nothing in the loops counted it before `gap_watch`:

| Damage | Evidence (company 0, this run) | Noticed by |
|---|---|---|
| Levelling paid, then the airport was still refused | 7 lines "levelled tile N for X ... still refused: ERR_FLAT_LAND_REQUIRED", 24,428 in total; 10 more sites refused before levelling | nobody |
| Rail stations built, then removed for want of a path | 4 × "no rail path between the stations; removing them" | nobody |
| Producers held by another company's rail station | "48 producers on land, 9 ours, 19 (later 21) claimed by another company's rail station", 3 times | the bot itself (its own-line search skips them) |
| Plans applied in part | `fr10_*` triples applied one or two at a time; batches of ~40 with 1-4 rows executed | nobody |
| A restrictive setting kept for two years | `air_routes_max=2` since 1951-11-23, `air_grow_money=0` since 1951-08-02 | nobody: replan proposes only settings that differ from its plan, and its plan says 2 |
| Survival borrowing that freezes construction | 6 × "construction frozen: the company borrowed to stay afloat" | watchdog, only as "money below the line budget" when stalled |

## The questions of the brief

- **Does any loop compare us with the leader and react to a growing gap?** replan compares, per
  transport type, and records it. Nothing reacts. `gap_watch` is the first loop that reacts to
  the share trend.
- **Does anything detect idle cash or a falling share live?** No. Neither the loan repayment nor
  the leader's borrowing is read by any loop. The headless hoarding rule would not fire either.
- **Do replan proposals take effect?** 25 of 719 (3.5 %), each one only when the owner typed its
  approval. With the default verdict `keep`, an unanswered plan changes nothing; a half-answered
  plan changes part of itself.
- **Does watchdog escalate?** No. It says a stall once and repeats nothing until the company
  makes progress, and it never flags slow growth.
- **What is collateral damage, and does any loop notice it?** See the table above. None did.

## What gap_watch adds, and what is still open

`gap_watch.rhai` reads `replan.<c>` and `watchdog.<c>` instead of repeating their analysis. It adds
what they lack: our share of the leader's value over six months, cash above the reserve, credit
unused while the leader borrows, the bot's failed, refused and removed builds, levelling paid for
nothing, and whether filed plans land. It classifies each game month, tells the owner when a flag
has held two months, and records everything in `gap.<c>` for replan and Battlebrain to read. Only
with the owner's switch (`GAP_WATCH_PROPOSE=1`) does it file one owner-only
`openttd.ai.set_settings`. That proposal restores declared defaults (`invest_borrow`,
`air_grow_money`, `air_routes_max`, and two road settings) when the evidence for that transport type
holds. Codex reviewed the design before implementation; its changes are listed in
[the reasoning record](#review). Open:

- **replan proposals** (owner of replan, not edited here): file one `set_settings` per round
  instead of ~40 rows; leave out of its diff the settings `gap.<c>.levers` names; carry the
  leader's vehicles and loan in `competition`.
- **watchdog** (not edited here): print one line per round so the supervisor stops restarting it;
  carry money, loan and `max_loan` in `watchdog.<c>`, so `gap_watch` can drop its observed loan
  ceiling.
- **The deadline policy** is being redesigned (a visible countdown for the owner's and co-play
  companies). `gap_watch` reads the policy from `openttd://approvals` when it files, and names it
  in the owner's line; it never assumes `keep`.
- **Effectiveness** is not measured: no paired-seed run compares a bot with `gap_watch` proposals
  against one without. The share-drop threshold comes from the shared key
  `derived.gap_watch.share_drop` when the self-evaluation service publishes it (the 75th
  percentile of 6-month drops over the store's runs, 0.0118 on the :8800 store), else `SHARE_DROP`
  0.05. The failure-regime detector's rules (`regime.rules`) judge 24-month trends, a different
  statistic, so they are not used for this flag. `IDLE_FACTOR` 2 and `PERSIST` 3 remain
  hypotheses, not calibrated values.
- **Host PAUSED/STOP files** (rule `project.long-runs-are-owned-bounded-and-stoppable`): Rhai in
  the panel has no file access, so the stop and pause switches are shared keys any session can set.
  The service adds no compute load to a host.

## Review

The design was critiqued before implementation (owner order): Codex (`codex exec --sandbox
read-only`) answered; Gemini refused (`IneligibleTierError: UNSUPPORTED_CLIENT`), and no local
model was available for the ollama fallback. Codex's verdict: "useful observer; keep proposals
disabled until ownership and outcome tracking are resolved". Adopted: proposals off by default, the
`unknown` flag, cash separated from credit, `air_grow_money` only from 0, air levers blocked while
levelling fails, a hold on a pending `set_setting` naming a lever, and the outcome of each proposal
tracked, with no repeat after two unapplied months. Not adopted: host files (see above) and a
paired-seed evaluation (open).

## Reads behind this audit

```sh
P=http://127.0.0.1:8800
curl -s $P/api/services; curl -s "$P/api/services/log?name=replan&since=0"; curl -s "$P/api/services/log?name=watchdog&since=0"
curl -s $P/api/status; curl -s $P/api/overview; curl -s $P/api/bots
# store, newest run only:
#   WITH r AS (SELECT max(started_ms) s FROM runs), e AS (SELECT * FROM events, r WHERE ts_ms >= r.s)
#   kinds: rhai.shared (replan.0, watchdog.*, audit.*, advice.0), panel.mcp, decision.recorded, ai.log (company 0), samples
```
