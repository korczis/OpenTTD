# Config evolution: from a lost game to a tested settings proposal

RESEARCH-ONLY. `battle.py evolve` closes the loop from "PrismaticAI falls behind" to a settings
change that was tested before anyone was asked to approve it. A trigger (the gap watch, the strategy
review, or a live game that was lost) starts it. It attributes the gap to transport modes and derives
candidate settings from the recorded battles. It plays every candidate against the baseline in
paired headless matches against the live game's seven BaNaNaS AIs, and confirms the best one on
fresh seeds. It files the winner as a proposal and never applies it.

The owner's order of 2026-11-23: PrismaticAI ran one train for a decade while AAAHogEx reached 38M.
AlgoGlyph or the genetic algorithm should have detected this. The loss should have been fixed,
documented, and fed back into the bot's configuration, automatically and from data. The detection
and review parts are the gap watch (`gap.<company>`, `tools/research/mcp-panel/rhai/gap_watch.rhai`)
and the strategy review cycle (ADR 0042, `review.<company>.month`). Live outcomes reach the GA
through `research/results/live-*.json` ([LAN AI battle: Live outcomes](ai-battle/README.md#live-outcomes)).
This page covers the step that was missing: turning a detected gap into a measured candidate.

## The loop, once

```
trigger  ->  baseline on the screening seeds  ->  attribution per mode  ->  candidates
         ->  screening pairs  ->  confirmation pairs  ->  proposal (record + blackboard)
```

1. **Trigger**, read-only, one of:
   - `--live research/results/live-<seed>-<run>.json --company C`: the company's configuration is
     judged by `live_entries` (battle.json `tuning.live`). It is lost when its value is under
     `loss_ratio` of AAAHogEx, or under `paired_margin` of the game's best PrismaticAI when one game
     had several. The baseline is the company's start settings.
   - `--panel <url> --company C`: `GET /api/rhai/shared?key=gap.C` fires at `evolve.trigger.gap_level`
     (2: a flag held two months) or above. `review.C.month` fires on any alarm, and its `gaps` (ADR 0042:
     `[{type, volume, yield}]`) replace the headless attribution. The baseline is the running bot's
     settings (`GET /api/bots`), or `--baseline`.
   - `--force --baseline <settings>`: by hand.

   No trigger means nothing runs. The same trigger (kind, source, company, baseline) does not run
   again within `cooldown_hours` unless `--force` is given.
2. **Baseline.** The baseline settings (map slots `fr*_`, `line*_`, `rail_a/b`, `air_a/b` dropped)
   play one null-driver match per screening seed (`evolve.seeds`: 15805, 2927, 31337, 4242). Each
   match runs `evolve.years` (10) years against `evolve.field`, the seven BaNaNaS AIs of the live
   game, with PrismaticAI in `evolve.slot`.
3. **Attribution** (ADR 0042, section 2). The referee logs, for every company, the fleet of each
   mode and its profit last year. For each mode the gap to AAAHogEx splits into a volume term
   `(n_R - n_O) x p_R` and a yield term `n_O x (p_R - p_O)`. The modes that make up `cover` (80%) of
   the positive volume gap are attributed. When no mode has a volume gap, the profit gap is used
   instead. One train against AAAHogEx's dozens is a rail volume gap.
4. **Candidates** (hypotheses, not causes). The pool is the GA's gene space (battle.json
   `tuning.genes`, with bounds from info.nut) restricted to the attributed modes and capital. A
   gene's mode comes from `evolve.gene_modes`, else from the first name token that
   `evolve.mode_tokens` lists (`air_routes_max` air, `max_routes` road, `own_lines` rail), else it
   is capital.
   - Direction and rank come from every recorded battle match (`research/results/battle-*.json`,
     about 15,000 matches). They use Spearman's rho between the gene's value in a PrismaticAI
     configuration and `log(value / AAAHogEx's value)` in the same match, centred per arena edition.
     Each configuration counts once per edition and seed. A gene is directed when |rho| >=
     `min_rho` over >= `min_history` matches and enough distinct values.
   - Up to `levers` single-gene candidates are made. Attributed-mode genes come before capital,
     and directed genes before undirected ones. Each moves `step_frac` of its range in its
     direction; an undirected gene is tried both ways. A further candidate combines every directed
     move.
   - The live interface of the GA (`live_seeds`, `live_veto`) adds and removes candidates. The
     winner of a paired live comparison joins as `live<k>`. A candidate whose genes equal a
     configuration that lost a live game is vetoed and listed under `vetoed`.
5. **Screening.** Every candidate plays the screening seeds. The null-driver match is
   deterministic: the same tree, binary and seed give the same table. So each candidate and the
   baseline on one seed are an exact pair, the headless counterpart of the held start. For each
   pair:
   - `ours` = candidate value / baseline value;
   - `spread` = how far AAAHogEx (unchanged) moved between the two arms;
   - `relative` = the change in our ratio to AAAHogEx.

   A pair favours the candidate only when `ours > 1 + spread`, the match exited 0, and it logged
   no more Squirrel errors than the baseline. A candidate is kept when it has `min_pairs` (3)
   pairs, every pair favours it, and the geometric mean of `ours` reaches `min_gain` (1.05).
6. **Confirmation.** The kept candidate with the best geometric mean `relative` plays baseline
   and candidate again on `evolve.confirm_seeds` (20208, 930967890, 1807087905), which screening
   never saw. It must pass the same judgement there. This limits the selection bias of picking
   the best of several on the same seeds.
7. **Proposal.** The record `<cache>/evolve/evolve-<utc>-c<company>.json` (schema
   `config-evolution/v1`) holds:
   - the trigger and the attribution (headless and from the trigger);
   - the evidence for each gene and every candidate with every pair;
   - the vetoed candidates and the verdict.

   It also holds `binds`: the company, the source game, the commit, the baseline and its SHA-256,
   the field, the slot, the years and the seeds. It carries `expires_at` (`expiry_hours`) and a
   `proposal_sha` over the binds, the winner and the expiry.

   `status` is one of `proposed`, `inconclusive`, `incomplete` or `not_run`. With
   `--propose <url>`, a `proposed` record goes to the blackboard key `evolve.proposal.<company>`
   (writer `evolve`). Any other verdict goes to `evolve.status.<company>`. With `--write`, every
   seed's rows also become an ordinary ablation record under `research/results/`, rendered like
   any other.

## What the loop never does

- It never applies a setting, never calls `openttd.ai.set_setting(s)` and never approves
  anything. The rules are `project.decision-authority-boundaries` and nothing approves its own
  request. The proposal's `lifecycle` lists `proposed -> requested -> approved|denied|retired ->
  applied -> verified|failed`, and this loop writes only the first state. The owner settles the
  proposal (the countdown design of PR #254 for the owner's own companies). Alternatively, the
  executor ADR 0042 names files it as one `set_settings` approval after checking the binds again:
  the same company, the same baseline SHA, and not past `expires_at`.
- It writes nothing to a live game. A panel is read through `GET /api/rhai/shared` and
  `GET /api/bots`. `--propose` writes one blackboard key, which acts on nothing. The writer name
  `evolve` attributes the write and does not authenticate it.
- It does not change a committed default. A default still changes only through held-start pairs
  over three seeds and a pull request (rule `project.a-tuning-claim-needs-held-start-pairs`).
  Headless determinism stands in for the held start in this screening, but no rule says the two
  are equivalent. A proposal is a candidate for a live setting change.

## Bounded, owned, stoppable

Rule `project.long-runs-are-owned-bounded-and-stoppable`:

- **One at a time.** The lock directory `<cache>/evolve/lock` holds an `OWNER` file: host, pid,
  start, commit, the match marker, and how to stop it. A second `evolve` exits 10 while a live
  process holds the lock. It takes over the lock of a dead one.
- **Every match is marked.** Each match carries `TTD_BATTLE_ORCHESTRATOR=evolve:<host>:<pid>` in
  its environment. A match that outlives its timeout is killed, and so are the others of its
  batch.
- **Every batch is checked first.** Before each batch the loop reads:
  - `<cache>/STOP` and `<cache>/PAUSED*` (any session may create them);
  - the match budget (`evolve.budget`, `--budget`);
  - the wall-clock cap (`evolve.max_minutes`, `--max-minutes`);
  - this machine's load and its free-disk floor (battle.json `resource_floor`; 35 GiB, the same
    floor as the arena).

  A batch is as large as the floor admits. When the loop stops early, the record says why, with
  status `incomplete`.
- **The budget covers confirmation.** Screening keeps two confirmation matches per confirmation
  seed in reserve, so the screening seeds take every candidate and the confirmation still fits.

## Cadence

The loop is a CLI subcommand, not a Rhai service. A panel service runs in the panel's process and
has no way to start or bound thirty OpenTTD processes. ADR 0042 also keeps compute work off the
game host. The trigger runs from a routine on the battle machine (cron, launchd, or a scheduled
Claude routine), for example every few hours:

```sh
python3 tools/research/ai-battle/battle.py evolve --panel http://127.0.0.1:<panel> --company 0 \
  --propose http://127.0.0.1:<panel>
```

A run that finds no trigger, or the same trigger within its cooldown, plays nothing.
`--config` selects the configuration whose arena cache holds the field's AIs (`battle.py fetch`
downloads a configuration's BaNaNaS contestants into its cache).

## Interfaces it builds on

| Interface | Owner | Used as |
|---|---|---|
| `gap.<company>` (`level`, `flags`, `levers`) | gap watch (`feature/gap-watch`) | trigger; `levers` recorded |
| `review.<company>.month` (`alarms`, `gaps`) | strategy review cycle (ADR 0042) | trigger; attribution |
| `live-*.json`, `live_entries`, `live_veto`, `live_seeds`, `live_genes`, `settings_dict`, `live_tuning` | live outcome learning (PR #256) | trigger, veto, live seed candidate, gene space |
| ablation rows' `standings` | this change | AAAHogEx's value and mode mix per arm |
| `evolve.proposal.<company>`, `evolve.status.<company>` | this change | the proposal for the owner or the ADR 0042 executor |
| `GET /api/services/scores?horizon=3` (`service_eval`) | service self-evaluation (`feature/rhai-self-evaluation`) | not wired yet: a further trigger or prior; its verdicts are associative, never proof that a variant is better |

## Validation

`tools/research/test_ai_battle_evolve.py` covers the loop offline. A fake game binary reads its own
start script and prints referee lines. A fake `run` produces every verdict: proposed, inconclusive
in screening, inconclusive in confirmation, incomplete through the budget, STOP or the wall clock,
and not run. A local HTTP server stands in for the panel, for the trigger, `/api/bots` and the
blackboard write. The tests also cover the lock, the OWNER file and taking over a dead owner's
lock; the PAUSED file, the floor and an unreadable load; the live veto and the live seed; and
`ablate_rows`' standings, its slot-aware error count, its owner marker and the match killed at its
timeout.
