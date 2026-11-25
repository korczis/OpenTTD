# AAAHogEx source-code audit against PrismaticAI

**RESEARCH-ONLY.** This audit reads the source of AAAHogEx v115 and traces how it gets from startup to
revenue. Each mechanism is then checked against the LAN AI battle's records and compared with
PrismaticAI at the revision those records ran. Neither AI was executed for this audit, and no
game state was changed.

Every claim carries a classification:
- `OBSERVED`: a battle record shows it.
- `DERIVED`: it follows directly from the code and the recorded configuration.
- `INFERRED`: a plausible reading that no record confirms.
- `UNKNOWN`: no evidence either way.

## 1. Exact source versions

| | AAAHogEx | PrismaticAI |
|---|---|---|
| Package | BaNaNaS `484f4745-AAAHogEx-115.tar`, unique ID `484f4745` (`HOGE`) | `PrismaticAI.tar`, `git archive` of `tools/research/prismatic-ai/PrismaticAI` |
| Version | `GetVersion()` 115, `GetDate()` "2026-03-18", API 14, author Rei Ishibashi (`info.nut`) | git rev `8c8d025388e85261a60027d9983d8588d720a2e9`, not dirty |
| Checksum (sha256) | `40796e8beb127965e3aa9cd1a9683ca689adaefe9c131d849b69def65bba9cb4` | arena tar `d57f9905645cf461100a5e8273d5ce6c87ddb357cfad019ed54e52ed72859f77` |
| Where | `~/ttd-battle/arena/ai/`, `~/ttd-battle/content/content_download/ai/`, `~/Documents/OpenTTD/ai/`. All three copies are byte-identical. | `~/ttd-battle/arena/PROVENANCE.json` |
| Retrieved | already on disk (BaNaNaS download by `battle.py fetch`). Nothing was fetched from the network for this audit. | git |

- **Version attribution.** Every battle record that names AAAHogEx names `484f4745-AAAHogEx-115.tar`.
  That is 350 of 350 result files under `~/ttd-battle/results`, and each record's
  `provenance.ais` list.
- **Content attribution for older editions.** The records hold the tar *name*, not its
  checksum. For the three editions analysed below, the checksum is `OBSERVED` from the arena
  they ran in. For older editions it is `DERIVED`: there was only ever one AAAHogEx tar on
  disk, and a BaNaNaS version is immutable once uploaded.
- **No other AAAHogEx source was consulted.** That includes the upstream repository's current
  branch.

**Runs compared.** These are the three newest editions, all on PrismaticAI rev `8c8d025388`,
prepared 2026-11-21T22:24Z:

| Edition | Record |
|---|---|
| `e8396af19787e711` | `research/results/battle-20261122T035646-e8396af1.*` |
| `fc6acf8cb06f73b1` | `…041253-fc6acf8c.*` |
| `dfcf62a2a44de527` | `…042734-dfcf62a2.*` |

- 115 completed matches, from `~/ttd-battle/results/battle.sqlite`, tables `matches`, `standings`
  and `samples`, restricted to `status='done'`.
- Each match runs 20 years from 1960 on an 8×8 (256²) temperate map, with breakdowns reduced.
- Max loan is 300000. Infrastructure maintenance and inflation are at the engine defaults (off;
  `src/table/settings/economy_settings.ini:131,320`).
- The lineup has five AIs. PrismaticAI's slot rotates among PrismaticAI configurations.

**Evidence limit that shapes every runtime claim below.**
- The match logs (`matches/<edition>/m-*/match.log`) keep only error-level script output.
  The `[N]` in a line is the company index.
- The only error lines in them come from LuDiAI AfterFix ("days wasted", "FindPath return")
  and AdmiralAI ("no staton build").
- **AAAHogEx logs everything through `HgLog.Info/Warning` (`utils.nut:548-559`). PrismaticAI
  uses `AILog.Info`. Neither emits anything at error level, so no internal decision of either
  AI appears in any run log.**
- Runtime evidence is therefore limited to the referee's quarterly lines (`REF Q`: value,
  money, income, vehicles and profit last year per mode, cargo) and to the post-mortems built
  from them.
- Neither the loan nor any route-level event is recorded.

AAAHogEx citations below are paths inside the v115 tar (`main.nut:3895` = `AAAHogEx-115/main.nut`).
PrismaticAI citations are `PrismaticAI/<file>:<line>` at `8c8d025388`.

## 2. Run outcome

Means over completed matches (115 for AAAHogEx; 18 for `PrismaticAI` at default settings; 20
for `PrismaticAI pre-g150`). All values are `OBSERVED`.

| Quarter | AAAHogEx value / cash / vehicles | PrismaticAI value / cash / vehicles |
|---|---|---|
| 1960-04 | 0.00 M / 23 K / 4.0 (4.1 air) | 0.00 M / 186 K / 1.0 (1.0 air) |
| 1960-07 | 0.07 M / 30 K / 6 — income 96 K/q | 0.01 M / 159 K / 3 — income 40 K/q |
| 1961-01 | 0.33 M / 35 K / 12 | 0.09 M / 166 K / 6 |
| 1963-01 | 1.87 M / 132 K / 79 | 0.80 M / 484 K / 20 |
| 1965-01 | 3.80 M / 914 K / 148 | 1.69 M / 1.18 M / 30 |
| 1970-01 | 22.1 M / 17.8 M / 221 | 4.90 M / 4.24 M / 43 |
| 1980-01 | **103.3 M** / 96.2 M / 377 | **16.3 M** / 15.7 M / 55 |

- AAAHogEx ranks first in 115 of 115 matches. The best PrismaticAI configurations average rank
  2.0–2.1.
- AAAHogEx's lead in value grows from 2.4× (1962) to 4.5× (1970) to 6.3× (1980).

**Per-mode results, 1980-01** (vehicles; profit last year; profit per vehicle):

| Mode | AAAHogEx | PrismaticAI |
|---|---|---|
| Rail | 95.1; 5.37 M; **56 K** | 19.2; 0.54 M; **28 K** |
| Air | 47.9; 5.36 M; **112 K** | 19.9; 0.95 M; **48 K** |
| Road | 230; 0.17 M; 0.8 K | 16.2; 0.02 M; 1.2 K |
| Water | 4.5; 0.07 M; 16 K. Ships appear in 71 of 115 matches. | none in any match |

- Total profit per vehicle is about the same: ≈29 K for AAAHogEx and ≈28 K for PrismaticAI.
- **The 7× profit gap is a fleet-size gap (377 vs 55 vehicles).** It also comes from AAAHogEx
  earning about twice as much per train and per plane. AAAHogEx's road fleet is large and
  earns almost nothing, which dilutes its average. `OBSERVED`.
- Both AIs end the game holding most of their value as cash: 93% for AAAHogEx and 96% for
  PrismaticAI. They differ in *when* they hold it:
  - AAAHogEx holds 23–48 K from 1960 to 1962, while its value rises 0 → 1 M.
  - PrismaticAI holds 160–480 K over the same period, more than its company value in 1961.
  - This is `OBSERVED`. What it means is discussed in section 4.

## 3. AAAHogEx's implemented decision pipeline

### 3.1 Startup and scheduling
- **Start-up.** `Start()` (`main.nut:266`) does three things:
  1. It detects the YETI, ECS and FIRS NewGRF families (`:577-594`).
  2. It switches the company's **autorenew off** (`:718`), because the AI renews vehicles itself.
  3. It builds a Fibonacci grid of distances used to sample the estimator (20, 30, 50, … `:720-729`).
- **Main loop** (`:746-765`). Each turn:
  1. `CalculateProfitModel()` (`:781-806`), then the estimate cache is cleared (`:751`).
  2. `DoStep` 0–3 (`:838`). Step 1 is ECS/YETI supply chains (`:2681`), a no-op on this map.
     Step 2 is `ScanRoutes`, which extends rail routes and adds return legs. Step 3 is
     `ScanPlaces`, which builds new routes.
- **`_DoInterval`** (`:3865-3952`) runs at most every 7 days and is called from inside every
  wait and every pathfinding round. It runs, in order:
  1. loan
  2. production checks
  3. events
  4. old vehicles
  5. statues
  6. train routes
  7. common routes
  8. pending constructions
- **No ops budgeting.** `GetOpsTillSuspend` is used only in `Save()`. Searches are bounded in
  game days:
  - 365 days of candidate generation once usable money exceeds 100 K (`:885`)
  - 1000 candidates (`:880`)
  - 2 days per cargo estimate (`:1440`)
  - a build window of about 2 years (`:919`)
- **Settings** (`info.nut`). The only one with a real strategic effect is `usable_cargos`
  (default 3, all cargos). There is no aggressiveness knob.
  - The AI reads game settings instead: inflation, maintenance, breakdowns, max loan,
    `distant_join`, vehicle limits (`main.nut:4440-4495`).
  - The battle runs it with an empty settings string, so `medium` defaults apply.

### 3.2 Three profit models
`CalculateProfitModel` (`main.nut:781-806`) chooses the objective that ranks all candidates:
- **`roiBase`** while the company is not rich, or always when inflation is on. It ranks by
  `routeIncome*1000 / (price*vehicles + buildingCost + lostOpportunity)` (`estimator.nut:77-94`).
- **`buildingTimeBase`** once rich, while some vehicle type has at least 100 free slots and is
  below 70% of its limit. It ranks by `routeIncome / buildingTime`.
- **`vehicleProfitBase`** otherwise. It ranks by `routeIncome / vehicles`.
- **`IsRich`** (`:4324-4331`) is true when usable money exceeds 10 M, or when it exceeds 2 M (or
  500 K with quarterly income of at least 100 K) and the loan is 0 or falling.
- **The objective follows the company's money.** It is capital efficiency early, then build speed
  once money stops being the constraint. With inflation off, the battle's trajectory crosses
  `IsRich` around 1965–1966 (cash 0.9 M in 1965 and 17.8 M in 1970). `INFERRED`: the switch is
  not logged.

### 3.3 Candidate generation
- **Source selection** (`main.nut:1333-1576`). For every cargo, the sources are:
  - the 300 places with the highest production that have no route yet
    (`Place.GetNotUsedProducingPlaces(cargo, 300, …)`, `:1415`; `place.nut:1082-1118`)
  - plus unserved industries at the destinations of its own routes (`:1392-1406`)
- **Mode × distance pre-ranking.**
  - A generic `Route.Estimate` is run for each mode (train, road, water, air) at each sampled
    distance, with a standard production (`:1479-1496`).
  - The result decides which distance bands are searched first, and for which mode.
  - Each cargo and mode keeps `max(16, 128/numCargos)` places (`:1498-1536`).
- **Pairs** (`CreateRouteCandidates`, `:1578-1872`).
  - Destinations are walked by Manhattan-distance band in that pre-ranked order.
  - Filters:
    - `CanUseNewRoute`
    - the NG blacklists (`IsNgPlace`, `IsNgPathFindPair`)
    - water and airport feasibility
    - land connectivity for rail and road (`:1820-1844`)
  - An expected production below 27 aborts the source (`:1738-1741`).
  - Rail scores are scaled by slope (`utils.nut:1203-1222`).
  - At most 4 results per call and 25 per source (`:1322`).
- **Competition.**
  - A town may have `population/10000+1` station groups (`place.nut:1291-1299`).
  - With infrastructure maintenance off, **air ignores the "one route per source" rule entirely**
    (`place.nut:1287`). This applies here.
  - Avoiding a processing industry that another company already serves is enabled only on easy
    (`place.nut:897-910`).
  - The term meant to divide production among competitors evaluates to 0 every time: in
    `!x || x ? 0 : 1` the condition is a tautology (`place.nut:1942`). Competing companies at
    the same source are therefore never discounted.
- **Feeders and supply.**
  - After every build, transfer candidates, supply plans for the source, and plans for the
    destination's own output are pushed back into the same queue (`route.nut:3567-3647`).
  - Supply-before-source routes are skipped under `roiBase` (`main.nut:2767`).

### 3.4 Profit estimator
**Road, water and air** (`CommonEstimation.Estimate`, `estimator.nut:264-315`; `CalculateIncome`,
`:54-65`):
- **Station rating:** `stationRate = GetStationRate(speed) + 170`, out of 255 (`utils.nut:1267-1272`).
- **Rated production:** `production * stationRate / 255`, capped by the route's capacity.
- **Speed and days:** cruise speed is derated by reliability and capped by the infrastructure
  speed (`:788`).
  - `days = d*664/v/24` (`utils.nut:1225`).
  - Air adds 30 tiles for landing, plus 50 with breakdowns, plus a share of slow broken-down
    flight (`:843-856`). Road adds 4 days.
- **Loading:** `loadingTime = ceil(capacity/loadingSpeed)`, with road 18, air 74 and water 148
  (halved for mail) (`:813-835`).
- **Fleet size:** `vehiclesPerRoute = ratedProd*12*days/(365*capacity)+1`, clamped by vehicle slots
  and by `GetMaxBuildingCost = usable + 5×max(0, yearly income − maintenance)` (`:549-552, 872-879`).
- **Revenue:** `AICargo.GetCargoIncome(cargo, distance, cruiseDays)`, the engine's own payment
  curve, times the full capacity (`:285-287`).
  - Every trip is priced as a full load.
  - Air also prices the return trip as full (`:290`, commented "dest側も満タン待機", "full-load
    wait at the destination too").
  - Air passenger routes add mail both ways (`:298-307`).
- **Waiting at stations:** a station-rating stock simulation (`utils.nut:1293-1340`).
- **Net route income:** `routeIncome` subtracts running cost, depreciation (with breakdowns on) and
  infrastructure (`:54-62`).
- **Engines:** every refittable engine and infrastructure type is estimated, and the set is sorted
  by `value` (`:948-950`).
- **Building cost** (`estimator.nut:555-592`):
  - air is 4 × airport cost
  - road is about `(roadCost*2+216)*distance + 4000`
  - rail is about `(track*2+216)*2*distance + track*220`
- **Building time** (`:487-516`): air 300, road `125+3d`, rail `200+max(100,4d)` (×0.6 for single
  track), plus 3 per vehicle. This is the objective under `buildingTimeBase`.

**Rail** (`TrainEstimation.Estimate`, `estimator.nut:341-446`) differs from the above:
- Consists can carry several cargos, and sub-cargos from the same pair are appended
  (`AppendSubCargoIncome`).
- It computes the train count, then adds one.
- A single-train route lowers the station rating.
- Single and double track are both estimated. Rail engine sets are ordered by `roi` under
  `roiBase`, otherwise by `routeIncome` (`:1034-1046`).

**No calibration.** There is no feedback from realized profit into the estimate. The cache is
cleared every turn. A measured road-speed ratio exists (`roadTrafficRate`, `route.nut:1771`), but
its use is commented out (`estimator.nut:794-797`). Realized profit is used only to prune (see
3.8).

### 3.5 Choosing a mode
- **No mode is hard-coded first.** All modes compete in one `SortedList` ordered by
  `estimate.value` (`main.nut:4512-4638`).
- **Gates:**
  - A mode stops taking new routes once its vehicle slots are 80% (road), 90% (rail and water) or
    95% (air) used (`road.nut:289`, `trainroute.nut:505`, `water.nut:74`, `air.nut:221`).
  - Non-air modes are limited to distances of 1000 tiles or less.
  - Air needs a noise-compliant airport at both ends and the plane's maximum order distance.
  - The first rail route needs `price*3/2 ≤ usable` and holds back 20 K (`estimator.nut:1428, 1680`).
- **Why air wins early** (DERIVED from 3.2–3.4):
  - Under `roiBase` the denominator is capital.
  - An air route's infrastructure is only two airports and needs no path.
  - The estimator prices both directions as full loads plus mail.
  - With maintenance off, air bypasses the one-route-per-source rule.
- This matches the run: by 1960-04 all 115 matches had 4.1 planes on average and nothing else.
  The `OBSERVED` part is the mode mix; that it was chosen *because of* these terms is `INFERRED`.

### 3.6 Pathfinding and construction
**Rail** (`pathfinder.nut`, own `AyStar` in `aystar.nut`; parameters in `InitializeParameters`,
`:102-195`):
- **Costs:** tile 100, diagonal 67, turn 300, tight turn 100 (or 1500 at 40 tiles or more),
  slope 100, bridges 100 per tile with growing penalties from length 7 and 11.
  - **Tunnels cost 0 per tile** ("to tunnel rather than climb", `:139`), up to 11 tiles, or 14 when
    rich.
  - Level crossing 900.
- **Weak engines.** For low-tractive-effort engines, and on mountainous maps, slopes rise to 1500
  (`:164-179`).
- **Heuristic:** octile × 2, which is greedy and not admissible (`:802, 820`).
- **No reuse of its own existing rail** except at goal and fork tiles (`:1065, 1105-1117`).
- **Budget:** `pathFindLimit` 80 × 3 rounds of 50 (`:199-209`; `main.nut:242`), tripled while
  poor. `DoInterval` runs between rounds.
- **When the budget runs out:**
  - The pathfinder returns the best partial path (`:227-238`).
  - The builder fails unless the goal was found (`railbuilder.nut:2374-2377`).
  - A mid-build failure re-pathfinds from the break (`RetryToBuild`, `railbuilder.nut:1291-1331`).
- **Double track:**
  - Two searches. The second follows the first with `_cost_guide`, and the first pays +3000
    wherever no parallel space exists (`:716-761, 806-812`; `railbuilder.nut:2635-2690`).
  - One-way PBS signals every 7 tiles, at bridge and tunnel mouths, and on downhill runs
    (`railbuilder.nut:1100-1235, 1513-1527`).
  - Source stations have 3 platforms, terminal stations 2 (`station.nut:1528-1537, 1611-1623`).
  - Platform length is `max(est, 4+d/68)`, capped at `6+d/8` (`:1342-1356`).
  - With breakdowns on, the line gets paired depots at intervals (`railbuilder.nut:2441, 2638`).

**Road** (`road.nut:852-928`, `roadpathfinder.nut`):
- An existing road tile costs 50 and a new tile 190, so existing roads are strongly preferred.
- Level crossing 1000. It never crosses its own rail.
- It can plan through town buildings it may demolish, with a budget set by the town rating
  (`roadpathfinder.nut:304-315`).
- Budget: 100 rounds of `FindPath(100)` under `roiBase`, otherwise 50.

**Water** (`water.nut:1490-1570`): staying on the coast is free, leaving it costs 5000, a canal tile
costs 190, a lock costs 500.

**Order of build** (`route.nut:3720-4036`; rail: `trainroute.nut:2885-3155`):
1. Estimate.
2. Choose both station sites in test mode, destination first.
3. Build the stations (exec).
4. Build the path.
5. Build the depots.
6. Buy the vehicles.

Every built element is pushed to a rollback list. A failure demolishes it (`route.nut:3172,
3268-3323`), and unfinished constructions are rolled back after a load (`main.nut:4162-4166`).

**Two vehicles from the start.** `BuildVehicleFirst` builds one vehicle and clones a second at
once (`route.nut:2990-2998`). On double track, `BuildFirstTrain` does the same
(`trainroute.nut:958-967`).

**Town rating.** When a station is refused, it plants trees (`station.nut:2102-2114`), at most every
90 days per town (`utils.nut:984-997`). It builds statues once cash exceeds 400 K
(`main.nut:3636-3669`).

### 3.7 Money
- **Just-in-time loan.**
  - Every `_DoInterval` sets `SetMinimumLoanAmount(loan − balance + 10000)` (`main.nut:3895`).
    This repays everything above about 10 K and never draws the loan ahead of need.
  - Before a build, `WaitForPrice` raises the loan to cover the price plus a buffer
    (`main.nut:4203-4253`).
- **When the money still does not cover a build.** `WaitForPrice` loops with no timeout by
  default. On each pass it:
  - sells vehicles stopped in depot
  - culls vehicles that are not profitable
  - may remove losing routes (`CheckReduce`, only from here)
- **Abort rule.** A build is abandoned when both the last four quarters' income and usable money
  are below its cost (`IsTooExpensive`, `main.nut:4299`).
- **Pacing.** After a build under `roiBase`, `ScanPlaces` waits up to 180 days until the money
  covers:
  - the remaining vehicles of routes just built
  - plus the next candidate (`main.nut:1019-1061`)
- **Stop rule.** It stops when the next candidate's value drops below 200 (`:1054`).
- **Run evidence.** Cash of 23–48 K from 1960 to 1962, while the company grows to 1 M, is what
  this policy produces. `OBSERVED` cash; the loan itself was not recorded, so the mechanism is
  `DERIVED`.
- **After the loan is repaid.** Once the loan is 0, the argument goes negative and cash
  accumulates: 0.9 M in 1965, 17.8 M in 1970, 96 M in 1980. `OBSERVED`. What the engine does
  with a negative minimum-loan argument was not traced; most likely the call simply fails.
  `INFERRED`.

### 3.8 Fleet management
For road, air and water, `_CheckBuildVehicle` (`route.nut:2629-2988`) runs on every interval for
every route, in random order.
- **Selling:** it first sells every vehicle stopped in a depot.
- **Adding:** it adds vehicles when waiting cargo exceeds `needsProduction`.
  - `needsProduction` is 30 under `roiBase` (10 otherwise) while the route has fewer than 10
    vehicles, and 100 after that (`:2838`).
  - It adds `(waiting − bottom)/capacity` vehicles at a time, at most 4. Low ratings force one.
  - It adds nothing while a vehicle is already queued at the station (`:3012`).
  - There is no fixed per-route vehicle cap.
- **Growing the ceiling:** `maxVehicles` grows by `finalMax/12` with probability 3% or 15% when
  cargo overflows (`:2799-2810`).
- **Removing vehicles:**
  - In Q4, vehicles older than 800 days that lost money over two years are removed
    (`:2480-2497`).
  - Idle vehicles beyond 1–3 per station are trimmed (`:2502-2620`).
  - With breakdowns on, non-rail vehicles with less than 600 days of life left are removed in Q4
    (`CheckOldVehicles`, `:1622-1645`). This, together with re-buying on demand, is its renewal.
- **Engine re-selection** happens every 1500 days, or when the current engine becomes
  unavailable. Vehicles of an old engine are sent to depot and replaced (`:2295-2325, 2729-2747`).
- **Removing routes** (`CheckReduce`, `:1647-1786`) happens only under money pressure: a 5-year
  average profit of 0 or less, the bottom 3% when too many vehicles, and halving road routes
  whose profit fell by more than half.
- **Trains** (`TrainRoute.CheckTrains`, `trainroute.nut:2256-2515`) were not traced in detail.
  `UNKNOWN`.
- **Industry closure:** `CheckClose` removes the route or closes it until the destination accepts
  again (`route.nut:1295-1449`).
- **Engine disappears:** an unavailable engine invalidates the engine set (`:2259`).

## 4. Mechanism comparison

| Area | AAAHogEx (v115) | PrismaticAI (8c8d025388) | Likely consequence | Class |
|---|---|---|---|---|
| **Opportunity generation and ranking** | Sources: the top 300 per cargo across every cargo; pairs walked by a mode×distance pre-ranking; one queue across all modes, ranked by a single `value` (`main.nut:1333-1872, 4512`). | A separate fixed generator per mode, tried in a fixed order each heartbeat: air, then rail freight, then road (`main.nut:386-579`). Air pairs scored `min(pop)*dist` (`autopilot.nut:51-89`); freight needs `prod≥40`, untouched, 100–260 tiles, scored `prod*income/(2*days+10)` (`rail.nut:1215-1242`); road among the 10 largest towns, 20–80 tiles apart (`intercity.nut:19-43`). | AAAHogEx sees many more opportunities and compares them on one scale. PrismaticAI's set is narrow, and its order is fixed regardless of return. **Used:** AAAHogEx ran rail+road+air in 115/115 matches and water in 71; PrismaticAI ran rail+road+air in 18/18 and water in 0. | OBSERVED (modes); DERIVED (cause) |
| **Profit estimator and calibration** | Per engine: speed, reliability, loading, a station-rating simulation, `GetCargoIncome` with full loads, running cost, depreciation; the objective changes with wealth (`estimator.nut:54-315`). No calibration. | No unified estimator. Freight uses `GetCargoIncome` at a fixed 3.7 tiles a day, ignoring capacity, engine and cost; air uses `min(pop)*dist` with no money term (`rail.nut:1218-1233`). No calibration. | AAAHogEx picks the engine and route pair by estimated net income. PrismaticAI ranks air by a population proxy. **Per vehicle:** AAAHogEx earns 112 K per plane and 56 K per train against 48 K and 28 K (1980). The gap is observed; attributing it to the estimator is not (no log shows which pairs were chosen). | OBSERVED (gap); INFERRED (cause) |
| **Mode breadth** | Rail (single or double track, multi-cargo), road (bus, truck, tram), air (6 airport types), water (docks, canals, locks); multi-mode feeders. | Rail freight (double track by default, `rail.nut:1136`), rail passengers only with a planner, buses only (no road freight), air. No water (no `VT_WATER` or `AIMarine`). | Ships add 70 K/yr to AAAHogEx: negligible. Road freight: AAAHogEx's 230 road vehicles earn 0.17 M/yr, also negligible. **Mode breadth is not the main cause.** | OBSERVED |
| **Pathfinding and buildability** | Own A\*; tunnels free per tile, bridges up to 30, crossings allowed; greedy heuristic; budget measured in game days, with maintenance interleaved; partial-path retry; full rollback. | Own A\*: rail with bridges 3–11, tunnels up to 15, no 90° turns, 150000-node budget (`rail.nut:39-286`); road A\* that treats slopes as impassable (`intercity.nut:58`). Gaps: a failed rail build does not remove its stations or the track laid so far (`rail.nut:794-812, 933-993`); a road pair whose depot failed is not blacklisted (`intercity.nut:143-152`). | AAAHogEx can build over harder terrain, and a failed build costs it less. **Neither AI logs its builds at a captured level**, so failure rates and wasted capital are not measured. | UNKNOWN (runtime) |
| **Plan to first service** | Air first, without a path; both stations, then path, then depot; **two vehicles at once** (`route.nut:2990`, `trainroute.nut:958`). | One airport survey every 20 ticks (PACE); **one plane per air route** (`air_planes=1`), with more added later by `ManageFleet`; one train per rail line, more only from the monthly steward (`steward.nut`). | By 1960-04: AAAHogEx 4.1 planes and 6 stations; PrismaticAI 1.0 plane, 4 stations. Income 96 K/q against 40 K/q by 1960-07. AAAHogEx earns earlier and grows faster from the very first quarter. | OBSERVED (timing); DERIVED (cause) |
| **Vehicle and fleet management** | Adds up to 4 at a time when waiting cargo exceeds 30, 10 or 100; no fixed per-route cap; ceiling grows on overflow; removes in Q4 on two years of losses; replaces old vehicles itself (autorenew off). | `ManageFleet` adds when 1.5 loads are waiting; **hard cap of 8 road vehicles per route** (`fleet.nut:162`), air capped at `invest_air_cap` (6); `InvestSurplus` adds **at most one vehicle per heartbeat**, only to routes whose vehicles are at least a year old and only if `ppv*invest_ppv_div ≥ price` (`fleet.nut:67-110`); trains are governed only by the steward; the game's autorenew. | **The largest single observed difference.** Vehicles: 377 against 55 (1980). PrismaticAI's fleet grows 43 → 55 from 1970 to 1980 while its cash grows 4.2 → 15.7 M. Its station count is flat at 66–67 from 1970. Code-level caps exist; **which cap actually binds is UNKNOWN** (no PrismaticAI log captured). | OBSERVED (plateau); DERIVED (caps exist); UNKNOWN (binding cap) |
| **Capital use and debt risk** | Just-in-time loan, about 10 K of cash, borrows per build (`main.nut:3895, 4203`); `WaitForPrice` culls and sells under stress; `IsTooExpensive` abort. | Draws the full loan for each build and gives it back if nothing was built (`main.nut:422-511`); `RepaySurplus(max(2*cash_reserve, afloat))` every heartbeat; autopilot reserve 5000 (`main.nut:343-353`); route `need ≥ 60000` for air (`autopilot.nut:112`). | 1960-04 to 1962: AAAHogEx holds 23–48 K while its value grows to 1 M; PrismaticAI holds 155–255 K, more than its company value in 1961. **About 150 K of idle capital in the first two years, while compounding matters most.** Loan balances are not recorded, so debt risk cannot be compared. | OBSERVED (cash); UNKNOWN (loan, debt) |
| **Reinvestment and expansion** | Continuous: `ScanPlaces` every turn and vehicle adds on every interval; the objective changes from ROI to build speed once rich; spends on statues when cash exceeds 400 K and on rail upgrades when it exceeds 1 M; feeders and extensions after each build. | Caps: `air_routes_max` (15) plus `money/air_grow_money` (2.5 M); own lines 10 → 32; twins `rail_twin_max`; `max_routes` 30; `InvestSurplus` paced. No comparison between a new route and growing an existing one. | Both hold more than 90% of their value as cash in 1980, so **neither reinvests late**. AAAHogEx's late cash builds up once it is rich and near its vehicle-slot thresholds. That thresholds bind is `INFERRED`; vehicle limits were not read from the run configuration. The 1965–1970 window decides the result: AAAHogEx adds 73 vehicles and 32 trains, PrismaticAI 13 vehicles and 7 trains. | OBSERVED (growth); INFERRED (stopping cause) |
| **Execution failures and recovery** | Rollback of every element; time-limited blacklists (place 1000 days or 60 days for an authority refusal, pathfinding pair permanent); 2 failures per `DoRoutePlans`; retries with shared stations. | Blacklists `AIR_BAD_TOWNS`, `air_tried`, `freight_tried`, `FAILED_PAIRS`; `own_fail_pause` (6, 0 in pre-g150); incomplete cleanup on failed rail; nothing saved or loaded. | No captured failure events for either AI. The post-mortems show no Squirrel errors (`errors=0` in all 115 matches for both). | UNKNOWN |

## 5. Explanations, each tied to evidence

1. **Faster first service compounds.**
   - AAAHogEx: two vehicles per new route (`route.nut:2990-2998`), air ranked first by an ROI
     objective that prices round trips as full (`estimator.nut:77-94, 285-290`).
   - PrismaticAI: one plane per air route (`air_planes=1`, `air.nut` via `autopilot.nut:112`); one
     survey per PACE.
   - Evidence: 4.1 planes against 1.0 at 1960-04; income 96 K/q against 40 K/q at 1960-07;
     value 0.33 M against 0.09 M at 1961-01. `OBSERVED` timing, `DERIVED` mechanism.
2. **Idle early capital.**
   - AAAHogEx: just-in-time loan (`main.nut:3895, 4245`).
   - PrismaticAI: full loan per build, then `RepaySurplus` down to the floor
     (`main.nut:422-511, 569`).
   - Evidence: AAAHogEx cash 23–48 K, PrismaticAI cash 155–255 K, from 1960 to 1962.
   - Why PrismaticAI's cash stays high despite `RepaySurplus` is `UNKNOWN`. Candidates: the loan
     step, the `afloat` term, or builds waiting on `need`. A recorded loan would settle it.
3. **The fleet stops growing.**
   - AAAHogEx: adds on waiting cargo, with no fixed route cap (`route.nut:2836-2971`).
   - PrismaticAI: road 8 per route, air 6 per route, one investment per heartbeat with a
     payback test (`fleet.nut:67-110, 158-164`); trains only through the steward.
   - Evidence: PrismaticAI's vehicles go 43 → 55 and stations 66 → 67 (1970 → 1980) while cash
     goes 4.2 M → 15.7 M. AAAHogEx's vehicles go 221 → 377.
   - Which PrismaticAI gate binds is `UNKNOWN` without its own log.
4. **Higher profit per train and per plane.**
   - AAAHogEx: per-engine estimation (`estimator.nut:630-952, 1365-1842`), a rail source
     station with 3 platforms, double track, platforms sized by distance.
   - PrismaticAI: fixed 3.7 tiles a day, first rail type only (`rail.nut:1127`), air pairs by
     population.
   - Evidence: 56 K against 28 K per train, 112 K against 48 K per plane (1980, `prail`/`pair`
     divided by the vehicle count).
   - The cause is `INFERRED`. The referee records no load factor, distance or engine, so a
     difference in route length or in engines cannot be separated from a difference in how the
     route is operated.
5. **Not a cause: mode breadth.**
   - In 1980, water earned AAAHogEx 0.07 M and road 0.17 M, about 2% of its profit.
   - Rail and air carry about 98% for both AIs. `OBSERVED`.

## 6. Capability that was never shown in use

These AAAHogEx paths are in the code but show no runtime evidence in these runs. Treat them as
capability, not as advantage:
- ECS, YETI and FIRS supply chains: these NewGRFs are absent, so the paths are inactive. `DERIVED`.
- Feeder and transfer chains, rail extensions and return legs (`ScanRoutes`): `UNKNOWN`. The
  referee does not record transfers.
- `buildingTimeBase` and `vehicleProfitBase`: whether and when either became active is `UNKNOWN`
  (not logged).
- Tree planting, statues and the road demolition budget: `UNKNOWN`.
- Train fleet management (`CheckTrains`): not traced, so `UNKNOWN`.

## 7. Defects in AAAHogEx v115 that bear on its predictions

Found by reading the code. None is confirmed at runtime.
- The competitor discount is a tautology, always 0 (`place.nut:1942`). It over-estimates
  production at sources that others share.
- Every trip is priced as a full load, and air returns are priced full too
  (`estimator.nut:287-290`). This over-estimates thin routes.
- `if(!IsProcessing)` tests a function reference rather than calling it (`place.nut:1663`). The
  FIRS caps never apply, but FIRS was not loaded here.
- `_GetSupportEstimateMin` refers to undefined names (`place.nut:1611-1641`). It is reachable only
  with FIRS.
- The `ScanRoutes` 10-year comparison looks inverted against its comment (`main.nut:1212`).
- `GetSetting("many_types_of_freight_as_possible")` is not declared in `info.nut`
  (`main.nut:4384`).

## 8. What would turn the UNKNOWNs into evidence

Run neither AI for this audit. These are proposals for the next battle editions:
- **Capture `script` debug level 3 for the AI companies.** That covers `AILog.Info`, `HgLog.Info`
  and PrismaticAI's heartbeat. The AAAHogEx strings to match are `"#### TryBuild "`,
  `"# RouteBuilder Succeeded "`, `"# RouteBuilder Failed "`, `"######## turn:… roiBase|…"`,
  `"wait for money:"` and `"RemoveRoute averageProfit:"`. PrismaticAI's are
  `"PrismaticAI heartbeat:"`, `"invest:"`, `"fleet: … added vehicle"`, `"autopilot air N/max"`
  and `"self-steward line"`. Expect a much larger log and an observer effect on wall time;
  in-game decisions should not change.
- **Add `loan=` (`GSCompany.GetLoanAmount`) to the referee's `REF Q` line.** This settles
  capital use and debt risk for every contestant.
- **Record vehicle-slot limits and `max_loan` in the match record.** This tells whether
  AAAHogEx's late cash pile-up comes from its vehicle-slot thresholds.

## Method and limitations

- **Version.** AAAHogEx v115 was extracted from the arena tar into a scratch directory and read
  there. PrismaticAI was read from `git archive 8c8d025388`. The working tree's `air.nut`,
  `town.nut` and `town_model.nut` differ from that revision and were not used.
- **Who read what.** The source trace was split between four parallel readers: AAAHogEx control
  and money, AAAHogEx planning and estimator, AAAHogEx construction, and PrismaticAI. The
  citations that the conclusions rest on were re-read by hand: `main.nut:718, 3895, 4295-4300`,
  `estimator.nut:77-94, 280-292`, `place.nut:1287, 1942`, `route.nut:2836-2848`, PrismaticAI
  `main.nut:343-353`, `fleet.nut:158-164`, `rail.nut:1101-1106, 1218-1233`,
  `autopilot.nut:101-113`. Other line numbers come from the readers and were not re-checked one
  by one.
- **Statistics.** `sqlite3 -readonly` on `~/ttd-battle/results/battle.sqlite`, completed matches of
  the three editions only. Per-vehicle profit is a mode's profit last year divided by that mode's
  vehicle count at the same quarter; vehicles bought during the year bias it slightly low.
- **Scope of conclusions.** Everything holds for this configuration only: a 256² temperate map
  from 1960, 20 years, breakdowns reduced, max loan 300 K, no maintenance, no inflation, no
  NewGRFs, against this lineup.
