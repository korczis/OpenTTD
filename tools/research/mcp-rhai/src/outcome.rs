//! RESEARCH-ONLY: does the plan match what the game pays? Outcome feedback for the planner.
//!
//! The planner predicts each line's monthly profit from the game's payment formula; the running
//! game says what each line really earned (calibrate.rs: realised / predicted per vehicle-month,
//! vehicles at least two years old). Here those readings are split by owner (our company's lines
//! against every other company's), turned into an outcome prior per mode, and the plan's own
//! candidates are chosen again with it, by the same capability as the plan (cargo.rs `portfolio`:
//! AlgoGlyph knapsack-dp, or the built-in DP labelled as the fallback). Beside it: our fleet per
//! vehicle type against the best other company's (openttd://vehicles profit last year).
//!
//! The answer names each signal with its numbers and a verdict (`mismatch`, `watch`, `consistent`,
//! `insufficient`). It proposes and never applies: no setting, slot or default changes here, the
//! outcome plan is a candidate, not a decision (ADR 0037 B2), and calibration stays off by default
//! until held-start pairs on three seeds favour it (rule a-tuning-claim-needs-held-start-pairs).
//! Why it exists: on seed 12609 (2026-11-23) the plan kept choosing buses and passenger rail that
//! earned 6 % and 11 % of their prediction, while the plan's air prediction was about right
//! (research/mechanisms/planner-and-algoglyph.md "Outcome feedback").
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

use crate::cargo::Kernels;

/// Weight of another company's line in a mode's prior: it says what the map pays, not how we build.
pub const OTHERS_WEIGHT: f64 = 0.5;
/// Pseudo-lines at the map's overall factor (`default`: the weighted median over every line of every
/// mode) every mode's prior is shrunk with: one or two lines move it little, a dozen most of the way
/// (the A/B on one thin reading averaged about zero). Shrinking toward 1 instead would let a mode
/// nobody measured beat every measured one (knowledge the-planner-overpredicts-what-a-line-earns).
pub const SHRINK_LINES: f64 = 2.0;
/// The weighted line count from which a mode's prior is evidence enough for a signal.
pub const STRONG_LINES: f64 = 3.0;
/// A mode whose prior is below this earns less than half of what the plan says.
pub const OVERPREDICTED: f64 = 0.5;
/// The best other company earns at least this multiple of ours per vehicle of a type.
pub const LEADER_EDGE: f64 = 1.25;
/// The outcome plan beats the model's own lines, scored with the prior, by at least this share of
/// their score before the difference counts as a signal (a mode-mix change on a tie is none).
pub const MATERIAL_GAIN: f64 = 0.10;
/// Vehicles a year old or more a fleet needs before its profit per vehicle counts.
pub const MIN_FLEET: usize = 3;
/// A vehicle's last year counts once it is this many days old.
pub const FLEET_AGE_DAYS: f64 = 365.0;

/// The rule's parameters, as the answer carries them.
pub fn rule_params() -> Value {
    json!({"others_weight": OTHERS_WEIGHT, "shrink_lines": SHRINK_LINES, "strong_lines": STRONG_LINES,
           "overpredicted": OVERPREDICTED, "material_gain": MATERIAL_GAIN, "leader_edge": LEADER_EDGE, "min_fleet": MIN_FLEET,
           "fleet_age_days": FLEET_AGE_DAYS})
}

/// How the prior is formed, named in every answer.
pub const RULE: &str = "per mode: the weighted median of realised / predicted profit per vehicle-month over the lines \
    calibrate.rs measured (our lines weight 1, other companies' 0.5), shrunk toward the map's overall factor: \
    prior = (n_eff * median + 2 * default) / (n_eff + 2), n_eff = ours + 0.5 * others, default = the same weighted \
    median over every line; a mode with no line gets default, and with no line at all every factor is 1";

fn round2(x: f64) -> f64 { (x * 100.0).round() / 100.0 }

/// The weighted median of (value, weight): the smallest value at which half the weight is reached,
/// the mean of the two values on either side when half is reached exactly between them.
pub fn weighted_median(mut v: Vec<(f64, f64)>) -> Option<f64> {
    v.retain(|(x, w)| x.is_finite() && *w > 0.0);
    if v.is_empty() { return None; }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let half = v.iter().map(|(_, w)| w).sum::<f64>() / 2.0;
    let mut cum = 0.0;
    for (i, (x, w)) in v.iter().enumerate() {
        cum += w;
        if (cum - half).abs() < 1e-9 { return Some(v.get(i + 1).map_or(*x, |n| (x + n.0) / 2.0)); }
        if cum > half { return Some(*x); }
    }
    v.last().map(|l| l.0)
}

/// The station pairs of `vehicles` (openttd://vehicles data) with their owner: a line is a pair of
/// stations its vehicles shuttle between (as calibrate.rs `lines` groups them).
pub fn owners(vehicles: &Value) -> BTreeMap<(i64, i64), i64> {
    let mut out = BTreeMap::new();
    for v in vehicles.get("vehicles").and_then(Value::as_array).into_iter().flatten() {
        let st: Vec<i64> = v["orders"].as_array().into_iter().flatten()
            .filter(|o| o["type"] == "goto_station").filter_map(|o| o["destination"].as_i64()).collect();
        if st.len() != 2 || st[0] == st[1] { continue; }
        if let Some(o) = v["owner"].as_i64() { out.insert((st[0].min(st[1]), st[0].max(st[1])), o); }
    }
    out
}

/// Per mode, from calibrate.rs's measured `lines` (each `{stations, mode, ratio}`): our lines and the
/// other companies', each with its count and median, and the prior with the lines it rests on; and
/// `default`, the map's overall factor every prior is shrunk toward and an unmeasured mode gets.
/// A ratio was measured against the plan's prediction, which `used` (the plan's params) may already
/// have calibrated; it is scaled back to the model's own prediction, so the prior is absolute.
pub fn priors(lines: &Value, owner_of: &BTreeMap<(i64, i64), i64>, company: i64, used: &Value) -> Map<String, Value> {
    let mut by_mode: BTreeMap<String, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for l in lines.as_array().into_iter().flatten() {
        let (Some(mode), Some(r)) = (l["mode"].as_str(), l["ratio"].as_f64()) else { continue };
        let r = r * crate::plan::calibration(used, mode);
        let (Some(a), Some(b)) = (l["stations"][0].as_i64(), l["stations"][1].as_i64()) else { continue };
        let e = by_mode.entry(mode.to_string()).or_default();
        if owner_of.get(&(a.min(b), a.max(b))) == Some(&company) { e.0.push(r) } else { e.1.push(r) }
    }
    let median = |v: &[f64]| weighted_median(v.iter().map(|x| (*x, 1.0)).collect());
    let weighted = |ours: &[f64], others: &[f64]| -> Vec<(f64, f64)> {
        ours.iter().map(|x| (*x, 1.0)).chain(others.iter().map(|x| (*x, OTHERS_WEIGHT))).collect()
    };
    let all: Vec<(f64, f64)> = by_mode.values().flat_map(|(o, t)| weighted(o, t)).collect();
    let n_all: f64 = all.iter().map(|(_, w)| w).sum();
    let default = weighted_median(all).unwrap_or(1.0);
    let mut out: Map<String, Value> = by_mode.into_iter().map(|(mode, (ours, others))| {
        let n_eff = ours.len() as f64 + OTHERS_WEIGHT * others.len() as f64;
        let m = weighted_median(weighted(&ours, &others)).unwrap_or(default);
        let prior = (n_eff * m + SHRINK_LINES * default) / (n_eff + SHRINK_LINES);
        (mode, json!({"ours": {"lines": ours.len(), "median": median(&ours).map(round2)},
                      "others": {"lines": others.len(), "median": median(&others).map(round2)},
                      "n_eff": n_eff, "median": round2(m), "prior": round2(prior)}))
    }).collect();
    if n_all > 0.0 {
        out.insert("default".into(), json!({"n_eff": n_all, "median": round2(default), "prior": round2(default),
                                            "why": "every line of every mode; a mode with no line of its own gets it"}));
    }
    out
}

/// A mode's factor in `prior` (`priors`): its own, else `default`; None without any.
fn factor_of(prior: &Map<String, Value>, mode: &str) -> Option<f64> {
    prior.get(mode).or_else(|| prior.get("default")).and_then(|p| p["prior"].as_f64())
}

/// The candidates of a plan answer, as plan_pipeline chooses its portfolio from: every list of
/// `by_mode`, then `freight`, best score first.
pub fn candidates(out: &Value) -> Vec<Value> {
    let mut c: Vec<Value> = out["by_mode"].as_object().into_iter().flat_map(|m| m.values())
        .flat_map(|ps| ps.as_array().cloned().unwrap_or_default()).collect();
    c.extend(out["freight"].as_array().cloned().unwrap_or_default());
    c.sort_by(|a, b| b["score"].as_f64().unwrap_or(0.0).total_cmp(&a["score"].as_f64().unwrap_or(0.0)));
    c
}

/// A candidate re-scored with `prior` (mode -> factor): each option's score is payback months of
/// net profit minus its investment (plan.rs `payback_score`), so with the ratio r of the new factor
/// to the one the plan used (`used`, the plan's `params`), score' = (score + investment) * r -
/// investment. Fleet sizes stay as the model chose them, so a mode the prior lowers may score a
/// little less than a re-run with a smaller fleet would. An option without a route (score at the
/// floor, linecost.rs) stays where it is. The candidate's mode and score become its best option's.
pub fn rescore(c: &Value, prior: &Map<String, Value>, used: &Value) -> Value {
    let mut c = c.clone();
    let mut best: Option<(f64, Value)> = None;
    if let Some(opts) = c["options"].as_array_mut() {
        for o in opts.iter_mut() {
            let (Some(mode), Some(score)) = (o["mode"].as_str().map(str::to_string), o["score"].as_f64()) else { continue };
            if score > f64::MIN / 2.0 {
                if let Some(f) = factor_of(prior, &mode) {
                    let inv = o["investment"].as_f64().unwrap_or(0.0);
                    let r = f / crate::plan::calibration(used, &mode);
                    o["score"] = json!(((score + inv) * r - inv).round());
                }
            }
            let s = o["score"].as_f64().unwrap_or(f64::MIN);
            if best.as_ref().map_or(true, |(b, _)| s > *b) { best = Some((s, json!(mode))); }
        }
    }
    if let Some((s, mode)) = best {
        c["score"] = json!(s);
        c["mode"] = mode;
    }
    c
}

/// One line of a portfolio, as a sentence: mode, ends, cargo.
fn line_name(c: &Value) -> String {
    let end = |a: &str, b: &str| c[a]["name"].as_str().or_else(|| c[b]["name"].as_str()).unwrap_or("?").to_string();
    let cargo = c["cargo"].as_str().map(|s| format!(" ({s})")).unwrap_or_default();
    format!("{} {} -> {}{}", c["mode"].as_str().unwrap_or("?"), end("producer", "a"), end("acceptor", "b"), cargo)
}

/// A portfolio (cargo.rs `portfolio`) in brief: solver, total score, investment, lines, mode mix.
pub fn brief(p: &Value) -> Value {
    let chosen = p["chosen"].as_array().cloned().unwrap_or_default();
    let mut mix: BTreeMap<String, u64> = BTreeMap::new();
    for c in &chosen { *mix.entry(c["mode"].as_str().unwrap_or("?").to_string()).or_default() += 1; }
    json!({"solver": p["solver"], "total_score": p["total_score"], "investment_k": p["investment_k"],
           "budget_k": p["budget_k"], "lines": chosen.iter().map(line_name).collect::<Vec<_>>(), "mix": mix})
}

/// The score of a line as its own mode's option scores after `rescore`: what the model's chosen
/// line is worth under the prior.
pub fn score_as_chosen(c: &Value, prior: &Map<String, Value>, used: &Value) -> f64 {
    let mode = c["mode"].clone();
    let r = rescore(c, prior, used);
    r["options"].as_array().into_iter().flatten().find(|o| o["mode"] == mode)
        .and_then(|o| o["score"].as_f64()).unwrap_or_else(|| r["score"].as_f64().unwrap_or(0.0))
}

/// Per company and vehicle type: vehicles, those a year old or more, and their profit last year.
fn fleet(vehicles: &Value) -> BTreeMap<(i64, String), (u64, u64, f64)> {
    let mut out: BTreeMap<(i64, String), (u64, u64, f64)> = BTreeMap::new();
    for v in vehicles.get("vehicles").and_then(Value::as_array).into_iter().flatten() {
        let (Some(o), Some(t)) = (v["owner"].as_i64(), v["type"].as_str()) else { continue };
        let e = out.entry((o, t.to_string())).or_default();
        e.0 += 1;
        if v["age"].as_f64().unwrap_or(0.0) >= FLEET_AGE_DAYS {
            e.1 += 1;
            e.2 += v["profit_last_year"].as_f64().unwrap_or(0.0);
        }
    }
    out
}

/// Our fleet per vehicle type against the best other company's (at least MIN_FLEET vehicles a year
/// old), profit per vehicle last year; `names` maps company ids to names (openttd://companies).
pub fn mix(vehicles: &Value, company: i64, names: &BTreeMap<i64, String>) -> Vec<Value> {
    let f = fleet(vehicles);
    let mut types: Vec<String> = f.keys().map(|(_, t)| t.clone()).collect();
    types.sort();
    types.dedup();
    let per = |(_, counted, profit): &(u64, u64, f64)| if *counted == 0 { None } else { Some((profit / *counted as f64).round()) };
    types.into_iter().map(|t| {
        let ours = f.get(&(company, t.clone()));
        let leader = f.iter().filter(|((o, ty), s)| *o != company && *ty == t && s.1 as usize >= MIN_FLEET)
            .max_by(|a, b| (a.1.2 / a.1.1 as f64).total_cmp(&(b.1.2 / b.1.1 as f64)));
        json!({"type": t,
               "ours": {"vehicles": ours.map_or(0, |s| s.0), "counted": ours.map_or(0, |s| s.1), "per_vehicle_year": ours.and_then(per)},
               "leader": leader.map(|((o, _), s)| json!({"company": o, "name": names.get(o), "vehicles": s.0, "counted": s.1,
                                                          "per_vehicle_year": per(s)}))})
    }).collect()
}

/// The signals and verdict of an assessment, from its priors, the two portfolios in brief and the
/// fleet mix.
pub fn signals(priors: &Map<String, Value>, model: &Value, outcome: &Value, mix: &[Value]) -> (Vec<Value>, &'static str, String) {
    let mut out = Vec::new();
    let strong = |p: &Value| p["n_eff"].as_f64().unwrap_or(0.0) >= STRONG_LINES;
    for (mode, n) in model["mix"].as_object().into_iter().flatten() {
        let Some(p) = priors.get(mode) else { continue };
        if strong(p) && p["prior"].as_f64().unwrap_or(1.0) < OVERPREDICTED {
            out.push(json!({"kind": "overpredicted_mode_planned", "severity": "mismatch", "mode": mode, "planned_lines": n,
                            "prior": p["prior"], "median": p["median"], "lines": {"ours": p["ours"]["lines"], "others": p["others"]["lines"]},
                            "why": format!("the plan holds {n} {mode} line(s); {mode} lines earn {:.0} % of their prediction (median over {} of ours and {} of others)",
                                           p["median"].as_f64().unwrap_or(0.0) * 100.0, p["ours"]["lines"], p["others"]["lines"])}));
        }
    }
    let under = model["score_under_prior"].as_f64().unwrap_or(0.0);
    let best = outcome["total_score"].as_f64().unwrap_or(0.0);
    let gain = best - under;
    if model["lines"] != outcome["lines"] && gain >= MATERIAL_GAIN * under.abs().max(1.0) {
        out.push(json!({"kind": "outcome_plan_differs", "severity": "watch", "model": model["mix"], "outcome": outcome["mix"],
                        "model_score_under_prior": under.round(), "outcome_score": best.round(), "gain": gain.round(),
                        "why": format!("re-scored with the outcome prior, the same candidates and budget choose other lines worth {:.0} more \
                                        than the model's own ({:.0} under the prior); a sensitivity, fleet sizes as the model chose them", gain, under)}));
    }
    for m in mix {
        let (Some(ours), Some(lead)) = (m["ours"]["per_vehicle_year"].as_f64(), m["leader"]["per_vehicle_year"].as_f64()) else { continue };
        if m["ours"]["counted"].as_u64().unwrap_or(0) as usize >= MIN_FLEET && ours > 0.0 && lead >= LEADER_EDGE * ours {
            out.push(json!({"kind": "leader_out_earns", "severity": "watch", "type": m["type"], "leader": m["leader"],
                            "ours": m["ours"], "edge": round2(lead / ours),
                            "why": format!("company {} earns {:.2}x ours per {} (last year): the map pays this type more than we realise; \
                                            look at route choice and station size, not the mode model",
                                           m["leader"]["company"], lead / ours, m["type"].as_str().unwrap_or("vehicle"))}));
        }
    }
    let measured: f64 = priors.get("default").and_then(|p| p["n_eff"].as_f64()).unwrap_or(0.0);
    let (verdict, why) = if out.iter().any(|s| s["severity"] == "mismatch") {
        ("mismatch", "the plan spends on a mode the outcomes say does not pay".to_string())
    } else if !out.is_empty() {
        ("watch", format!("{} signal(s), none of them a planned mode that does not pay", out.len()))
    } else if measured < STRONG_LINES {
        ("insufficient", format!("{measured} weighted measured line(s), fewer than {STRONG_LINES}"))
    } else {
        ("consistent", "the outcomes agree with the plan".to_string())
    };
    (out, verdict, why)
}

/// The outcome assessment of a plan answer `out` (plan_pipeline::run with `calibration`) for
/// `company`: priors, the model's and the outcome portfolio in brief, the fleet mix, the signals,
/// the verdict and the proposal (`applied: false` always). `companies` is openttd://companies data,
/// for names only.
pub fn assess(out: &Value, vehicles: &Value, companies: &Value, company: i64, kernels: &Kernels) -> Value {
    let owner_of = owners(vehicles);
    let used = &out["params"];
    let priors = priors(&out["calibration"]["lines"], &owner_of, company, used);
    let rescored: Vec<Value> = {
        let mut v: Vec<Value> = candidates(out).iter().map(|c| rescore(c, &priors, used)).collect();
        v.sort_by(|a, b| b["score"].as_f64().unwrap_or(0.0).total_cmp(&a["score"].as_f64().unwrap_or(0.0)));
        v
    };
    let mut model = brief(&out["portfolio"]);
    let under: f64 = out["portfolio"]["chosen"].as_array().into_iter().flatten().map(|c| score_as_chosen(c, &priors, used)).sum();
    model["score_under_prior"] = json!(under.round());
    let outcome = brief(&crate::cargo::portfolio(&rescored, used["budget"].as_f64(), kernels));
    let names: BTreeMap<i64, String> = companies.get("companies").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|c| Some((c["id"].as_i64()?, c["name"].as_str()?.to_string()))).collect();
    let mix = mix(vehicles, company, &names);
    let (signals, verdict, why) = signals(&priors, &model, &outcome, &mix);
    let factors: Map<String, Value> = priors.iter().map(|(m, p)| (m.clone(), p["prior"].clone())).collect();
    json!({"company": company, "verdict": verdict, "why": why, "signals": signals,
           "modes": priors, "plan": {"model": model, "outcome": outcome,
                    "rescore": "sensitivity: each option's score scaled by the prior, fleet sizes as the model chose them (no re-run of the planner)"},
           "mix": mix,
           "proposal": {"params": {"calibration": factors}, "applied": false,
                        "how": "a run applies it with mcp-plan --params or a calibrated duel; a default changes only after held-start pairs on three seeds (rule a-tuning-claim-needs-held-start-pairs)"},
           "rule": RULE, "rule_params": rule_params(),
           "authority": "proposes only: no setting, slot or default is changed; the outcome plan is a candidate, never a decision (ADR 0037 B2)"})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kernels() -> Kernels { crate::cargo::testing::kernels(|_, _| Err(crate::cargo::KernelFailure::Unset)) }

    fn veh(owner: i64, kind: &str, a: i64, b: i64, age: f64, profit: f64) -> Value {
        json!({"owner": owner, "type": kind, "age": age, "profit_last_year": profit,
               "orders": [{"type": "goto_station", "destination": a}, {"type": "goto_station", "destination": b}]})
    }

    #[test]
    fn the_weighted_median_counts_weights_and_splits_an_exact_half() {
        assert_eq!(weighted_median(vec![(1.0, 1.0), (3.0, 1.0), (2.0, 1.0)]), Some(2.0));
        assert_eq!(weighted_median(vec![(1.0, 1.0), (3.0, 1.0)]), Some(2.0), "half reached between them");
        assert_eq!(weighted_median(vec![(0.1, 1.0), (0.9, 0.5), (0.8, 0.5)]), Some(0.45), "0.1 holds half the weight");
        assert_eq!(weighted_median(vec![(0.1, 0.5), (0.9, 1.0)]), Some(0.9));
        assert_eq!(weighted_median(vec![]), None);
        assert_eq!(weighted_median(vec![(f64::NAN, 1.0), (2.0, 0.0)]), None);
    }

    #[test]
    fn a_prior_splits_our_lines_from_the_others_and_is_shrunk_toward_the_model() {
        let v = json!({"vehicles": [veh(0, "road", 1, 2, 800.0, 100.0), veh(0, "road", 3, 4, 800.0, 100.0),
                                    veh(0, "road", 5, 6, 800.0, 100.0), veh(1, "road", 8, 7, 800.0, 100.0),
                                    veh(1, "aircraft", 9, 10, 800.0, 100.0)]});
        let owner_of = owners(&v);
        assert_eq!(owner_of.get(&(7, 8)), Some(&1), "a pair is keyed lowest station first");
        let lines = json!([{"stations": [1, 2], "mode": "bus", "ratio": 0.05}, {"stations": [3, 4], "mode": "bus", "ratio": 0.1},
                           {"stations": [5, 6], "mode": "bus", "ratio": 0.02}, {"stations": [7, 8], "mode": "bus", "ratio": 0.9},
                           {"stations": [9, 10], "mode": "air", "ratio": 1.2}, {"stations": [1, 2], "mode": "bus", "why": "unmatched"}]);
        let p = priors(&lines, &owner_of, 0, &json!({}));
        assert_eq!(p["bus"]["ours"], json!({"lines": 3, "median": 0.05}));
        assert_eq!(p["bus"]["others"], json!({"lines": 1, "median": 0.9}));
        assert_eq!(p["bus"]["n_eff"], json!(3.5));
        assert_eq!(p["bus"]["median"], json!(0.05), "the other company's line weighs half");
        // Every line: 0.02, 0.05, 0.1 (ours), 0.9, 1.2 (others, half weight): half the weight at 0.05.
        let default = (0.05 + 0.1) / 2.0;
        assert_eq!(p["default"]["n_eff"], json!(4.0));
        assert_eq!(p["default"]["prior"], json!(round2(default)));
        assert_eq!(p["bus"]["prior"], json!(round2((3.5 * 0.05 + 2.0 * default) / 5.5)));
        assert_eq!(p["air"]["prior"], json!(round2((0.5 * 1.2 + 2.0 * default) / 2.5)),
                   "one foreign line moves the map's factor little: an unmeasured mode never beats the measured ones");
        assert_eq!(factor_of(&p, "rail"), Some(round2(default)), "a mode with no line gets the default");
        assert_eq!(factor_of(&Map::new(), "rail"), None, "no line at all: no factor, the model stands");
        assert!(p.get("rail").is_none(), "a mode with no line has no prior");
        // Measured against a plan already calibrated at 0.5 for air: the line earned 0.6 of the model.
        let p = priors(&lines, &owner_of, 0, &json!({"calibration": {"air": 0.5}}));
        assert_eq!(p["air"]["median"], json!(0.6), "a residual ratio is scaled back to the model");
    }

    #[test]
    fn a_rescore_scales_net_profit_relative_to_the_factor_the_plan_used() {
        let c = json!({"mode": "rail", "score": 100000.0, "options": [
            {"mode": "rail", "score": 100000.0, "investment": 50000.0},
            {"mode": "air", "score": 40000.0, "investment": 200000.0},
            {"mode": "bus", "score": f64::MIN.round(), "investment": 1.0}]});
        let prior: Map<String, Value> = [("rail".to_string(), json!({"prior": 0.2})), ("air".to_string(), json!({"prior": 1.0}))].into_iter().collect();
        let r = rescore(&c, &prior, &json!({}));
        assert_eq!(r["options"][0]["score"], json!(-20000.0), "150000 * 0.2 - 50000");
        assert_eq!(r["options"][1]["score"], json!(40000.0));
        assert_eq!(r["options"][2]["score"], c["options"][2]["score"], "an option without a route stays at the floor");
        assert_eq!((r["mode"].as_str(), r["score"].as_f64()), (Some("air"), Some(40000.0)), "the best option now");
        // A plan already calibrated at 0.5 for rail: the prior 0.2 is 0.4 of what it used.
        let r = rescore(&c, &prior, &json!({"calibration": {"rail": 0.5}}));
        assert_eq!(r["options"][0]["score"], json!(10000.0), "150000 * 0.4 - 50000");
    }

    /// The live game of 2026-11-23 in small: the model plans a bus line and a passenger-rail line;
    /// our three bus lines earn a tenth of their prediction, one other company's passenger-rail line
    /// does too, our freight rail earns nearly all of it. Re-scored, the same candidates and budget
    /// choose freight rail, and the verdict is a mismatch that names the bus mode (passenger rail
    /// rests on one foreign line: too little for a signal of its own); a leader earning 1.5x ours
    /// per aircraft is a watch signal.
    #[test]
    fn the_plan_changes_toward_what_the_outcomes_pay_and_the_mismatch_is_named() {
        let cand = |mode: &str, name: &str, score: f64, inv: f64, cargo: Option<&str>| {
            let mut c = json!({"mode": mode, "score": score, "options": [{"mode": mode, "score": score, "investment": inv}]});
            if let Some(cg) = cargo {
                c["cargo"] = json!(cg);
                c["producer"] = json!({"id": name, "name": name});
                c["acceptor"] = json!({"name": "Plant"});
            } else {
                c["a"] = json!({"name": name});
                c["b"] = json!({"name": "Town"});
            }
            c
        };
        let out = json!({
            "params": {"budget": 100000.0},
            "by_mode": {"bus": [cand("bus", "Busville", 60000.0, 40000.0, None)],
                        "rail": [cand("rail", "Railton", 70000.0, 50000.0, None)]},
            "freight": [cand("freight_rail", "Mine A", 50000.0, 50000.0, Some("Coal")),
                        cand("freight_rail", "Mine B", 45000.0, 50000.0, Some("Coal"))],
            "calibration": {"lines": [
                {"stations": [1, 2], "mode": "bus", "ratio": 0.05}, {"stations": [3, 4], "mode": "bus", "ratio": 0.1},
                {"stations": [5, 6], "mode": "bus", "ratio": 0.08}, {"stations": [7, 8], "mode": "rail", "ratio": 0.11},
                {"stations": [9, 10], "mode": "freight_rail", "ratio": 1.0}, {"stations": [11, 12], "mode": "freight_rail", "ratio": 0.9},
                {"stations": [13, 14], "mode": "freight_rail", "ratio": 0.95}]}});
        let mut out = out;
        let c = candidates(&out);
        assert_eq!(c[0]["mode"], "rail", "best score first");
        out["portfolio"] = crate::cargo::portfolio(&c, Some(100000.0), &kernels());
        let mut vs = vec![veh(0, "road", 1, 2, 800.0, 400.0), veh(0, "road", 3, 4, 800.0, 400.0), veh(0, "road", 5, 6, 800.0, 400.0),
                          veh(1, "train", 7, 8, 800.0, 9000.0), veh(0, "train", 9, 10, 800.0, 30000.0),
                          veh(0, "train", 11, 12, 800.0, 30000.0), veh(0, "train", 13, 14, 800.0, 30000.0)];
        for i in 0..3 { vs.push(veh(0, "aircraft", 20 + 2 * i, 21 + 2 * i, 800.0, 12000.0)); }
        for i in 0..4 { vs.push(veh(1, "aircraft", 40 + 2 * i, 41 + 2 * i, 800.0, 18000.0)); }
        vs.push(veh(2, "aircraft", 60, 61, 800.0, 90000.0));
        let vehicles = json!({"vehicles": vs});
        let companies = json!({"companies": [{"id": 0, "name": "Ours"}, {"id": 1, "name": "Leader"}]});
        let a = assess(&out, &vehicles, &companies, 0, &kernels());
        assert_eq!(a["plan"]["model"]["mix"], json!({"bus": 1, "rail": 1}), "the model plans bus and passenger rail");
        assert_eq!(a["plan"]["outcome"]["mix"], json!({"freight_rail": 2}), "the outcomes pay freight rail");
        assert_eq!(a["plan"]["outcome"]["lines"][0], "freight_rail Mine A -> Plant (Coal)");
        assert_eq!(a["modes"]["default"]["prior"], json!(0.11), "every line: 0.05 0.08 0.1 | 0.11 (half) | 0.9 0.95 1.0");
        assert_eq!(a["modes"]["freight_rail"]["prior"], json!(round2((3.0 * 0.95 + 2.0 * 0.11) / 5.0)));
        assert_eq!(a["verdict"], "mismatch");
        let kinds: Vec<&str> = a["signals"].as_array().unwrap().iter().map(|s| s["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["overpredicted_mode_planned", "outcome_plan_differs", "leader_out_earns"]);
        let d = &a["signals"][1];
        assert!(d["gain"].as_f64().unwrap() >= MATERIAL_GAIN * d["model_score_under_prior"].as_f64().unwrap().abs(), "{d}");
        assert_eq!(a["plan"]["model"]["score_under_prior"], d["model_score_under_prior"]);
        assert_eq!(a["signals"][0]["mode"], "bus", "passenger rail rests on one foreign line: no evidence for a signal");
        assert_eq!(a["signals"][2]["type"], "aircraft");
        assert_eq!(a["signals"][2]["leader"]["company"], 1, "company 2 has one aircraft, fewer than MIN_FLEET");
        assert_eq!(a["signals"][2]["leader"]["name"], "Leader");
        assert_eq!(a["signals"][2]["edge"], json!(1.5));
        assert_eq!(a["proposal"]["applied"], false);
        assert_eq!(a["proposal"]["params"]["calibration"]["bus"], a["modes"]["bus"]["prior"]);
        assert!(a["plan"]["outcome"]["solver"].as_str().unwrap().starts_with("builtin"), "the fallback is labelled, never as the kernel: {}", a["plan"]["outcome"]["solver"]);
    }

    #[test]
    fn without_measured_lines_the_verdict_is_insufficient_and_the_plan_stays() {
        let out = json!({"params": {"budget": 100000.0}, "by_mode": {"bus": [{"mode": "bus", "score": 10.0, "a": {"name": "A"}, "b": {"name": "B"},
                         "options": [{"mode": "bus", "score": 10.0, "investment": 1000.0}]}]}, "calibration": {"lines": []}});
        let mut out = out;
        out["portfolio"] = crate::cargo::portfolio(&candidates(&out), Some(100000.0), &kernels());
        let a = assess(&out, &json!({"vehicles": []}), &Value::Null, 0, &kernels());
        assert_eq!(a["verdict"], "insufficient");
        assert_eq!(a["plan"]["model"]["mix"], a["plan"]["outcome"]["mix"]);
        assert_eq!(a["signals"], json!([]));
        assert_eq!(a["mix"], json!([]));
    }

    /// RESEARCH-ONLY: the assessment of a saved plan answer and vehicles (OUTCOME_PLAN, a GET
    /// /api/plan?calibrate=1 answer; OUTCOME_VEHICLES, a GET /api/resource?uri=openttd://vehicles
    /// answer), printed: `cargo test --bin mcp-plan outcome::tests::saved -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn saved() {
        let read = |k: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(std::env::var(k).expect(k)).unwrap()).unwrap() };
        let out = read("OUTCOME_PLAN");
        let v = read("OUTCOME_VEHICLES");
        let v = if v.get("data").is_some() { v["data"].clone() } else { v };
        let company = std::env::var("OUTCOME_COMPANY").ok().and_then(|c| c.parse().ok()).unwrap_or(0);
        println!("{}", serde_json::to_string_pretty(&assess(&out, &v, &Value::Null, company, &kernels())).unwrap());
    }
}
