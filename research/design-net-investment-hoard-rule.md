# Design: a net-investment hoard rule for the fitness check

RESEARCH-ONLY. A design for review; nothing here is implemented yet. Owner of the work item: session
openttd-7d (R1b in `research/analysis-2026-11-23-why-prismatic-loses.md`). Reviewer: openttd-36,
whose conditions are listed under "What it must prove".

## The problem

`hoarding()` in `tools/research/ai-battle/battle.py` (#89, #244, #249) fails a contestant for one
window of 8 quarterly samples in which all of the following hold:

- its fleet and its stations stay flat (at most 10% above the window's start);
- its money net of the loan at least doubles;
- that money ends at 500k or more.

That catches a company that stops. It does not catch one that grows slowly while it banks most of
what it earns. Game 10487, run on to 2131, ended with PrismaticAI holding 185.7M in cash and running
40 trains, while AAAHogEx was worth 4.87B with 1,085 vehicles (analysis §1). AAAHogEx reinvested
98-100% of what it earned. Any window in which our fleet grew by more than 10%, or our stations did,
passes the rule today, whatever share of the profit was kept.

## The measure: the share of operating profit that was not kept

The referee logs per company and quarter (`REF Q`): `money` (the bank balance, loan included),
`loan`, `income` and `expenses` of the last completed quarter.

- In OpenTTD, `expenses` collects only the running costs (train, road vehicle, aircraft, ship),
  property maintenance and loan interest (`EXPENSESTYPES_EXPENSES`, `src/company_cmd.cpp:306:EXPENSESTYPES_EXPENSES`).
- `income` collects only the income types.
- Construction, new vehicles and their purchase go into neither.

So a quarter's operating profit is `P = income + expenses` (`expenses` is negative). Over a window
of quarters, net money changes by the profit less what was spent on everything that is not running
the company:

    Δ(money − loan) = ΣP − I,   so   I = ΣP − Δ(money − loan)

A repayment or a new loan moves `money` and `loan` together, and so does not move `money − loan`.
The **reinvested share** of the window is

    r = I / ΣP = 1 − Δ(money − loan) / ΣP

`r` is close to 1 for a company that spends what it earns. AAAHogEx's 98-100% is this quantity.
`r` is close to 0 for one that banks it, and can be negative for one that sells its vehicles.

**One definition, two readers.** This is the live capital watch's measure, not a second one.
`capital_watch.rhai` (#278) computes the same identity over the panel's `company_month`: operating
cash flow is income plus expenses, implied capex is that flow minus the net kept, and reinvestment
is capex over the flow. It uses the materiality floor `MIN_OCF` (10,000): a quarter whose operating
cash flow is smaller is unknown, not 0 % or 100 %. The definition lives in one place,
`research/mechanisms/rhai-scripting.md` "The capital watch". The fitness rule cites it and takes
the same floor, so the gate and the live watch cannot diverge. The capital watch measured game 12609
at 62 %, 44 % and then 12 % for PrismaticAI, against AAAHogEx's 98-100 %.

Why not use company value? `CalculateCompanyAssetValue` (`src/economy.cpp:118:CalculateCompanyAssetValue`) counts only
station facilities and 1.5 × vehicle value. Track, depots, signals and terraforming are not
assets, so a rail-heavy company that invests heavily would look as if it threw money away.
openttd-36's proxy, (cash − loan) / trailing-year income plus vehicle value, has the same blind spot
for track, and as a stock ratio it also punishes an old company for money it banked decades ago.
`r` is a flow over the window and judges only what that window did.

## The proposed rule

A contestant hoards when some window of `hoard_window` samples (default 8) has all three, over
the window's known quarters only (operating cash flow at least `MIN_OCF`, as in the capital watch):

1. `ΣP ≥ invest_profit_floor`: enough profit for the share to mean something (proposed default
   200k over the window; to be calibrated);
2. `r < invest_min_share`: it kept more than it spent (default to be calibrated, see below);
3. `money − loan ≥ hoard_floor` at the window's end (the existing 500k): a small company keeping a
   reserve is not hoarding.

It reports as `hoards:<t0>..<t1>:kept=<1−r>@<ΣP>`, beside today's form.

- **The early years get a short window.** In 1950-52 the loan policy can freeze construction
  (the frozen-days gate, #276), and 8 quarters reacts slowly there. In a contestant's first 3 years
  a window of 4 known quarters is judged, with a stricter `invest_min_share_early`.
- The existing `hoarding()` stays as it is: it catches the company that stops.
- `must_not_hoard` in `battle.json` decides which contestants are held to it, as today.
- A window whose samples lack `loan` or `expenses` is not judged. It does not count them as 0:
  a missing loan makes a repayment look like investment.

## What it must prove (openttd-36's conditions)

- **Refuses master's live trace.** Game 10487 (`growth-on-plan own_lines=4`, 185.7M in cash and 40
  trains in 2131), and games 12609 and 15805 in the windows the analysis names. Each needs `REF Q`
  samples from a replay of its saves with today's referee (the save copied, a scratch HOME, the null
  video driver). None exist yet.
- **Passes healthy traces.**
  - AAAHogEx in every live game and arena match: by construction it reinvests almost everything.
  - Our best fitness seed: the self-steward contestant, which once read as a hoard while it built
    a rail line on the whole loan (the case #244 fixed). `r` must stay high there, because
    `money − loan` falls while it builds.
- **A threshold from the distributions,** not a guess: `r` per contestant per window over
  - the live games 12609 and 15805 (replayed),
  - the arena,
  - headless fitness runs on several seeds.

## What the data can and cannot give today

- **The arena logs carry no loan.** All 11,207 `match.log` files under `~/ttd-battle` on this host
  come from a referee older than `loan=` (2026-11-23). Their `money` includes borrowed money, so `r`
  would read every repayment as investment.
- **The store drops the loan.** `SAMPLE_COLS` in `battle.py` has `expenses` but no `loan`, so every
  match the orchestrator stores in `battle.sqlite` reads loan 0, even from a referee that logs it.
  The rules judge the in-memory series before it is stored (nothing reads `samples` back), so
  today's verdicts are right. Every offline analysis and calibration from the store is wrong.
- **The arena logs name companies, not AIs.** The lineup lives with the orchestrator (the GA host).
  A distribution per AI needs that mapping.

## Order of work

1. Add `loan` to `SAMPLE_COLS`, with a migration that adds the column to existing stores, and a test
   that a stored sample keeps its loan. This is a fix on its own.
2. The arena referee (`~/ttd-battle/arena/game/BattleReferee.tar`) follows the repository's referee,
   so new matches log `loan=`. The arena runs on the remote orchestrator (dispatched over SSH from
   another host on the LAN), so this is its owner's call, asked through the owner.
3. Replay 10487, 12609 and 15805 headless with today's referee, and run the fitness lineup on 5
   seeds. Tabulate `r` per contestant and window.
4. Pick `invest_min_share` and `invest_profit_floor` from that table. Both falsifiers must hold:
   every named live window fails, and every AAAHogEx window and the best seed pass.
5. Implement `reinvesting()` beside `hoarding()`, with tests over the recorded traces, and record
   the thresholds and their source in `research/ai-battle/README.md`.

## Review (openttd-36, 2026-11-25)

- The identity replaces the reviewer's stock proxy, which is dropped. It is one definition with the
  capital watch (above).
- 8 quarters is right for the verdict, with the materiality floor and the short window for the
  first years (above).
- Vehicle sales count. Sale proceeds raise money and are not expenses, so the identity reads
  disinvestment (a line closed, an unprofitable vehicle sold) as money not put to work. An
  autorenew sale and purchase in one quarter nets out by itself, so no exception is needed.
- The loan column is a separate PR, with a test that a stored sample keeps its loan.
- Calibration proves the rule both ways. It must fail the live traces (12609 1952-54, the 10487
  hoard) and pass AAAHogEx (loan near its maximum, cash 6-108k) and our best headless seed.

## Calibration (measured 2026-11-25)

**Source.** No save of 12609 or 10487 exists on the owner's Mac. The only save of 15805 is from
its end, and a loaded live save carries no Game Script, so a replay cannot log `REF Q`. The
quarters come instead from a read-only clone of the live panel store
(`~/.local/state/prismatic-mcp/store/panel-8800.duckdb`):

- the `company_month` view: month-end money, loan, net, vehicles and value of every company;
- the `rhai.shared` records `replan.0`: the finished quarters' income and expenses.

Each quarter is anchored at the date inside its record, not at the time it was published. Checked
against month-end company value, 2,700 of 2,700 quarters match within 5 %.

The measure reproduces the analysis:

| Game, company | Yearly r, 1952 / 1953 / 1954 | The analysis gave |
|---|---|---|
| 12609, PrismaticAI | 61.9 / 43.6 / 12.4 % | the same |
| 12609, AAAHogEx | 98.0 / 97.2 / 99.5 % | 97.8 / 97.7 / 99.5 % |

**r alone does not separate the falsifiers.** A window is judged when its net at the end is at
least 500k and its ΣP at least 200k.

| Game, company | Judged 8-quarter windows | r min | r median | Where r falls |
|---|---|---|---|---|
| 12609 PrismaticAI | 1 of 11 | 0.303 | — | 1952Q4..1954Q3: ΣP 1.08M, net 552k, 32 vehicles |
| 10487 PrismaticAI | 723 | −0.59 | 0.041 | r 0.345 at 1955Q3, at most 0.16 from 1957 |
| 10487 AAAHogEx | 712 | −0.08 | 0.084 | at least 0.8 until 1958Q3, then 0.47 / 0.20 / 0.09 by 1961, with about 600 vehicles and 5-17M net |
| 21647 AAAHogEx | 124 | 0.042 | 0.151 | 0.42 by 1961Q1, about 600 vehicles |
| 21647 PrismaticAI | 128 | −0.06 | 0.034 | 0.36 at 1955Q4 |

Every share threshold that fails PrismaticAI's 10487 hoard also fails AAAHogEx in 756-826 of its
935 live windows. It does so from about 1959, once its fleet has stopped growing at 500-600
vehicles and it banks 10M or more. A leader that has saturated the map banks money as well; r
cannot tell its kind of banking from ours.

- In the first 3 years no 4-quarter window is judged: every company still carries its loan, so its
  net is below 500k.
- Without the net floor, PrismaticAI's early r is at least 0.58 (12609) and AAAHogEx's at least
  0.86.
- r does not see the 1.19M of loan repayments in 12609 in 1950-53, because a repayment does not
  move money − loan.

## The owner's decision: r with a leader-relative gate (2026-11-25)

Asked through AskUserQuestion by session 9ae82e7a-b0e4-44c0-a305-9c69cd1acf6a (openttd-7d).

- **Question, verbatim:** "The hoard-rule calibration on the live panel store (games 12609, 10487,
  15805, 21647) found that no threshold on the reinvested share r separates the falsifiers. Every
  threshold that fails PrismaticAI's 10487 hoard also fails AAAHogEx once its fleet saturates
  (about 1959, 500-600 vehicles, 10M+ banked). Up to 1957 a threshold of 0.35-0.5 does separate
  them. How should the rule continue?"
- **Answer, verbatim:** "r + leader-relative gate (Recommended)".
- **The option read:** "Judge r only while the company trails the leader: fail when r < threshold
  and value/leader < K. A saturated leader that banks is not failed, a trailing company that banks
  is. Calibrate K and the threshold on the fitness seeds next."

**On the live data the gate separates them.** Here value/leader is the company's value over the
highest value at the window's end month.

| invest_min_share, K | AAAHogEx fails | PrismaticAI fails (first window) |
|---|---|---|
| 0.35-0.5, K < 0.5 | 0 of 855 judged windows | 10487 from 1955Q3, 21647 from 1957Q2; 12609 does not fail (its ratio is above 0.5) |
| 0.35-0.5, K < 0.7 | 0 of 855 | 12609 1954Q3, 10487 from 1955Q3, 21647 from 1957Q2, 31337 3 of 3 |

The provisional choice is `invest_min_share = 0.4` with `invest_leader_ratio = 0.7`, the smallest
gate that fails all three live traces. Two caveats come with it:

- **AAAHogEx passes by construction,** because it is always the leader. The falsifier "a healthy
  leader passes" holds trivially here. The fitness seeds, where the leader is not always the same
  AI, are the real test.
- **Other rivals fail too** (LuDiAI, CivilAI and others, from about 1969). That is right: they bank
  while they trail. Only the `must_not_hoard` contestants are held to the rule.

## Still to do before implementing

1. Run the fitness lineup on 5 seeds (930967890, the gate's seed, plus 15805, 2927, 31337 and
   4242). These test the falsifier "our best seed passes" and the gate where the leader changes.
   The package is staged on lundra, waiting for a build slot here.
2. Fix both thresholds from those runs together with the live table above, then implement
   `reinvesting()` beside `hoarding()`, with tests over the recorded windows.
