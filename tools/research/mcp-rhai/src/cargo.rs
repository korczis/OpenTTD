//! RESEARCH-ONLY: freight routes and the investment portfolio of planner v4.
//!
//! - Freight pairs: every producing industry (openttd://world/graph, `produces` with last month's
//!   production) against every industry that accepts that cargo, within a distance band. Priced
//!   with the cargo's own payment rate and transit periods (openttd://engines `cargos`, the inputs
//!   of GetTransportedGoodsIncome), carried by trucks or a freight train that the game offers for
//!   that cargo now. Freight runs loaded one way and empty back: one load per round trip.
//! - Portfolio: from the best option of every passenger and freight candidate, the set of
//!   investments with the highest total score that fits the budget, a 0/1 knapsack. Solved by
//!   AlgoGlyph's knapsack-dp kernel when its runner is available (MCP_ALGOGLYPH_DIR, a checkout of
//!   algoglyph's flow CLI), else by the same dynamic programme built in here; the answer names the
//!   solver, the kernel, its input and its work, so it is visible which one decided.
use serde_json::{json, Value};

use crate::plan::{calibration, carried_per_vehicle, income_per_unit, net_profit, payback_score, transit_periods};

/// One long-lived AlgoGlyph `flow serve` per process, behind MCP_ALGOGLYPH_SERVE (I0086).
#[path = "flow_serve.rs"]
pub mod flow_serve;

fn num(v: &Value, k: &str) -> f64 { v.get(k).and_then(Value::as_f64).unwrap_or(0.0) }

/// A freight vehicle for `cargo` (by name): the road vehicle with the best capacity x speed, or the
/// fastest locomotive with `wagons` of the biggest wagon for that cargo.
pub fn freight_vehicle(engines: &Value, cargo: &str, rail: bool, wagons: f64) -> Option<(String, f64, f64, f64, f64)> {
    let list = engines.get("engines")?.as_array()?;
    let carries = |e: &Value| e.get("cargo").and_then(Value::as_str) == Some(cargo);
    if !rail {
        let e = list.iter().filter(|e| e["type"] == "road" && carries(e) && num(e, "capacity") > 0.0)
            .max_by(|a, b| (num(a, "capacity") * num(a, "effective_speed_kmh")).total_cmp(&(num(b, "capacity") * num(b, "effective_speed_kmh"))))?;
        return Some((e["name"].as_str().unwrap_or("").into(), num(e, "capacity"), num(e, "effective_speed_kmh"), num(e, "price"), num(e, "running_cost_per_year")));
    }
    let loco = list.iter().filter(|e| e["type"] == "rail" && num(e, "capacity") == 0.0 && num(e, "effective_speed_kmh") > 0.0)
        .max_by(|a, b| num(a, "effective_speed_kmh").total_cmp(&num(b, "effective_speed_kmh")))?;
    let wagon = list.iter().filter(|e| e["type"] == "rail" && carries(e) && num(e, "capacity") > 0.0)
        .max_by(|a, b| num(a, "capacity").total_cmp(&num(b, "capacity")))?;
    Some((format!("{} + {} x {}", loco["name"].as_str().unwrap_or(""), wagons, wagon["name"].as_str().unwrap_or("")),
          wagons * num(wagon, "capacity"), num(loco, "effective_speed_kmh"),
          num(loco, "price") + wagons * num(wagon, "price"),
          num(loco, "running_cost_per_year") + wagons * num(wagon, "running_cost_per_year")))
}

/// Freight pairs, best option first. `p` is the planner's effective parameters.
pub fn freight_pairs(world: &Value, engines: &Value, p: &Value, limit: usize) -> Vec<Value> {
    let f = |path: &str, d: f64| p.pointer(path).and_then(Value::as_f64).unwrap_or(d);
    let (min_d, max_d) = (f("/freight/bands/0", 20.0), f("/freight/bands/1", 250.0));
    let share = f("/freight/transported_share", 0.7);
    let (load, payback, overhead) = (f("/load_factor", 0.8), f("/payback_months", 24.0), f("/overhead_periods", 2.0));
    let (road_tile, rail_tile, rail_stations) = (f("/costs/road_tile", 190.0), f("/costs/rail_tile", 100.0), f("/costs/rail_stations", 1480.0));
    let truck_stops = f("/freight/truck_stops", 400.0);
    let wagons = f("/rail_wagons", 4.0);
    let budget = p["budget"].as_f64();
    let enabled: Vec<String> = p["modes"].as_array().map(|a| a.iter().filter_map(|m| m.as_str().map(str::to_string)).collect())
        .unwrap_or_else(|| vec!["truck".into(), "freight_rail".into()]);
    let cargos: Vec<Value> = engines.get("cargos").and_then(Value::as_array).cloned().unwrap_or_default();
    let industries: Vec<&Value> = world.get("nodes").and_then(Value::as_array).map(|n| n.iter().filter(|x| x["kind"] == "industry").collect()).unwrap_or_default();
    let mut out = Vec::new();
    for prod in &industries {
        for pc in prod.get("produces").and_then(Value::as_array).into_iter().flatten() {
            let monthly = num(pc, "last_month") * share;
            if monthly <= 0.0 { continue; }
            let cname = pc["name"].as_str().unwrap_or("");
            let Some(cs) = cargos.iter().find(|c| c["name"] == cname) else { continue };
            let payment = num(cs, "payment");
            let (td1, td2) = (cs["transit_periods"][0].as_f64().unwrap_or(0.0), cs["transit_periods"][1].as_f64().unwrap_or(0.0));
            for acc in &industries {
                if acc["id"] == prod["id"] { continue; }
                if !acc.get("accepts").and_then(Value::as_array).into_iter().flatten().any(|a| a["name"] == cname) { continue; }
                let d = (num(prod, "x") - num(acc, "x")).abs() + (num(prod, "y") - num(acc, "y")).abs();
                if d < min_d || d > max_d { continue; }
                let mut best: Option<Value> = None;
                let mut options = Vec::new();
                for (mode, rail) in [("truck", false), ("freight_rail", true)] {
                    if !enabled.iter().any(|m| m == mode) { continue; }
                    let Some((vname, cap, speed, price, running)) = freight_vehicle(engines, cname, rail, wagons) else { continue };
                    let periods = transit_periods(d, speed.max(1.0), overhead);
                    // A ratio of net profit (plan.rs net_profit says why).
                    let cal = calibration(p, mode);
                    let per_unit = income_per_unit(d, periods, payment, (td1, td2));
                    let per_vehicle = carried_per_vehicle(cap, load, periods, false); // loaded one way, empty back
                    let build = if rail { d * rail_tile + rail_stations } else { d * road_tile * 0.5 + truck_stops };
                    if budget.is_some_and(|b| build > b) { continue; }
                    let top = (monthly / per_vehicle.max(0.1)).ceil().clamp(1.0, if rail { 4.0 } else { 12.0 });
                    let mut chosen = (f64::MIN, 1.0, 0.0, 0.0);
                    let mut n = 1.0;
                    while n <= top {
                        if budget.is_some_and(|b| build + n * price > b) { break; }
                        let carried = monthly.min(n * per_vehicle);
                        let profit = net_profit(carried, per_unit, n * running / 12.0, cal);
                        let score = payback_score(profit, payback, build, n * price);
                        if score > chosen.0 { chosen = (score, n, carried, profit); }
                        n += 1.0;
                    }
                    if chosen.0 == f64::MIN { continue; }
                    let (score, n, carried, profit) = chosen;
                    let opt = json!({"mode": mode, "transit_periods": (periods * 10.0).round() / 10.0,
                        "income_per_unit": (per_unit * 100.0).round() / 100.0, "carried_per_month": carried.round(),
                        "monthly_profit": profit.round(), "build_cost": build.round(), "investment": (build + n * price).round(),
                        "vehicles": {"name": vname, "count": n, "capacity": cap, "effective_speed_kmh": speed}, "score": score.round()});
                    if best.as_ref().map_or(true, |b| score > num(b, "score")) { best = Some(opt.clone()); }
                    options.push(opt);
                }
                let Some(b) = best else { continue };
                out.push(json!({
                    "kind": "freight", "cargo": cname, "cargo_id": pc["cargo"], "distance": d, "supply_per_month": monthly.round(),
                    "producer": {"id": prod["id"], "name": prod["name"], "x": prod["x"], "y": prod["y"]},
                    "acceptor": {"id": acc["id"], "name": acc["name"], "x": acc["x"], "y": acc["y"]},
                    "mode": b["mode"], "score": b["score"], "options": options,
                }));
            }
        }
    }
    out.sort_by(|a, b| num(b, "score").total_cmp(&num(a, "score")));
    out.truncate(limit);
    out
}

/// The 0/1 knapsack the portfolio needs: maximise the sum of values with weights within capacity.
fn knapsack_builtin(weights: &[usize], values: &[f64], capacity: usize) -> (f64, Vec<usize>) {
    let n = weights.len();
    let mut dp = vec![vec![0.0f64; capacity + 1]; n + 1];
    for i in 1..=n {
        for c in 0..=capacity {
            dp[i][c] = dp[i - 1][c];
            if weights[i - 1] <= c { dp[i][c] = dp[i][c].max(dp[i - 1][c - weights[i - 1]] + values[i - 1]); }
        }
    }
    let (mut c, mut items) = (capacity, Vec::new());
    for i in (1..=n).rev() {
        if dp[i][c] != dp[i - 1][c] { items.push(i - 1); c -= weights[i - 1]; }
    }
    items.reverse();
    (dp[n][capacity], items)
}

/// Why an AlgoGlyph kernel gave no answer: the runner is not configured, it ran and refused (its
/// own `errors`), or it crashed (spawn, I/O or an answer that is not JSON).
#[derive(Clone, Debug, PartialEq)]
pub enum KernelFailure {
    Unset,
    Refused(Value),
    Crashed(String),
}

impl KernelFailure {
    /// The reason as `algoglyph` has always worded it.
    fn message(&self, kernel: &str) -> String {
        match self {
            KernelFailure::Unset => "MCP_ALGOGLYPH_DIR is not set".to_string(),
            KernelFailure::Refused(errors) => format!("{kernel} refused: {errors}"),
            KernelFailure::Crashed(m) => m.clone(),
        }
    }
}

/// A checkout this process chose itself (`use_checkout`), for when `MCP_ALGOGLYPH_DIR` is not set.
static CHECKOUT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Use AlgoGlyph at `dir` when `MCP_ALGOGLYPH_DIR` is not set. The panel's strategist
/// (src/panel_strategy.rs) passes what hub_algoglyph::dir() found; mcp-plan and mcp-intent never
/// call it, so for them an unset variable still means the built-in DP. First call wins.
#[allow(dead_code)] // only the panel's strategist chooses a checkout
pub fn use_checkout(dir: &std::path::Path) {
    let _ = CHECKOUT.set(dir.display().to_string());
}

/// The checkout kernels run in: `$MCP_ALGOGLYPH_DIR`, else the one `use_checkout` chose.
pub fn checkout() -> Result<String, String> {
    std::env::var("MCP_ALGOGLYPH_DIR").ok().filter(|d| !d.is_empty()).or_else(|| CHECKOUT.get().cloned())
        .ok_or_else(|| "MCP_ALGOGLYPH_DIR is not set".to_string())
}

/// An AlgoGlyph kernel through its flow CLI (`npm run flow -- kernel <id> -`), when MCP_ALGOGLYPH_DIR
/// names a checkout of it. Err says why there is no answer: no runner, a failed run, or a refusal.
/// With `MCP_ALGOGLYPH_SERVE=1` every call of the process goes to one long-lived `flow serve`
/// (src/flow_serve.rs), and to the per-call runner only if that serve cannot start or fails.
#[allow(dead_code)] // mcp-intent includes cargo.rs and calls no kernel by name
pub fn algoglyph(kernel: &str, input: &Value) -> Result<Value, String> {
    algoglyph_run(kernel, input).map_err(|f| f.message(kernel))
}

/// `algoglyph` with the reason kept as a kind, for a caller that records why it fell back.
pub fn algoglyph_run(kernel: &str, input: &Value) -> Result<Value, KernelFailure> {
    let crashed = |e: &dyn std::fmt::Display| KernelFailure::Crashed(format!("{kernel}: {e}"));
    let refused = |v: &Value| KernelFailure::Refused(v.get("errors").cloned().unwrap_or(v.clone()));
    let dir = checkout().map_err(|_| KernelFailure::Unset)?;
    if flow_serve::wanted() {
        match flow_serve::kernel(std::path::Path::new(&dir), kernel, input) {
            Ok(v) if v["ok"] == true => return Ok(v),
            Ok(v) => return Err(refused(&v)),
            Err(_) => flow_serve::fell_back(),
        }
    }
    // The answer goes to a file, not a pipe: the flow CLI ends with process.exit(), which drops what
    // a pipe has not taken yet, so an answer above 64 KiB arrived cut off (2026-11-16).
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let path = std::env::temp_dir().join(format!("mcp-algoglyph-{}-{nanos}.json", std::process::id()));
    let file = std::fs::File::create(&path).map_err(|e| crashed(&e))?;
    let mut child = std::process::Command::new("npm")
        .args(["run", "--silent", "flow", "--", "kernel", kernel, "-"])
        .current_dir(&dir).stdin(std::process::Stdio::piped()).stdout(file)
        .stderr(std::process::Stdio::null()).spawn().map_err(|e| crashed(&e))?;
    use std::io::Write;
    child.stdin.take().ok_or_else(|| KernelFailure::Crashed("no stdin".into()))?
        .write_all(input.to_string().as_bytes()).map_err(|e| crashed(&e))?;
    child.wait().map_err(|e| crashed(&e))?;
    let text = std::fs::read(&path).map_err(|e| crashed(&e));
    let _ = std::fs::remove_file(&path);
    let v: Value = serde_json::from_slice(&text?).map_err(|e| crashed(&format!("not JSON ({e})")))?;
    if v["ok"] == true { Ok(v) } else { Err(refused(&v)) }
}

/// How many candidates enter the knapsack at most, unless the caller names another cap.
pub const PORTFOLIO_CAP: usize = 40;

/// A knapsack-dp runner: AlgoGlyph's (`algoglyph_run`) in production, a fake one in tests.
pub type KernelRunner<'a> = &'a dyn Fn(&str, &Value) -> Result<Value, KernelFailure>;

/// The kernel's answer, or why the built-in solver decides instead. An answer the runner gives is
/// accepted only when it passes knapsack-dp's domain check (battlebrain_contract.rs, the adapter
/// registry): indices in range and unique, the capacity held, best equal to the chosen items' sum.
fn knapsack_algoglyph(input: &Value, run: KernelRunner) -> Result<Value, Value> {
    const KERNEL: &str = "knapsack-dp";
    match run(KERNEL, input) {
        Err(f) => {
            let message = f.message(KERNEL);
            Err(match f {
                KernelFailure::Unset => json!({"reason": "unset_dir", "message": message}),
                KernelFailure::Refused(errors) => json!({"reason": "runner_refused", "errors": errors, "message": message}),
                KernelFailure::Crashed(_) => json!({"reason": "crashed", "message": message}),
            })
        }
        Ok(v) => match crate::battlebrain_contract::domain_check(KERNEL, input, &v) {
            Ok(()) => Ok(v),
            Err(r) => Err(json!({"reason": "domain_check_failed", "code": r.code, "path": r.path, "message": r.message,
                                 "refused_answer": {"kernel": v.get("kernel"), "result": v.get("result")}})),
        },
    }
}

/// The label of a portfolio the AlgoGlyph kernel answered.
const KERNEL_SOLVER: &str = "algoglyph knapsack-dp";
/// The label of the built-in fallback: a separately named solver that never passes as the kernel
/// (ADR 0037; rule project.decision-authority-boundaries, B7). battlebrain reports a solver whose
/// label starts with "builtin" as `fallback_solver` (src/bin/battlebrain.rs).
/// Why it decided (no runner, a refusal, a crash, an answer that failed its domain check) is the
/// portfolio's `fallback`, never a variant of this label.
const BUILTIN_SOLVER: &str = "builtin knapsack (no checked kernel answer; the reason is in fallback)";

/// Which solver answered the portfolio, and its answer: the kernel's when it answered, else the
/// built-in copy, labelled as itself and with no kernel. (solver, best, chosen items, kernel)
/// `kernel_answer` is one that passed its domain check (`knapsack_algoglyph`): its indices are
/// in range and unique, so the portfolio may index the items by them.
fn solve(kernel_answer: Option<Value>, weights: &[usize], values: &[f64], capacity: usize) -> (&'static str, f64, Vec<usize>, Value) {
    match kernel_answer {
        Some(v) => {
            let idx: Vec<usize> = v["result"]["items"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(|i| i as usize).collect();
            (KERNEL_SOLVER, num(&v["result"], "best"), idx, json!({"id": v["kernel"]["id"], "plate": v["kernel"]["plate"], "work": v["work"]}))
        }
        None => {
            let (b, idx) = knapsack_builtin(weights, values, capacity);
            (BUILTIN_SOLVER, b, idx, Value::Null)
        }
    }
}

/// The portfolio: from the candidates (each with its best option's investment and score), the set
/// with the highest total score within the budget. Money is counted in thousands for the solver.
pub fn portfolio(candidates: &[Value], budget: Option<f64>) -> Value {
    portfolio_with(candidates, budget, PORTFOLIO_CAP, &algoglyph_run)
}

/// `portfolio` with the candidate cap and the knapsack-dp runner named by the caller.
pub fn portfolio_with(candidates: &[Value], budget: Option<f64>, cap: usize, run: KernelRunner) -> Value {
    // A producer's cargo is counted once: each freight candidate assumes it carries the producer's
    // whole production, so two lines from one mine would each count all of it. Only the best-scoring
    // candidate of each (cargo, producer) enters the knapsack; the others are reported as shadowed.
    let source = |c: &Value| -> Option<String> {
        let from = c.pointer("/producer/id").or_else(|| c.get("from").filter(|_| c["passengers"] != true))?;
        Some(format!("{}@{}", c["cargo"].as_str().unwrap_or(""), from.as_str().map_or_else(|| from.to_string(), str::to_string)))
    };
    let mut best_of: std::collections::HashMap<String, f64> = Default::default();
    for c in candidates.iter().filter(|c| num(c, "score") > 0.0) {
        if let Some(k) = source(c) { let e = best_of.entry(k).or_insert(f64::MIN); *e = e.max(num(c, "score")); }
    }
    let mut shadowed = Vec::new();
    let mut taken = std::collections::HashSet::new();
    let mut items: Vec<&Value> = candidates.iter().filter(|c| num(c, "score") > 0.0).filter(|c| match source(c) {
        Some(k) if num(c, "score") < best_of[&k] || !taken.insert(k.clone()) => { shadowed.push(k); false }
        _ => true,
    }).collect();
    // The cap is explicit: what it leaves out is counted, never dropped in silence.
    let eligible = items.len();
    items.truncate(cap);
    let truncation = json!({"cap": cap, "eligible": eligible, "entered": items.len(), "dropped": eligible - items.len()});
    let invest = |c: &Value| -> f64 {
        let mode = c["mode"].as_str().unwrap_or("");
        c["options"].as_array().into_iter().flatten().find(|o| o["mode"] == mode).map_or(0.0, |o| num(o, "investment"))
    };
    let Some(budget) = budget else {
        return json!({"solver": "none", "note": "no budget given: every positive candidate is affordable", "chosen": items,
                      "shadowed": shadowed, "truncation": truncation});
    };
    let weights: Vec<usize> = items.iter().map(|c| (invest(c) / 1000.0).ceil() as usize).collect();
    let values: Vec<f64> = items.iter().map(|c| num(c, "score").round()).collect();
    let capacity = (budget / 1000.0).floor().max(0.0) as usize;
    let input = json!({"weights": weights, "values": values, "capacity": capacity});
    let (answer, fallback) = match knapsack_algoglyph(&input, run) {
        Ok(v) => (Some(v), Value::Null),
        Err(why) => (None, why),
    };
    let check = match fallback["reason"].as_str() {
        None => json!({"status": "passed"}),
        Some("domain_check_failed") => json!({"status": "failed", "code": fallback["code"], "path": fallback["path"], "message": fallback["message"]}),
        Some(_) => json!({"status": "not_run"}),
    };
    let (solver, best, chosen, kernel) = solve(answer, &weights, &values, capacity);
    json!({"solver": solver, "kernel": kernel, "input": input, "total_score": best, "shadowed": shadowed,
           "investment_k": chosen.iter().map(|&i| weights[i]).sum::<usize>(), "budget_k": capacity,
           "chosen": chosen.iter().map(|&i| items[i].clone()).collect::<Vec<_>>(),
           "fallback": fallback, "domain_check": check, "truncation": truncation})
}

#[cfg(test)]
mod tests {
    use super::knapsack_builtin;
    #[test]
    fn a_producer_enters_the_portfolio_once_with_its_best_line() {
        let c = |acc: u32, score: f64| serde_json::json!({"cargo": "Coal", "producer": {"id": 13}, "acceptor": {"id": acc}, "mode": "freight_rail",
            "score": score, "options": [{"mode": "freight_rail", "investment": 1000.0}]});
        let other = serde_json::json!({"cargo": "Coal", "producer": {"id": 44}, "mode": "freight_rail", "score": 5.0, "options": [{"mode": "freight_rail", "investment": 1000.0}]});
        let pax = serde_json::json!({"kind": "investment", "cargo": "Passengers", "passengers": true, "from": "t1", "mode": "rail", "score": 4.0, "options": [{"mode": "rail", "investment": 1000.0}]});
        let p = super::portfolio(&[c(20, 10.0), c(38, 9.0), other, pax], Some(100000.0));
        let chosen = p["chosen"].as_array().unwrap();
        assert_eq!(chosen.len(), 3);
        assert_eq!(chosen.iter().filter(|x| x.pointer("/producer/id") == Some(&serde_json::json!(13))).count(), 1);
        assert_eq!(p["shadowed"], serde_json::json!(["Coal@13"]));
    }

    #[test]
    fn calibration_scales_net_profit_never_income_alone() {
        // One truck line, 100 tiles: a factor of 0.5 must halve the monthly profit exactly. Scaling the
        // income and then subtracting the running cost charged the cost twice (codex review, 2026-11-17).
        let world = serde_json::json!({"nodes": [
            {"kind": "industry", "id": 1, "x": 10, "y": 10, "produces": [{"name": "Coal", "last_month": 200}]},
            {"kind": "industry", "id": 2, "x": 110, "y": 10, "accepts": [{"name": "Coal"}]}]});
        let engines = serde_json::json!({
            "cargos": [{"name": "Coal", "payment": 5916, "transit_periods": [7, 255]}],
            "engines": [{"type": "road", "name": "Truck", "cargo": "Coal", "capacity": 20, "effective_speed_kmh": 48, "price": 4000, "running_cost_per_year": 2400}]});
        let p = |cal: Option<f64>| {
            let mut v = serde_json::json!({"modes": ["truck"], "freight": {"bands": [20, 250]}});
            if let Some(c) = cal { v["calibration"] = serde_json::json!({"truck": c}); }
            v
        };
        let profit = |cal| super::freight_pairs(&world, &engines, &p(cal), 5)[0]["options"][0]["monthly_profit"].as_f64().unwrap();
        let (plain, half) = (profit(None), profit(Some(0.5)));
        assert!(plain > 0.0);
        assert!((half - plain / 2.0).abs() <= 1.0, "{half} is not half of {plain}");
    }

    #[test]
    fn freight_output_is_pinned() {
        // Captured from the inline formulas before they moved into the shared economics functions
        // (refactor/planner-shared-economics): the freight answer must stay byte-identical.
        let world = serde_json::json!({"nodes": [
            {"kind": "industry", "id": 1, "x": 10, "y": 10, "produces": [{"name": "Coal", "cargo": 1, "last_month": 180}]},
            {"kind": "industry", "id": 2, "x": 95, "y": 40, "accepts": [{"name": "Coal"}]},
            {"kind": "industry", "id": 3, "x": 30, "y": 20, "accepts": [{"name": "Coal"}]}]});
        let engines = serde_json::json!({
            "cargos": [{"name": "Coal", "payment": 5916, "transit_periods": [7, 255]}],
            "engines": [
                {"type": "road", "name": "Truck", "cargo": "Coal", "capacity": 20, "effective_speed_kmh": 48, "price": 4000, "running_cost_per_year": 2400},
                {"type": "rail", "name": "Loco", "capacity": 0, "effective_speed_kmh": 101, "price": 11000, "running_cost_per_year": 1300},
                {"type": "rail", "name": "Hopper", "cargo": "Coal", "capacity": 30, "effective_speed_kmh": 0, "price": 1500, "running_cost_per_year": 120}]});
        let plain = serde_json::json!({"modes": ["truck", "freight_rail"]});
        let calibrated = serde_json::json!({"modes": ["truck", "freight_rail"], "calibration": {"truck": 0.8, "default": 1.2}, "budget": 30000});
        for (p, want) in [(plain, PINNED_FREIGHT), (calibrated, PINNED_FREIGHT_CALIBRATED)] {
            assert_eq!(serde_json::Value::Array(super::freight_pairs(&world, &engines, &p, 5)).to_string(), want);
        }
    }
    const PINNED_FREIGHT: &str = r#"[{"acceptor":{"id":2,"name":null,"x":95,"y":40},"cargo":"Coal","cargo_id":1,"distance":115.0,"kind":"freight","mode":"freight_rail","options":[{"build_cost":11325.0,"carried_per_month":3.0,"income_per_unit":75.74,"investment":15325.0,"mode":"truck","monthly_profit":55.0,"score":-14009.0,"transit_periods":28.5,"vehicles":{"capacity":20.0,"count":1.0,"effective_speed_kmh":48.0,"name":"Truck"}},{"build_cost":12980.0,"carried_per_month":118.0,"income_per_unit":80.26,"investment":63980.0,"mode":"freight_rail","monthly_profit":9048.0,"score":153168.0,"transit_periods":14.6,"vehicles":{"capacity":120.0,"count":3.0,"effective_speed_kmh":101.0,"name":"Loco + 4 x Hopper"}}],"producer":{"id":1,"name":null,"x":10,"y":10},"score":153168.0,"supply_per_month":126.0},{"acceptor":{"id":3,"name":null,"x":30,"y":20},"cargo":"Coal","cargo_id":1,"distance":30.0,"kind":"freight","mode":"freight_rail","options":[{"build_cost":3250.0,"carried_per_month":11.0,"income_per_unit":21.42,"investment":7250.0,"mode":"truck","monthly_profit":30.0,"score":-6519.0,"transit_periods":8.9,"vehicles":{"capacity":20.0,"count":1.0,"effective_speed_kmh":48.0,"name":"Truck"}},{"build_cost":4480.0,"carried_per_month":109.0,"income_per_unit":21.58,"investment":21480.0,"mode":"freight_rail","monthly_profit":2202.0,"score":31361.0,"transit_periods":5.3,"vehicles":{"capacity":120.0,"count":1.0,"effective_speed_kmh":101.0,"name":"Loco + 4 x Hopper"}}],"producer":{"id":1,"name":null,"x":10,"y":10},"score":31361.0,"supply_per_month":126.0}]"#;
    const PINNED_FREIGHT_CALIBRATED: &str = r#"[{"acceptor":{"id":2,"name":null,"x":95,"y":40},"cargo":"Coal","cargo_id":1,"distance":115.0,"kind":"freight","mode":"freight_rail","options":[{"build_cost":11325.0,"carried_per_month":3.0,"income_per_unit":75.74,"investment":15325.0,"mode":"truck","monthly_profit":44.0,"score":-14272.0,"transit_periods":28.5,"vehicles":{"capacity":20.0,"count":1.0,"effective_speed_kmh":48.0,"name":"Truck"}},{"build_cost":12980.0,"carried_per_month":39.0,"income_per_unit":80.26,"investment":29980.0,"mode":"freight_rail","monthly_profit":3619.0,"score":56879.0,"transit_periods":14.6,"vehicles":{"capacity":120.0,"count":1.0,"effective_speed_kmh":101.0,"name":"Loco + 4 x Hopper"}}],"producer":{"id":1,"name":null,"x":10,"y":10},"score":56879.0,"supply_per_month":126.0},{"acceptor":{"id":3,"name":null,"x":30,"y":20},"cargo":"Coal","cargo_id":1,"distance":30.0,"kind":"freight","mode":"freight_rail","options":[{"build_cost":3250.0,"carried_per_month":11.0,"income_per_unit":21.42,"investment":7250.0,"mode":"truck","monthly_profit":24.0,"score":-6665.0,"transit_periods":8.9,"vehicles":{"capacity":20.0,"count":1.0,"effective_speed_kmh":48.0,"name":"Truck"}},{"build_cost":4480.0,"carried_per_month":109.0,"income_per_unit":21.58,"investment":21480.0,"mode":"freight_rail","monthly_profit":2642.0,"score":41930.0,"transit_periods":5.3,"vehicles":{"capacity":120.0,"count":1.0,"effective_speed_kmh":101.0,"name":"Loco + 4 x Hopper"}}],"producer":{"id":1,"name":null,"x":10,"y":10},"score":41930.0,"supply_per_month":126.0}]"#;

    #[test]
    fn the_builtin_fallback_is_never_labelled_as_the_kernel() {
        // ADR 0037: the built-in knapsack is a separately named solver. Without a kernel answer the
        // portfolio names the built-in copy and no kernel; battlebrain counts it as a fallback.
        let (w, v) = ([135, 46, 20, 5], [196159.0, 64387.0, 16633.0, 1000.0]);
        let (solver, best, items, kernel) = super::solve(None, &w, &v, 150);
        assert_eq!((best, items), (197159.0, vec![0, 3]));
        assert!(solver.starts_with("builtin"), "battlebrain's fallback_solver test reads this prefix: {solver}");
        assert!(!solver.contains("algoglyph") && !solver.contains("knapsack-dp"), "{solver}");
        assert!(kernel.is_null(), "a built-in answer names no kernel");
        // The kernel's own answer is labelled as the kernel's, with its id.
        let answer = serde_json::json!({"ok": true, "kernel": {"id": "knapsack-dp", "plate": "p"}, "work": {}, "result": {"best": 197159.0, "items": [0, 3]}});
        let (solver, best, items, kernel) = super::solve(Some(answer), &w, &v, 150);
        assert_eq!((solver, best, items), (super::KERNEL_SOLVER, 197159.0, vec![0, 3]));
        assert_eq!(kernel["id"], "knapsack-dp");
        assert!(!solver.starts_with("builtin"));
    }

    #[test]
    fn knapsack_matches_the_algoglyph_case() {
        // The case checked against AlgoGlyph's kernel: items 0 and 3, best 197159.
        let (best, items) = knapsack_builtin(&[135, 46, 20, 5], &[196159.0, 64387.0, 16633.0, 1000.0], 150);
        assert_eq!(best, 197159.0);
        assert_eq!(items, vec![0, 3]);
    }

    // The portfolio's knapsack-dp answer and its fallbacks, against a fake runner. The four items
    // are the AlgoGlyph case below: weights 135, 46, 20, 5 (thousands), capacity 150, best 197159.
    use super::{portfolio_with, KernelFailure, PORTFOLIO_CAP};
    use serde_json::{json, Value};

    fn line(producer: usize, investment: f64, score: f64) -> Value {
        json!({"cargo": "Coal", "producer": {"id": producer}, "mode": "freight_rail", "score": score,
               "options": [{"mode": "freight_rail", "investment": investment}]})
    }
    fn four() -> Vec<Value> {
        vec![line(1, 135000.0, 196159.0), line(2, 46000.0, 64387.0), line(3, 20000.0, 16633.0), line(4, 5000.0, 1000.0)]
    }
    fn answer(best: f64, items: Value) -> Value {
        json!({"ok": true, "kernel": {"id": "knapsack-dp", "plate": "knapsack"}, "work": {"cells": 600},
               "result": {"best": best, "items": items}})
    }
    fn producers(p: &Value) -> Vec<Value> {
        p["chosen"].as_array().unwrap().iter().map(|c| c["producer"]["id"].clone()).collect()
    }
    /// Every fallback keeps the old keys, names the built-in solver and decides as it does.
    fn fell_back(p: &Value, reason: &str) {
        assert!(p["solver"].as_str().unwrap().starts_with("builtin knapsack"), "{}", p["solver"]);
        assert_eq!(p["kernel"], Value::Null);
        assert_eq!(p["fallback"]["reason"], reason);
        assert_eq!(p["total_score"], json!(197159.0));
        assert_eq!(producers(p), vec![json!(1), json!(4)]);
        assert_eq!(p["investment_k"], json!(140));
        for k in ["input", "shadowed", "budget_k"] { assert!(p.get(k).is_some(), "{k}"); }
    }

    #[test]
    fn a_checked_kernel_answer_decides() {
        let p = portfolio_with(&four(), Some(150000.0), PORTFOLIO_CAP, &|_, _| Ok(answer(197159.0, json!([0, 3]))));
        assert_eq!(p["solver"], super::KERNEL_SOLVER);
        assert_eq!(p["kernel"]["id"], "knapsack-dp");
        assert_eq!(p["domain_check"]["status"], "passed");
        assert_eq!(p["fallback"], Value::Null);
        assert_eq!(producers(&p), vec![json!(1), json!(4)]);
    }

    #[test]
    fn a_kernel_answer_that_fails_its_domain_check_is_refused() {
        for (why, best, items) in [("out of range", 1000.0, json!([9])), ("repeated", 2000.0, json!([3, 3])),
                                   ("over capacity", 260546.0, json!([0, 1])), ("wrong best", 999999.0, json!([0, 3]))] {
            let bad = answer(best, items.clone());
            let p = portfolio_with(&four(), Some(150000.0), PORTFOLIO_CAP, &|_, _| Ok(bad.clone()));
            fell_back(&p, "domain_check_failed");
            assert_eq!(p["solver"], super::BUILTIN_SOLVER, "{why}");
            assert_eq!(p["fallback"]["code"], "invalid_output", "{why}");
            assert_eq!(p["domain_check"]["status"], "failed", "{why}");
            assert!(p["fallback"]["message"].as_str().is_some_and(|m| !m.is_empty()), "{why}");
            assert_eq!(p["fallback"]["refused_answer"]["result"]["items"], items, "{why}");
        }
    }

    #[test]
    fn every_fallback_reason_is_kept() {
        let unset = portfolio_with(&four(), Some(150000.0), PORTFOLIO_CAP, &|_, _| Err(KernelFailure::Unset));
        fell_back(&unset, "unset_dir");
        assert_eq!(unset["solver"], super::BUILTIN_SOLVER);
        assert_eq!(unset["fallback"]["message"], "MCP_ALGOGLYPH_DIR is not set");
        assert_eq!(unset["domain_check"]["status"], "not_run");

        let errors = json!([{"path": "capacity", "message": "must be an integer"}]);
        let refused = portfolio_with(&four(), Some(150000.0), PORTFOLIO_CAP, &|_, _| Err(KernelFailure::Refused(errors.clone())));
        fell_back(&refused, "runner_refused");
        assert_eq!(refused["solver"], super::BUILTIN_SOLVER);
        assert_eq!(refused["fallback"]["errors"], errors);

        let crashed = portfolio_with(&four(), Some(150000.0), PORTFOLIO_CAP,
                                     &|k, _| Err(KernelFailure::Crashed(format!("{k}: not JSON (EOF while parsing)"))));
        fell_back(&crashed, "crashed");
        assert_eq!(crashed["solver"], super::BUILTIN_SOLVER);
        assert_eq!(crashed["fallback"]["message"], "knapsack-dp: not JSON (EOF while parsing)");
    }

    #[test]
    fn the_candidate_cap_is_recorded_never_silent() {
        let many: Vec<Value> = (0..45).map(|i| line(i, 1000.0, 100.0 - i as f64)).collect();
        let p = portfolio_with(&many, Some(1_000_000.0), PORTFOLIO_CAP, &|_, _| Err(KernelFailure::Unset));
        assert_eq!(p["truncation"], json!({"cap": 40, "eligible": 45, "entered": 40, "dropped": 5}));
        assert_eq!(p["chosen"].as_array().unwrap().len(), 40);
        let p = portfolio_with(&many, None, 3, &|_, _| Err(KernelFailure::Unset));
        assert_eq!(p["truncation"], json!({"cap": 3, "eligible": 45, "entered": 3, "dropped": 42}));
        let p = portfolio_with(&four(), Some(150000.0), PORTFOLIO_CAP, &|_, _| Err(KernelFailure::Unset));
        assert_eq!(p["truncation"]["dropped"], 0);
    }
}
