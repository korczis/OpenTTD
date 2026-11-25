# Battle brain and live tuner

**RESEARCH-ONLY.** Self-improvement in two loops, both fed from the LAN battle's recorded data:

1. **Between rounds (the brain):** a learned model of the battle proposes the next settings to try.
   It is an ensemble of small neural networks (Axon) compiled by EXLA (Nx's XLA backend). They are
   trained on every recorded PrismaticAI result: the settings it ran with, mapped to its log value
   share of the winner. The model scores tens of thousands of candidates from the genetic
   algorithm's own gene space in one batched call. It proposes the best by upper confidence bound,
   UCB = mean + κ · spread of the ensemble. With little data the spread dominates, so the brain
   explores; with more data the mean dominates. The proposals are input for the battle's genetic
   algorithm, which adds them as offspring: evolution guided by a model as well as by mutation.
2. **During a game (the live tuner):** `rhai/prismatic_tune.rhai` tunes the owner's live PrismaticAI
   every 10 minutes. While the company trails the leader and its money is idle, it raises the
   growth knobs. Otherwise it follows the genetic algorithm's best measured variant (`/api/tuning`)
   and the brain's safe proposal. Every change goes through `openttd.ai.set_setting`,
   approval-gated like every MCP change.

Neither loop changes a default or a contestant by itself. The brain's proposals become contestants
only when the battle's owner (the genetic algorithm, ADR 0036) takes them. The tuner's changes wait
for the owner's Approve, or for auto-approve when the owner switches it on.

```mermaid
flowchart LR
  store["battle.sqlite<br/>(battle.py)"] --> pm["battle-postmortem<br/>analytics.duckdb"]
  pm --> brain["battle-brain<br/>Axon ensemble on EXLA"]
  tuning["/api/tuning<br/>GA gene space + fitness"] --> brain
  brain --> json["brain.json<br/>UCB proposals"]
  json --> ga["battle GA<br/>next generation"]
  ga --> store
  tuning --> tune["prismatic_tune.rhai<br/>panel service, every 10 min"]
  json -.->|"$TUNE_BRAIN_URL"| tune
  comps["openttd://companies<br/>gap to the leader"] --> tune
  tune -->|"set_bot: approval-gated"| live["live PrismaticAI<br/>(mcp-e2e game)"]
```

## Run

```bash
tools/research/battle-brain/brain                # once: writes ~/ttd-battle/results/brain.json
tools/research/battle-brain/brain --every 900    # the background loop (every 15 min)
```

And on the panel's Scripts page, start `prismatic_tune` as a service, or:

```bash
curl -X POST 127.0.0.1:8800/api/services/start -H 'content-type: application/json' \
  -H 'origin: http://127.0.0.1:8800' -d '{"name":"prismatic_tune"}'
```

The brain needs the duckdb CLI, the battle API on `:8765`, and Elixir with Nx, EXLA, Axon, Polaris
and Jason compiled. By default it uses the prismatic-platform checkout's `_build/dev/lib` with its
Elixir 1.19 / Erlang 28 (`$BRAIN_LIBS`, `ASDF_*_VERSION`), so XLA is not downloaded again.

The runner does not depend on the post-mortem watch running: before each fit it runs the
post-mortem for any of its editions that has no rows in `analytics.duckdb`. On 2026-11-20 the watch
was down for two hours, ten editions went unanalysed, and the brain read 0 rows and withdrew its
proposals until they were backfilled.

## The brain in detail (`tools/research/battle-brain/brain.exs`)

| Step | What |
|---|---|
| Data | `analyses JOIN contestants` from the post-mortem's `analytics.duckdb`, **paired**: every PrismaticAI variant's result with its settings, against the default `PrismaticAI` on the same edition and seed (`analyses.seed`, backfilled for every edition in the store) |
| Features | each gene of the GA's space scaled to [0, 1]; a gene the setting does not name takes the midpoint and a 0 "named" flag, so the net can tell a default from a value |
| Target | log of (the variant's value share of the winner / the default's on the same seed), clamped below at `$BRAIN_SHARE_FLOOR` (0.05) so that a bankruptcy (value 1, a ratio near 1e-7) scores as the worst outcome without dominating the squared loss; `brain.json` names it in `target` and counts `floored_rows`, and `predicted_share` is that ratio |
| Model | 5 members, each 2 × 32 tanh with dropout 0.1, trained by Adam (lr 0.01, 400 steps) on its own bootstrap sample |
| Search | 50,000 candidates on the GA's grid (min, max, step per gene), scored in one batched EXLA call per member: half uniform over the space, half local (one of the 5 best measured settings, with at least `$BRAIN_PARENT_MIN_ROWS` (4) rows each, with 1 to 3 genes moved) |
| Pick | 8 proposals, alternating `safe` (best lower confidence bound, mean − κ·spread) and `explore` (best upper confidence bound, mean + κ·spread, among candidates whose lower bound is at least `$BRAIN_EXPLORE_FLOOR` × the default, 0.9 by default), κ = 1, each with its kind, predicted share and spread |

`brain.json` (schema `battle-brain/v1`) records the data it had (`rows`, `distinct_settings`) and the
in-sample R² of the ensemble mean, which is a sanity figure and not a validation. `enough_data` is
false while there are fewer distinct settings than twice the number of genes. Until then the
proposals are exploration, and the record says so.

The first run, on 2026-11-19, had 364 rows but only 8 distinct settings for 17 genes, with an
in-sample R² of 0.19. It trained 5 nets in 15 s and scored 50,000 candidates in 208 ms on EXLA (CPU).
Its proposals are exploration, by design. Each GA generation adds distinct settings, and the
surrogate improves with them.

Tooling language: the repository's rule `project.tooling-language-preference` asks for Rust tools.
The owner asked for EXLA and Axon by name, and both are Elixir libraries, so the learned part is a
single Elixir script behind a thin runner. Its input and output are files (CSV in, JSON out), and
nothing else in the repository depends on Elixir.

### The champion, and why the target is paired

The brain's safe proposal in generation 13, `g13.brainSafe`, carried into the GA as `g14.0` and
`g15.0`, was paired with the default on 20 common seeds over 20 years. It won all 20, with a mean
value 30.9 % higher. Its genes became the bot's defaults (`1f3400b85d`): fewer air routes, and
investment that starts earlier, borrows and grows routes further. A variant with a higher GA
score over 6 to 7 matches, `g15.3`, lost its pairing (0.903). Short GA scores rank noisily, and
paired comparisons on the same seeds decide. The brain's target is therefore the paired ratio
against the default, not the raw share. On the first paired run (2026-11-19) the brain had 607
paired rows with 53 distinct settings for 19 genes, enough data for the first time. Its safe pick
predicted 1.48 × the default, with a spread of 0.09 in log terms.

### Only fresh maps teach it

Before `46dd8814f9`, every battle edition restarted its seed counter. Every GA generation, and so
every training row, therefore replayed the same 6 to 8 maps. On fresh maps the picture changed:
- In paired comparisons the champion was only even with the old default (pre-champion won 4 of
  10 pairs).
- Both safe picks of generation 30, `g30.brainSafe` and `g30.brainSafe1`, each ended last once.

The brain now trains only on the newest editions (`BRAIN_EDITIONS`, default 10, the editions since
seeds include the edition), and `target` says so. The first such run had 195 rows and 26 distinct
settings for 21 genes, so the brain explores again (`enough_data` false). That is honest: it has
not yet seen enough fresh maps.

### Why the explorer has a floor

In edition `69577ebe`, `GA g66.brainExplore` finished last in 1 of 7 matches. Its value share of the
winner was 0.017 and its fleet decayed from 49 to 31 (`fleet_decays`, `low_yield`). Unconstrained UCB
had picked it for its spread (0.47), not its mean: short air routes (`air_max_tiles=200`, `air_towns=10`)
with full load. The explorer now chooses only among candidates whose lower bound is at least 0.9 ×
the default, and falls back to the safe order when none qualifies. `brain.json` records the floor
as `explore_floor`. The first constrained run's explorer had a spread of 0.27.

### Why safe picks stay near measured settings while data is thin

In edition `1cff10c1`, the safe pick `GA g67.brainSafe` finished last in 1 of 3 matches (value
share 0.087, fleet 33). Its settings were far from anything measured: low growth caps
(`invest_road_cap=8`, `invest_air_cap=4`, `air_planes=1`, `max_routes=8`). The ensemble's
lower bound was confident there only because no data contradicted it. While `enough_data` is false,
safe picks now come only from the local candidates, a trust region around the best measured
settings. `brain.json` records `trust_region` and the number of `parents`. On the first such run, the
safe pick predicted 1.19 × the default with a spread of 0.05, down from 1.37 with 0.19.

### When the default changes

The target pairs every variant with the default `PrismaticAI` of its own edition, so a new default
moves the baseline. In `6111dc0fef` the GA's elite became the default (geometric mean 1.12 over the
former default on 40 new maps, 95 % CI 1.03 to 1.21), first played in edition `ad811447`. The
runner rebases rows from editions before `$BRAIN_BASELINE_SINCE` (default `ad8114473cb357a6`) by
1 / `$BRAIN_BASELINE_REBASE` (default 1.12), so old and new rows measure against the same default.
The rebase is a single measured factor, not a per-map one, and `target` in `brain.json` names it.
The first run after the change had 206 rows and 26 distinct settings for 20 genes, so it still
explores. On the next default change, set both variables to the new edition and measured ratio.
A behaviour change that interacts with the genes cannot be rebased by one factor. One example is a
fix that makes levelling cost more, which moves the best `air_level_max`. For such a change, set
`BRAIN_TRAIN_SINCE` to the first edition with it, and older editions are left out entirely.
The first such cut is the airport levelling fix (`f4fcbbfef8`, first edition `2d4e0173`). It also
changed the default (`air_level_max` 10 → 35), so `BRAIN_BASELINE_SINCE` is the same edition.
Until there are `BRAIN_MIN_ROWS` (20) paired rows since the cut, the runner writes no proposals and
moves `brain.json` aside to `brain.withdrawn.json`. While it is absent, the GA takes no brain
offspring.

The second cut is `fad8c1af62` (first edition `406a089b`). It levels airport sites only within
`air_level_step` height steps (default 2, before it about 8). The effect varies too much per seed for a
rebase: from 0.6× to about 10,000×, where a map that went bankrupt now reaches 9.9M. Ten minutes later `2cab9e0314` made
`air_level_step=0` the default (first edition `6590a606`): it levels at any height step until the game
refuses a levelled site, then only one-step sites. That is the third cut. Edition `406a089b` ran only a
few matches.

On 2026-11-20 the GA's `g150.0` became the default (`962699034c`, geometric mean 1.154 over 36 pairs,
CI lower bound 1.046), first edition `121f34cd`. That change touches only settings, so older rows
since the cut are rebased by 1/1.154 (`BRAIN_BASELINE_SINCE=121f34cd`, `BRAIN_BASELINE_REBASE=1.154`)
rather than cut.

The fourth cut is `61b2483618` (first edition `cd02f5e8`): when a hilly map has no flat site for a
double-track terminus, the bot now levels a sloped one instead of dropping the own line. The default's
20-year value moved 0.94× to 2.36× per seed (about +32% geometric), and the effect depends on the
`own_lines` genes, so older editions are cut, not rebased.

An hour later `ce3fcf5c29` (first edition `b7e14fbf`) fixed fleet-growth churn: Invest now keeps
unjudged vehicles below judged ones on a route. The default moved 0.94× to 1.16× per seed, about +7%
geometric. That spread is narrow, so rows since the `cd02f5e8` cut are rebased by 1/1.07
(`BRAIN_BASELINE_SINCE=b7e14fbf`, `BRAIN_BASELINE_REBASE=1.07`) rather than cut again.

The fifth cut is `05aa9a18c3` (first edition `5e5b9f93`): PickRailEngines now requires
`AIEngine.HasPowerOnRail`. It used to pick electric locomotives by `CanRunOnRail`, a plain depot
refused them, and finished lines stayed without trains (the restore loop failed up to 774 times a
game). The peer found it by replaying `g288.brainSafe`'s last place exactly (`ablate --match
20a48c81/m-0009`): its genes went from 1.02M to 1.89M. The default moved -5.7% to +27.5% over 7
real-lineup games (about +1.3% geometric), so the mean shift is small, but the effect falls only on
maps and genes that pick electric engines. Older rows understate exactly those rail genes, so they are
cut, not rebased. The rebase (`BRAIN_BASELINE_SINCE=b7e14fbf`) no longer touches any kept row.

Then `531922be7d` (first edition `000b3198`) made the own freight planner skip industries on water.
Oil rigs ranked first by output, every attempt failed for want of a rail station site, and each failure
counted against `own_lines` and `own_fail_pause`. Over 8 replayed matches (`ablate --match`, real
lineups, editions `5e5b9f93` to `799899b1`) the change moved value 0% to +9.5%, +5.3% geometric, with
no loss. The spread is narrow and no configuration lost, so rows since the `5e5b9f93` cut are rebased
by 1/1.053 (`BRAIN_BASELINE_SINCE=000b3198`, `BRAIN_BASELINE_REBASE=1.053`) rather than cut.

`8c8d025388` (first edition `ee8e6884`) keeps the reserve for two airports and a plane while an
airport site is levelled: for a site estimated above 30,000, the loan is paid down during the levelling
so that CmdLevelLand stops before the reserve. It turned the year-one bust of `g304.brainExplore`
(`512c95be/m-0052`) into 2.9M, and the 8 oil-rig replays came out byte-identical. It acts only on rare
steep-site busts, which the brain's target already clamps at `BRAIN_SHARE_FLOOR`, so the brain needs
neither a cut nor a rebase for it.

No arena edition ran between `ac8bc71d06` (#46, rich growth with its air lifts, default on) and
`c560f5565f` (#68, `rich_air=0`): the tournament was paused from 06:02 to past both merges on
2026-11-22. #46 as merged replayed at -29.2% against `fced659668` (8 matches); with #68, master
`c5bf32182a` replays at -1.8% geometric against `c86fbd1ccb`, the last master before #46 (24 matches,
14 up and 10 down, 0.56 to 1.51). That is within replay noise, so #46 plus #68 needs neither a cut nor
a rebase of its own. The rebase pending from `fced659668` (rail_len=4 and TrainsOf, +5.7% over 8
matches against `386b0bf3b5`) still applies once the arena resumes: older rows by 1/1.057, with the
`5e5b9f93` cut kept.

Master `5cc9cdbf8d` then merged three bot changes: #213 (a rail search that cannot find its line
stops within its distance's budget), #163 (ship routes, off by default) and #217 (a road depot's exit
always leads onto the route). Together they replay at +11.8% geometric against `c5bf32182a` over the
same 24 matches (19 up and 5 down, 0.73 to 1.88). #213 alone (`3e4ec154ff`) carries it: +14.4% against
`c5bf32182a` (17 up), and #163 with #217 on top replays at -2.3% (14 up), within noise. A rail search
that ran most of a game year left the bot's money and its other plans idle for that year. That step is large enough to need a rebase of its
own when the arena resumes: rows played before `5cc9cdbf8d` by 1/1.118, on top of the 1/1.057 for rows
before `fced659668`.

Master `6333347512` (#232: a second track is no longer refused for a cost its search only estimated;
two companies no longer build from the same producer; air income logged per aircraft) replays at
-1.0% against `5cc9cdbf8d` (24 matches, 12 up), within noise: no rebase of its own.

### Why trust-region parents need four rows

In edition `eb9cae38` the safe pick `g241.brainSafe` finished last on seed 199435860. The four other
PrismaticAI configurations on that seed, all from the same start slot, came second. Its genes sat next
to `g227.0`, which went bankrupt earlier (`own_lines=24`, `own_fail_pause=0`, `air_level_max=5`). A
parent with only two rows can rank high on one lucky slot, and the trust region then centres on it. Parents
now need at least four rows.

### Start slots and pairing

The start slot alone can swing a configuration's 20-year value about 4× on one map (seed 371319927:
g226.0 scored 11.8M from slot 0 and 2.9M from slot 2). Until `d60734cf23` (first edition `569322cd`) the
lineup rotated the slot with every match, so the configurations sharing a seed each started elsewhere.
Pairs from before then are noisier but not biased: each member sat a fixed offset from the default,
and the absolute slot cycled across seeds. From `569322cd` on, every PrismaticAI configuration on a seed
starts from the same slot, so the brain's join on (edition, seed) compares like with like. No cut was
made; the older rows only add noise, which the ensemble's spread and the bootstrap CI already carry.

### Why a safe pick comes first

Until 2026-11-19 the brain proposed by UCB only, so both of the GA's brain offspring were
explorers. In edition `508356ae`, `GA g10.brain0` finished last in 1 of 6 matches. In match m-0039
its fleet collapsed from 25 to 6 vehicles (`fleet_decays`, `low_yield`, `idle_stations`). Its
settings sat far from the data: `rail_double=1`, which the battle owner had measured as a loss,
`own_lines=22`, and `air_add_waiting=90`, which means almost no planes. Now the first proposal is
the best by lower confidence bound, where the ensemble agrees, and the second is the explorer.
The GA takes the first two, so each generation gets one of each. The GA's last-place penalty
culls an explorer that ends last.

## The live tuner in detail (`tools/research/mcp-panel/rhai/prismatic_tune.rhai`)

The tuner is a runner of a **chain of policy stages**. Each stage is a Rhai module
`rhai/policy/<stage>.rhai` that exports one pure function, `propose(ctx, params)`. It returns
proposals `#{name, value, why, stage}`, vetoes `#{name, veto: true, why}` and notes `#{note}`, and
calls no host function, so any stage can be tested offline. The chain is data: the stage order,
the merge rule and every threshold a stage uses live in `policy/chain.rhai`, never in a stage's
logic. A genetic algorithm, the hub or the owner can change them at run time through the Rhai
shared store, without editing a file.

```mermaid
flowchart LR
  def["policy/chain.rhai<br/>chain(): defaults"] --> chain["chain in force"]
  shared["shared store key<br/>policy.chain"] -->|"merged over, every round"| chain
  env["TUNE_EVERY_S, TUNE_MAX_REQUESTS,<br/>TUNE_IDLE_BUDGETS"] -->|"win over both"| chain
  reads["api_bots, /api/status, openttd://companies,<br/>engines, railtypes, vehicles,<br/>/api/tuning, $TUNE_BRAIN_URL"] --> ctx["ctx<br/>built once a round"]
  chain --> rich["rich"]
  ctx --> rich
  rich --> pressure["pressure"] --> modernize["modernize"] --> ga["ga"] --> brain["brain"]
  brain --> merge["merge<br/>first or priority, veto"]
  merge --> filter["drop undeclared, fit to min/max/step,<br/>skip pending, max_requests"]
  filter -->|"ttd::set_bot: approval-gated"| live["live PrismaticAI"]
```

**A round** runs every `every_s` wall-clock seconds (600 by default), not every game tick:

1. **Chain.** `chain()` from `policy/chain.rhai`. When the shared-store key `policy.chain` holds a
   map, the runner merges it over the defaults: `stages`, `merge`, `max_requests` and `every_s`
   replace the default, `sources` and each stage's `params` merge key by key. The
   environment wins over both. The round's log names the source, for example
   `policy/chain.rhai + shared policy.chain rev 3 by ga`. The runner only reads the key.
2. **ctx**, built once from host reads: `company` (`$TUNE_COMPANY`, else the first running company
   of the panel's AI, as `narrator.rhai` finds it), `bot`, `date`, `companies`, `standing` (value,
   vehicles and money against the leader, the other company of the highest value; `gap` is the
   larger of the value and vehicle ratios), `settings` (every in-game setting of the running bot
   with its `value`, `min`, `max` and `step` from `api_bots()`, so every bound comes from the
   running bot's info.nut), `pending` (the company's pending `openttd.ai.set_setting` requests),
   `tuning` (the GA's `/api/tuning`), `brain` (brain.json from `$TUNE_BRAIN_URL`, loopback only;
   the script host reads no files) and `tech` (for each engine type, the slowest and fastest
   engine the company runs and the fastest it may buy now; the rail types open to it, the ones its
   trains use, and the fastest buildable locomotive for each).
3. **Stages** run in chain order. Each gets `params.common` merged under its own params. It sees
   every earlier proposal in `ctx.proposed`, so it can refine one or veto it. A stage that fails,
   is missing, or has a name that is not letters, digits and `_` is noted, and the chain goes on.
   Stages are resolved by name (`` import `policy/${name}` ``), so a new file in `policy/` can be
   used from the chain without changing the runner.
4. **Merge.** `first`: the earliest stage to propose a setting sets it. `priority` (the default):
   the highest `params.<stage>.priority` sets it, and a tie goes to the later stage. A veto from any
   stage drops the setting for the round.
5. **Requests.** A setting that the running bot does not declare as in-game is dropped, with a log
   line. Each value is fitted to the setting's bounds and step. A setting already at that value, or
   with a request already pending for the company, is skipped (no stacking). At most
   `max_requests` (3) go out through `ttd::set_bot`; each one is said in the game's chat and waits
   for the owner's Approve, or for auto-approve when the owner switches it on. The tuner never
   approves. `$TUNE_DRY=1` logs the proposals and sends none.

**Stages and their params** (defaults in `policy/chain.rhai`; `common` holds `idle_budgets` 3 and
`growth`, each knob's direction, +1 up or −1 down; `policy/lib.rhai` holds the shared pure
helpers):

| Stage | Proposes | Params (default) |
|---|---|---|
| `rich` | Money above `cash_reserve` + `idle_budgets` × `line_budget` is idle: it sets `switches` and moves each of `knobs` 1 step towards growth, plus 1 per doubling of the idle money past `budgets_per_step` budgets | `priority` 1, `budgets_per_step` 10, `steps_max` 3, `switches` {rich_growth: 1}, `knobs` invest_air_cap, invest_road_cap, max_routes, air_routes_max, air_reuse, own_lines, invest_ppv_div |
| `pressure` | More than `min_gap` × behind the leader, with idle money when `require_idle` is set: it sets `switches` and moves `knobs` 1 step, plus 1 per `gap_per_step`-fold of the gap. An earlier, larger move is kept | `priority` 2, `min_gap` 1.2, `gap_per_step` 2, `steps_max` 4, `require_idle` true, `switches` {compete: 1}, `knobs` as `rich` plus rail_twin_max |
| `modernize` | `engine` (modernize on) when a buildable engine is `speed_gain` × faster than the slowest the company runs; `track` (upgrade_track on) when an open rail type that its trains do not use carries a locomotive `speed_gain` × faster than its fastest train | `priority` 3, `speed_gain` 1.2, `engine` {modernize: 1}, `track` {upgrade_track: 1} |
| `ga` | The genes (`genes_only`) of the best variant with at least `min_matches` matches. With `hold`, while a `hold_for` stage proposed this round, a move against `growth` is noted and not proposed | `priority` 0, `min_matches` 4, `genes_only` true, `hold` true, `hold_for` [rich, pressure] |
| `brain` | The first proposal of `kind`, else the first proposal when `any_kind` is set. With `need_data`, a brain without enough data is skipped. `hold` works as in `ga` | `priority` −1, `kind` safe, `any_kind` true, `need_data` false, `hold` true, `hold_for` [rich, pressure] |

Example: a GA retunes the tuner without a file edit (`POST /api/rhai/shared`, or `shared_set` from
any script):

```json
{"key": "policy.chain", "writer": "ga",
 "value": {"stages": ["modernize", "rich", "ga"], "merge": "first", "max_requests": 2,
           "params": {"rich": {"steps_max": 1}, "common": {"idle_budgets": 5}}}}
```

**Hot reload.** The panel restarts a service within a second (`/api/services` `reload_every_ms`)
when its own file or any library it imports changes, a library in a subdirectory included
(`import "policy/lib"`). A template import (`` import `policy/${name}` ``) makes every script under
the directory of its literal prefix a dependency, so an edit of any stage reloads the runner. The
directories come from the tree (`panel_scripts.rs` `scripts_under`, `template_dirs_of`), not from
a list of names; a stage file added to `policy/` is watched from the service's next start. A change
to `policy.chain` takes effect at the next round. The library's parse test,
`every_script_of_the_library_parses`, walks the library recursively, so `policy/` is parsed too.

Verified offline on 2026-11-22: every file parsed (`mcp-rhai --panel http://127.0.0.1:9`), and a
harness ran the runner's pure ctx builder, chain and merge on live GET data (company 0, 15.9× behind
AAAHogEx, 5.2 M idle). It passed 17 checks: settings the running bot does not declare yet
(`rich_growth`, `compete`, `modernize`) are dropped with a log line; a setting with a pending
request is not asked again; values stay on the bot's grid; the GA's counter-growth gene is held; a
shared override merges, and the environment wins over it; `priority`, `first` and veto work; bad
and missing stages are noted; `modernize` fires on the 473 vs 947 km/h aircraft gap.

First run on 2026-11-19: from `PrismaticAI GA g0.1` (score 0.618 over 6 matches) it submitted
`aggressiveness 50 -> 80`, `air_planes 2 -> 1`, `cash_reserve 20000 -> 10000` and
`line_budget 80000 -> 70000`. They showed as pending decisions 1 to 4 in the panel, waiting for the
owner.

## Related

[Battle post-mortem](battle-postmortem.md), [LAN AI battle](ai-battle/README.md), ADR 0036 (the
genetic algorithm), [Rhai scripting](mechanisms/rhai-scripting.md), [Live arena](live-arena.md).
