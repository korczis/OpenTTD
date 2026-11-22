//! RESEARCH-ONLY: one run of the planner over a game, as `mcp-plan` prints it and as the panel's
//! strategist (src/panel_strategy.rs) computes it in process: passenger pairs (plan.rs), freight
//! pairs (cargo.rs), station sites (sites.rs), optionally calibration (calibrate.rs), rail lines
//! priced on the map (linecost.rs) and transfer yards (yards.rs), the portfolio within the budget
//! (cargo.rs, AlgoGlyph knapsack-dp or the built-in DP) and optionally AlgoGlyph's
//! transport-investment (invest.rs). Moved here unchanged from src/bin/mcp-plan.rs so both callers
//! run one implementation; only where resources come from differs (`read`).
use serde_json::{json, Value};

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
    /// Also decide with AlgoGlyph's transport-investment (`investment`).
    pub invest: bool,
    /// Answer only the transport-investment kernel input.
    pub invest_inputs: bool,
}

/// The game's resources a track plan reads, as hub_algoglyph assembles them (from/to added later).
fn linecost_input(read: &dyn Fn(&str) -> Result<Option<Value>, String>) -> Result<Value, String> {
    let ends = json!({"from": {}, "to": {}});
    crate::hub_algoglyph::track_input(&ends, read)
}

/// One planner run. `read(name)` answers the data of resource `openttd://<name>`.
pub fn run(read: &dyn Fn(&str) -> Result<Value, String>, o: &Options) -> Result<Value, String> {
    let limit = o.limit;
    let towns = read("towns")?;
    let g = crate::graph::derive_with_ways(&read("map")?, &towns, &read("stations")?, &read("map/ways").unwrap_or(Value::Null))?;
    // Live vehicles when the game offers openttd://engines; the plan says so when it does not.
    let engines = read("engines").unwrap_or(Value::Null);
    // Construction costs follow the game's prices now (openttd://economy/prices) where it offers
    // them; a parameter the caller set wins.
    let prices = read("economy/prices").unwrap_or(Value::Null);
    let params = crate::plan::with_prices(&o.params, &prices);
    let mut out = crate::plan::plan(&towns, &g, limit, &params, &engines)?;
    // Freight from the world graph's industries, and the portfolio over passengers and freight.
    let world = read("world/graph").unwrap_or(Value::Null);
    let freight = crate::cargo::freight_pairs(&world, &engines, &out["params"], limit.max(10));
    out["freight"] = Value::Array(freight);
    // Station sites for every rail candidate (freight and passengers), by the bot's own rule
    // (src/sites.rs), so an executor builds the plan without a rule of its own.
    let map = read("map")?;
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
    if o.calibrate {
        let mut cands: Vec<Value> = out["pairs"].as_array().cloned().unwrap_or_default();
        cands.extend(out["freight"].as_array().cloned().unwrap_or_default());
        out["calibration"] = crate::calibrate::calibrate(&read("vehicles")?, &read("stations")?, &cands, crate::calibrate::REACH_TILES);
    }
    // track N: the N best rail lines of each list priced on the real map by AlgoGlyph's
    // track-planning (src/linecost.rs) instead of distance x tile cost, before the portfolio.
    if o.track > 0 {
        let opt = |r: &str| -> Result<Option<Value>, String> { Ok(read(r).ok()) };
        let mut base = linecost_input(&opt)?;
        if let Some(b) = base.as_object_mut() { b.remove("from"); b.remove("to"); }
        let stations = out["params"]["costs"]["rail_stations"].as_f64().unwrap_or(1480.0);
        let rail_tile = out["params"]["costs"]["rail_tile"].as_f64().unwrap_or(100.0);
        let mut log = Vec::new();
        for list in ["freight", "rail"] {
            let arr = if list == "freight" { out["freight"].as_array_mut() } else { out["by_mode"]["rail"].as_array_mut() };
            if let Some(arr) = arr { log.extend(crate::linecost::price(arr, &base, stations, rail_tile, o.track)); }
        }
        out["line_costs"] = json!({"solver": "algoglyph track-planning on map, map/ways, map/terrain, economy/prices, bridges",
                                   "priced": log});
        // yards N: prove up to N transfer yards offline (src/yards.rs): the better line's route
        // blocked, can the other still leave from a platform beside it?
        if o.yards > 0 {
            let mut lines: Vec<Value> = out["freight"].as_array().cloned().unwrap_or_default();
            lines.extend(out["by_mode"]["rail"].as_array().cloned().unwrap_or_default());
            out["yards"] = json!({"solver": "algoglyph track-planning, the better line's tiles blocked (src/yards.rs)",
                                  "pairs": crate::yards::prove(&lines, &base, o.yards)});
        }
    }
    let mut candidates: Vec<Value> = Vec::new();
    for (_, ps) in out["by_mode"].as_object().cloned().unwrap_or_default() {
        for p in ps.as_array().cloned().unwrap_or_default() { candidates.push(p); }
    }
    candidates.extend(out["freight"].as_array().cloned().unwrap_or_default());
    candidates.sort_by(|a, b| b["score"].as_f64().unwrap_or(0.0).total_cmp(&a["score"].as_f64().unwrap_or(0.0)));
    out["portfolio"] = crate::cargo::portfolio(&candidates, out["params"]["budget"].as_f64());
    if o.invest_inputs { return Ok(crate::invest::input(&out, &engines, &towns).unwrap_or(Value::Null)); }
    if o.invest { out["investment"] = crate::invest::decide(&out, &engines, &towns); }
    out["model"] = json!("v4: v3 for passengers, freight pairs from the world graph's industries priced with each cargo's own payment rate and transit periods, and a portfolio chosen by knapsack within the budget");
    Ok(out)
}
