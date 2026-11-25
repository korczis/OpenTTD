//! RESEARCH-ONLY: one run of the planner over a game, as `mcp-plan` prints it and as the panel's
//! strategist (src/panel_strategy.rs) computes it in process: passenger pairs (plan.rs), freight
//! pairs (cargo.rs), station sites (sites.rs), optionally calibration (calibrate.rs), rail lines
//! priced on the map (linecost.rs) and transfer yards (yards.rs), the portfolio within the budget
//! (cargo.rs, AlgoGlyph knapsack-dp or the built-in DP) and optionally AlgoGlyph's
//! transport-investment (invest.rs). Moved here unchanged from src/bin/mcp-plan.rs so both callers
//! run one implementation; only where resources come from differs (`read`).
use crate::cargo::Kernels;
use serde_json::{json, Value};

/// The solver label of a capability: the kernel the matcher chose for it, or that none was.
pub fn solver_of(kernels: &Kernels, cap: &str) -> String {
    kernels.chosen(cap).map_or_else(|| format!("no kernel ({cap} unmatched, see kernels)"), |k| format!("algoglyph {k}"))
}

/// The record of the line pricing (track): which capability and kernel priced the lines, on
/// which resources, and each line's entry.
pub fn line_costs(kernels: &Kernels, priced: Vec<Value>) -> Value {
    json!({"capability": crate::linecost::RAIL_LINE_PRICE, "kernel": kernels.chosen(crate::linecost::RAIL_LINE_PRICE),
           "solver": format!("{} on map, map/ways, map/terrain, economy/prices, bridges", solver_of(kernels, crate::linecost::RAIL_LINE_PRICE)),
           "priced": priced})
}

/// What one run computes beyond the plan and its portfolio.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub limit: usize,
    /// Parameter overrides (`--params`); Null for none.
    pub params: Value,
    /// Price the N best rail lines of each list on the map (AlgoGlyph track-planning).
    pub track: usize,
    /// With `track`, prove up to N transfer yards.
    pub yards: usize,
    pub calibrate: bool,
    /// Also assess the plan against the game's outcomes for each company of `plan_for`
    /// (src/outcome.rs, `outcome`); implies `calibrate`.
    pub outcome: bool,
    /// Also decide with AlgoGlyph's transport-investment (`investment`).
    pub invest: bool,
    /// Answer only the transport-investment kernel input.
    pub invest_inputs: bool,
    /// The companies the plan is for (mcp-plan `--for`): a freight candidate whose producer another
    /// company holds with a rail station is left out (cargo.rs `with_claims`). None: every
    /// candidate stays, its holders named.
    pub plan_for: Option<Vec<i64>>,
}

/// One planner run. `read(name)` answers the data of resource `openttd://<name>`; every AlgoGlyph
/// call goes to the kernel `kernels` matches for its capability (cargo.rs `Kernels`).
pub fn run(read: &dyn Fn(&str) -> Result<Value, String>, o: &Options, kernels: &Kernels) -> Result<Value, String> {
    let limit = o.limit;
    let towns = read("towns")?;
    let (map, stations, ways) = (read("map")?, read("stations")?, read("map/ways").unwrap_or(Value::Null));
    let g = crate::graph::derive_with_ways(&map, &towns, &stations, &ways)?;
    // Live vehicles when the game offers openttd://engines; the plan says so when it does not.
    let engines = read("engines").unwrap_or(Value::Null);
    // Construction costs follow the game's prices now (openttd://economy/prices) where it offers
    // them; a parameter the caller set wins.
    let prices = read("economy/prices").unwrap_or(Value::Null);
    let params = crate::plan::with_prices(&o.params, &prices);
    let mut out = crate::plan::plan(&towns, &g, limit, &params, &engines)?;
    // Freight from the world graph's industries, and the portfolio over passengers and freight.
    let world = read("world/graph").unwrap_or(Value::Null);
    // A producer another company holds with a rail station is left out before the list is cut, so
    // the rivals' mines do not take the places of the lines the bot may build.
    let freight = crate::cargo::freight_pairs(&world, &engines, &out["params"], usize::MAX);
    let (mut freight, claims) = crate::cargo::with_claims(freight, &crate::cargo::rail_claims(&world, &map, &ways), o.plan_for.as_deref());
    freight.truncate(limit.max(10));
    out["freight"] = Value::Array(freight);
    out["claims"] = claims;
    // Station sites for every rail candidate (freight and passengers), by the bot's own rule
    // (src/sites.rs), so an executor builds the plan without a rule of its own.
    let pos = |v: &Value| (v["x"].as_i64().unwrap_or(0), v["y"].as_i64().unwrap_or(0));
    let add_sites = |c: &mut Value| {
        let (a, b) = if c.get("producer").is_some() { (pos(&c["producer"]), pos(&c["acceptor"])) } else { (pos(&c["a"]), pos(&c["b"])) };
        c["stations"] = json!({
            "from": crate::sites::site(&map, a.0, a.1, b.0, b.1, 3), "to": crate::sites::site(&map, b.0, b.1, a.0, a.1, 3),
            "rule": "axis toward the other end, buildable flat rectangle and exit tile, nearest site (as PrismaticAI rail.nut)"});
    };
    if let Some(arr) = out["freight"].as_array_mut() {
        for c in arr.iter_mut().filter(|c| c["mode"] == "freight_rail") { add_sites(c); }
    }
    if let Some(arr) = out["by_mode"]["rail"].as_array_mut() { for c in arr.iter_mut() { add_sites(c); } }
    // calibrate: what the running lines really earn against what this plan predicts for the
    // same towns or industries, per mode (src/calibrate.rs); `calibration.params` goes back in
    // as params for the next plan. Read before track pricing changes any score.
    if o.calibrate || o.outcome {
        let mut cands: Vec<Value> = out["pairs"].as_array().cloned().unwrap_or_default();
        cands.extend(out["freight"].as_array().cloned().unwrap_or_default());
        // Asked for itself, a failed read fails the run as before; asked only for the outcome, which
        // proposes and never decides, it is recorded and the plan stands.
        match (read("vehicles"), read("stations")) {
            (Ok(v), Ok(st)) => out["calibration"] = crate::calibrate::calibrate(&v, &st, &cands, crate::calibrate::REACH_TILES),
            (Err(e), _) | (_, Err(e)) if !o.calibrate => out["calibration"] = json!({"error": format!("not calibrated: {e}")}),
            (Err(e), _) | (_, Err(e)) => return Err(e),
        }
    }
    // track N: the N best rail lines of each list priced on the real map by AlgoGlyph's
    // track-planning (src/linecost.rs) instead of distance x tile cost, before the portfolio.
    if o.track > 0 {
        let opt = |r: &str| -> Result<Option<Value>, String> { Ok(read(r).ok()) };
        // The game's resources as hub_algoglyph assembles them, the map and terrain of one land.
        let (base, read_as) = crate::linecost::coherent_input(&opt, crate::linecost::READ_TRIES)?;
        let stations = out["params"]["costs"]["rail_stations"].as_f64().unwrap_or(1480.0);
        let rail_tile = out["params"]["costs"]["rail_tile"].as_f64().unwrap_or(100.0);
        let mut log = Vec::new();
        for list in ["freight", "rail"] {
            let arr = if list == "freight" { out["freight"].as_array_mut() } else { out["by_mode"]["rail"].as_array_mut() };
            if let Some(arr) = arr { log.extend(crate::linecost::price(arr, &base, stations, rail_tile, o.track, kernels)); }
        }
        out["line_costs"] = line_costs(kernels, log);
        out["line_costs"]["input"] = read_as;
        // yards N: prove up to N transfer yards offline (src/yards.rs): the better line's route
        // blocked, can the other still leave from a platform beside it?
        if o.yards > 0 {
            let mut lines: Vec<Value> = out["freight"].as_array().cloned().unwrap_or_default();
            lines.extend(out["by_mode"]["rail"].as_array().cloned().unwrap_or_default());
            let pairs = crate::yards::prove(&lines, &base, o.yards, kernels);
            out["yards"] = json!({"solver": format!("{}, the better line's tiles blocked (src/yards.rs)", solver_of(kernels, crate::linecost::RAIL_LINE_PRICE)),
                                  "pairs": pairs});
        }
    }
    let mut candidates: Vec<Value> = Vec::new();
    for (_, ps) in out["by_mode"].as_object().cloned().unwrap_or_default() {
        for p in ps.as_array().cloned().unwrap_or_default() { candidates.push(p); }
    }
    candidates.extend(out["freight"].as_array().cloned().unwrap_or_default());
    candidates.sort_by(|a, b| b["score"].as_f64().unwrap_or(0.0).total_cmp(&a["score"].as_f64().unwrap_or(0.0)));
    out["portfolio"] = crate::cargo::portfolio(&candidates, out["params"]["budget"].as_f64(), kernels);
    // outcome: per company, does the plan match what the game pays (src/outcome.rs)? Proposes only.
    // A failed read of the vehicles costs the assessment only, never the plan: the bot is not
    // started unplanned for a feature that only proposes.
    if o.outcome {
        let outcome = match read("vehicles") {
            _ if o.plan_for.as_ref().map_or(true, Vec::is_empty) =>
                json!({"why": "no company to assess: --for names none (a company not started yet has no outcomes)"}),
            Err(e) => json!({"why": format!("no outcome assessed: openttd://vehicles: {e}")}),
            Ok(vehicles) => {
                let companies = read("companies").unwrap_or(Value::Null);
                Value::Object(o.plan_for.iter().flatten()
                    .map(|c| (c.to_string(), crate::outcome::assess(&out, &vehicles, &companies, *c, kernels))).collect())
            }
        };
        out["outcome"] = outcome;
    }
    if o.invest_inputs { return Ok(crate::invest::input(&out, &engines, &towns).unwrap_or(Value::Null)); }
    if o.invest { out["investment"] = crate::invest::decide(&out, &engines, &towns, kernels); }
    out["model"] = json!("v4: v3 for passengers, freight pairs from the world graph's industries priced with each cargo's own payment rate and transit periods, and a portfolio chosen by knapsack within the budget");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    /// One run on a small map with two coal mines, one held by company 2's rail station and one by
    /// company 0's: planned for company 0 only the second mine's lines are candidates, and the plan's
    /// `claims` names the one left out; without --for both stay, each naming its holder.
    #[test]
    fn a_plan_for_a_company_leaves_out_the_producers_another_company_holds() {
        let (map, ways) = crate::cargo::testing::claims_map(&[(2, 2), (3, 2), (12, 2), (13, 2)], &[(7, 2, b'2'), (15, 3, b'0')], &[]);
        let world = json!({"nodes": [
            {"kind": "industry", "id": "industry:1", "name": "Mine A", "x": 2, "y": 2, "produces": [{"name": "Coal", "cargo": 1, "last_month": 180}]},
            {"kind": "industry", "id": "industry:2", "name": "Mine B", "x": 12, "y": 2, "produces": [{"name": "Coal", "cargo": 1, "last_month": 180}]},
            {"kind": "industry", "id": "industry:3", "name": "Plant", "x": 19, "y": 11, "accepts": [{"name": "Coal"}]}]});
        let engines = json!({"cargos": [{"name": "Coal", "payment": 5916, "transit_periods": [7, 255]}], "engines": [
            {"type": "rail", "name": "Loco", "capacity": 0, "effective_speed_kmh": 101, "price": 11000, "running_cost_per_year": 1300},
            {"type": "rail", "name": "Hopper", "cargo": "Coal", "capacity": 30, "effective_speed_kmh": 0, "price": 1500, "running_cost_per_year": 120}]});
        let read = |name: &str| -> Result<Value, String> {
            match name {
                "towns" => Ok(json!({"towns": []})),
                "map" => Ok(map.clone()),
                "map/ways" => Ok(ways.clone()),
                "stations" => Ok(json!({"stations": []})),
                "engines" => Ok(engines.clone()),
                "world/graph" => Ok(world.clone()),
                _ => Err(format!("{name}: not in this test")),
            }
        };
        let kernels = crate::cargo::testing::kernels(|_, _| Err(crate::cargo::KernelFailure::Unset));
        let opts = |plan_for: Option<Vec<i64>>| super::Options { limit: 5, params: json!({"modes": ["freight_rail"], "freight": {"bands": [5, 250]}}),
                                                                 plan_for, ..Default::default() };
        let from = |out: &Value| -> Vec<String> {
            out["freight"].as_array().unwrap().iter().map(|c| c["producer"]["name"].as_str().unwrap().to_string()).collect()
        };
        let out = super::run(&read, &opts(Some(vec![0])), &kernels).unwrap();
        assert_eq!(from(&out), ["Mine B"]);
        assert_eq!(out["freight"][0]["producer"]["rail_stations"], json!([0]));
        assert_eq!(out["claims"]["left_out"], json!([{"producer": "industry:1", "name": "Mine A", "companies": [2]}]));
        assert!(out["portfolio"]["chosen"].as_array().unwrap().iter().all(|c| c["producer"]["name"] == "Mine B"));
        let all = super::run(&read, &opts(None), &kernels).unwrap();
        let mut names = from(&all);
        names.sort();
        assert_eq!(names, ["Mine A", "Mine B"]);
        assert_eq!(all["claims"]["left_out"], json!([]));

        // The outcome only proposes: openttd://vehicles failing (this test has none) costs the
        // calibration and the assessment, never the plan. Calibration asked for itself still fails.
        let outcome = super::Options { outcome: true, ..opts(Some(vec![0])) };
        let out = super::run(&read, &outcome, &kernels).unwrap();
        assert_eq!(from(&out), ["Mine B"], "the plan stands");
        assert_eq!(out["calibration"]["error"], json!("not calibrated: vehicles: not in this test"));
        assert_eq!(out["outcome"]["why"], json!("no outcome assessed: openttd://vehicles: vehicles: not in this test"));
        let calibrate = super::Options { calibrate: true, ..opts(Some(vec![0])) };
        assert_eq!(super::run(&read, &calibrate, &kernels).unwrap_err(), "vehicles: not in this test");
    }
}
