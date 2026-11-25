# Analysis 2026-11-23: why PrismaticAI loses to AAAHogEx, and why nothing reacted

Post-mortem and synthesis for the owner's live 8-company games on `:8800` (seeds 2927, 15805 with
its continuation, 12609 and 10487) and the headless evaluations of 2026-11-22/23. It answers five
questions. Why does PrismaticAI lose to AAAHogEx? What do the two earn and lose? Why did no
planner, service, learner or gate react while the gap grew? Why was this strategy chosen? Why did
learning not catch it? It uses only findings that a second, independent pass confirmed. The claims
that pass refuted are listed under [Corrected claims](#corrected-claims) so that nobody reuses
them, and the [gaps ledger](#gaps-ledger) lists what was not verified.

**Provenance.** The analysis ran in three passes in the openttd-36 session (2026-11-23, 04:37 to
05:30 CET, continued 2026-11-24):

- Pass 1 had six independent lenses: economics, strategy, reaction, learning, rival and coverage.
- In pass 2, four agents tried to refute every pass-1 finding. They recomputed the numbers,
  re-read the locators and looked for counter-evidence. They also reconciled the three-pass analysis
  openttd-7d had run on its own.
- Codex gave one outside review.
- Pass 3, this document, re-read the panel store of game 10487 up to its current date, 2131. It
  also absorbs the interrupted drafts of agents H (`doc/live-game-12609-economics`) and K
  (`doc/strategy-postmortem`) and takes their place.

The passes' working files are session scratch and do not last, so every finding below carries its
own locator.

**Tags.** **O** = OBSERVED (seen in the data, with a locator). **D** = DERIVED (computed from
OBSERVED data). **I** = INFERRED (a hypothesis, with what would confirm it).

**Sources.**

- **store**: the panel's DuckDB store `panel-8800` (`samples`, `events`, `runs`). It was read from
  `-readonly` copies or through the live panel's read-only `GET /api/db/query`. Runs are delimited
  by `runs.started_ms`:

  | Run | `started_ms` |
  |---|---|
  | 2927 | 1795374123856 |
  | 15805 | 1795381184362 |
  | "31337" (15805's continuation) | 1795390211482 |
  | 12609 | 1795400722667 |
  | 10487 | 1795405920135 |

- **vehicles**: the `openttd://vehicles` snapshot of 12609 at 1954-01-04, whose
  `profit_last_year` is 1953.
- **headless**: the BattleReferee `REF Q` lines of the 2026-11-23 evaluations. Each ran 4 seeds
  (15805, 2927, 31337, 4242) from 1950 to 1960-01 with the live settings, against the live lineup
  of 7 BaNaNaS AIs. Three builds were run: master before the fixes, "now" and #245.
- **arena**: the 351 battle records `research/results/battle-*.json`. Every one starts in 1960, with
  8×8 lineups and 20 years.
- **code**: master `eb0d839ce0`. `P/` is short for `tools/research/prismatic-ai/PrismaticAI/`, and
  `rhai/` for `tools/research/mcp-panel/rhai/`.

## 1. The answer in five sentences

1. PrismaticAI loses mainly on how many lines it builds, less on how well a line runs. In live
   game 12609 a mature train of ours earned 31.4k in 1953, against 30.3k for one of AAAHogEx's. In
   the headless runs AAAHogEx also earns 1.51× a train and 2.9× a plane. On every candidate
   diagnostic from 1952-10-17 to 1955-01-02 our search found nothing: 19-21 producers were skipped
   as "claimed", and 0 pairs were tried. The money that piled up meanwhile (721k by 1955-03) had no path to buy
   trains, while AAAHogEx reinvested 98-100% of what it earned.
2. Our vehicles earned 535,835 in 1953 against AAAHogEx's 1,292,848: trains 417,910 vs 862,927,
   aircraft 112,148 vs 427,523. By 1954Q3 the income ratio was 2.76×. In game 15805 we earned
   93-193k a year, while AAAHogEx's income rose from 1.53M (1953) to 6.89M (1963). Our losses were
   small but telling. Lone air routes lost −1,088 and −2,370 a year, and buses earned 242 a vehicle.
   42 loan repayments, 1.19M in total, froze construction on about half of the days of 1950-52.
3. Nothing reacted, for four reasons:
   - The re-planner files one approval per setting. The 30-day deadline retired those approvals
     about 60 wall seconds after filing: 25 of 755 executed in 12609, and 0 of 372 in 15805's
     continuation.
   - The owner had no usable sign-in before #242.
   - The rows that did land assembled a freight slot piecemeal. That silently switched the bot to
     "planned", which drops its growth floors.
   - Battlebrain never ran, and no service senses the gap to the leader. replan acted in 7 of 219
     losing streaks.
4. The strategy was chosen on evidence that could not show the failure:
   - 80 duels of PrismaticAI against a copy of itself;
   - arena matches that start in 1960 with the bot unplanned, where the autopilot overrides exactly
     the settings that bind live under a plan (`own_lines=4`, `max_routes`);
   - a live genome, g347.3, picked from 6 matches at +6.3% over the default.
5. Learning did not catch it:
   - The GA fitness is dominated by rank. Halving the gap to the leader is worth +0.030, while
     dropping one rank in one match costs −0.042.
   - AAAHogEx won 15,065 of 15,065 arena matches, and that changed no decision.
   - The live tuner never started in any live game, so no live game ever reached a learner.

Game 10487 has since run on to 2131, and what it shows is consistent with the whole chain. A re-plan was adopted on
1952-04-07, and the 7-day `approve` deadline executed it. The bot has run `growth-on-plan
own_lines=4` ever since. In 2131 it holds 185.7M in cash and runs 40 trains, while AAAHogEx is
worth 4.87B with 1,085 vehicles (§5).

## 2. Numbers

### Live games

| Game | Figure | PrismaticAI (company 0) | AAAHogEx | Locator | Tag |
|---|---|---|---|---|---|
| 12609 | Vehicle profit 1953, train / air / road | 417,910 / 112,148 / 5,777 | 862,927 / 427,523 / 2,398 | vehicles, summed by owner and type | O |
| 12609 | Shares of the 757,013 gap | trains 58.8%, air 41.7% | | same | D |
| 12609 | Mature trains (≥ 369 days) × 1953 profit each | 12 × 31,361 | 24 × 30,333 | same | D |
| 12609 | Planes × 1953 profit each | 6 × 18,005 | 16 × 25,732 | same | D |
| 12609 | Lone air routes, profit | 375 / −1,088 (1953); −39 / −2,370 (1954) | | ai.log `air report` lines of 1954-01-01 and 1955-01-02 | O |
| 12609 | Reinvestment, 1950 / 51 / 52 / 53 / 54Q1-3 | 289.7 / 109.7 / 61.9 / 43.6 / 12.4% | 175.4 / 105.5 / 97.8 / 97.7 / 99.5% | capex = income + expenses − Δ(money − loan), quarters from `rhai.shared replan.0` `finances.quarters` | D |
| 12609 | Return on cumulative capex, 1951 / 52 / 53 / 54 | 91 / 59 / 49 / 42% | 110 / 82 / ≈63 / ≈50% | same | D |
| 12609 | Value, 1955-01-01 | 1,536,394 | 2,631,237 (1.71×) | `samples` | O |
| 12609 | Quarterly income, 1954Q3 | 165,914 | 458,709 (2.76×) | `replan.0` | O |
| 12609 | Money / loan / vehicles, 1955-03 | 721,075 / 0 / 33 | 47,311 / 300,000 / 121 | `company_month` | O |
| 12609 | Idle while rich | 95 heartbeats with `rich=1`, 1954-03-10 → 1955-03-27; money 271,732 → 721,346; 33-35 vehicles | | ai.log heartbeats | O |
| 12609 | Income, 1950Q1 / Q2 | 0 / 0 | 13,610 / 97,809 | `replan.0` | O |
| 12609 | Loan, 1950-53 | 42 repayments, 1,190,000 in total (1950-03-18 → 1953-07-13); construction frozen on about half of the days of 1950-52 (52% or 47.9%, by method) | 180-300k; cash ≤ 108k | ai.log `repaid`, `samples`, `P/main.nut:282-287` | O / D |
| 12609 | Interest 1950-53, at the default 2% | 17,668 | 23,621 | `samples` average loan × 0.02 (`initial_interest` is not set) | D |
| 12609 | `set_setting` approvals: filed / executed / retired unanswered | 755 / 25 (3.3%) / 694; the other 36 are 1 superseded (#713 by #719) and 35 cleared when the server stopped | | `console.line`: `pending approval #`, `approved #`, `retired unanswered` | O |
| 12609 | Filing → retirement, median | 59.9 s (59.9-61.0) | | the same rows, wall time | D |
| 15805 + 31337 | Year-end value ratio, AAAHogEx / ours | 1.15 (1950), 1.92, 4.93, 9.79, 17.93 (1954), 20.31 (1957), 30.18 (1960), 34.51 (1963), 35.74 (1964) | | `samples`, the last sample of each year | D |
| 15805 | Value, 1963-01-01 | 881,262 | 29,382,385 | `samples` | O |
| 15805 | Income a year | 93-193k (expenses 19-34% of it) | 1.53M (1953) → 4.96M (1958) → 6.89M (1963) | `replan.0` | D |
| 15805 | Vehicles / 1957 profit at 1958-06-16, train; road; air | 1 / 33,855; 15 / 5,562; 10 / 79,761 | 106 / 2,868,750; 375 / 339,913; 29 / 182,995 | vehicles, 1958-06-16 | O |
| 15805 | Days frozen | 815 of 1,387 | | [incident 15805](incident-2026-11-23-live-game-15805.md) | O |
| 31337 | Approvals filed / approved | 372 / 0 (354 retired, median 60.2 s; 18 still open at the stop) | | `console.line` | O |
| 31337 | Money / loan / vehicles, 1964-10 | 790,738 / 0 / 26 | 615 vehicles | `company_month` | O |
| 10487 | Year-end value, ours / AAAHogEx (ratio) | 1951 215k / 873k (4.06×); 1953 981k / 2.15M (2.19×); 1960 3.38M / 19.3M (5.7×); 1980 15.6M / 178M (11.4×); 2000 26.5M / 462M (17.4×); 2114 166.0M / 4.22B (25.4×) | | `samples`, the last sample of each year (pass 3) | O / D |
| 10487 | 2131-01: money, loan, vehicles | 185,657,267 / 0; 42 vehicles (40 trains, 2 road, 20 stations) | value 4,866,411,199; 1,085 vehicles (26.1×) | `samples`; `watchdog.0` signature of 2131-01-23 (pass 3) | O |
| 10487 | Mode | `autopilot=engaged own_lines=10` to 1952-04-11; `growth-on-plan own_lines=4` from 1952-04-15 to 2131 | | ai.log heartbeats (pass 3) | O |

### Headless evaluations (4 seeds, 1950 to 1960-01, live settings and lineup)

| Figure | PrismaticAI | AAAHogEx | Locator | Tag |
|---|---|---|---|---|
| Value over the 4 seeds: master / now / #245 | 13.74 / 19.26 / 18.32M | 82.0 / 75.1 / 76.2M | headless, the 1960-01 rows | D |
| Rail's share of the vehicle-profit gap | 76.0 / 91.0 / 88.0% | | same | D |
| Trains at 1960-01 | 57-68 | 313-375 | same | O |
| Profit per train, 1959 | 26.4-30.6k | 45.5-55.1k (1.51× on "now") | same; `prail` = `GetProfitLastYear` (`referee/BattleReferee/main.nut:9,113`) | D |
| Rail lines built | 37 | 112 `TrainRoute` builds, 34 of them `T:` transfer feeders | `rail line N` records; AAAHogEx's log | O |
| Trains per line | 1.54 | 3.24 | same, at 1960-01 | D |
| Log decomposition of the rail gap (ln 9.64 = 2.27) | | lines 49% (33% without the feeders), trains per line 33%, profit per train 18% | the three ratios above | D |
| Planes × 1959 profit each ("now") | 207 × 8,829 | 120 × 25,318 (2.9× a plane) | same | D |
| Expense ratio, 1959 ("now") | 17.9% | 6.4-6.5% | the four quarter-end lines | D |
| Producers skipped as claimed | 16-20 a seed (31-41% of the producers on land); 0-5 pairs tried | competes for served producers by default | eval-now diagnostics; `hog:info.nut` medium_value 0 | O |

### Arena

| Figure | Value | Locator | Tag |
|---|---|---|---|
| Matches AAAHogEx won | 15,065 of 15,065 | the 351 battle records, deduplicated on (edition, id, seed) | D |
| AAAHogEx / the best PrismaticAI in a match | median 7.56 (p10 4.65, p90 17.16, min 1.60) | same | D |
| Default PrismaticAI / AAAHogEx, by day (11-19 / 20 / 21 / 22) | 0.067 / 0.149 / 0.164 / 0.164, over the 2,213 matches that include the default | same | D |
| Live genome g347.3 | 6 matches; geomean 0.1375 of AAAHogEx; +6.3% over the default | `battle-20261122T045411-a80cc94a.json`, contestant 8 | D |
| Rank reliability of 6 matches | r₆ ≈ 0.29 (τ 0.212, r₁ 0.065; 0.8 needs 58 seeds) | [freight-margin pre-registration](battlebrain/experiment-freight-margin.md) | D |

## 3. Root causes, ranked by impact

Each cause is a chain of findings that pass 2 confirmed. The ranking is by the evidence of impact
in money, and it is a hypothesis order, not a measured one. RC-a's shares are accounting
decompositions of the gap, not effects measured by paired runs. RC-b and RC-c have no paired
estimate at all. The fixes come in a different order (§4): RC-b goes first because it confounds
every other experiment, not because it is proven to be the larger loss.

### RC-a. Growth stops: nothing to build after 1952, and idle money never buys trains

In the accounting decomposition it covers about 100% of the mature-train gap in 12609, and 49% +
33% of the log rail gap headless. These are descriptive shares; §7's experiment would measure the
effect.

1. **Count, not yield.**
   - In 12609 a mature train earns the same for both companies: 31.4k vs 30.3k (O).
   - Train count makes 59% of the 1953 vehicle-profit gap, and plane count 42% (O).
   - Headless, line count and trains per line make 82% of the log rail gap, and profit per train
     18% (D).
2. **The candidate set is empty from 1952-10.**
   - `NextOwnFreightLine` (`P/rail.nut:1681` ff.) takes only producers of ≥ 40 a month with an
     acceptor 100-260 tiles away, unless the bot is rich.
   - It skips a producer that another company's rail station "claims". The test runs before the
     rich served-below test, so it applies in every mode (`P/rail.nut:1694`).
   - Live 12609 logged `no own freight line` with 19 of 48 producers claimed (1952-10-17,
     1953-01-01) and 21 of 49 (1954-01-01, 1955-01-02), with "0 pairs tried" each time (O).
   - Headless, 16-20 producers a seed were skipped the same way (O).
3. **Rich mode cannot reopen it.**
   - `rich=1` from 1954-03-10 to 1955-03-27 (95 heartbeats): the bank rose from 271,732 to
     721,346, and vehicles stayed at 33-35 (O).
   - The rich threshold is 5 × `line_budget` = 250k (`P/rich.nut:53-55`).
4. **Idle money has no path to trains.**
   - `InvestSurplus` buys road, air and water vehicles only (`P/fleet.nut:270`).
   - Trains grow only through the steward's waiting-cargo and rating rules. On double track the
     steward capped lines at about 2 trains, even with `trains_max` 33-39 ("2 of 33", "2 of 39",
     1955-01-02) (O).
   - Full-load orders hide waiting cargo, so the rule underestimates demand (O, `P/steward.nut`).
5. **The line gate sits below one train.**
   - `line_budget` 30000 is less than a locomotive, which cost 30,513-30,793 in 1950-51.
   - In 10487 a finished line waited 99 days for its first train, from 1950-10-07 (`locomotive not
     bought: ERR_NOT_ENOUGH_CASH`) to 1951-01-14 (O; `P/main.nut:567-568`).
6. **The rival competes where we do not.**
   - AAAHogEx's `IsAvoidSecondaryIndustryStealing` resolves to 0 (`script_info.cpp:184`,
     `hog:info.nut`). It builds at producers others serve, while we skip 31-41% of the producers on
     land (O).
   - It sizes trains from production (`hog:estimator.nut:342-358`) and builds feeders (O).

### RC-b. The planned-mode trap: any plan switches off the floors, even one assembled piecemeal

This trap held company 0 to one train for years in 15805. It capped bus routes for 19 months in
12609, and it has bound for 179 game years in 10487. Its money effect has not been measured by a
paired run.

1. **A plan drops the floors.**
   - `b.planned` is true as soon as one freight slot, an air pair or a rail pair is set.
   - Then `autopiloted` is false, and the floors are skipped: `own_lines` 10, `max_routes` 30,
     intercity, self-steward and `cash_reserve` (`P/main.nut:393-402`) (O).
   - A slot counts only when `from`, `to` and `cargo` are all ≥ 0 (`P/main.nut:412-421`) (O).
2. **12609: completed piecemeal.**
   - Approvals from plans nine months apart completed slot fr10: #227 `fr10_to` (1951-11-01), #460
     `fr10_cargo` (1952-07-13) and #461 `fr10_from` (1952-07-15) (O).
   - The behaviour line shows `max_routes=4` from 1952-07-31. The heartbeats go from `engaged
     max_routes=30` (1952-06-16) to `growth-on-plan max_routes=4 routes=17/4` (1952-10-12). Rich
     mode put the floor of 30 back on 1954-03-10 (`P/rich.nut:110`) (O).
   - Nobody lowered `max_routes`: 4 is its `info.nut` default (`P/info.nut:44-52`), exposed once the
     floor went away (O).
   - The panel itself asked six times for 30 (#498 … #713). None was approved. The owner approved 6
     (#719, 1953-11-19) (O).
   - `max_routes` gates bus routes only (`P/main.nut:602,610`) (O).
   - The exact tick of the flip is INFERRED: no heartbeat falls between 1952-06-16 and 1952-10-12.
3. **15805: planned from the start.**
   - The start line set `rail_a=1, rail_b=3`. Company 0 ran `growth-on-plan own_cap=4 own_lines=4`
     from 1950-04-18 to 1958-12-05 (O).
   - `own_cap` also stayed at 4 because the bank never reached 60k, which is 2 × `line_budget`
     (`P/steward.nut:202-206`) (O).
4. **10487: a whole plan executed by the deadline** (pass 3, §5). The game started unplanned with
   `own_lines=4`, which the floor raised to 10. A 36-row re-plan landed on 1952-04-10/12, and the
   bot has been `growth-on-plan own_lines=4` since 1952-04-15 (O).
5. **The settings that bind under a plan are invisible where they were chosen.**
   - In the arena the bot is unplanned, so `own_lines` and `max_routes` are floored.
   - `max(x, 30)` is always 30 (`P/info.nut:47`), and `own_lines=4` cannot show any harm there.
   - In 467 arena matches with `own_lines=4`, only 0.2% had ≤ 1 train at year 10 (D, 5,930 local
     matches).

### RC-c. Decide → act is broken, and nothing senses the gap

The mechanism is confirmed. Its money effect is unmeasured, because no paired replay was run. It
therefore ranks as "the loop is open", not as a proven money cause.

1. **Filed one key at a time.**
   - replan files one `ai.set_setting` approval per setting, in Rhai map order, which is alphabetical
     (`rhai/build_plan.rhai:66`).
   - The atomic `ai.set_settings` is owner-only (`OWNER_ONLY_TOOLS`,
     `tools/research/mcp-rhai/src/panel.rs:550`) (O).
2. **Retired in about a minute.**
   - The deadline was "30 game days, then keep". At about 2 s a game day, a row was retired about
     60 wall seconds after filing: median 59.9 s in 12609, 60.2 s in 31337 (D).
   - Of 755 rows in 12609, 25 executed. They sat at the head of their plans, all within the first
     13 positions. One complete fr10 triple landed (#594-#596, 1953-03-06), and the rest were
     half-triples (O).
3. **The owner gate.**
   - `/api/approve` was refused twice for lack of an owner secret (12609, 02:45:43Z and 02:46:03Z).
   - The one owner action that passed was a blanket auto-approve (03:17:40Z).
   - Sign-in arrived with #242 (03:28:26Z) (O).
4. **15805: replan held for years, invisibly.**
   - A stale pre-#211 panel list held replan for 58 rounds and 1,155 proposed rows (O).
   - The hold leaves `held: null` in the record (`rhai/replan.rhai:340-347,361`), so nothing shows
     it. The cold-start hold in 10487 is just as silent (O).
5. **Nothing measures the gap to the leader.**
   - The shared keys hold `advice.*`, `audit.*`, `replan.0` and `watchdog.*`, but no `gap.*` (O).
   - Battlebrain produced 0 events in 15805, 31337, 12609 and 10487 (O).
   - replan judges its own record and payback only (`rhai/replan.rhai:228`). That record is pooled
     across modes: in 10487 on 1951-09-29, losing buses held a rail plan back (O).
   - `ApplyPressure` runs only when rich (`P/main.nut:407`). It fired in 15805 only when the gap
     was already 26.9× (1959-10) (O).
   - The watchdog triggers on "no change in vehicles or stations". In 15805 it blamed a ghost queue
     87 times (O).
   - Agent R's derived ledger shows that replan acted in 7 of 219 deterioration episodes (3.2%), and
     the watchdog in 19 of 157 (`feature/rhai-self-evaluation` 13d557b4ba,
     `research/mechanisms/service-self-evaluation.md`). Pass 2 did not re-derive these figures.
6. **The approvals died with the world.**
   - 15805's 29 approvals were cleared at the 1952-10-23 load (`29 approval(s) cleared: a new or
     loaded world`).
   - 28 of the 30 later reminders were about that stale list (O).

### RC-d. The learners are blind to the leader

1. **Fitness rewards rank.**
   - Fitness = normalised points + 0.1 × the mean of log10(value / the winner's) − a last-place
     penalty × the share of last places (`tools/research/ai-battle/battle.py:536-554`) (O).
   - Halving the gap adds +0.030. One rank lower in one of six matches costs −0.042 (D).
2. **The arena is not the live world.**
   - All 351 records start in 1960, with 8×8 lineups and 20 years. 15,167 of 15,251 sampled
     matches had 5 companies, and the bot runs unplanned (O).
   - The live games start in 1950 with 8 companies and a planned path (O).
   - At year 4 the arena's ratio is 2.35; live 15805's is 9.79 (D).
3. **Promotions measure against ourselves.**
   - In the 80 duel records the planned bot "won" 68 times against its own unplanned copy.
     AAAHogEx appears in none of them (O).
   - Promotions pair a configuration with the previous default (O).
   - g347.3 came from 6 matches, while `promotion.min_pairs` is 16 (O).
4. **No live game reaches a learner.**
   - `prismatic_tune` appears 0 times in the five live runs, the current one included (O).
   - `calibrate` is off by default ([planner](mechanisms/planner-and-algoglyph.md)) (O).
   - The fitness check runs only when the bot changes, and only on the gate's seed
     (`tools/gate.sh:683-686`) (O).
   - The `trainless_max` rule was added during the game, at 2026-11-23 00:53Z (O).
5. **The decision record's planned arm cannot be checked.** It reports a planned start losing
   35-70% against an unplanned one on the real field, but its raw files were not found (gap).

### RC-e. The 1950-52 loan policy: an accelerator, not the first fix

1. **The policy minimises debt.**
   - `RepaySurplus` → `StayAfloat` → `BuildFrozen` for 90 days (`P/main.nut:282-287`).
   - 12609: 42 repayments of 1.19M in total, and construction frozen on about half of the days of
     1950-52 (O / D).
   - 15805: frozen on 815 of 1,387 days (O).
2. **AAAHogEx counts the undrawn loan as spendable.**
   - It plans against bank + loan headroom (`hog:main.nut:4295-4297`).
   - It held its loan at 180-300k from 1950-04 to 1955-01 and never kept more than 108k in the bank
     (O).
3. **Bounded.**
   - The loan can add at most `max_loan` = 300k of deployed capital. At a return of 49-91% a year,
     that is at most 150-270k a year from year 2 (D).
   - That is a one-off step in capital, which compounds only through the lines it funds. The fix is right and cheap, and it speeds up
     years 1-3, but only if RC-a and RC-b leave something to spend on.
4. **Money binds only early.** "Money was not the binding constraint" holds from 1952-10, and
   without exception from 1954-03 (O). It does not hold for 1950-52.

## 4. Remedies, owners and order

The order is the one codex and pass 2 recommend:

1. **The planned-mode trap.** A silently binding cap confounds every later experiment, so it is
   fixed first.
2. **Funnel instrumentation before a wider search.** The search is widened only once the funnel
   shows that more candidates become running lines.
3. **A live-like arena before any tuning claim.**

| # | Remedy | Owner | Branch / PR | Verified by |
|---|---|---|---|---|
| R0 | Funnel instrumentation: every candidate logged at found, affordable, path found, stations built, first train and profitable after a year, with the game days and money spent at each stage | openttd-36 | no worktree yet | one replay of a 15805 save reproduces the known counts: 0 pairs tried after 1952-10, 17 single-track fallbacks, 5 path searches out of nodes |
| R1a | A slot counts only when whole and from one plan; the floors stay on under a plan; early rail | agent E | `fix/prismatic-handed-slot-guard` (98b3b5e061), `fix/prismatic-planned-and-early-rail` (pushed, no PR) | no `growth-on-plan` heartbeat without one approved whole plan |
| R1b | One `ai.set_settings` per re-plan; supersession of a batch settled; a net-investment hoard rule; the countdown design last | openttd-7d | `fix/approval-plan-batch`, #254 | a plan lands whole or not at all |
| R1c | The deadline verdict, a governance routes allowlist and a `governance.changed` event; `start_ai` behind the owner gate (still unowned; openttd-2e is the natural owner) | openttd-2e | #264 (approver-company verdict), #255 / #265, #258 | an unauthenticated start or approve is refused |
| R1d | Running the stack, the replan service and the deploys; holding the deadline `approve` verdict for the co-play company until R1a and R1b are deployed | openttd-51 | the live stack | the store shows no plan rows approved by the deadline |
| R2a | Claims: a foreign station claims a producer only while it is fresh (< 365 days); the rich path scores the share left | agent A | `fix/prismatic-claims-clean` (uncommitted) | `no own freight line` reports pairs tried > 0 |
| R2b | Wider candidates: < 100 tiles, town acceptors, a second acceptor, secondary cargo; after R0 | openttd-36 | no worktree yet | R0's funnel shows more lines reaching "first train" |
| R2c | Contesting or defending a producer a rival serves, on the normal path (not with statues: the ablation measured −30%) | **unowned** | – | – |
| R3a | A capital allocator: the loan counts as headroom, and rail gets a money path | agent I | `fix/prismatic-capital-allocator` (uncommitted) | days frozen < 15% in years 0-3; trains bought from surplus |
| R3b | A production-based train target, double track by volume, and a capital-deployed watchdog | openttd-36 | no worktree yet | trains per line and idle cash over time |
| R4a | A live-like arena (1950, 7 rivals, the planned path), before any tuning claim; it hosts codex's 4-arm paired falsifier (§7) | openttd-36 | no worktree yet | the arena's year-4 ratio falls within the live range |
| R4b | A lost live game vetoes the configurations that lost it | agent N | #256 | a lost live game vetoes its genome |
| R4c | A gap signal becomes a tested proposal | agent P | #262 | – |
| R4d | The gap and the post-mortems become derived surfaces | agent Q | #259 | – |
| R4e | A failure-regime detector | agent S | `feature/failure-regime-detector` (pushed) | – |
| R4f | The strategy post-mortem and the rule `prismatic-ai-is-judged-against-its-strongest-rival` (an uncommitted draft; this document absorbs its findings) | agent K | `doc/strategy-postmortem` | – |
| R5a | A leader-gap watch (`gap.<c>`) | agent J | #261 | – |
| R5b | The review cycle (ADR 0042) | agent L | #260 | – |
| R5c | Planner outcome feedback (`outcome.<c>`). It is written only on the planned-bot path, and the live panel runs `--no-plan-bot` | agent M | #257 | `GET /api/rhai/shared?prefix=outcome.` |
| R5d | A strategy digest that reads `gap` and `outcome` | agent O | `feature/strategy-digest` (pushed) | – |
| R5e | Services judge themselves by what followed their actions | agent R | #263 | – |

<!-- open-finding kind=staged -->
Five holes found by this analysis have no owner: contesting or defending a producer a rival serves,
replan's record pooled across modes, replan's unrecorded hold reasons, replan's mesh write refused
`out_of_scope`, and the watchdog's restarts. In detail:

- contesting or defending a producer a rival serves (R2c);
- replan's record is pooled across modes, so losing buses can hold back a rail plan;
- replan does not record why it holds (`held: null`, both for the wait on the owner and for the
  cold start);
- replan's mesh write is refused `out_of_scope` in every round, because the panel's session does not
  claim `RECORD_SCOPE` (`rhai/replan.rhai:70`);
- the watchdog restarted 7 times in 1.9 game years of 10487; J lists this as open but has no code.

## 5. Game 10487: the predicted risk happened

Pass 2 (2026-11-23, about 05:20 CET) judged 10487 "one adopted re-plan away from 15805's
configuration":

- replan held a 36-key plan for company 0 (`rail_a`/`rail_b`, `fr*`).
- Only its cold-start rule held it back: `pays_lower` 0.26 < `MIN_PAYS` 0.5.
- The deadline was "7 game days, then approve".

Pass 3 then re-read the store. Every item below is OBSERVED, from SQL on `events` with `ts_ms >=
1795405920135`.

- **The plan was adopted and filed.**
  - `replan.0` of 1952-04-07: `adopt=true`, `pays_lower` 0.768, 36 rows proposed.
  - 36 `pending approval` lines were filed from 1952-04-05 to 1952-04-07: fr10, fr11, fr2 … fr,
    `rail_a=15`, `rail_after=8` and `rail_b=17`.
- **It landed within seconds.**
  - #1 (`fr10_cargo=8`) was approved by chat, by the registered approver in company 0 (05:19:53
    wall).
  - #2-#36 were approved "via deadline (the operator's standing verdict: approve)" at 05:19:57
    wall, four seconds later.
- **The list was not empty.** The approval list only looked empty: the deadline drains it within
  seconds. Up to 1965-01-17, 1,666 rows were filed. 1,647 were approved by the deadline and 17 by
  chat.
- **The bot flipped.** The heartbeat went from `autopilot=engaged own_lines=10 max_routes=30`
  (1952-04-11) to `growth-on-plan own_lines=4` (1952-04-15). `own_lines=4` is the run's own
  setting, which the floor had raised to 10.
- **The outcome.**
  - The ratio was near its best when the bot flipped: 2.19× at the end of 1953.
  - From 1955 the cash grew while the fleet did not:

    | Year | Cash | Vehicles |
    |---|---|---|
    | 1955 | 568k | 33 |
    | 1960 | 2.88M | 23 |
    | 2000 | 26.0M | 22 |
    | 2114 | 165.1M | 42 |

  - In 2131, 185.7M of the company's 186.2M value is cash. It runs 40 trains, 2 road vehicles and
    20 stations, and its heartbeat reads `rich=1 own_cap=32 own_lines=4 routes=29/30`. AAAHogEx is
    worth 4.87B with 1,085 vehicles (26.1×).
  - The last candidate diagnostic (2131-01-03) reads "91 producers on land, 13 ours, 28 claimed …
    165 pairs tried", and no own line was built.
- **The chain, as read from these observations (D / I, not itself observed):**
  - RC-b: the deadline applied a plan row by row, and the floors dropped.
  - RC-a: there were no candidates, and idle cash has no path to trains.
  - RC-c: nothing reacted for 179 game years.
  - RC-d: no learner saw it.

The mitigations follow from this:

- **Deploy agent E's guard and the floors under a plan** (R1a) to the live bot: `rescan_ai`, save,
  load. After that a plan no longer drops the floors.
- **Turn the deadline `approve` verdict off for plan rows**, or restrict it to a company a
  registered approver plays (#264), until a plan lands whole (R1b).
- **Let the owner answer each plan explicitly**, before its 7 game days run out. That is about 14
  wall seconds, so a visible countdown (#254) must come first.
- **Reset the slot in the running game.** An owner-approved `ai.set_settings` that sets
  `fr*`/`rail_*` back to −1 removes the plan. Only the owner can approve it.

openttd-51 runs the stack and owns these steps. As of 2026-11-24 17:34 CET (game 2131-01-19) the
heartbeat still reads `growth-on-plan own_lines=4`, so none of them is in effect in this game (O).
Whether any is scheduled is not part of this record.

## 6. The owner's answer recorded this session

Session `7709ab47-7786-494d-bbcc-679b293569a7` (openttd-36), 2026-11-23, about 04:56 CET,
AskUserQuestion. The question is quoted as the coordinator relayed it; the ellipsis is the relay's.

- Question: "~/dev/.build-slot-1 is a stale lock … What should I do?"
- Answer: "Release the slot (Recommended)"

## 7. The test that would falsify RC-a

Codex proposed this test, and pass 2 adopted it with two corrections. It is one paired 4-arm
experiment:

- **Starts:** 40 live-like starts: 1950, seven rivals, the same initial state in every arm, six
  years, with R1 applied to every arm.
- **Arms:**
  - the baseline;
  - R2 (claims and candidates);
  - R3 (capital);
  - R2 + R3.
- **Metric:** profitable operating rail vehicles at year 6.
- **Manipulation check:** the extra buildable, fundable candidates exposed a year in years 3-6.
- **Falsifier:** the combined arm exposes at least two extra candidates a year, yet the upper 95%
  bound of its paired fleet gain is below 25%, or that of its gap reduction is below 20%.
- **If no extra candidates appear,** that falsifies the intervention, not the hypothesis.
- **Prerequisite:** R4a. The GA's arena is not live-like. The 1950 headless evaluation comes
  closer, but it runs one match a seed and only the unplanned path.
- **Not covered:** depth (trains per line), which waits for a later 10-year run.

## 8. After the fixes: held-start pairs

Six PrismaticAI fixes merged after this analysis, without the held-start pairs that rule
`project.a-tuning-claim-needs-held-start-pairs` asks for: #271 (`fr_` slot guard), #272 (growth
funnel logging), #277 (claims-clean), #279 (floors under a plan, pair waits), #280 (steward
cadence, JudgeAge) and #276 (capital allocator, merged under an owner-approved fitness exception).
This section records those pairs (openttd-36, 2026-11-25).

**Setup.**

- **Arena.** `battle.py ablate --live-like` (#274): 1950, 256x256, temperate, reduced breakdowns,
  `script_max_opcode_till_suspend` 250000, PrismaticAI in slot 0 against the seven live rivals at
  their live versions. Both arms ran, `unplanned` and `planned` (battle.json `live_like.arms`),
  for 10 game years to 1960-01, on seeds 15805, 2927, 4242 and 31337.
- **Trees.**
  - The baseline is `eb0d839ce0`, the commit live game 21647 started from.
  - The fixed tree is master `8f35e35995`. Its PrismaticAI tree is the same object as at
    `42e3d37ae7` (`a416fe3e04`), so it is the bot of `42e3d37ae7`.
  - `git diff eb0d839ce0 42e3d37ae7 -- src/` is empty. Both trees ran one binary, built at
    `53cd7b467d`, whose `src/` is the same as both.
- **Harness.** Both trees ran master's `battle.py`, `battle.json` and BattleReferee. They also
  shared one private arena: the seven rival tars and libraries of `~/Documents/OpenTTD/ai`, and
  OpenGFX 8.0. Only PrismaticAI differs between the two arms of a pair.
- **Held start.** A headless `ablate` is deterministic for a given tree, binary and seed: the
  match starts at tick 0, with no wall clock and no approvals. That makes it the headless form of
  the held start. The pairs differ only in the bot.
- **Load governor.** The 1-minute load was mostly above 24, so the runner SIGSTOPped the games
  above it and resumed them below 22. The null driver counts ticks, so a pause does not change
  the result.
- **Timeout.** `ablate`'s 1800 s `communicate` timeout was lifted for these runs only, because a
  paused match outlives it. The change was not committed.
- **The planned arm off 15805.** The planned arm's `fr_*` ids name industries of seed 15805's
  map, which this match does not regenerate (see
  [the live-like arena](ai-battle/README.md#the-live-like-arena-and-the-15805-replay)). On every
  seed it is therefore a fixed slot set naming arbitrary industries, the same in both trees.

**Records.** Each `ablate` writes one record with both arms:

| Seed | Baseline `eb0d839ce0` | Master |
|---|---|---|
| 15805 | [ablation-20261125T092209-seed15805](results/ablation-20261125T092209-seed15805.md) | [ablation-20261125T025837-seed15805](results/ablation-20261125T025837-seed15805.md) |
| 2927 | [ablation-20261125T102535-seed2927](results/ablation-20261125T102535-seed2927.md) | [ablation-20261125T095621-seed2927](results/ablation-20261125T095621-seed2927.md) |
| 4242 | [ablation-20261125T121043-seed4242](results/ablation-20261125T121043-seed4242.md) | [ablation-20261125T111726-seed4242](results/ablation-20261125T111726-seed4242.md) |
| 31337 | [ablation-20261125T124527-seed31337](results/ablation-20261125T124527-seed31337.md) | [ablation-20261125T123153-seed31337](results/ablation-20261125T123153-seed31337.md) |

**Where the numbers come from.** All of them come from the runs' `-d script=4` logs (scratch,
not kept):

- Value and rail trains are the last `REF Q` sample (1960-01). The rank is among the 8
  companies. The ratio is PrismaticAI's value over AAAHogEx's (company 1).
- Open lines are the `state=open` lines of the bot's last `rail line N: label=...` dump
  (`P/lines.nut`). On master this equals FUNNEL `lines` in all 8 runs.
- Trains per line is rail trains divided by open lines.
- "Most lost" is `battle.py funnel_most_lost` of master's last FUNNEL rail row, for 1959. The
  baseline has no FUNNEL, because it predates #272.

| Seed | Arm | Tree | Value | Rank | Rail trains | Open lines | Trains/line | AAAHogEx | Ratio | FUNNEL most lost (1959) |
|---|---|---|---|---|---|---|---|---|---|---|
| 15805 | unplanned | base | 1,695,263 | 2/8 | 10 | 7 | 1.43 | 18,119,788 | 0.094 | (no FUNNEL) |
| 15805 | unplanned | master | 1,672,695 | 2/8 | 11 | 5 | 2.20 | 16,098,440 | 0.104 | discovery served 28/42 |
| 15805 | planned | base | 1,093,652 | 2/8 | 8 | 4 | 2.00 | 13,127,277 | 0.083 | (no FUNNEL) |
| 15805 | planned | master | 1,501,066 | 2/8 | 18 | 9 | 2.00 | 18,044,458 | 0.083 | discovery served 25/43, build stations 3/3, run paying 1/9 |
| 2927 | unplanned | base | 4,905,754 | 2/8 | 23 | 11 | 2.09 | 16,299,596 | 0.301 | (no FUNNEL) |
| 2927 | unplanned | master | 5,348,189 | 2/8 | 47 | 14 | 3.36 | 17,926,041 | 0.298 | discovery served 35/56, build stations 2/2, run paying 1/14 |
| 2927 | planned | base | 2,376,819 | 2/8 | 13 | 7 | 1.86 | 23,686,876 | 0.100 | (no FUNNEL) |
| 2927 | planned | master | 2,452,521 | 2/8 | 29 | 14 | 2.07 | 19,525,450 | 0.126 | discovery served 40/55, build stations 5/5, run paying 3/14 |
| 4242 | unplanned | base | 3,143,918 | 2/8 | 19 | 9 | 2.11 | 20,218,593 | 0.155 | (no FUNNEL) |
| 4242 | unplanned | master | 1,196,978 | 4/8 | 16 | 6 | 2.67 | 23,079,790 | 0.052 | discovery served 45/56, build lay 1/1, run paying 3/6 |
| 4242 | planned | base | 2,310,998 | 2/8 | 10 | 6 | 1.67 | 21,128,423 | 0.109 | (no FUNNEL) |
| 4242 | planned | master | 1,767,480 | 3/8 | 15 | 7 | 2.14 | 26,308,550 | 0.067 | discovery served 39/56, build path 4/4, run paying 3/7 |
| 31337 | unplanned | base | 4,291,757 | 2/8 | 22 | 11 | 2.00 | 11,589,201 | 0.370 | (no FUNNEL) |
| 31337 | unplanned | master | 4,223,784 | 2/8 | 29 | 13 | 2.23 | 14,941,668 | 0.283 | discovery served 30/51, build path 2/2, run paying 3/13 |
| 31337 | planned | base | 2,095,865 | 2/8 | 11 | 6 | 1.83 | 15,874,165 | 0.132 | (no FUNNEL) |
| 31337 | planned | master | 2,286,892 | 2/8 | 22 | 12 | 1.83 | 17,360,841 | 0.132 | discovery served 42/51, run paying 2/12 |

The rank says little. AAAHogEx is first in all 16 runs. The other six rivals stay small: none
ends above 2.7M, and CivilAI ends at a value of 1 in 6 of the 16. PrismaticAI is second in 14 of
the 16 runs.

**The paired change, master against the baseline.** The rival spread is AAAHogEx's own change
between the two runs of a pair. AAAHogEx is the same AI in both, so the spread is how far a
change in PrismaticAI alone moves the rest of the game.

| Seed | Arm | Value | Ratio to AAAHogEx | AAAHogEx (rival spread) | Rail trains | Trains/line |
|---|---|---|---|---|---|---|
| 15805 | unplanned | −1.3% | +11.1% | −11.2% | 10 → 11 | 1.43 → 2.20 |
| 2927 | unplanned | +9.0% | −0.9% | +10.0% | 23 → 47 | 2.09 → 3.36 |
| 4242 | unplanned | −61.9% | −66.6% | +14.2% | 19 → 16 | 2.11 → 2.67 |
| 31337 | unplanned | −1.6% | −23.7% | +28.9% | 22 → 29 | 2.00 → 2.23 |
| 15805 | planned | +37.3% | −0.1% | +37.5% | 8 → 18 | 2.00 → 2.00 |
| 2927 | planned | +3.2% | +25.2% | −17.6% | 13 → 29 | 1.86 → 2.07 |
| 4242 | planned | −23.5% | −38.6% | +24.5% | 10 → 15 | 1.67 → 2.14 |
| 31337 | planned | +9.1% | −0.2% | +9.4% | 11 → 22 | 1.83 → 1.83 |

Geometric means over the four seeds (D):

- **Unplanned.**
  - Value −20.3%. Without 4242 it is +1.9%.
  - Ratio to AAAHogEx −27.2%.
  - Master's value is higher on 1 of 4 seeds.
  - Rail trains +25.7%; AAAHogEx's trains moved −18.6% to +55.0% across the four pairs.
  - Open lines −8.0%.
- **Planned.**
  - Value +4.3%.
  - Ratio to AAAHogEx −6.4%.
  - Master's value is higher on 3 of 4 seeds.
  - Rail trains +97.0%, higher on 4 of 4 seeds. AAAHogEx's trains moved −23.4% to +4.8% in the
    same pairs.
  - Open lines +80.0%.

**Against the noise.** Prior notes put two behaviour-identical trees ±13 to 20% apart on one
seed. Here the rival spread runs from 9% to 38%.

- **Value.** Seven of the eight value changes are inside the spread of their own pair; seed 4242
  unplanned, at −61.9%, is the exception. The value claim is therefore inconclusive (rule
  `project.never-report-unrun-validation-as-pass`).
- **The planned arm's rail fleet.** It rose by 50% to 125% on every seed, more than the rival
  moved in any planned pair (−23% to +5%). This fits #279 (floors under a plan), which keeps
  building own lines while a plan is set.
- **The unplanned arm's rail fleet.** Mixed: trains fell on 4242 and rose on the other three.
- **The gap to AAAHogEx.** It does not close in either arm. The best master ratio is 0.298
  (2927, unplanned), against 0.370 for the baseline on 31337.
- **Seed 4242, unplanned.** It is worse beyond the spread. Master is behind from 1951:
  - 1951-10: 31k value and 2 trains, against 138k and 4 for the baseline.
  - Master kept the 300k loan to 1955 and borrowed back to 300k in 1958. The baseline was
    debt-free from 1954.
  - At the end, 1959-10, master had 1.21M and 56 vehicles, the baseline 3.11M and 104.
  - Not diagnosed. The candidates are the early line choice and the capital allocator's
    borrowing (#276, `invest_borrow=1` in the arm).
- **FUNNEL (master, 1959).**
  - In all 8 runs, the stage that loses the largest share is discovery `served`: 25 to 45 of 42
    to 56 producers.
  - Building loses next. Every station try failed in 2927 (2/2 and 5/5) and in the 15805 planned
    arm (3/3), and every path try failed in 4242 planned (4/4) and 31337 unplanned (2/2).
  - Discovery is still the stage that stops growth after the fixes (RC-a).

**Verdict: inconclusive.** The fixes double the planned arm's rail fleet and lines, beyond the
noise. Its value moved +4.3%, inside it. The unplanned arm is not better, and on one seed of four
it is worse beyond the noise. No default should change on these pairs, and no claim that the
fixes close the gap to AAAHogEx holds on them.

**Not run.**

- More than four seeds.
- A second run of the same tree on the same seed. It would be byte-identical, as `ablate` is
  deterministic, so the noise was read from the rival spread instead.
- Live or panel duels with `PRISMATIC_START_PAUSED=2`.
- A planned arm whose plan was made on each map.
- A per-fix ablation: the six fixes were measured together.
- Horizons other than 10 years.

## Corrected claims

Pass 2 refuted or corrected the claims in the left column, from pass 1 and from peers. Do not reuse
them.

| Claim (refuted) | Correction | Basis |
|---|---|---|
| Headless profit per train is about equal: 3% of the rail gap | AAAHogEx earns 1.51× a train: 18% of the log rail gap | the 3% divided 1958 profit by the 1959-10 fleet; on a consistent basis it is 1959 profit over the 1960-01 fleet |
| The rail gap splits into lines 57%, depth 40%, yield 3% | 49% / 33% / 18% (log, 1960-01); the linear split is count 62%, yield 38% | the same basis error |
| AAAHogEx's 112 lines against our 37 are 57% of the gap | 34 of the 112 are `T:` feeders: 49%, or 33% without the feeders | AAAHogEx's log |
| AAAHogEx builds 7-23 feeders a seed | 7-10 a seed, 34 in all | the `T:` builds; `ARC:` does not count feeders |
| Live short rail tries: 11 of 21 | 9 of 19 | ai.log TryBuild |
| AAAHogEx's loan stays at 274-300k, its cash ≤ 64k, its TryBuild ROI 272-876‰ | loan 180-300k; cash ≤ 108k (1954-01); ROI 272-2016‰ (median 616) | `samples`, ai.log |
| `max_routes` was lowered to 4 from outside in 1952-10 | 4 is the `info.nut` default, exposed when the planned flip (from 1952-07-31) dropped the floor; it caps bus routes only | heartbeats; approvals #227/#460/#461; `P/main.nut:602,610` |
| Every line is built with `trains_max = platforms` (2) | that is only the initial record; on double track the cap is the steward's waiting-cargo rule | `P/rail.nut:1171`; steward lines "2 of 33" |
| The 500-600k `InvestSurplus` threshold held back a live game | it held back none: the threshold was 100k, 2927/15805 stayed below it, and 12609 sat far above it. What holds is that trains never get that money | `P/fleet.nut:264,270`, `samples` |
| The 250k rich threshold kept the 1953-55 money idle (peer RC4) | `rich=1` from 1954-03-10, and still nothing was spent; the 1952-10 → 1954-03 cap was the planned flip | heartbeats |
| Money was never the binding constraint | only from 1952-10; in 1950-52 the loan policy froze construction on about half of the days | `samples`, ai.log |
| The loan policy is the first fix (peer ranking) | it is an accelerator, bounded at ≤ 300k of capital | D, `max_loan` |
| The approved rows are a strict alphabetical prefix, and fr10 never completed | they are head-biased (positions ≤ 13); one complete fr10 triple landed (#594-#596) | `console.line` |
| The owner gate always refused | it refused `/api/approve` twice; turning auto-approve on passed it at 03:17:40Z | the panel log |
| The arena ratio rests on 14,793 matches | it rests on the 2,213 that include the default PrismaticAI | battle records |
| g347.3 is about 10% better than the default | +6.3% over the default; the +9.5% is against `pre-g150` | leaderboard a80cc94a |
| The max_routes=16 / invest_min=600 stall was in game 15805 | it was in game 2927 | start lines |
| Each of the four tuning commits disclaims held-start pairs | three of the four do | `git show` |
| `digest.rhai` reads keys nobody writes | fixed since: `SIGNALS = ["gap","outcome"]` | agent O, 205d74fbed |
| PrismaticAI's reinvestment from 1953 in game 2927 was −4% to 80% | −38% to 128% (−38% to 80% from 1954); AAAHogEx 91-103% | `replan.0` |
| In 15805, 199 re-plans "all logged pending approval, and none was approved" | 29 were cleared at the 1952-10-23 load; 170 were retired by the deadline about 60 s after filing | `console.line`; [the trainless-lines incident](incident-2026-11-23-trainless-lines.md), now corrected |
| 10487: "the approval list was empty the whole time" (relayed 2026-11-24) | 36 rows were filed in 1952-04, and the 7-day deadline drained them within seconds; 1,666 were filed by 1965 | `console.line` (pass 3) |

## Gaps ledger

<!-- open-finding kind=not-run -->
The money effect of the planned-mode trap and of the broken decide-to-act loop was not measured: no
paired replay from held starts was run. This and the other items below were not verified, or could
not be:

- **The money effect of RC-b and RC-c.** No paired replay was run from held starts (autopilot alone
  against autopilot plus delivered re-plans).
- **The planned arm of the planned-vs-unplanned decision record (−35% to −70%).** Its raw files were
  not found.
- **AAAHogEx's profit per mode in 12609 for 1954-55.** No vehicle snapshot exists after 1954-01-04,
  so openttd-7d's 1954 figures (1,195,656 trains, 398,223 air) stay unverified.
- **Days frozen in 1950-52.** The figure is 52% or 47.9%, depending on the method. Neither method
  replays `BuildFrozen` exactly.
- **Headless profit per train is not clean by cohort**, because ages are mixed. The line counts are
  builds, not the routes still alive in 1960.
- **Shared producers in 12609: 4 of 7 (7d) or 5 of 7 (agent H).** Settling it needs the 0429 save.
- **The live map size**, compared with the arena's.
- **10487 after 1965-01-17.** Not examined: why no `pending approval` line was filed after that
  date, and why "165 pairs tried" in 2131 still built no line.
- **10487's vehicle profit by mode** was not read.
- **Agent R's 7 of 219 episodes** were not re-derived by pass 2. The ledger lives on an unmerged
  branch.
- **The candidate note on approvals.** It says approvals outlive a new world
  (`mcp-approvals-survive-stop-rotation-and-a-new-world`), while the 15805 store logs `29
  approval(s) cleared: a new or loaded world` at the 1952-10-23 load. The two were not reconciled.
- **The build commit of the live binary** (`build-gate-research/openttd-e2e-*`) is unknown, so it
  is unknown whether its deadline verdict is restricted to the approver's company.
- **The outside view is one model.** Codex answered. gemini was refused at the service level (the
  CLI account tier is retired), and no local ollama model was available.

## Related records

- [Incident: one train for eight years (15805)](incident-2026-11-23-live-game-15805.md)
- [Incident: finished rail lines without a train](incident-2026-11-23-trainless-lines.md)
- [Incident: four PrismaticAI companies on the same mines](incident-2026-11-22-sibling-industries.md)
- [PrismaticAI](mechanisms/prismatic-ai.md), [Approval gate](mechanisms/approval-gate.md),
  [Planner and AlgoGlyph](mechanisms/planner-and-algoglyph.md),
  [Rail steward and monitor](mechanisms/rail-steward-and-monitor.md),
  [Battle brain and live tuner](battle-brain.md), [Duel evidence audit](duel-evidence-audit.md)
- Knowledge:
  - [breadth, not caps](../.ai/repo/knowledge/candidates/prismatic-ai-falls-behind-aaahogex-on-breadth-not-caps.md)
  - [one train per line without a steward](../.ai/repo/knowledge/curated/prismatic-ai-without-a-steward-keeps-one-train-per-line.md)
  - [the planner overpredicts what a line earns](../.ai/repo/knowledge/curated/the-planner-overpredicts-what-a-line-earns.md)
  - [auto-approve is the owner's switch](../.ai/repo/knowledge/curated/auto-approve-is-the-owners-switch.md)
- Rule: [a tuning claim needs held-start pairs](../.ai/repo/rules/project/a-tuning-claim-needs-held-start-pairs.v1.md)
