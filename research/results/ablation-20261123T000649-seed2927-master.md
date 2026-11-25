# ablation-20261123T000649-seed2927-master

Derived from its .json by `tools/research/ai-battle/battle.py ablate --write`; do not edit.

One local match per variant of PrismaticAI against AAAHogEx, LuDiAI AfterFix, CivilAI, AdmiralAI, NoCAB, DictatorAI, SimpleAI, seed 2927, 10 years from 1950, at `13b7005480`. Each value is the variant's last quarterly referee sample. One seed: a direction, not a verdict.

Imported after the run from its kept log `master.log` (sha256 `35ef9cb0becc18a0`), binary `20261122-master-gfdc13851cf`, by `tools/research/strategy-surfaces import`; the run's exit status was not kept. Evaluation of master before the 2026-11-23 fixes (openttd-36): the live game's settings against the 7 BaNaNaS AIs of the owner's live game, 10 years from 1950.

| Variant | Value | vs first | Vehicles | Rail | Road | Air | Squirrel errors | Settings |
|---|---|---|---|---|---|---|---|---|
| master | 3,335,947 | +0.0% | 71 | 14 | 41 | 16 | 0 | `aggressiveness=80,air_add_waiting=90,air_full_load=0,air_grow_money=0,air_keep_last=1,air_level_max=20,air_level_once=1,air_level_step=8,air_max_tiles=440,air_planes=5,air_reuse=1,air_routes_max=2,air_towns=35,cash_reserve=5000,catchment_stops=4,intercity=1,invest_air_cap=10,invest_borrow=1,invest_min=50,invest_ppv_div=6,invest_road_cap=12,line_budget=30000,max_routes=30,own_fail_pause=4,own_lines=4,rail_twin_max=10,self_steward=1,statue_money=0` |

<!-- ablation-charts:begin -->
<!-- derived from ablation-20261123T000649-seed2927-master.json by `battle.py render`; `battle.py check` fails when it is stale -->

Highest final value: master (+0.0% against master); lowest: master (+0.0%). Spread between them: 0.0% of the lowest, on one seed (2927).

```mermaid
---
config:
  xyChart:
    width: 900
    height: 360
  themeVariables:
    xyChart:
      plotColorPalette: "#d11f1a, #8a8a8a, #0b0b0b, #ece4d6"
---
xychart-beta
    title "Final company value by variant, and the best opponent in its match (seed 2927, 10 years)"
    x-axis ["master"]
    %% series: Final value | Best opponent
    %% links: ablation-20261123T000649-seed2927-master.md?col=Variant&filter=master
    y-axis "Company value" 0 --> 29600000
    bar [3335947]
    line [26950796]
```

```mermaid
---
config:
  xyChart:
    width: 900
    height: 360
  themeVariables:
    xyChart:
      plotColorPalette: "#d11f1a, #8a8a8a, #0b0b0b, #ece4d6"
---
xychart-beta
    title "Final company value against the baseline master (%)"
    x-axis ["master"]
    %% series: Change against the baseline (%)
    %% links: ablation-20261123T000649-seed2927-master.md?col=Variant&filter=master
    y-axis "Change (%)" 0 --> 2
    bar [0]
```

```mermaid
---
config:
  xyChart:
    width: 900
    height: 360
  themeVariables:
    xyChart:
      plotColorPalette: "#d11f1a, #8a8a8a, #0b0b0b, #ece4d6"
---
xychart-beta
    title "Final company value as a share of the best opponent (%)"
    x-axis ["master"]
    %% series: Share of the best opponent (%)
    %% links: ablation-20261123T000649-seed2927-master.md?col=Variant&filter=master
    y-axis "Share (%)" 0 --> 13.6
    bar [12.4]
```

```mermaid
---
config:
  xyChart:
    width: 900
    height: 360
  themeVariables:
    xyChart:
      plotColorPalette: "#d11f1a, #8a8a8a, #0b0b0b, #ece4d6"
---
xychart-beta
    title "Fleet at the end by variant: rail, road, air vehicles"
    x-axis ["master"]
    %% series: Rail | Road | Air
    %% links: ablation-20261123T000649-seed2927-master.md?col=Variant&filter=master
    y-axis "Vehicles" 0 --> 46
    bar [14]
    line [41]
    line [16]
```

<!-- ablation-charts:end -->
