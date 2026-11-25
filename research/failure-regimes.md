# Failure regimes

RESEARCH-ONLY. A failure regime is a failure that lasts: a company that stops expanding while its
rivals grow, falls behind the strongest rival faster than the rest of the field, runs a mode whose
vehicles lose money, or whose planner's plan never reaches the bot. In the owner's live game of seed
15805 PrismaticAI kept 25 to 32 vehicles from 1951 to 1964 while AAAHogEx went from 15 to 615, and
the planner adopted a plan the bot never ran in 41 rounds from 1953 to 1961. Nothing flagged
either (owner order 2026-11-23: "this should have been detected automatically"). The detector is
[`tools/research/ai-battle/regimes.py`](../tools/research/ai-battle/regimes.py); its tests are
[`tools/research/test_regimes.py`](../tools/research/test_regimes.py) and
`test_a_healthy_prismatic_that_stops_expanding_fails_with_a_named_regime` in
[`tools/research/test_ai_battle.py`](../tools/research/test_ai_battle.py).

## The regimes

Every threshold is relative to the rivals in the same game, or to the planner's own cycle. The few
constants are data, one source for every reader:
[`tools/research/ai-battle/regimes.json`](../tools/research/ai-battle/regimes.json) (`rules`, each
with its reason under `why`). `regimes.py` loads them as `regimes.RULES`, `battle.json`
`fitness.regimes` overrides any of them for the headless check, a Rust crate can embed the file,
and the live judge's `--post` writes it to the blackboard key `regime.rules`, where a Rhai service
reads it with `shared_get("regime.rules").rules`.

| Regime | Fires when | Gates fitness |
|---|---|---|
| `no_expansion` | for at least 24 game months our fleet stays within 10% of its count at the start of the run (`hoard_fleet_slack`), the median of the rivals' own fleet growth over the same run is above 10%, and our value over the strongest rival's ends the run lower than it began. The record says whether net money (money less loan) rose over the run (`capital: idle`) or not (`starved`) | yes |
| `widening_gap` | over at least 24 months the Theil-Sen slope of ln(our value / the strongest rival's) is negative and below the median of the other rivals' slopes of ln(their value / the strongest rival's) by ln 2 over the span: we fall behind the leader materially faster than the field | no: master's bot shows it (below) |
| `mode_never_pays` | two year ends in a row at which our vehicles of a mode, counted as those that ran the whole year, made a loss on average | no (finding below) |
| `mode_underperforms` | two year ends in a row at which our profit a vehicle of a mode is below every rival running it and below half their median | no: AAAHogEx shows it for its road vehicles in 5 of the 6 fitness matches |
| `unapplied_plan` | live only: at least 3 rounds of the planner (`replan.<company>`) adopt a plan the bot does not run (`proposed` not empty, no other planner holds the company); a round that holds neither ends nor counts | live only |

`widening_gap` is a trend. The level of the value ratio to the strongest rival is judged by the
rival ratio check (rule `project.prismatic-ai-is-judged-against-its-strongest-rival`), and this
detector adds no second ratio level. A regime record carries `since`, `until`, `detected_at` (the
first date a watcher reading the samples would have known), its `evidence` and its `threshold`.

## Where it runs

- **Headless.** `battle.py fitness` prints every regime of every contestant
  (`fitness[i]: <key> regime ...`) and fails on a gated regime of a PrismaticAI contestant in
  `fitness.must_be_healthy`, as `regime:<name>: ...`, against the contestants that are not
  PrismaticAI (a sibling copy is not a rival).
- **Live.** `tools/research/ai-battle/regimes.py --panel http://127.0.0.1:8800 --post` judges every
  running PrismaticAI company of the newest run from the panel's store (read-only SQL, POST
  `/api/db/query`: `company_month`, and every `replan.<company>` write as `run_events` kept it)
  and writes `regime.<company>` and `regime.rules` to the blackboard (writer `regimes`, a
  `rhai.shared` event). It
  reuses the leader-gap watch: `gap.<company>`'s leader is the strongest rival, and its flags,
  streaks and level ride along in the record. `--db <copy of a store>` reads a copy with the
  duckdb CLI, `--record research/results/live-*.json` an exported live record. It exits 10 when a
  regime was found.
- **Search.** `regimes.live_regimes(record)` returns `{company: [regimes]}` for an exported live
  record (schema `ai-live-outcome/v1`), `regimes.detect(series, who, rivals)` the regimes of one
  match, and `regimes.penalty(regimes)` their length in game years, for a genetic algorithm that
  wants a penalty term. Nothing here is wired into the GA.

## AlgoGlyph

AlgoGlyph was looked at first and is not used. Its catalogue has the fitting primitives,
`page-hinkley` (change detection) and `least-squares`, but at the pinned commit (`1b8e009`) and on
its `main` neither has a data form, so the flow runner does not list them (`algoglyph.pin.json`
`kernels`) and runs them only on their own seeded demo stream. The linked crate
`algoglyph-kernels` carries sorting kernels only. A design review also found Page-Hinkley's
running-mean statistic blind to a gap that widens steadily from the start, which is this
failure. The trend is a Theil-Sen slope in the fork.

## Evidence

Measured 2026-11-23 on master `eb0d839ce0`, binary `20261122-master-gfdc13851cf`:

- **Live game 15805** (store `panel-8800`, run 1795381184362, monthly samples, a copy read with
  `--db`): `no_expansion` from 1953-04 to 1961-12, known 1955-04 (fleet 30 to 25, the rivals'
  median grew 294%, capital idle), and `unapplied_plan` from 1953-11-28 to 1961-08-24, known
  1954-05-02 (53 rounds, 41 adopted, 21 settings differ, no approval filed in the run). Its
  exported yearly record gives `no_expansion` 1953-12 to 1964-11, known 1955-12.
- **Live game 12609** (run 1795400722667): PrismaticAI reached 1.54M against AAAHogEx's 2.62M;
  no series regime. `unapplied_plan` 1951-04-08 to 1954-12-17 (20 rounds, 16 adopted, 753
  approvals filed in 16 rounds).
- **Live game 2927** (run 1795374123856, four PrismaticAI companies): `no_expansion` and
  `widening_gap` for company 0, `unapplied_plan` 1951-04-26 to 1957-11-12.
- **Headless, master fitness** (seed 930967890, six lineups, `battle.py fitness` on this branch:
  PASS, 205 s): no gated regime for a contestant in `must_be_healthy`. Printed, not failing:
  `no_expansion` for `PrismaticAI solo-max` (not held healthy), for LuDiAI and AdmiralAI in lineup
  2 and for AAAHogEx in lineup 1 (a rival is judged against the other non-PrismaticAI contestants
  only, there AdmiralAI); `widening_gap` for LuDiAI in five lineups and AdmiralAI in one, never for
  AAAHogEx; `mode_never_pays` for AAAHogEx's road vehicles in lineup 5. Lineup 0 on the rival
  seeds 31337 and 1383023900: no gated regime.
- **Headless, master at 42cae0fc33** (2026-11-25, lundra, binary 20261122-master-g2a6a616221,
  seven lineups, two runs): `widening_gap` for `PrismaticAI planned` in both runs (ln(value /
  AAAHogEx) -0.06 a year against the other rivals' median +0.10) and for `PrismaticAI` in one.
  Gated, it would fail every later change, so `regimes.json` gates only `no_expansion` until the
  bot closes that gap; `widening_gap` is still printed and posted live.

## Finding: PrismaticAI's aircraft lose money

In 5 of the 8 headless matches above PrismaticAI's aircraft earned less than every rival's, and in
four of them they lost money: -1,485 (planned), -5,942 (sibling B), -1,663 (seed 31337) and -2,379
a vehicle a year (seed 1383023900, two year ends in a row, `mode_never_pays`), while AAAHogEx's
earned 27,541 to 59,358 and AdmiralAI's 15,315 to 35,580. `mode_never_pays` is not gated because
it fires on the default PrismaticAI on a rival seed: gating it would fail every change until the
air routes are fixed.

## Limits

- Fleet counts cannot see capacity: a company that buys longer trains instead of more of them looks
  flat. `no_expansion` therefore also needs our share of the strongest rival to fall.
- The live judge reads the newest run only (or `--run-ms`); a game loaded from a save starts a new
  run, so a decade split across loads is judged from the last load on.
- The panel store has no per-mode profit, so the mode regimes run headless only.
- Two years is the earliest `no_expansion` can know; in 15805 that is 1955, not 1964.
