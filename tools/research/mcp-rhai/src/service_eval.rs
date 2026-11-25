//! RESEARCH-ONLY: the panel's services judged by what followed their actions
//! (research/mechanisms/service-self-evaluation.md). GET /api/services/scores.
//!
//! The actions are not written by the services for this: the store's view `service_actions` derives
//! them from the shared-memory writes the services already make (a replan proposal or hold, a
//! watchdog stall alert, a build_audit defect flag, a rail_monitor advice flag, a gap_watch alert).
//! The named query `service_outcomes` joins each to the company's state `horizon` game months later;
//! `service_coverage` asks the converse, from deterioration episodes derived from the samples alone:
//! did a service act while the company lost ground; `baselines` gives the percentiles the episode
//! threshold is derived from. Here those answers become one score per service and action:
//!
//! - a detector's action (stall_alert, defect_flag, gap_alert) by its resolution: was it followed,
//!   within the horizon, by the service's own measure getting better (progress, fewer defects, a
//!   lower gap level);
//! - any other action (replan's proposal and hold, rail_monitor's advice_flag) by its relative
//!   outcome: the company's share of the field's value over the horizon against its own pre-action
//!   trend.
//!
//! Each count gets a Jeffreys Beta posterior; `p_below_half` is its probability that the success
//! rate is under one half. The verdict is `insufficient` below MIN_N outcomes, `poor` when
//! p_below_half reaches POOR_P (and, for a non-detector, the median relative outcome is negative: a
//! practical deficit), `helps` at the mirror image, else `unclear`. The outcomes are associative, not
//! causal: an alert that fires when things are already bad may be followed by a recovery that would
//! have come anyway, horizons of successive actions overlap, and every company of a run shares the
//! run (`runs` says how many independent games the count rests on). The answer says so in `caveat`.
//!
//! The service that keeps the verdict over time, the poor streak and the log line is
//! mcp-panel/rhai/service_eval.rhai; this route judges and never disables or changes anything.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::panel::{Game, Input, Query};
use crate::store_sql::Named;

/// Game months between an action's state and the outcome it is judged by, unless ?horizon= says.
/// A design choice of the evaluation, not a threshold the data gives.
pub const HORIZON: i64 = 3;
/// Outcomes below which a verdict is `insufficient`. Cold-start constant.
pub const MIN_N: usize = 8;
/// The posterior probability of a success rate under one half from which an action is `poor`.
pub const POOR_P: f64 = 0.9;
/// Months of gap_watch.rhai's WINDOW, which the baseline leader_ratio_drop is measured over.
pub const GAP_WINDOW: i64 = 6;
/// The monthly share drop that makes a deterioration episode: the 90th percentile of the drops the
/// store saw before the latest run (`baselines` share_drop) once it holds DROP_MIN_N of them; until
/// then DROP_FALLBACK, the declared cold-start value.
pub const DROP_FALLBACK: f64 = 0.01;
pub const DROP_MIN_N: i64 = 30;
/// Months before an episode's first month in which an action still counts as a reaction to it.
pub const LEAD: i64 = 1;
/// The actions judged by their resolution; every other one by its relative outcome.
pub const DETECTORS: [&str; 3] = ["stall_alert", "defect_flag", "gap_alert"];

const CAVEAT: &str = "associative, not causal: an outcome followed the action, it was not shown to be caused by it; \
horizons overlap and the companies of one run share it (see runs)";

/// The rows of a store answer `{columns, rows}` as objects by column name.
pub fn objects(answer: &Value) -> Vec<Map<String, Value>> {
    let cols: Vec<&str> = answer["columns"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    answer["rows"].as_array().into_iter().flatten().filter_map(Value::as_array)
        .map(|r| cols.iter().zip(r).map(|(c, v)| (c.to_string(), v.clone())).collect()).collect()
}

fn ln_gamma(x: f64) -> f64 {
    // Lanczos, g = 7, n = 9.
    const C: [f64; 9] = [0.999_999_999_999_809_9, 676.520_368_121_885_1, -1_259.139_216_722_402_8,
                         771.323_428_777_653_1, -176.615_029_162_140_6, 12.507_343_278_686_905,
                         -0.138_571_095_265_720_12, 9.984_369_578_019_572e-6, 1.505_632_735_149_311_6e-7];
    if x < 0.5 {
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let t = x + 7.5;
    let a = C[1..].iter().enumerate().fold(C[0], |a, (i, c)| a + c / (x + i as f64 + 1.0));
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// One guarded step of the modified Lentz recurrence.
fn lentz(v: f64) -> f64 {
    if v.abs() < 1e-300 { 1e-300 } else { v }
}

/// The continued fraction of the incomplete beta (modified Lentz).
fn beta_cf(x: f64, a: f64, b: f64) -> f64 {
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 / lentz(1.0 - qab * x / qap);
    let mut h = d;
    for m in 1..300 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 / lentz(1.0 + aa * d);
        c = lentz(1.0 + aa / c);
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 / lentz(1.0 + aa * d);
        c = lentz(1.0 + aa / c);
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-12 { break; }
    }
    h
}

/// P(X <= x) for X ~ Beta(a, b): the regularised incomplete beta function.
pub fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 { return 0.0; }
    if x >= 1.0 { return 1.0; }
    let front = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) { front * beta_cf(x, a, b) / a } else { 1.0 - front * beta_cf(1.0 - x, b, a) / b }
}

fn median(mut xs: Vec<f64>) -> Option<f64> {
    if xs.is_empty() { return None; }
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    Some(if n % 2 == 1 { xs[n / 2] } else { (xs[n / 2 - 1] + xs[n / 2]) / 2.0 })
}

fn round(x: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (x * f).round() / f
}

/// One score per service and action from the rows of `service_outcomes`, in service, action order.
pub fn scores(outcomes: &[Map<String, Value>]) -> Vec<Value> {
    let mut groups: BTreeMap<(String, String), Vec<&Map<String, Value>>> = BTreeMap::new();
    for r in outcomes {
        let key = (r["service"].as_str().unwrap_or("?").to_string(), r["action"].as_str().unwrap_or("?").to_string());
        groups.entry(key).or_default().push(r);
    }
    groups.into_iter().map(|((service, action), rows)| {
        let detector = DETECTORS.contains(&action.as_str());
        let mut status: BTreeMap<String, usize> = BTreeMap::new();
        for r in &rows { *status.entry(r["status"].as_str().unwrap_or("?").to_string()).or_default() += 1; }
        let scored: Vec<&Map<String, Value>> = rows.iter().copied().filter(|r| r["status"] == "scored").collect();
        let relative: Vec<f64> = scored.iter().filter_map(|r| r["relative_outcome"].as_f64()).collect();
        // The endpoint: a detector's resolution where it was measured, else the relative outcome.
        let judged: Vec<(bool, String)> = if detector {
            rows.iter().filter_map(|r| r["resolved"].as_bool().map(|b| (b, r["run_ms"].to_string()))).collect()
        } else {
            scored.iter().filter_map(|r| r["relative_outcome"].as_f64().map(|x| (x > 0.0, r["run_ms"].to_string()))).collect()
        };
        let n = judged.len();
        let k = judged.iter().filter(|(ok, _)| *ok).count();
        let runs: BTreeSet<&String> = judged.iter().map(|(_, run)| run).collect();
        let (a, b) = (k as f64 + 0.5, (n - k) as f64 + 0.5);
        let p_below = beta_cdf(0.5, a, b);
        let med = median(relative);
        let deficit = detector || med.is_some_and(|m| m < 0.0);
        let surplus = detector || med.is_some_and(|m| m > 0.0);
        let verdict = if n < MIN_N { "insufficient" }
            else if p_below >= POOR_P && deficit { "poor" }
            else if p_below <= 1.0 - POOR_P && surplus { "helps" }
            else { "unclear" };
        let days = median(rows.iter().filter_map(|r| r["resolved_days"].as_f64()).collect());
        let changes: Vec<f64> = scored.iter().filter_map(|r| r["value_change"].as_f64()).collect();
        json!({
            "service": service, "action": action,
            "endpoint": if detector { "resolution" } else { "relative_outcome" },
            "expected": rows.iter().find_map(|r| r["expected"].as_str()),
            "actions": rows.len(), "status": status, "n": n, "successes": k, "runs": runs.len(),
            "rate": if n == 0 { Value::Null } else { json!(round(k as f64 / n as f64, 4)) },
            "posterior": {"prior": "jeffreys", "a": a, "b": b, "mean": round(a / (a + b), 4), "p_below_half": round(p_below, 4)},
            "median_relative_outcome": med.map(|m| round(m, 6)),
            "mean_value_change": if changes.is_empty() { Value::Null } else { json!(round(changes.iter().sum::<f64>() / changes.len() as f64, 0)) },
            "median_resolved_days": days,
            "verdict": verdict,
        })
    }).collect()
}

/// A threshold from a `baselines` row: `{value, source, metric, percentile, n, runs, min_n,
/// fallback, query}`, `source` derived when the metric has at least `min_n` values, else fallback.
pub fn derived(baselines: &[Map<String, Value>], metric: &str, pct: &str, fallback: f64, min_n: i64) -> Value {
    let row = baselines.iter().find(|r| r["metric"] == metric);
    let n = row.and_then(|r| r["n"].as_i64()).unwrap_or(0);
    let v = row.and_then(|r| r[pct].as_f64()).filter(|_| n >= min_n);
    json!({"value": v.unwrap_or(fallback), "source": if v.is_some() { "derived" } else { "fallback" },
           "metric": metric, "percentile": pct, "n": n, "runs": row.and_then(|r| r["runs"].as_i64()).unwrap_or(0),
           "min_n": min_n, "fallback": fallback, "query": "baselines"})
}

/// The whole answer of GET /api/services/scores from what `run` answers for a named query, the
/// panel's `eval.*` shared keys and the request's query: ?horizon= (1..24, else HORIZON) and
/// ?scope=run (the newest run with an action only) or all (default).
pub fn answer(run: impl Fn(&Named) -> (u16, Value), eval: Vec<(String, Value)>, q: &Query) -> (u16, Value) {
    let horizon = q.get("horizon").and_then(|h| h.parse::<i64>().ok()).filter(|h| (1..=24).contains(h)).unwrap_or(HORIZON);
    let (code, base) = run(&Named::new("baselines", json!({"months": GAP_WINDOW})));
    if code != 200 { return (code, base); }
    let base = objects(&base);
    let drop = derived(&base, "share_drop", "p90", DROP_FALLBACK, DROP_MIN_N);
    let (code, out) = run(&Named::new("service_outcomes", json!({"horizon": horizon})));
    if code != 200 { return (code, out); }
    let mut rows = objects(&out);
    let (code, cov) = run(&Named::new("service_coverage", json!({"drop": drop["value"], "lead": LEAD})));
    if code != 200 { return (code, cov); }
    let mut coverage = objects(&cov);
    let scope = if q.get("scope") == Some("run") { "run" } else { "all" };
    if scope == "run" {
        let newest = rows.iter().filter_map(|r| r["run_ms"].as_i64()).max();
        rows.retain(|r| r["run_ms"].as_i64() == newest);
        coverage.retain(|r| r["run_ms"].as_i64() == newest);
    }
    let scored = scores(&rows);
    let poor: Vec<String> = scored.iter().filter(|s| s["verdict"] == "poor")
        .map(|s| format!("{}.{}", s["service"].as_str().unwrap_or("?"), s["action"].as_str().unwrap_or("?"))).collect();
    (200, json!({
        "horizon": horizon, "scope": scope, "caveat": CAVEAT,
        "rule": {"min_n": MIN_N, "poor_p": POOR_P, "detectors": DETECTORS,
                 "poor": "p_below_half >= poor_p and (a detector, or the median relative outcome < 0)"},
        "scores": scored, "poor": poor, "actions": rows.len(),
        "coverage": {"drop": drop, "lead_months": LEAD, "runs": coverage.into_iter().map(Value::Object).collect::<Vec<_>>()},
        "baselines": base.into_iter().map(Value::Object).collect::<Vec<_>>(),
        "eval": eval.into_iter().collect::<Map<String, Value>>(),
    }))
}

/// GET /api/services/scores.
pub fn h_scores(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let eval = crate::rhai_shared::shared().list("eval.").into_iter().map(|(k, e)| (k, e.value)).collect();
    Ok(answer(crate::panel_store::run_named, eval, &i.query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_beta_cdf_matches_closed_forms() {
        // Beta(1, 1) is uniform; Beta(2, 1) has CDF x^2; Beta(1, 2) has 1 - (1 - x)^2.
        for x in [0.1, 0.25, 0.5, 0.9] {
            assert!((beta_cdf(x, 1.0, 1.0) - x).abs() < 1e-9, "{x}");
            assert!((beta_cdf(x, 2.0, 1.0) - x * x).abs() < 1e-9, "{x}");
            assert!((beta_cdf(x, 1.0, 2.0) - (1.0 - (1.0 - x) * (1.0 - x))).abs() < 1e-9, "{x}");
        }
        assert_eq!((beta_cdf(0.0, 2.0, 2.0), beta_cdf(1.0, 2.0, 2.0), beta_cdf(-1.0, 2.0, 2.0)), (0.0, 1.0, 0.0));
        // Symmetric: half the mass under one half; Jeffreys with 0 of 10 is almost surely under.
        assert!((beta_cdf(0.5, 5.5, 5.5) - 0.5).abs() < 1e-9);
        assert!(beta_cdf(0.5, 0.5, 10.5) > 0.99);
        assert!(beta_cdf(0.5, 10.5, 0.5) < 0.01);
        assert!((ln_gamma(0.25) - 1.288_022_524_698_077_5).abs() < 1e-9, "the reflection branch");
        assert_eq!((lentz(0.0), lentz(2.0)), (1e-300, 2.0));
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![3.0, 1.0]), Some(2.0));
    }

    fn row(service: &str, action: &str, status: &str, rel: Option<f64>, resolved: Option<bool>, run: i64) -> Map<String, Value> {
        json!({"service": service, "action": action, "status": status, "relative_outcome": rel, "resolved": resolved,
               "run_ms": run, "value_change": 10.0, "resolved_days": resolved.map(|_| 20), "expected": "x"})
            .as_object().unwrap().clone()
    }

    #[test]
    fn a_detector_is_judged_by_resolution_and_a_planner_by_its_relative_outcome() {
        let mut rows = Vec::new();
        // Ten stall alerts, one resolved: poor whatever the share did.
        for i in 0..10 { rows.push(row("watchdog", "stall_alert", "scored", Some(0.01), Some(i == 0), 1 + i % 2)); }
        rows.push(row("watchdog", "stall_alert", "pending", None, None, 2));
        // Ten holds that all fell below their trend: poor; ten proposals all above it: helps.
        for _ in 0..10 { rows.push(row("replan", "hold", "scored", Some(-0.02), None, 1)); }
        for _ in 0..10 { rows.push(row("replan", "proposal", "scored", Some(0.03), None, 1)); }
        // Few advice flags: insufficient; mixed defect flags: unclear.
        rows.push(row("rail_monitor", "advice_flag", "scored", Some(-0.1), None, 1));
        for i in 0..10 { rows.push(row("build_audit", "defect_flag", "scored", Some(0.0), Some(i % 2 == 0), 1)); }
        let s = scores(&rows);
        let by = |svc: &str, act: &str| s.iter().find(|x| x["service"] == svc && x["action"] == act).unwrap().clone();
        let w = by("watchdog", "stall_alert");
        assert_eq!((w["endpoint"].clone(), w["n"].clone(), w["successes"].clone(), w["runs"].clone(), w["verdict"].clone()),
                   (json!("resolution"), json!(10), json!(1), json!(2), json!("poor")), "{w}");
        assert_eq!(w["status"], json!({"pending": 1, "scored": 10}));
        assert_eq!((w["actions"].clone(), w["median_resolved_days"].clone(), w["mean_value_change"].clone()),
                   (json!(11), json!(20.0), json!(10.0)));
        assert_eq!(by("replan", "hold")["verdict"], "poor");
        assert_eq!(by("replan", "hold")["median_relative_outcome"], json!(-0.02));
        assert_eq!(by("replan", "proposal")["verdict"], "helps");
        assert_eq!(by("rail_monitor", "advice_flag")["verdict"], "insufficient");
        assert_eq!(by("build_audit", "defect_flag")["verdict"], "unclear");
        assert_eq!(by("build_audit", "defect_flag")["rate"], json!(0.5));
        // A planner whose outcomes are all exactly on trend has no deficit: never poor.
        let flat: Vec<_> = (0..10).map(|_| row("replan", "hold", "scored", Some(0.0), None, 1)).collect();
        assert_eq!(scores(&flat)[0]["verdict"], "unclear");
        let none = scores(&[row("replan", "hold", "ended", None, None, 1)]);
        assert_eq!((none[0]["rate"].clone(), none[0]["mean_value_change"].clone()), (Value::Null, Value::Null));
    }

    #[test]
    fn a_threshold_is_derived_once_the_store_holds_enough_else_the_declared_fallback() {
        let base = objects(&json!({"columns": ["metric", "n", "runs", "p50", "p75", "p90"],
                                   "rows": [["share_drop", 40, 3, 0.001, 0.002, 0.005], ["quiet_days", 4, 1, 5.0, 9.0, 30.0]]}));
        let d = derived(&base, "share_drop", "p90", 0.01, 30);
        assert_eq!((d["value"].clone(), d["source"].clone(), d["n"].clone(), d["runs"].clone()),
                   (json!(0.005), json!("derived"), json!(40), json!(3)));
        let f = derived(&base, "quiet_days", "p90", 7.0, 5);
        assert_eq!((f["value"].clone(), f["source"].clone()), (json!(7.0), json!("fallback")));
        assert_eq!(derived(&[], "nothing", "p50", 1.5, 1)["source"], "fallback");
    }

    /// The real SQL end to end on an in-memory store: two runs of monthly samples, replan and
    /// watchdog writes; the actions are derived, joined to their outcomes, scored, the deterioration
    /// episodes found, and a failing query is answered as it came.
    #[cfg(feature = "store")]
    #[test]
    fn the_route_scores_the_stores_actions_end_to_end() {
        use crate::bus_sinks::{db::Store, envelope, sample_rows};
        let mut st = Store::open_in_memory("panel-e").unwrap();
        let shared = |seq: i64, ts: i64, key: &str, date: &str, value: Value| envelope(&json!({"seq": seq, "kind": "rhai.shared",
            "data": {"op": "set", "key": key, "rev": seq, "writer": "t", "value": value}}),
            "panel-e", &json!("s"), Some(7), ts, Some(&json!({"t": 1, "date": date})));
        let mut seq = 0;
        for (run, start) in [(1i64, 1_000i64), (2, 1_000_000)] {
            st.insert_run(&json!("s"), Some(run), start, (None, None, None)).unwrap();
            let mut envs = Vec::new();
            // 14 months; company 0 loses share from month 4 on and stops buying, company 1 gains.
            for m in 0..14i64 {
                let date = format!("{}-{:02}-28", 1950 + (m / 12), m % 12 + 1);
                let (a, b) = if m < 4 { (100.0, 100.0) } else { (100.0 - 5.0 * (m - 3) as f64, 100.0 + 10.0 * (m - 3) as f64) };
                let ts = start + 10 + m * 10;
                let s = json!({"t": 74 * 30 * m, "date": date, "companies": [
                    {"id": 0, "name": "Ours", "value": a, "money": a, "fleet": if m < 4 { m } else { 4 }},
                    {"id": 1, "name": "Them", "value": b, "money": b, "fleet": m}]});
                st.insert_samples(&sample_rows(&s, ts)).unwrap();
                seq += 1;
                envs.push(shared(seq, ts + 1, "replan.0", &date, json!({"pending": if m == 5 { json!([3]) } else { json!([]) }})));
                seq += 1;
                envs.push(shared(seq, ts + 2, "watchdog.0", &date, json!({"stalled": (6..9).contains(&m), "quiet_days": 10})));
            }
            st.insert_events(&envs).unwrap();
        }
        let r = st.reader().unwrap();
        let run = |q: &Named| match r.query_bound(q.sql(), &q.params) { Ok(v) => (200, v), Err(e) => (400, json!({"error": e})) };
        let q = |sql: &str| r.query(sql).unwrap()["rows"].clone();
        assert_eq!(q("SELECT service, action, count(*) FROM service_actions GROUP BY ALL ORDER BY ALL"),
                   json!([["replan", "hold", 26], ["replan", "proposal", 2], ["watchdog", "stall_alert", 2]]));
        assert_eq!(q("SELECT month, round(share, 4), gap, vehicles FROM service_actions WHERE action = 'stall_alert' AND run_ms = 1000"),
                   json!([["1950-07", 0.4286, 30.0, 4]]), "the state of the month before the action");
        let eval = vec![("eval.replan".to_string(), json!({"verdict": "poor", "streak": 3}))];
        let (code, a) = answer(run, eval, &Query::parse("horizon=4"));
        assert_eq!(code, 200, "{a}");
        assert_eq!((a["horizon"].clone(), a["scope"].clone(), a["actions"].clone()), (json!(4), json!("all"), json!(30)));
        let s = |act: &str| a["scores"].as_array().unwrap().iter().find(|x| x["action"] == act).unwrap().clone();
        // The stall of 1950-07 ends in 1950-10, within four months of its state (1950-06): resolved.
        let w = s("stall_alert");
        assert_eq!((w["n"].clone(), w["successes"].clone(), w["runs"].clone(), w["median_resolved_days"].clone()),
                   (json!(2), json!(2), json!(2), json!(92.0)), "{w}");
        let (_, short) = answer(run, vec![], &Query::parse("horizon=2"));
        let w2 = short["scores"].as_array().unwrap().iter().find(|x| x["action"] == "stall_alert").unwrap().clone();
        assert_eq!((w2["n"].clone(), w2["successes"].clone()), (json!(2), json!(0)), "not within two months: {w2}");
        assert_eq!(s("hold")["endpoint"], "relative_outcome");
        assert!(s("hold")["status"]["scored"].as_u64().unwrap() > 0, "{}", s("hold"));
        assert_eq!(a["eval"]["eval.replan"]["streak"], 3);
        // The baselines come from the first run only, never the latest.
        let base = a["baselines"].as_array().unwrap();
        let quiet = base.iter().find(|b| b["metric"] == "quiet_days").unwrap();
        assert_eq!(quiet["runs"], 1, "{quiet}");
        assert_eq!(a["coverage"]["drop"]["source"], "fallback", "{}", a["coverage"]["drop"]);
        let cov = a["coverage"]["runs"].as_array().unwrap();
        let wd = cov.iter().find(|c| c["service"] == "watchdog" && c["run_ms"] == 1000).unwrap();
        assert_eq!((wd["episodes"].clone(), wd["covered"].clone()), (json!(1), json!(1)), "{wd}");
        let rp = cov.iter().find(|c| c["service"] == "replan" && c["run_ms"] == 1000).unwrap();
        assert_eq!((rp["actions"].clone(), rp["covered"].clone()), (json!(1), json!(1)), "the proposal of month 6 reacted: {rp}");
        // ?scope=run keeps the newest run only; a bad horizon falls back to the default.
        let (_, one) = answer(run, vec![], &Query::parse("scope=run&horizon=99"));
        assert_eq!((one["scope"].clone(), one["horizon"].clone(), one["actions"].clone()), (json!("run"), json!(HORIZON), json!(15)));
        assert!(one["coverage"]["runs"].as_array().unwrap().iter().all(|c| c["run_ms"] == 1_000_000));
        // Each query's failure is answered as it came.
        for name in ["baselines", "service_outcomes", "service_coverage"] {
            let failing = |q: &Named| if q.name == name { (503, json!({"error": "store off"})) } else { run(q) };
            assert_eq!(answer(failing, vec![], &Query::default()), (503, json!({"error": "store off"})), "{name}");
        }
    }

    /// The answer assembled from canned store answers (no DuckDB needed): the derived coverage
    /// threshold is passed to the coverage query, ?scope=run keeps the newest run, the poor list
    /// names service.action; and the route itself without a store answers the store's 503.
    #[test]
    fn the_answer_is_assembled_from_the_three_queries() {
        let seen = std::sync::Mutex::new(Vec::<Value>::new());
        let run = |q: &Named| -> (u16, Value) {
            seen.lock().unwrap().push(q.record()["params"].clone());
            match q.name {
                "baselines" => (200, json!({"columns": ["metric", "n", "runs", "p50", "p75", "p90"],
                                            "rows": [["share_drop", 31, 2, 0.001, 0.002, 0.004]]})),
                "service_outcomes" => {
                    let mut rows = vec![json!(["replan", "hold", "scored", -0.01, null, 1, 5.0, null, "x"]); 10];
                    rows.push(json!(["replan", "proposal", "pending", null, null, 2, null, null, "y"]));
                    (200, json!({"columns": ["service", "action", "status", "relative_outcome", "resolved", "run_ms", "value_change", "resolved_days", "expected"],
                                 "rows": rows}))
                }
                _ => (200, json!({"columns": ["service", "run_ms", "episodes", "covered"],
                                  "rows": [["replan", 1, 3, 0], ["replan", 2, 2, 1]]})),
            }
        };
        let eval = vec![("eval.poor".to_string(), json!({"poor": []}))];
        let (code, a) = answer(&run, eval, &Query::default());
        assert_eq!(code, 200);
        assert_eq!(*seen.lock().unwrap(), vec![json!({"months": GAP_WINDOW}), json!({"horizon": HORIZON}), json!({"drop": 0.004, "lead": LEAD})]);
        assert_eq!((a["actions"].clone(), a["poor"].clone(), a["coverage"]["drop"]["source"].clone()),
                   (json!(11), json!(["replan.hold"]), json!("derived")));
        assert_eq!((a["eval"]["eval.poor"].clone(), a["caveat"].as_str().unwrap().starts_with("associative")), (json!({"poor": []}), true));
        let (_, one) = answer(&run, vec![], &Query::parse("scope=run&horizon=6"));
        assert_eq!((one["actions"].clone(), one["horizon"].clone(), one["poor"].clone()), (json!(1), json!(6), json!([])));
        assert_eq!(one["coverage"]["runs"], json!([{"service": "replan", "run_ms": 2, "episodes": 2, "covered": 1}]));
        // The route on a panel without a store: the store's own answer.
        let g = crate::panel::Game::new(crate::panel::parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", "21"].map(String::from)).unwrap());
        let (code, out) = h_scores(&g, &Input { query: Query::parse("horizon=2"), body: Value::Null, who: crate::panel::Who::none() }).unwrap();
        assert_eq!(code, 503, "{out}");
        assert!(out["error"].as_str().unwrap().starts_with("store off"), "{out}");
    }

    /// The baselines read comparable earlier runs only: a loaded continuation (its first sample
    /// within a month of the previous run's last) and a run of another lineup size are left out,
    /// and so is the latest run itself.
    #[cfg(feature = "store")]
    #[test]
    fn the_baselines_leave_out_loaded_runs_other_lineups_and_the_latest_run() {
        use crate::bus_sinks::{db::Store, sample_rows};
        let mut st = Store::open_in_memory("panel-b").unwrap();
        // (run start, first month index from 1950-01, months, companies)
        let runs = [(1_000i64, 0i64, 12i64, 2usize), (2_000, 11, 12, 2), (3_000, 0, 12, 8), (4_000, 0, 12, 2), (5_000, 0, 3, 2)];
        for (start, first, months, companies) in runs {
            st.insert_run(&json!("s"), Some(start), start, (None, None, None)).unwrap();
            for m in 0..months {
                let mi = first + m;
                let date = format!("{}-{:02}-15", 1950 + mi / 12, mi % 12 + 1);
                // Every company buys a vehicle every second month: quiet spells of 60 days.
                let cs: Vec<Value> = (0..companies).map(|c| json!({"id": c, "name": "C", "value": 100.0 + (c as f64) * 10.0 - m as f64,
                                                                    "money": 1.0, "fleet": m / 2})).collect();
                st.insert_samples(&sample_rows(&json!({"t": 74 * 30 * mi, "date": date, "companies": cs}), start + 1 + m)).unwrap();
            }
        }
        let r = st.reader().unwrap();
        let out = r.query_bound(crate::store_sql::query("baselines").unwrap(), json!({"months": 2}).as_object().unwrap()).unwrap();
        let rows = objects(&out);
        let quiet = rows.iter().find(|r| r["metric"] == "quiet_days").unwrap();
        // Runs 1 000 and 4 000 only: 2 000 continues 1 000, 3 000 has eight companies, 5 000 is the latest.
        assert_eq!((quiet["runs"].clone(), quiet["p50"].clone()), (json!(2), json!(60.0)), "{rows:?}");
        assert_eq!(quiet["n"], json!(2 * 2 * 4), "two runs, two companies, four completed spells each (the first is censored)");
        assert!(rows.iter().any(|r| r["metric"] == "share_drop" && r["runs"] == 2), "{rows:?}");
    }

    // ------------------------------------------------------------------ the Rhai side

    type Shared = std::sync::Arc<std::sync::Mutex<BTreeMap<String, Value>>>;

    fn engine() -> rhai::Engine {
        let mut e = rhai::Engine::new();
        e.set_module_resolver(rhai::module_resolvers::FileModuleResolver::new_with_path(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel/rhai")));
        e
    }

    fn dynv(v: Value) -> rhai::Dynamic {
        rhai::serde::to_dynamic(v).unwrap()
    }

    fn rhai_file(name: &str) -> String {
        std::fs::read_to_string(format!("{}/../mcp-panel/rhai/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    /// The blackboard and the printed lines of a scripted run.
    fn board(e: &mut rhai::Engine) -> (Shared, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let shared: Shared = Default::default();
        let s = shared.clone();
        e.register_fn("shared_get", move |k: &str| s.lock().unwrap().get(k).cloned().map_or(rhai::Dynamic::UNIT, dynv));
        let s = shared.clone();
        e.register_fn("shared_set", move |k: &str, v: rhai::Dynamic| -> i64 {
            s.lock().unwrap().insert(k.to_string(), rhai::serde::from_dynamic(&v).unwrap());
            1
        });
        let printed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let p = printed.clone();
        e.on_print(move |t| p.lock().unwrap().push(t.to_string()));
        (shared, printed)
    }

    fn baselines_answer() -> Value {
        json!({"columns": ["metric", "n", "runs", "p50", "p75", "p90"],
               "rows": [["quiet_days", 50, 3, 6.0, 13.0, 34.0], ["leader_ratio_drop", 40, 2, 0.002, 0.012, 0.043],
                        ["share_drop", 5, 1, 0.0003, 0.0013, 0.005]]})
    }

    /// derive_lib: a threshold is the percentile times the factor within bounds once the store holds
    /// enough values, else the declared fallback with the reason; a store that cannot answer, or a
    /// panel without the function, gives no rows.
    #[test]
    fn derive_lib_derives_a_threshold_or_says_why_it_falls_back() {
        let mut e = engine();
        e.register_fn("api_dbNamedRun", |q: rhai::Map| -> Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
            match q["params"].clone_cast::<rhai::Map>()["months"].as_int().unwrap() {
                6 => Ok(dynv(baselines_answer())),
                1 => Ok(dynv(json!({"error": "store off"}))),
                _ => Err("no panel".into()),
            }
        });
        let out: rhai::Array = e.eval(r#"
            import "derive_lib" as dl;
            let rows = dl::baselines(6);
            let d = dl::pick(rows, "quiet_days", "p90", 1.5, 7, 20, 7, 365);
            let low = dl::pick(rows, "quiet_days", "p50", 0.1, 7, 20, 7, 365);
            let high = dl::pick(rows, "quiet_days", "p90", 100.0, 7, 20, 7, 365);
            let few = dl::pick(rows, "share_drop", "p90", 1.0, 0.01, 30, 0.0, 1.0);
            let none = dl::pick(dl::baselines(1), "quiet_days", "p90", 1.5, 7, 20, 7, 365);
            [rows.len(), d.value, d.source, d.n, d.runs, d.why, low.value, high.value, few.value, few.source, few.why,
             none.source, none.why, type_of(dl::baselines(2))]
        "#).unwrap();
        let got: Vec<String> = out.iter().map(|x| x.to_string()).collect();
        assert_eq!(got, vec!["3", "51.0", "derived", "50", "3", "1.5 x p90 34.0 of 50 value(s) over 3 earlier run(s), within 7..365",
                             "7", "365", "0.01", "fallback", "5 value(s) in the store, fewer than 30: the declared fallback 0.01",
                             "fallback", "the store did not answer", "()"]);
    }

    /// The watchdog's cold start comes from the store's quiet spells (whole days, bounded), is kept
    /// in watchdog.tuning with its source, and is the threshold until MIN_GAPS gaps of its own.
    /// A watchdog run of two looks against a scripted panel whose store answers `store`.
    fn watchdog_run(store: Value) -> (BTreeMap<String, Value>, Vec<String>) {
        let mut e = engine();
        let (shared, printed) = board(&mut e);
        let looks = std::sync::Arc::new(std::sync::Mutex::new(0i64));
        e.register_fn("env", |name: &str| -> String { if name == "WATCHDOG_ROUNDS" { "2".into() } else { String::new() } });
        let l = looks.clone();
        e.register_fn("sleep", move |_ms: i64| { *l.lock().unwrap() += 1; });
        let l = looks.clone();
        e.register_fn("read", move |uri: &str| -> rhai::Dynamic {
            let n = *l.lock().unwrap();
            match uri {
                "openttd://game/state" => dynv(json!({"game_tick": 74 * (100 + 60 * n), "game_date": "1951-01-01"})),
                "openttd://stations" => dynv(json!({"data": {"stations": [{"owner": 0}]}})),
                _ => dynv(json!({"data": {"companies": [{"id": 0, "name": "Ours", "money": 1, "fleet": {"road": 2}}]}})),
            }
        });
        e.register_fn("status", || dynv(json!({"pending": []})));
        e.register_fn("api_services", || dynv(json!({"services": []})));
        e.register_fn("api_bots", || dynv(json!({"running": []})));
        e.register_fn("con_say", |_: &str| {});
        e.register_fn("api_dbNamedRun", move |_: rhai::Map| dynv(store.clone()));
        e.run(&rhai_file("watchdog.rhai")).unwrap();
        let shared = shared.lock().unwrap().clone();
        let printed = printed.lock().unwrap().clone();
        (shared, printed)
    }

    #[test]
    fn the_watchdog_starts_from_the_stores_quiet_spells() {
        let (shared, printed) = watchdog_run(baselines_answer());
        let t = &shared["watchdog.tuning"];
        assert_eq!((t["threshold_days"].clone(), t["threshold_source"].clone(), t["cold_start"]["value"].clone(), t["cold_start"]["source"].clone()),
                   (json!(51), json!("store"), json!(51), json!("derived")), "{t}");
        // 60 quiet days on the second look are more than the 51 the store gave: stalled.
        assert_eq!((shared["watchdog.0"]["quiet_days"].clone(), shared["watchdog.0"]["stalled"].clone()), (json!(60), json!(true)));
        assert!(printed.iter().any(|p| p.starts_with("watchdog: cold start 51 day(s), derived: 1.5 x p90")), "{printed:?}");
        // A store whose answer is no table (derive_lib's baselines answer () and never throw): the
        // start value, said so, and round 0 still reads and records the company.
        let (shared, printed) = watchdog_run(json!({"columns": 5, "rows": [[1]]}));
        let t = &shared["watchdog.tuning"];
        assert_eq!((t["threshold_days"].clone(), t["threshold_source"].clone(), t["cold_start"]["source"].clone()),
                   (json!(7), json!("start"), json!("fallback")), "{t}");
        assert_eq!((shared["watchdog.0"]["since_day"].clone(), shared["watchdog.0"]["quiet_days"].clone()), (json!(100), json!(60)));
        assert!(printed.iter().any(|p| p == "watchdog: cold start 7 day(s), fallback: the store did not answer"), "{printed:?}");
        // A store that is off: no rows, the start value.
        let (shared, _) = watchdog_run(json!({"error": "store off"}));
        assert_eq!(shared["watchdog.tuning"]["threshold_source"], "start");
        let functions = rhai_file("watchdog.rhai").split("\nlet looks = 0;").next().unwrap().to_string();
        let out: rhai::Array = engine().eval(&format!("{functions}\n[cold_start(()).value, cold_start(()).source, threshold_from([], 40), threshold_from([10, 10, 10, 10, 10], 40),
             source_of([], cold_start(())), source_of([1, 2, 3, 4, 5], cold_start(())), source_of([], #{{ source: \"derived\" }})]")).unwrap();
        assert_eq!(out.iter().map(|x| x.to_string()).collect::<Vec<_>>(), vec!["7", "fallback", "40", "15", "start", "this run", "store"]);
    }

    /// derive_lib's baselines never throw: a store answer that is no table of rows is (), as no
    /// answer is, so neither the watchdog's round nor service_eval's month is lost to it. A threshold
    /// the store cannot give yet is (), so gap_watch keeps its own SHARE_DROP under that name.
    #[test]
    fn the_baselines_never_throw_and_an_underived_threshold_is_null() {
        let run = |answer: Value| -> String {
            let mut e = engine();
            e.register_fn("api_dbNamedRun", move |_: rhai::Map| dynv(answer.clone()));
            let out: rhai::Dynamic = e.eval("import \"derive_lib\" as dl; type_of(dl::baselines(6))").unwrap();
            out.to_string()
        };
        assert_eq!(run(json!({"columns": 5, "rows": [[1]]})), "()", "columns that are no list");
        assert_eq!(run(json!({"columns": ["metric", "n"], "rows": [["quiet_days"]]})), "()", "a row shorter than its columns");
        assert_eq!(run(json!({"columns": ["metric"], "rows": 7})), "()", "rows that are no list");
        assert_eq!(run(json!({"error": "store off"})), "()");
        assert_eq!(run(baselines_answer()), "array");
        let functions = rhai_file("service_eval.rhai").split("\nlet looks = 0;").next().unwrap().to_string();
        let out: rhai::Array = engine().eval(&format!(r#"{functions}
            let t = thresholds((), #{{}});
            let g = t["derived.gap_watch.share_drop"];
            [type_of(g.value), g.source, g.why, t.keys().len()]"#)).unwrap();
        assert_eq!(out.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
                   vec!["()", "fallback", "the store did not answer: gap_watch keeps its own SHARE_DROP", "2"]);
    }

    /// The share drop's window is gap_watch's WINDOW, in the route and in service_eval.rhai alike.
    #[test]
    fn the_share_drop_window_is_gap_watchs() {
        let window = |file: &str, name: &str| -> i64 {
            let text = rhai_file(file);
            let line = text.lines().find(|l| l.starts_with(&format!("const {name} = "))).unwrap_or_else(|| panic!("{file} {name}"));
            line.trim_end_matches(';').rsplit(' ').next().unwrap().parse().unwrap()
        };
        assert_eq!(window("gap_watch.rhai", "WINDOW"), GAP_WINDOW);
        assert_eq!(window("service_eval.rhai", "GAP_WINDOW"), GAP_WINDOW);
    }

    fn score(service: &str, action: &str, verdict: &str, n: i64) -> Value {
        json!({"service": service, "action": action, "verdict": verdict, "n": n, "successes": 1, "runs": 2, "rate": 0.1,
               "endpoint": if action == "stall_alert" { "resolution" } else { "relative_outcome" },
               "posterior": {"p_below_half": 0.97}, "median_relative_outcome": -0.01, "expected": "it helps"})
    }

    /// service_eval.rhai end to end: one evaluation a game month; the poor streak advances only on
    /// newly matured outcomes and resets when the verdict is no longer poor; a streak of PERSIST is in
    /// eval.poor and said in the log; the coverage of the newest run and the derived thresholds go on
    /// the blackboard; a failing route is printed and the service goes on.
    #[test]
    fn the_evaluation_service_keeps_streaks_and_says_a_persistently_poor_service() {
        let mut e = engine();
        let (shared, printed) = board(&mut e);
        let looks = std::sync::Arc::new(std::sync::Mutex::new(0i64));
        e.register_fn("env", |name: &str| -> String { if name == "SERVICE_EVAL_ROUNDS" { "7".into() } else { String::new() } });
        let l = looks.clone();
        e.register_fn("sleep", move |_ms: i64| { *l.lock().unwrap() += 1; });
        // Looks 0..6: the month changes every look but the second (look 1 repeats January).
        let months = ["1951-01-05", "1951-01-20", "1951-02-01", "1951-03-01", "1951-04-01", "1951-05-01", "1951-06-01"];
        let l = looks.clone();
        e.register_fn("read", move |_: &str| dynv(json!({"game_date": months[*l.lock().unwrap() as usize]})));
        let l = looks.clone();
        e.register_fn("api_serviceScores", move |q: rhai::Map| -> rhai::Dynamic {
            assert_eq!(q["horizon"].as_int().unwrap(), 3);
            let n = *l.lock().unwrap();
            if n == 6 { return dynv(json!({"error": "store off"})); }
            // The hold is poor at every evaluation; its outcomes do not grow at look 3 (the same
            // evidence), and it turns unclear at look 5. The watchdog helps throughout.
            let hold_n = [10, 10, 11, 11, 12, 13][n as usize];
            let hold = if n == 5 { "unclear" } else { "poor" };
            dynv(json!({"horizon": 3, "actions": 30, "rule": {"min_n": 8}, "caveat": "associative",
                        "scores": [score("replan", "hold", hold, hold_n), score("watchdog", "stall_alert", "helps", 9)],
                        "coverage": {"drop": {"value": 0.005, "source": "derived"}, "runs": [
                            {"service": "replan", "run_ms": 1, "episodes": 4, "covered": 4, "coverage": 1.0, "median_latency_months": 0, "actions": 4, "outside_episodes": 0},
                            {"service": "replan", "run_ms": 2, "episodes": 5, "covered": 0, "coverage": 0.0, "median_latency_months": null, "actions": 2, "outside_episodes": 2}]}}))
        });
        e.register_fn("api_dbNamedRun", |_: rhai::Map| dynv(baselines_answer()));
        e.run(&rhai_file("service_eval.rhai")).unwrap();
        let shared = shared.lock().unwrap();
        let printed = printed.lock().unwrap();
        // Evaluations at looks 0, 2, 3, 4, 5: streaks 1, 2, 2 (no new outcomes), 3, then reset.
        let poor: Vec<&String> = printed.iter().filter(|p| p.starts_with("[service_eval] POOR")).collect();
        assert_eq!(poor.len(), 1, "{printed:?}");
        assert!(poor[0].starts_with("[service_eval] POOR replan.hold: 1 of 12 outcome(s) above the company's own trend (P(rate < 1/2) 0.97, 2 run(s), median relative outcome -0.01); poor 3 evaluation(s) in a row."), "{}", poor[0]);
        let hold = &shared["eval.replan.hold"];
        assert_eq!((hold["verdict"].clone(), hold["streak"].clone(), hold["persistent"].clone(), hold["date"].clone(), hold["horizon"].clone()),
                   (json!("unclear"), json!(0), json!(false), json!("1951-05-01"), json!(3)), "{hold}");
        assert_eq!(shared["eval.watchdog.stall_alert"]["streak"], 0);
        assert_eq!(shared["eval.poor"]["poor"], json!([]), "reset with the hold");
        assert_eq!(shared["eval.coverage"]["services"]["replan"]["covered"], 0, "the newest run's coverage: {}", shared["eval.coverage"]);
        assert_eq!(shared["derived.gap_watch.share_drop"]["value"], json!(0.012));
        assert!(!shared.contains_key("derived.watchdog.cold_start_days"), "the watchdog derives its own cold start");
        assert_eq!(shared["derived.service_eval.coverage_drop"]["source"], "derived");
        assert!(printed.iter().any(|p| p.starts_with("service_eval 1951-01-05: 30 action(s); [\"replan.hold poor (1/10)\"")), "{printed:?}");
        assert!(printed.iter().any(|p| p == "service_eval: scores: store off"), "{printed:?}");
        // One heartbeat a look, whether or not the month changed or the scores failed: the
        // supervisor's `// stall: 600` is never reached while it runs (#308's bug, in the watchdog).
        let beats: Vec<&String> = printed.iter().filter(|p| p.starts_with("service_eval: look ")).collect();
        assert_eq!(beats.len(), 7, "{printed:?}");
        assert_eq!(beats[1], "service_eval: look 2, game month 1951-01");
        assert_eq!(printed.last().unwrap(), "service_eval: look 7, game month 1951-05");
        // The streak alone, pure.
        let functions = rhai_file("service_eval.rhai").split("\nlet looks = 0;").next().unwrap().to_string();
        let out: rhai::Array = engine().eval(&format!(r#"{functions}
            [streak_of((), #{{ verdict: "poor", n: 3 }}), streak_of(#{{ streak: 2, n: 3 }}, #{{ verdict: "poor", n: 3 }}),
             streak_of(#{{ streak: 2, n: 3 }}, #{{ verdict: "poor", n: 4 }}), streak_of(#{{ streak: 2, n: 3 }}, #{{ verdict: "helps", n: 4 }}),
             streak_of(7, #{{ verdict: "poor", n: 1 }})]"#)).unwrap();
        assert_eq!(out.iter().map(|x| x.to_string()).collect::<Vec<_>>(), vec!["1", "2", "3", "0", "1"]);
    }
}
