# Experiment report: battlebrain-freight-margin (pre-registration, Phase 06)

RESEARCH-ONLY. This is a pre-registration. It was written before any strategy run, and it fixes
the hypotheses, the metrics, the seeds, the sample size, the stopping rules and the pass/fail
criteria before the first pair is played. **No strategy comparison has been run.** On 2026-11-22
the owner ordered that only one game runs on the mesh: the owner's co-play game on :8800, which
belongs to another session. Every strategy result below is therefore NOT RUN. Nothing written
here may be read as a result of a game.

- **Date:** 2026-11-22
- **Base revision:** `3dbdba818d` (origin/master when the branch `feature/battlebrain-experiment`
  was cut).
- **Goal / hypotheses:** see [Hypotheses](#hypotheses).
- **Changed files:** this document, one section of [`README.md`](README.md), the regression gate
  `tools/research/mcp-rhai/src/battlebrain_regression.rs` (included by one `#[cfg(test)]` line in
  `src/bin/battlebrain.rs`), the pair flags of `tools/research/mcp-duel` and their tests in
  `tools/research/test_mcp_duel.py`, and `battle.py power` with its tests in
  `tools/research/test_ai_battle_power.py`. The re-recorded result
  `research/results/battlebrain-shadow-20261122T084607Z-coplay-8800-freight-margin-2.{json,md}`.
- **Build/debug configuration:** none of the game's. The checks that ran are offline: a
  `cargo test --release` of the `battlebrain` binary and the Python unit tests. No game was built
  or started.

## Hypotheses

**H1 (correctness).** A freight-margin@2 decision never applies an unpriced or an unaffordable
line. Unpriced means that track-planning did not answer `found` for the line. Unaffordable means
that the investment exceeds money plus loan headroom minus the reserve, read at the decision's own
tick.

**H2 (strategy).** A company steered by battlebrain ends with a value share no lower than the same
company ends with under planner v4, on held-start pairs. The value share is A / (A + B) against
the default AI on the same seed.

H2 is tested as a non-inferiority claim with the margin δ fixed below. With any finite sample,
"no lower" can only be shown within a margin.

## Design

### Arms, baseline and instrument

The instrument is the held-start duel, `tools/research/mcp-duel`. It is the one the rule
`project.a-tuning-claim-needs-held-start-pairs` names. Each pair is two duels on one seed. Both
run 120 game months with `PRISMATIC_START_PAUSED=2` and the same rival B, the default AI:
`PrismaticAI` with its own defaults, which is mcp-duel's default opponent. Company A is the only
thing that differs.

| Arm | Company A | How it is started |
|---|---|---|
| **P** (baseline: planner v4) | PrismaticAI with the panel's plan-derived settings (`panel_bot.rs settings_from_plan`: up to sixteen `fr*` slots in the planner's order, own lines and steward as by default) | `mcp-duel --seed S --months 120 --label "bb-exp P S" --capture-at-start runs/S/p` |
| **BB** (battlebrain) | the same, plus the settings of battlebrain's live directive on arm P's held-start capture: `planned_freight_only=1`, one `fr_*` line, every other slot cleared | `mcp-duel --seed S --months 120 --label "bb-exp BB S" --directive runs/S/p/live.json --capture-at-start runs/S/bb` |

The decision is the one battlebrain makes on arm P's capture:

```sh
bb=tools/research/mcp-rhai/target/release/battlebrain
c=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["company"])' runs/S/p/held-start.json)
$bb snapshot --capture runs/S/p --instance "bb-exp-$S" --company "$c" > runs/S/p/snapshot.json
$bb live --snapshot runs/S/p/snapshot.json --plan runs/S/p/plan.json \
  --companies runs/S/p/companies.json --company "$c" --max-window-ticks 0 > runs/S/p/live.json
```

After arm BB, the same two commands run on `runs/S/bb`. Then
`tools/research/mcp-duel pair-check runs/S/p/live.json runs/S/bb/live.json` says whether
battlebrain decided the same on both starts. The pair is valid only if it did. The held start is
repeatable to a few dozen ticks, not bit for bit: the recorded held-start ticks of the duels range
from 1371 to 1449. So a pair is valid when both of its starts lead to the same decision. It is not
required that they are identical states.

Battlebrain decides **once, at the held start** (stage 1). Its decision every ~90 game days needs
the live loop of the Phase 04 cutover, which has not run. This experiment does not test that loop.

### Metrics

Per valid pair *i*, from the two duel records (`final` of company 1 = A and company 2 = B):

- **Primary (H2):** D_i = ln(A_BB / B_BB) − ln(A_P / B_P). This is the paired difference in the
  logit of the value share, because logit(A / (A + B)) = ln(A / B).
- **Secondary:** the last-place rate per arm (A's final value below B's), with the paired
  discordant counts.
- **Gates (H1 and safety):** each battlebrain live refusal, counted by its code. The codes are
  `incoherent_snapshot`, `unpriced_policy` and `stale_companies`. With the executor binding (I0117)
  they also include `fallback_solver`, `missing_kernel` and `stale_snapshot`, and with `--catalogue`
  they include `capability_unmatched` and `capability_mismatch`. Refusals by the mcp-duel
  `--directive` check are counted separately.
- For each BB arm: the selected line's `track` and `investment` against `comparison.budget`, and
  whether the bot was started with exactly the directive's settings (the duel record's
  `plan.bot_settings` `from_run` against `battlebrain_pair.directive.settings`).

### Seeds and maps

The seeds come from `python3 tools/research/ai-battle/battle.py power`. The first three are the
rule's three seeds (4242, 31337, 20208). The rest are the distinct seeds that the default
PrismaticAI played in the recorded battle, in record order. At `3dbdba818d` the first 93 are:

```
4242 31337 20208 1251879686 930967890 1814625749 397755790 852521374 622201523 1791605593
1016335253 1182050720 644316239 1005694260 542166871 1538202881 415608858 1623305058 589628189
1987023718 898469763 194341801 1112431946 431109973 864970882 1201871748 822940995 1503674030
1937674649 1710584728 700804041 2798290 373397060 1566354715 1738814815 700686639 478346528
145339768 202210594 1978581656 557130425 978315542 1131845159 1510845497 1419691949 1232613255
1110349149 1910910772 377297703 1968229012 751802737 1409843336 460626426 285172370 1535362987
7005316 496632888 82454744 1568204316 119790134 1039328191 1807087905 1609313107 234969011
266076295 1098158778 979034022 1297029967 345288325 61756459 1364593114 447329182 915958233
1820254859 112364617 725554694 600632846 1833374827 905704176 788855736 186005731 90692540
1859118493 251849652 204750381 268956824 878213703 20877647 1669891890 1791817241 263169837
89167670 866552619
```

That is the `pairs.seeds` array of the command's output at this revision, and this document
freezes it. If a later record changes the command's output, the run still uses the
list of `3dbdba818d`. Regenerate it with
`git worktree add /tmp/bb-exp 3dbdba818d && python3 /tmp/bb-exp/tools/research/ai-battle/battle.py power`.

The maps are the ones every recorded duel used: a new game made by the panel (`mcp-e2e`) with
`-G <seed>` and the duel's default game settings. They are not the arena map of battle.json
(`match`: 256×256, temperate, 1960, 20 years). The arena is a headless five-AI match without the
panel or the planner, so it cannot host arm P. The seed numbers are shared with the battle, but
the maps are not.

## Sample size (derived, not guessed)

The noise comes from the recorded battle: 351 records under `research/results/battle-*.json`.
`battle.py power` derives it from the records alone, and its offline tests are in
`tools/research/test_ai_battle_power.py`. Its output at `3dbdba818d`:

| Quantity | Value | How it is derived |
|---|---|---|
| Split-half Kendall τ of the GA score | **0.212** over 421 configurations (≥ 8 seeds each) | mean points of each configuration on its odd-ranked and its even-ranked seeds, then τ between the two halves |
| Reliability of one seed | r₁ = 0.065 (r_half = sin(π τ / 2) = 0.327 over a median of 7 seeds per half) | Greiner, then Spearman-Brown |
| Seeds a GA score needs for a rank reliability of 0.8 | **58** | Spearman-Brown on r₁ |
| σ of the paired log value ratio across seeds | **0.366** (median; quartiles 0.314 to 0.459) over 84 configurations | ln(value / default's value) per (edition, seed), each with ≥ 16 seeds paired with the default PrismaticAI, as `paired_vs` pairs them |

A τ near 0.2 reproduces the earlier fitness ablation (τ ≈ 0.23). The method here, odd and even
ranks over every configuration with at least eight seeds, gives 0.212. Most of a GA score is the
map it was played on, so GA selection order is not evidence about H2.

**The formula.** H2 is tested with a paired one-sided t test for non-inferiority, at α = 0.05 and
power 0.80, assuming no true difference:

n = ⌈ ((z₀.₉₅ + z₀.₈₀) · σ / δ)² + z₀.₉₅² / 2 ⌉. The second term is Guenther's correction for
the t distribution. n is never under the rule's three seeds.

**The margin δ = ln 1.10 = 0.0953.** That is a value against the rival at most 10% lower than the
planner's. The reason for 10%: it is twice the battle's smallest promoted gain (battle.json
`promotion.min_ratio` 1.05). It is also the size of the largest planner-attributable effects
measured so far: own lines +10.4%, which turned out to be noise, and a held-start repeat spread of
about ±10% over ten years. A loss that large would erase anything the planner was ever shown to
add.

**Result: n = 93 valid pairs.** The sensitivity of n to the margin, from the same command:

| margin | 1.05 | 1.10 | 1.15 | 1.20 | 1.25 |
|---|---|---|---|---|---|
| n (pairs) | 349 | **93** | 44 | 27 | 18 |

**A cross-check against the duel instrument.** This σ is measured in the five-AI, 20-year battle.
The held-start duel pairs on record give smaller paired differences D:

- defaults on/off at `a23cbe0df4`, where the knowledge record calls the effect inconclusive: −0.0008
  (4242), −0.0493 (31337), +0.0611 (20208);
- the frozen control at `55328bdb56`, identical arms: −0.0513 (31337), −0.2176 (4242); on 20208 B
  ended at value 1, so the pair is degenerate.

The battle's σ is therefore the conservative choice.

**A re-estimation that only raises n.** After the first 10 valid pairs, the 80% upper confidence
bound of the standard deviation of their D_i is computed: s · √(9 / χ²₀.₂₀,₉). If it exceeds 0.366,
n is recomputed with that bound. n is never lowered. The mean of the D_i is not looked at before
n pairs are complete.

**What the cost means.** In the records, a 120-month duel takes about 2.1 h of wall clock with two
duels side by side: the I0087 pair started at 10:20 and the next one at 12:31. Arm BB can start
minutes after arm P, as soon as P's capture is decided, so a pair takes about 2.1 h. 93 pairs take
about 8 days of one machine. With δ = 20% it would be 27 pairs, about 2.4 days. The margin stays
at 10% unless the owner changes it. Such a change would be recorded here with its date, before any
pair is run.

## Run limits

- **Duration:** 120 game months per arm. At most two duels run at a time, which is one pair. This
  follows the rule `project.a-tuning-claim-needs-held-start-pairs` and knowledge
  `duels-run-side-by-side-are-noisy`.
- **Ownership and stopping (rule `project.long-runs-are-owned-bounded-and-stoppable`):** no driver
  that runs the pairs unattended exists, and none was added: a second runner is out of scope. Until
  one exists, every pair is started by a session under a current instruction. Before each pair,
  that session checks the host's floors, the thresholds of battle.json `resource_floor`:
  - load per core ≤ 1.0;
  - free disk ≥ 35 GiB;
  - used swap ≤ 16 GiB;
  - no `~/ttd-battle/PAUSED*` on the host and no `~/ttd-battle/STOP`.

  If any check fails, no new pair starts, and the session records which check failed. Before the
  experiment may run unattended, its driver must write an owner file on every host it uses, carry
  the identity in the environment of the processes it starts, read the floors before each pair,
  and honour PAUSED and STOP, as the rule requires. Killing processes is never the stop.
- **The owner's order of 2026-11-22:** while only one game may run on the mesh, no pair runs. That
  is the situation today.
- **Stopping criteria:**
  1. *Stop the experiment* (no new pair) on any H1 violation, on any operator-safety violation, on
     a STOP file, or on an owner order.
  2. *Void a pair* when:
     - its capture is not at one tick;
     - battlebrain live refuses on either capture;
     - `pair-check` disagrees;
     - the stack fails or months are missing;
     - battlebrain answers HOLD.

     A void pair is recorded with its reason and replaced by the next seed of the list. If void
     pairs exceed 20% of the attempted pairs, stop: the harness is then the finding.
  3. There is no interim look at the efficacy of H2. The only adjustment is the re-estimation of
     n, which can only raise it.

## Validity

**Observer effects:**

- The capture reads about 13 `openttd://` resources through the panel while the game is paused.
  It issues no command.
- The capture lengthens the hold, because mcp-plan with track-planning takes minutes. The panel's
  hold gives up after 1800 s and unpauses the game. A capture that runs past that limit spans
  ticks, battlebrain live refuses it, and the pair is void, so it fails closed.
- Arm BB's company A differs from arm P's only by the directive's settings.

**Threats to validity:**

- *The arm is a whole policy.* Arm BB is "battlebrain's one line, with `planned_freight_only=1`".
  That turns off own lines, the steward's reinvestment in freight and the autopilot. Arm P is
  "the planner's sixteen slots plus the bot's own lines". So H2 compares two policies, not two line
  picks. A third arm, P1 (the planner's top line alone, with `planned_freight_only=1`), would
  separate the two effects. It is not part of H2 and is not pre-registered as a test.
- *One decision only.* Stage 1 decides once, at the start (see above).
- *Order.* Arm P always starts first, because arm BB needs its capture. The held start and the
  pair-check mitigate this, but they do not remove it.
- *Load.* Two duels run side by side. Mitigations: the held start, the load floor, and at most
  one pair at a time.
- *Instrument and noise source differ.* The duel is two companies over ten years. The σ comes from
  the five-AI battle over 20 years. The re-estimation covers this case.
- *A single rival type.* B is the default PrismaticAI only. The result says nothing about other
  rivals.
- *Revision drift.* All pairs must run at one commit of PrismaticAI, the panel, mcp-plan and
  battlebrain. A change in any of them means a new experiment.
- *The directive format.* On `3dbdba818d`, battlebrain's directive sets only `fr_*`. That leaves
  `fr2_*` to `fr16_*` to the planner, and `mcp-duel --directive` refuses it. Arm BB can run only
  once battlebrain's directive clears every other slot. That is the executor binding (I0117), in
  flight on `feature/battlebrain-executor-binding`.

**What would refute each hypothesis:**

- *H1* is refuted by one BB arm, or one recorded shadow or live result, whose selected line is not
  `track: found`, or whose investment exceeds the budget computed at its tick. It is also refuted by
  a bot started with settings other than the directive's.
- *H2.* Let L and U be the one-sided 95% lower and upper bounds of the mean D over n valid pairs
  (t with n − 1 degrees of freedom):
  - L > −δ: H2 holds within the margin.
  - U < 0: H2 as stated is refuted, because battlebrain ends significantly lower. If L > −δ holds
    as well, the result is reported as "lower, but by less than the margin". It is not rounded up.
  - Otherwise the result is inconclusive.

## Separate checks

Each check has its own pass/fail. One check never stands in for another.

| Check | What it measures | Pass | Fail | Status |
|---|---|---|---|---|
| **C1 correctness, offline** | the recorded shadow results of freight-margin@2, reproduced by `battlebrain shadow` on their captures (comparison block equal), and H1 on them (`battlebrain_regression.rs`) | every current-policy result reproduces and its selected line is priced and affordable | any difference, or a priced/affordable violation | **PASS**, 3 of 3 (after re-recording one; see [Result](#result)) |
| **C2 correctness, per pair (H1)** | each BB arm's decision and the settings its bot started with | 0 violations over all pairs | ≥ 1 violation (and the experiment stops) | NOT RUN (owner's one-game order) |
| **C3 performance** | `battlebrain live` wall time per decision; capture wall time and its tick window; track-planning answers other than `found`/`unreachable` (kernel errors, including the 120 s `KERNEL_TIMEOUT` of `hub_algoglyph.rs`) | every decision ≤ 10 s; every capture at one tick inside the 1800 s hold; ≤ 5% of priced candidates pooled end in a kernel error or timeout | otherwise | NOT RUN (owner's one-game order) |
| **C4 strategy (H2)** | mean D over n = 93 valid pairs; last-place rate per arm | L > −δ (see the refutation rule) | U < 0 | NOT RUN (owner's one-game order) |
| **C5 operator safety** | that no decision reaches any game but the duel's own | no `battlebrain serve`, no `/api/decision/pull`, no `/api/approve` from the experiment; every duel panel on an ephemeral port, never :8800; the co-play game untouched | any one of these | NOT RUN (owner's one-game order) |

## Validation performed

| Command | Exit status | Notes |
|---|---|---|
| `cargo test --offline --release --bin battlebrain` (`tools/research/mcp-rhai`), after merging origin/master `194552a5fa` | 0 | 56 passed, including the three C1 tests of `battlebrain_regression.rs` and the authority boundaries of Phase 07. The first run, before the re-recording, failed C1 on the 1953-02-09 result (see C1 below) |
| `python3 -B -m unittest tools/research/test_mcp_duel.py` | 0 | 36 tests, 1 skipped (DuckDB absent); 9 new pair tests, none starts a stack. On origin/master the file did not load: since `6ec3577d7b` `bus_sinks.rs` has no `TABLES` string. It now reads `sql/tables/*.sql` |
| `python3 -B -m unittest tools/research/test_ai_battle.py tools/research/test_ai_battle_power.py` | 0 | 64 + 5 tests |
| `tools/surfaces/surfaces sync` and `check` | 0 | 0 findings |
| `python3 tools/research/ai-battle/battle.py power` | 0 | the numbers in [Sample size](#sample-size-derived-not-guessed) |
| any duel, capture or `battlebrain live` on a game | NOT RUN | the owner's one-game order of 2026-11-22 |

## Result

**C1: PASS** (offline), after one finding. Its first run failed on
`battlebrain-shadow-20261122T084607Z-coplay-8800-freight-margin-2`: the recorded `reason` named
`freight-margin@1`, a label written before `fcac673236` made the reason name the running policy.
The same shadow command on the same capture re-recorded the result. Its lines, budget, divergence
and selection are unchanged; only the reason text and, through it, the decision record's id
changed (`provenance.rerecorded` says so). After that, all three freight-margin@2 results
reproduce, and each selected line is priced and affordable. The freight-margin@1 result is skipped
by name, because this code cannot reproduce it.

**C2 to C5: NOT RUN.** That covers every strategy, performance, per-pair correctness and safety
check. The owner ordered on 2026-11-22 that only one game runs, and that game belongs to another
session.

## Known limitations

- No driver yet runs 93 pairs unattended within the rule's owner, floor and stop requirements.
- Arm BB needs battlebrain's directive to clear every other slot (I0117).
- The sample size rests on the battle's σ. The duel's own σ is measured only by the re-estimation
  in the run itself.

## Status

`NOT RUN`, for the strategy comparison (C2 to C5). The pre-registration and the harness are
complete. The offline regression gate C1 is `PASS`.
