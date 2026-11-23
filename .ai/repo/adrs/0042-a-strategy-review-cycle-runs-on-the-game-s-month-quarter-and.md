---
schema: adr/v1
id: adr-0042
kind: adr
title: A strategy review cycle runs on the game's month, quarter and week, judges every company against the leader from recorded data, and turns a lasting gap into a replan request, an approval-gated plan, labelled learning rows and an owner digest
status: proposed
date: 2026-11-23
tags:
  - strategy
  - review
  - planning
  - learning
  - decision-authority
related:
  - file:tools/research/mcp-panel/rhai/replan.rhai
  - file:tools/research/mcp-panel/rhai/watchdog.rhai
  - file:tools/research/mcp-panel/rhai/prismatic_tune.rhai
  - file:tools/research/mcp-rhai/src/calibrate.rs
  - file:tools/research/battle-brain/brain.exs
  - file:research/mechanisms/planner-and-algoglyph.md
  - file:research/mechanisms/approval-gate.md
  - file:.ai/repo/rules/project/decision-authority-boundaries.v1.md
  - file:.ai/repo/project/milestones/the-strategy-is-reviewed-on-a-cadence.yaml
provenance:
  origin: extracted
  derived_from:
    - decision:adr-0034
    - decision:adr-0037
    - decision:adr-0038
    - file:tools/research/mcp-panel/rhai/replan.rhai
    - file:tools/research/mcp-panel/rhai/watchdog.rhai
    - file:research/battle-brain.md
---

# 42. A strategy review cycle runs on the game's month, quarter and week, judges every company against the leader from recorded data, and turns a lasting gap into a replan request, an approval-gated plan, labelled learning rows and an owner digest

## Context

In the live 8-company game of seed 12609 (1953), PrismaticAI's company value is about 0.84M and
AAAHogEx's about 1.58M. About 80% of the profit gap is aircraft. PrismaticAI holds about 372k cash
on a 300k loan. Each of its trains earns about 2.2 times what an AAAHogEx train earns, but it runs
fewer of them.

The owner: "this should never have happened; the planner / AlgoGlyph should have detected it. A
regular data-driven analysis of strategies through the blackboard and the other components every
week/month/quarter should have detected it, replanned, propagated and communicated it, updated the
neural networks and genetic algorithms, updated configs/dynamics/derived/inferred values, and
communicated it to the user."

### Why nothing detected it

Every part such a review needs existed on master. None of them closed the loop:

| Component | What it already sees | Why it did not act on this gap |
|---|---|---|
| `replan.rhai` (monthly) | For each vehicle type, the leader's profit per vehicle a year old and ours (`competition()` in `replan.<company>`) | The competition block only describes. The verdict asks one question: does the planner's new portfolio pay back? It answers with bayes-update and pert-monte-carlo against `MIN_PAYS` 0.5 and `HORIZON_MONTHS` 24, both typed in. The block holds no vehicle count for the leader, so a gap in fleet size cannot be seen. A vehicle type we do not run never becomes a question. The round also holds while any of its proposals waits. |
| The approval core (ADR 0038) and its deadline | Every proposal and its verdict | At the deadline, an unanswered proposal is retired with the verdict `keep`: nothing is enacted, by design. Nothing counted the retired proposals, so a month of held or retired plans looked like a quiet month. |
| `watchdog.rhai` | How long a company goes without building, against a threshold derived from the gaps it has seen | It detects a stall, not under-performance. A company that builds slowly is not stalled. |
| `prismatic_tune.rhai` (stage `policy/pressure.rhai`) | Trailing the leader while money sits idle | It runs on wall-clock rounds and is approval-gated, under the same deadline. No record a review could read says what it changed in this game. |
| The strategist (`strategy.*`) and `mcp-plan` with AlgoGlyph | The best portfolio for now | It recomputes the plan. It never compares the plan, or the company, with what the leader earns. |
| `calibrate.rs` | Realised against predicted profit, per mode | Off by default (`PRISMATIC_PLAN_CALIBRATION`). The three-seed held-start A/B averaged about zero (ADR 0034). |
| Battlebrain (ADR 0037) | The decision authority for the freight line | Not live. Its scope is the freight line, not the mix of vehicle types or capital. |
| The GA (`battle.py`, ADR 0036) and battle-brain | Headless battle results, fitness, a hoard check | Offline only. Live games never became rows. |
| The narrator, `steward.rhai` | Monthly chat; yes/no questions | They narrate events. No summary told the owner that the company was losing, and why. |

Each component answered a local question on a local cadence. No component owned the question "is the
strategy working against the competition, and if not, which lever?". Five more things were missing:

- a cadence longer than a month;
- a threshold relative to the leader;
- a count of the proposals that died unanswered;
- a path from a live outcome to a learner;
- a report of any of this to the owner.

### What the parallel work is adding (2026-11-23)

Since the gap was noticed, parallel sessions have been closing it. Each one runs, or is staged on its
branch, and writes its own key. This ADR is their **one contract**, so that they compose and nothing is
built twice (owner order: extend and integrate before adding).

| Agent | Branch | What it adds | Its key |
|---|---|---|---|
| J | `feature/gap-watch` | `gap_watch.rhai`, once a game month, for every company. It records flags (`losing_ground`, `idle_capital`, `unused_credit`, `stalled`, `plans_not_landing`, `collateral`, `replan_silent`, `unknown`), their streaks and its levers. It files at most one owner-only lever proposal, and only with `GAP_WATCH_PROPOSE=1`. | `gap.<company>`, `gap_watch.owner`, `gap_watch.stop`, `gap_watch.pause` |
| M | `fix/planner-outcome-feedback` | `outcome.rs`: the plan assessed against what the game pays, with a per-mode prior, its signals (`overpredicted_mode_planned`, `outcome_plan_differs`, `leader_out_earns`) and a verdict. It runs on the monthly planned-bot call. | `outcome.<company>` (writer `panel_bot`) |
| O | `feature/strategy-digest` | `digest.rhai`: on each game week, month and quarter, one digest per company. It covers standing, series, results by type, money, what failed, what changed, governance and approvals. It escalates through ok, unknown, watch and alert. | `digest.<company>`, `digest.<company>.<week\|month\|quarter>`, `digest.status`, `digest.governance` |
| R | `feature/rhai-self-evaluation` | `service_eval.rhai`: an action ledger over the services' shared writes, joined to outcomes months later. Its thresholds come from the store (`baselines`). | `eval.<service>`, `eval.poor`, `derived.<service>.<param>` |
| S | `feature/failure-regime-detector` | `regimes.py`: failure regimes (no expansion, a widening gap, losing on a mode, an unapplied plan). Every threshold is relative to the rivals, and the rules live in `regimes.json`. | regime records (`battle.json` `fitness.regimes`) |
| N | `feature/live-outcome-learning` | `battle.py live-export`: a live game becomes `research/results/live-<seed>-*.json`. A GA configuration that lost a live game is vetoed, and `live-start.json` derives the next live start. | result files |
| P | `feature/config-evolution-loop` | `battle.py evolve`: the gap signal leads to candidates, evaluated headless, then proposed and never applied. | `evolve.proposal.<company>` |
| K | `doc/strategy-postmortem` | Why the strategy was bad, and a fitness safeguard on the ratio to AAAHogEx | `battle.py` fitness |
| H | `doc/live-game-12609-economics` | The P&L of game 12609: the expected answer for a replay | research record |
| I | `fix/prismatic-capital-allocator` | The in-bot capital allocator | bot settings |

Two breaks between these are already visible:

- O's digest reads idle cash from `gap_watch.<company>`, but J writes `gap.<company>`.
- O's `SIGNALS` names the prefix `planner_mismatch`, but M writes `outcome.<company>`.

The contract below exists to stop such breaks.

### Neural networks: what exists

The repository has exactly one neural network: **battle-brain**.

- It is an ensemble of five small multilayer perceptrons (2 × 32 tanh, Axon on EXLA), in
  `tools/research/battle-brain/brain.exs` (see `research/battle-brain.md`).
- It trains offline on the LAN battle's recorded PrismaticAI results (`battle.sqlite`).
- It proposes GA offspring by upper confidence bound. The strategist reads its `brain.json`.
- No live game result trains it, and nothing checks its prediction against a live outcome.

Prismatic's `NeuralEngine` cannot be reached from this fork (`research/rhai-ide.md`: a hardcoded
fallback). It stays outside, because prismatic-platform is used from outside only. This ADR adds no
neural network.

## Decision

### 1. The cycle is the existing services, composed; no review service is added

| Cadence (game time) | What judges | What communicates |
|---|---|---|
| week | `watchdog.rhai` (stalls); `rail_monitor` and `build_audit` (construction) | O's week digest and its health alerts: stalled, bot stopped, approvals, idle cash, signals |
| month | `replan.rhai` (the plan and the competition); J's `gap_watch.rhai` (flags and levers); M's `outcome.<company>` (plan against outcome); R's `service_eval.rhai` (whether each service's actions helped) | O's month digest; J's owner line once a flag has held for `TELL` months |
| quarter | S's regimes over the company series; K's fitness safeguard; N's live export each quarter; P's evolve candidates; battle-brain's check on the live settings | O's quarter digest, which holds performance alerts until `GRACE_QUARTERS` have passed |

Each period comes from the game date read over MCP, never from the wall clock. This follows ADR 0025:
periodic work subscribes to the timer whose meaning it has.

Each service keeps its own loop. The cycle adds:

- a contract (section 2);
- one source for each kind of threshold (section 3);
- a request path (section 4);
- the gaps, as issues.

### 2. The blackboard contract: existing keys, one owner each, one envelope

These keys **are** the strategy blackboard. A reader reads only these, and a writer writes only its
own:

| Key | Owner (writer) | Cadence | Readers |
|---|---|---|---|
| `replan.<company>` | `replan.rhai` (openttd-51) | month | gap_watch, digest, watchdog |
| `planner.<company>` | the owner, or the Battlebrain cutover | on change | every freight-slot writer (`fr_author`) |
| `watchdog.<company>`, `watchdog.tuning` | `watchdog.rhai` (openttd-51) | round | gap_watch, digest |
| `gap.<company>`, `gap_watch.owner\|stop\|pause` | `gap_watch.rhai` (J) | month | digest, replan, service_eval |
| `outcome.<company>` | `panel_bot` through `outcome.rs` (M) | month (with replan's planned-bot call) | gap_watch, digest |
| `eval.<service>`, `eval.poor`, `derived.<service>.<param>` | `service_eval.rhai` (R) | month | the digest; each service reads its own `derived.*` |
| `evolve.proposal.<company>` | `battle.py evolve` (P) | when a candidate has been evaluated | digest |
| `digest.<company>`, `digest.<company>.<period>`, `digest.status`, `digest.governance` | `digest.rhai` (O) | week, month, quarter | the panel's digest page, the owner |
| `strategy.*` | the strategist | month | pages, `strategy_lib.rhai` |

Three sources outside the blackboard complete it:

- **Regime rules:** `tools/research/ai-battle/regimes.json` (S).
- **Live outcomes:** `research/results/live-*.json` and `live-start.json` (N).
- **Proposal history:** the approval core's decision events (openttd-2e), from the bus and the store.

**One envelope.** Every key carries the same fields:

- `company`;
- `period` (a game month, quarter or week) and `game_date`;
- `writer` and `version`;
- `reads`: the keys, resources and store queries it was computed from, with row counts;
- `unknown`: inputs that were missing or stale. A missing input is `unknown`, never "ok" and never
  zero.

J and O already write this envelope, and M stamps the seed, date, tick and plan seq.

The table is declared once, in code, and its documentation is derived from it (rule
`project.mcp-surfaces-are-derived`). A test reads every writer's `shared_set` and every reader's
`shared_get`. It fails on a key outside the table, on a second writer of a key, and on a reader of a
key nobody writes. That test catches the two breaks above (I0270).

**The Majordomus mesh** carries the cycle's records:

- an `observation` for each digest;
- a `hypothesis` for each regime or flag that has held (for example "aircraft volume explains most
  of the gap");
- a `plan` for each proposal or request;
- an `outcome` when R's ledger, or a later review, measures it.

Without a mesh session the cycle announces on the board, and the record says the mesh write was not
run.

### 3. Detection: one source for each kind of threshold

The thresholds are policy choices applied to recorded data; the data does not give them. Each kind of
threshold lives in exactly one place:

- **Regime thresholds: S's `regimes.json`**, with each rule's reason under `why`. The same rules
  judge the headless fitness check and the live game. The panel serves them to Rhai, because Rhai
  reads no files. J's `SHARE_DROP` and flag rules become the live reading of S's regimes (a widening
  gap, losing on a mode, no expansion, an unapplied plan), not a second definition (I0272).
- **Baselines from the store: R's `derived.<service>.<param>`**, from the named query `baselines`.
  Examples are `derived.gap_watch.share_drop` and the watchdog's cold start. A service reads its own
  `derived.*`. It falls back to its constant only as a declared bootstrap, and says so.
- **The owner's risk policy.** This covers replan's `MIN_PAYS` and `HORIZON_MONTHS`, and
  gap_watch's `PERSIST`, `COOLDOWN_MONTHS` and `DECLINED`. Each is declared and versioned, and the
  owner changes it.

S's rules and J's streaks already cover what the independent review asked for:

- A weak, flat company is caught by its level against the rivals. A slope rule alone misses it.
- Trends are Theil-Sen slopes of log ratios.
- Hysteresis has a separate entry and exit. Entry is J's streaks, `TELL` and `PERSIST`. A flag must
  clear before it is said again.
- Only the owner's companies' alarms lead to an action.

**The gap by vehicle type** (I0271, in replan's `competition`) splits into two terms, where n is the
vehicle count and p the profit per vehicle:

- a volume term `(n_leader − n_ours) · p_leader`;
- a yield term `n_ours · (p_leader − p_ours)`.

This attributes the gap; it does not identify an opportunity. A type we do not run is all volume. A
type whose fleet is mostly younger than a year is marked `young` and raises no alarm. In game 12609
the aircraft volume term dominates, the rail yield term is negative, and the rail volume term is
positive.

### 4. Decision authority and the request path (rule `project.decision-authority-boundaries`)

- **No command path.** A blackboard write never changes the game (boundary B3). gap_watch, digest,
  service_eval and the evolve proposal writer join B3's named list (I0282).
- **No freight slot.** Nothing in the cycle names an `fr_*` slot (B2); gap_watch already refuses
  one. The freight line stays Battlebrain's decision once it is live. Until then it is the decision
  of the writer that `planner.<company>` names.

**Who may propose what.** Every proposal goes through the one approval core (ADR 0038).

| Proposal | Proposer | Bounds |
|---|---|---|
| A bounded lever: restore a setting the bot declares toward its declared default or maximum | J's `gap_watch` | Owner-only (`openttd.ai.set_settings` at the game's console). Only with `GAP_WATCH_PROPOSE=1`. One at a time, not again after `DECLINED` refusals, and `COOLDOWN_MONTHS` between proposals. |
| A new plan for the company's non-freight settings, including a focus the review asks for (a mode, or "more of what earns") | `replan.rhai` | One superseding plan. replan reads `gap.<company>.requests` and records the request it answered (I0274). |
| The capital allocator's reserve or line budget | `gap_watch`, as a lever, once agent I declares those settings in `info.nut` | The same as any lever |
| Calibration on or off | The owner | The planner reads a runtime switch (I0277) |
| A GA configuration | The battle owner (ADR 0036) | Headless only. The cycle never applies `evolve.proposal` (P) or N's `live-start.json`. |

**Precedence on one company's settings**, highest first:

1. the owner;
2. Battlebrain, once live, for the freight line;
3. the writer `planner.<company>` names, for the freight slots;
4. replan;
5. gap_watch's levers;
6. the live tuner.

A writer proposes nothing for a setting that a higher writer's pending plan holds.

**The lifecycle of a request:**

1. gap_watch files the request in `gap.<company>.requests`.
2. At its next round, replan accepts or declines it, with a reason.
3. The proposal goes to the approval core.
4. The approval core settles it: approved, denied, or retired by the deadline.
5. R measures its effect, months later.

A request that is neither accepted nor declined within one month escalates in O's digest.

**Model updates are decisions too.** M's per-mode prior changes what the planner proposes, so M keeps
it a proposal and calibration stays off. Turning calibration on is the owner's switch. A default
changes only through held-start pairs over three seeds (ADR 0034 point 6; rule
`project.a-tuning-claim-needs-held-start-pairs`).

### 5. The proposal ledger is counted once

Two counts read the same events:

- J's `plans_not_landing`: proposals filed against proposals executed.
- O's count of the approvals that the deadline settled without the owner.

Both read the approval core's decision events, including `settled_by`, and neither keeps a second
queue (I0275).

### 6. Learning updates

- **The GA (N, with K and P).** N's live export turns each live game into labelled rows (`source:
  live`).
  - Each row holds the configuration, the history of its requested changes, the revision and the
    outcome.
  - A configuration that lost a live game is vetoed as an elite or a parent. The paired live winner
    joins the next generation.
  - One game is one correlated observation, so a live row never changes a default by itself.
    Defaults move only on held-start pairs.
  - K's fitness safeguard judges the ratio to AAAHogEx. S's regimes become fitness failures.
  - The export runs at each game quarter, not only after a game ends (I0278).
- **battle-brain.** Live rows are first a validation set. Each quarter, the brain's predicted share
  for the live configuration and its spread are published beside the observed share (I0279).
  Training on live rows is a later decision, made on that error over several games.
- **The planner and AlgoGlyph (M).** `outcome.<company>` supplies the per-mode prior.
  `calibrate.rs` splits realised against predicted by the line's owner. replan's bayes-update keeps
  drawing its priors from the company's own record.
- **The services themselves (R).** `eval.<service>` says whether a service's past actions helped,
  and `derived.*` moves its baselines.

### 7. Configs and derived values

| Value | Source after this ADR |
|---|---|
| Regime thresholds (live and headless) | `regimes.json` (S) |
| Service baselines (gap_watch's share drop, the watchdog's cold start, ...) | `derived.<service>.<param>` (R), from the store. Constants are declared bootstraps. |
| replan's `MIN_PAYS` and `HORIZON_MONTHS` | Owner policy, published beside the realised payback distribution of the game's own lines. Failed and unfinished investments count as censored (I0283). |
| The planner's per-mode prior and calibration | `outcome.<company>` (M). The switch is the owner's. |
| The next live start settings | `live-start.json` (N). Applied only when the owner starts a game. |
| The allocator's reserve and line budget | A lever of gap_watch, once agent I declares them |

A derived value carries `{value, method, inputs, n, bootstrap, version}`. One that changes a game
setting passes the approval core. One that changes a committed default passes held-start pairs and a
pull request.

### 8. What the owner sees

O's digest is the one channel, and it shows every key of section 2:

- **Week:** the health alerts.
- **Month:**
  - the company's standing;
  - the gap by type, with its volume and yield terms;
  - money and idle cash;
  - what was proposed and what became of it, and the requests still waiting;
  - J's flags and M's verdict.
- **Quarter:**
  - the share series;
  - the regimes;
  - the services R marks `eval.poor`;
  - the GA's live vetoes and evolve candidates;
  - battle-brain's error on the live settings;
  - the thresholds still at their bootstrap values.

J's owner line stays the fast in-game alert: it is said once, when a flag has held. Every number in
the digest links to its key (I0280).

### 9. Failure modes and how each is caught

| Failure | Caught by |
|---|---|
| A service stops writing its key | R's `eval.<service>` and the key's age against its cadence; the digest's "failed" section lists stalled services |
| A reader reads a key nobody writes, or two writers write one key | The contract test (I0270) |
| Proposals die unanswered | The one ledger (I0275), in the digest |
| A request is ignored | Its lifecycle: it escalates after one month |
| A threshold stays at its bootstrap | `bootstrap` on every value; the digest lists those that do |
| Oscillation | J's `PERSIST`, `COOLDOWN_MONTHS` and `DECLINED`; a newer plan supersedes the older one (ADR 0038) |
| A weak but flat company is never flagged | S's level rules against the rivals |
| Overfitting to one game | Live rows never change a default; held-start pairs do |
| A second decider appears | Boundary tests B1 to B3, with the cycle's writers in B3's list |
| The store or the mesh is off | ADR 0034's stated fallback; the signal reads `unknown`; NOT RUN is recorded |
| The cycle misses the real case | The replay of game 12609's store (I0284). It must flag the aircraft gap and the idle capital in the first quarter they were visible, against H's analysis. On the controls (a stable loser already answered, healthy growth, PrismaticAI leading) it must raise no alarm that leads to an action. |

### 10. Consultation

The owner ordered an independent review of the first draft. Only public repository content was sent.

**Codex** (`codex exec --sandbox read-only`) answered with these points:

- The draft named no owner for execution, and no precedence among replan, the tuner, the allocator
  and Battlebrain.
- Its thresholds are policy choices, not data.
- A slope rule misses a company that is weak and flat.
- The decomposition by type is an attribution, not an opportunity.
- The bootstrap ignored serial dependence, and two breaches in a row are persistence, not
  hysteresis.
- Live rows are correlated and need their intervention history.
- The operational contract was missing.
- Start with one monthly review.

The first seven points shaped sections 3 and 4. The peers' work covers "start monthly": the monthly
judges (J, M, R) exist, and O's week and quarter are communication. Codex's advice to defer the GA and
battle-brain work was not adopted, because the owner asked for those loops; they land as validation
and labelled rows only.

The other advisors gave no answer:

- **Gemini** did not run: the CLI refused this account's tier (`IneligibleTierError`).
- **ollama**, the fallback, had no model installed, and the disk was too full to pull one.
- **ChatGPT**, the reasoning plan's other advisor, was not consulted, because the session's
  permission policy refused the outbound API call.

The consultation is recorded with `majordomus reasoning record`.

**Peer review.** openttd-36 reviewed it on 2026-11-23. That review moved this ADR from `review.*`
keys of its own to the keys the services already write, and made S the one source of regime
thresholds. Review by openttd-51 and openttd-2e is pending. This ADR stays `proposed` until they and
the owner have answered.

## Alternatives rejected

- **A `review.*` namespace and a review runner of its own** (the first draft). It would be a second
  source beside `gap.*`, `outcome.*` and `digest.*`, against "extend and integrate before adding".
- **replan as the judge.** replan executes plan settings. A judge that executes cannot report that
  its own proposals went unanswered.
- **Leaving it to Battlebrain.** Battlebrain's authority is the freight line, and it is not live.
- **Fixing it only inside the bot** (agent I alone). An allocator spends better. It cannot see the
  leader, the planner's error or the unanswered proposals, and it tells nobody.
- **Thresholds typed into each service.** That made replan blind, and it would give J, S and R three
  definitions of one regime.
- **Training battle-brain on live rows now.** One game is one correlated observation.

## Consequences

- No new service and no new key namespace are added. The contract is a table in code with a test,
  plus fields on existing keys: `gap.<company>.requests`, and the volume and yield terms in
  `competition`.
- O's digest must read `gap.<company>` and `outcome.<company>`, not `gap_watch.<company>` and
  `planner_mismatch`.
- J's flag thresholds become the live reading of S's `regimes.json`. R's `derived.*` gives the
  baselines. The panel serves `regimes.json` to Rhai.
- The approval core's decision events are the one source of the proposal ledger.
- N's live export runs every quarter, and battle-brain is validated on its rows.
- The milestone `the-strategy-is-reviewed-on-a-cadence` carries the gaps as issues I0270 to I0284.
  Each issue names its owner. The replay of game 12609 (I0284) is the acceptance test of the whole
  cycle.
