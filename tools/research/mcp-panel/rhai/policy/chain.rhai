// Live-tuner chain (RESEARCH-ONLY, research/battle-brain.md "Live tuner"): the default policy of
// prismatic_tune.rhai. Every threshold a stage uses lives here, none in the stage's logic.
//
// chain() -> #{
//   stages:       [name]  run in this order; each is policy/<name>.rhai with propose(ctx, params);
//   params:       #{common: #{..}, <stage>: #{..}}  a stage gets common merged under its own;
//                 `priority` (a stage's own) ranks it for merge "priority";
//   merge:        "first"     the first stage to propose a setting sets it;
//                 "priority"  the highest priority sets it, a tie going to the later stage
//                             (which saw the earlier proposals in ctx.proposed);
//                 a veto from any stage drops the setting for the round;
//   max_requests: approval requests a round at most;  every_s: seconds between rounds;
//   sources:      #{tuning: loopback URL of the GA's /api/tuning}
// }
// Each round the runner merges the Rhai shared-store key `policy.chain` over this (a map of the
// same shape: stages/merge/max_requests/every_s/sources replace, params merge per stage and key),
// so the GA, the hub or the owner retune it without a file edit; env TUNE_EVERY_S,
// TUNE_MAX_REQUESTS and TUNE_IDLE_BUDGETS win over both. Pure: no host call.

fn chain() {
    #{
        stages: ["rich", "pressure", "modernize", "ga", "brain"],
        merge: "priority",
        max_requests: 3,
        every_s: 600,
        sources: #{ tuning: "http://127.0.0.1:8765/api/tuning" },
        params: #{
            common: #{
                // Money counts as idle above cash_reserve + this many line budgets.
                idle_budgets: 3,
                // The growth direction of every knob a stage may push: +1 raises it, -1 lowers it
                // (invest_ppv_div rises: fleet.nut InvestSurplus skips a route while
                // ppv * invest_ppv_div < price, so a larger divisor adds a vehicle at a lower yield;
                // the first live round of 2026-11-22 proposed 3 -> 1, which would have stopped it).
                growth: #{ rich_growth: 1, invest_air_cap: 1, invest_road_cap: 1, max_routes: 1,
                           air_routes_max: 1, air_reuse: 1, own_lines: 1, rail_twin_max: 1,
                           invest_ppv_div: 1, rich_air_cap_max: 1, rich_served_below: 1,
                           airport_fill: 1, rich_budgets: -1 },
            },
            rich: #{
                priority: 1,
                budgets_per_step: 10,
                steps_max: 3,
                switches: #{ rich_growth: 1 },
                knobs: ["invest_air_cap", "invest_road_cap", "max_routes", "air_routes_max",
                        "air_reuse", "own_lines", "invest_ppv_div"],
            },
            pressure: #{
                priority: 2,
                min_gap: 1.2,
                gap_per_step: 2,
                steps_max: 4,
                require_idle: true,
                switches: #{ compete: 1 },
                knobs: ["invest_air_cap", "invest_road_cap", "max_routes", "air_routes_max",
                        "air_reuse", "own_lines", "rail_twin_max", "invest_ppv_div"],
            },
            modernize: #{
                priority: 3,
                speed_gain: 1.2,
                engine: #{ modernize: 1 },
                track: #{ upgrade_track: 1 },
            },
            ga: #{
                priority: 0,
                min_matches: 4,
                genes_only: true,
                hold: true,
                hold_for: ["rich", "pressure"],
            },
            brain: #{
                priority: -1,
                kind: "safe",
                any_kind: true,
                need_data: false,
                hold: true,
                hold_for: ["rich", "pressure"],
            },
        },
    }
}
