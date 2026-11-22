//! RESEARCH-ONLY: what to build, decided over the live game by the AlgoGlyph kernel that satisfies
//! the capability `routes-by-return` in the discovered catalogue (`Kernels`; on the catalogue of
//! today, transport-investment).
//!
//! The planner (plan.rs, cargo.rs) already derives every candidate from the game: town pairs with
//! their passenger supply, freight pairs with each cargo's production, payment rate and transit
//! periods, and the vehicles the game offers now. This module hands those same facts to the kernel
//! as places, demands and modes, so the kernel can also consider routes the planner does not:
//! two legs through a hub (bus to a station, rail onwards), ranked by return on investment.
//!
//! A freight vehicle depends on its cargo (a coal wagon carries no wood), so each mode gives the
//! kernel its vehicles keyed by cargo (`vehicles: {cargo: vehicle}`) and a mode carries only the
//! cargos it has a vehicle for. One run covers every cargo, so a two-leg route through a hub is open
//! to freight as well. The routes the kernel ranks then go through the planner's portfolio (one
//! knapsack within the budget), exactly like the planner's own candidates. An answer that fails
//! its adapter's domain check is refused like a failed run, and the run names the kernel and the
//! catalogue it rests on, or why no kernel answered.
use serde_json::{json, Value};

use crate::cargo::{freight_vehicle, portfolio, Kernels};
use crate::plan::{calibration, pick_vehicle};

/// The capability the investment run is (battlebrain_contract.rs CAPABILITIES).
pub const ROUTES_BY_RETURN: &str = "routes-by-return";

/// The requirement fields of `ROUTES_BY_RETURN` that `input` builds; its `params` ride along.
const FIELDS: [&str; 4] = ["places", "demands", "modes", "prebuilt"];

fn num(v: &Value, k: &str) -> f64 { v.get(k).and_then(Value::as_f64).unwrap_or(0.0) }

/// An id as text: a string as it is, a number in digits ("industry:35", 7), never JSON-quoted.
fn id(v: &Value) -> String { v.as_str().map_or_else(|| v.to_string(), str::to_string) }

fn f(p: &Value, path: &str, d: f64) -> f64 { p.pointer(path).and_then(Value::as_f64).unwrap_or(d) }

fn vehicle(name: String, capacity: f64, speed: f64, price: f64, running: f64) -> Value {
    json!({"name": name, "price": price.round(), "running_per_year": running.round(), "capacity": capacity, "speed_kmh": speed})
}

/// The kernel's mode for one of the planner's modes, with a vehicle for each of `cargos` the game
/// offers one for; None when it offers none.
fn mode(name: &str, p: &Value, engines: &Value, cargos: &[String]) -> Option<Value> {
    let wagons = f(p, "/rail_wagons", 4.0);
    let mut vehicles = serde_json::Map::new();
    for c in cargos {
        let v = match name {
            "truck" | "freight_rail" => freight_vehicle(engines, c, name == "freight_rail", wagons).map(|(n, cap, s, pr, r)| vehicle(n, cap, s, pr, r)),
            _ => pick_vehicle(engines, name, wagons, p["small_airports"].as_bool().unwrap_or(true))
                .map(|v| vehicle(v.name, v.capacity, v.speed, v.price, v.running_per_year)),
        };
        if let Some(v) = v { vehicles.insert(c.clone(), v); }
    }
    if vehicles.is_empty() { return None; }
    let (road, rail) = (f(p, "/costs/road_tile", 190.0), f(p, "/costs/rail_tile", 100.0));
    let stop = f(p, "/freight/truck_stops", 400.0) / 2.0;
    // Per tile and per end: the planner's costs (each a pair of terminals) split in two.
    let (per_tile, terminal, band) = match name {
        "bus" => (road, stop, "/bands/bus"),
        "truck" => (road * 0.5, stop, "/freight/bands"),
        "rail" => (rail, f(p, "/costs/rail_stations", 1480.0) / 2.0, "/bands/rail"),
        "freight_rail" => (rail, f(p, "/costs/rail_stations", 1480.0) / 2.0, "/freight/bands"),
        "air" => (0.0, f(p, "/costs/airports", 14400.0) / 2.0, "/bands/air"),
        _ => return None,
    };
    let mut m = json!({"mode": name, "build_per_tile": per_tile, "terminal": terminal, "vehicles": vehicles,
                       "overhead_periods": f(p, "/overhead_periods", 2.0)});
    if let Some(lo) = p.pointer(&format!("{band}/0")).and_then(Value::as_f64) { m["min_leg"] = json!(lo); }
    if let Some(hi) = p.pointer(&format!("{band}/1")).and_then(Value::as_f64) { m["max_leg"] = json!(hi); }
    Some(m)
}

fn enabled(p: &Value, name: &str) -> bool {
    p["modes"].as_array().is_none_or(|a| a.iter().any(|m| m == name))
}

/// The kernel input for every cargo at once. `plan` is the planner's output (its `params`,
/// `by_mode` and `freight`), `engines` openttd://engines, `towns` openttd://towns. Every town is a
/// place (a hub for a two-leg route), and so is every industry of a freight pair.
pub fn input(plan: &Value, engines: &Value, towns: &Value) -> Option<Value> {
    let p = &plan["params"];
    // one_per_source off: the portfolio counts a producer's cargo once (cargo.rs `portfolio`), for
    // freight only; the kernel's own rule would also drop passenger pairs that share a town.
    let params = json!({"payback_months": f(p, "/payback_months", 24.0), "load_factor": f(p, "/load_factor", 0.8),
                        "max_legs": 2, "k": 8, "one_per_source": false});
    let cargos: Vec<Value> = engines["cargos"].as_array().cloned().unwrap_or_default();
    let mut places: Vec<Value> = towns["towns"].as_array().into_iter().flatten()
        .map(|t| json!({"id": format!("t{}", id(&t["id"])), "x": t["x"], "y": t["y"]})).collect();
    let mut demands = Vec::new();
    let mut prebuilt = Vec::new();

    let pax = cargos.iter().find(|c| c["passengers"] == true
        || c["name"].as_str().is_some_and(|n| n.to_lowercase().contains("passenger")));
    let pax_name = pax.and_then(|c| c["name"].as_str()).unwrap_or("Passengers").to_string();
    let (td1, td2) = pax.map_or((0.0, 24.0), |c| (c["transit_periods"][0].as_f64().unwrap_or(0.0), c["transit_periods"][1].as_f64().unwrap_or(24.0)));
    let payment = pax.map_or(f(p, "/payment", 3185.0), |c| num(c, "payment"));
    let mut seen = std::collections::HashSet::new();
    for (_, pairs) in plan["by_mode"].as_object().into_iter().flatten() {
        for c in pairs.as_array().into_iter().flatten() {
            let (a, b) = (id(&c["a"]["id"]), id(&c["b"]["id"]));
            if !seen.insert((a.clone(), b.clone())) { continue; }
            demands.push(json!({"id": format!("t{a}-t{b}"), "from": format!("t{a}"), "to": format!("t{b}"), "cargo": pax_name,
                "volume": (num(c, "supply") * f(p, "/supply_factor", 0.1)).round(), "payment": payment,
                "td1": td1, "td2": td2, "two_way": true}));
            if c["road_connected"] == true { prebuilt.push(json!({"from": format!("t{a}"), "to": format!("t{b}"), "mode": "bus"})); }
        }
    }

    let mut freight_cargos: Vec<String> = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for c in plan["freight"].as_array().into_iter().flatten() {
        let cargo = c["cargo"].as_str().unwrap_or("").to_string();
        let Some(cs) = cargos.iter().find(|x| x["name"] == cargo.as_str()) else { continue };
        for end in ["producer", "acceptor"] {
            let pid = format!("i{}", id(&c[end]["id"]));
            if ids.insert(pid.clone()) { places.push(json!({"id": pid, "x": c[end]["x"], "y": c[end]["y"]})); }
        }
        let (from, to) = (format!("i{}", id(&c["producer"]["id"])), format!("i{}", id(&c["acceptor"]["id"])));
        demands.push(json!({"id": format!("{from}-{to}"), "from": from, "to": to, "cargo": cargo,
            "volume": num(c, "supply_per_month"), "payment": num(cs, "payment"),
            "td1": cs["transit_periods"][0].as_f64().unwrap_or(0.0), "td2": cs["transit_periods"][1].as_f64().unwrap_or(0.0)}));
        if !freight_cargos.contains(&cargo) { freight_cargos.push(cargo); }
    }

    let pax_only = [pax_name];
    let modes: Vec<Value> = [("bus", &pax_only[..]), ("rail", &pax_only[..]), ("air", &pax_only[..]),
                             ("truck", &freight_cargos[..]), ("freight_rail", &freight_cargos[..])]
        .iter().filter(|(m, _)| enabled(p, m)).filter_map(|(m, cs)| mode(m, p, engines, cs)).collect();
    if demands.is_empty() || modes.is_empty() { return None; }
    Some(json!({"places": places, "demands": demands, "modes": modes, "prebuilt": prebuilt, "params": params}))
}

/// The label of a route's mode: its legs' modes joined, e.g. "bus+rail".
fn route_mode(route: &Value) -> String {
    route["legs"].as_array().into_iter().flatten().filter_map(|l| l["mode"].as_str()).collect::<Vec<_>>().join("+")
}

/// A kernel route with the planner's calibration applied: the kernel prices routes from the game's
/// payment formula and knows nothing of `params.calibration` (what each mode really earned against
/// what the planner predicted, calibrate.rs). Without this, a calibrated plan still ranked air at its
/// uncalibrated ROI. The calibration is a ratio of net profit (plan.rs net_profit), so it scales
/// `monthly_profit`; `score` and `roi` follow the kernel's own definitions (AlgoGlyph
/// transport-investment invest.ts: score = profit x payback_months - investment, roi = 12 x profit /
/// investment). A route of several legs takes the lowest calibration of its legs' modes.
pub fn calibrated(route: &Value, p: &Value) -> Value {
    let modes: Vec<&str> = route["legs"].as_array().into_iter().flatten().filter_map(|l| l["mode"].as_str()).collect();
    let cal = modes.iter().map(|m| calibration(p, m)).fold(f64::INFINITY, f64::min);
    if !cal.is_finite() || (cal - 1.0).abs() < 1e-12 { return route.clone(); }
    let profit = num(route, "monthly_profit") * cal;
    let investment = num(route, "investment");
    let mut r = route.clone();
    r["monthly_profit"] = json!(profit);
    r["score"] = json!(profit * f(p, "/payback_months", 24.0) - investment);
    r["roi"] = json!(if investment > 0.0 { 12.0 * profit / investment } else if profit > 0.0 { f64::INFINITY } else { 0.0 });
    r["calibration"] = json!(cal);
    r
}

/// Run the kernel once over every cargo and choose the portfolio over the routes it ranks.
pub fn decide(plan: &Value, engines: &Value, towns: &Value, kernels: &Kernels) -> Value {
    let Some(input) = input(plan, engines, towns) else {
        return json!({"solver": "none", "note": "no demand the game has a vehicle for", "runs": [], "candidates": 0,
                      "portfolio": portfolio(&[], plan["params"]["budget"].as_f64(), kernels)});
    };
    let demand = |d: &Value| input["demands"].as_array().into_iter().flatten().find(|x| x["id"] == *d).cloned().unwrap_or(Value::Null);
    let mut runs = Vec::new();
    let mut candidates = Vec::new();
    let called = kernels.call(ROUTES_BY_RETURN, |a| {
        let mut i = a.input(&FIELDS.map(|f| (f, input[f].clone())))?;
        i["params"] = input["params"].clone();
        Ok(i)
    }).and_then(|c| c.check().map(|_| c));
    let solver = match &called {
        Ok(c) => format!("{} over every cargo at once, then the portfolio knapsack", c.solver()),
        Err(_) => "none: no kernel answered routes-by-return (runs says why), so only the portfolio ran".to_string(),
    };
    match called {
        Ok(c) => {
            let (routes, picked, whys) = (c.get("routes"), c.get("chosen"), c.get("why"));
            // The kernel ranks each demand's routes by ROI; the portfolio maximises score (profit
            // over the payback period less investment). So each demand enters the portfolio with
            // its highest-score route among all the kernel's options (each with its best-score
            // fleet), and the kernel's ROI pick stays in `why`.
            for c in picked.as_array().into_iter().flatten() {
                let all: Vec<Value> = routes.as_array().into_iter().flatten()
                    .flat_map(|o| o.as_array().into_iter().flatten()).filter(|o| o["demand"] == c["demand"])
                    .map(|o| calibrated(o, &plan["params"])).collect();
                let fallback = calibrated(&c["option"], &plan["params"]);
                let route = all.iter().max_by(|a, b| num(a, "score").total_cmp(&num(b, "score"))).unwrap_or(&fallback);
                if num(route, "score") <= 0.0 { continue; }
                let label = route_mode(route);
                let mut why = whys.as_array().into_iter().flatten().find(|w| w["demand"] == c["demand"]).cloned().unwrap_or(Value::Null);
                if why.is_object() { why["by_score"] = json!({"mode": label, "roi": route["roi"], "score": num(route, "score").round()}); }
                let dm = demand(&c["demand"]);
                // `from` names the producer, so the portfolio counts a producer's cargo once;
                // passengers (two-way, any house) are not split that way.
                candidates.push(json!({"kind": "investment", "cargo": dm["cargo"], "from": dm["from"],
                    "passengers": dm["two_way"] == true, "demand": c["demand"], "mode": label,
                    "score": num(route, "score").round(), "roi": route["roi"], "why": why,
                    "options": [{"mode": label, "investment": num(route, "investment").round(), "score": num(route, "score").round(),
                                 "monthly_profit": num(route, "monthly_profit").round(), "roi": route["roi"], "legs": route["legs"],
                                 "carried": route["carried"], "transfers": route["transfers"], "transit_periods": route["transit_periods"]}]}));
            }
            runs.push(json!({"ok": true, "demands": input["demands"].as_array().map_or(0, Vec::len),
                             "modes": input["modes"].as_array().map_or(0, Vec::len),
                             "routes_considered": c.answer.pointer("/result/routes_considered"), "chosen": picked.as_array().map_or(0, Vec::len),
                             "kernel": c.kernel()}));
        }
        Err(f) => runs.push(json!({"ok": false, "error": f.message(ROUTES_BY_RETURN), "refused": f.to_json(ROUTES_BY_RETURN)})),
    }
    candidates.sort_by(|a, b| num(b, "score").total_cmp(&num(a, "score")));
    let multi = candidates.iter().filter(|c| c["mode"].as_str().is_some_and(|m| m.contains('+'))).count();
    json!({"solver": solver, "runs": runs, "candidates": candidates.len(), "multi_leg": multi,
           "portfolio": portfolio(&candidates, plan["params"]["budget"].as_f64(), kernels)})
}

#[cfg(test)]
mod tests {
    use super::{decide, input, ROUTES_BY_RETURN};
    use crate::cargo::{testing, KernelFailure};
    use serde_json::{json, Value};

    fn engines() -> Value {
        json!({
            "cargos": [{"name": "Passengers", "passengers": true, "payment": 3185, "transit_periods": [0, 24]},
                       {"name": "Coal", "payment": 5916, "transit_periods": [7, 255]}],
            "engines": [
                {"name": "Bus", "type": "road", "cargo": "Passengers", "capacity": 31, "effective_speed_kmh": 56, "price": 8000, "running_cost_per_year": 1500},
                {"name": "Coal Truck", "type": "road", "cargo": "Coal", "capacity": 20, "effective_speed_kmh": 48, "price": 9000, "running_cost_per_year": 1600}]})
    }
    fn plan() -> Value {
        json!({
            "params": {"modes": ["bus", "truck"], "supply_factor": 0.1, "bands": {"bus": [12, 70]}, "freight": {"bands": [20, 250]}, "budget": 100000},
            "by_mode": {"bus": [{"a": {"id": 1}, "b": {"id": 2}, "supply": 500, "road_connected": true}]},
            "freight": [{"cargo": "Coal", "supply_per_month": 90, "producer": {"id": "industry:7", "x": 1, "y": 1}, "acceptor": {"id": 9, "x": 40, "y": 1}}]})
    }
    fn towns() -> Value {
        json!({"towns": [{"id": 1, "x": 5, "y": 5}, {"id": 2, "x": 40, "y": 5}]})
    }
    fn route(demand: &str, modes: &[&str], investment: f64, score: f64) -> Value {
        json!({"demand": demand, "legs": modes.iter().map(|m| json!({"mode": m})).collect::<Vec<_>>(), "investment": investment,
               "score": score, "roi": 2.0, "monthly_profit": 100.0, "carried": 50, "transfers": modes.len() - 1, "transit_periods": 3})
    }
    /// The kernel's answer for `plan()`: a bus route and a truck + rail route through a hub.
    fn answer() -> Value {
        let (bus, freight) = (route("t1-t2", &["bus"], 1000.0, 500.0), route("iindustry:7-i9", &["truck", "freight_rail"], 2000.0, 800.0));
        json!({"ok": true, "kernel": {"id": "transport-investment", "plate": "invest"}, "work": {"routes": 7},
               "result": {"options": [[bus.clone()], [freight.clone(), route("iindustry:7-i9", &["truck"], 900.0, 100.0)]],
                          "chosen": [{"demand": "t1-t2", "option": bus}, {"demand": "iindustry:7-i9", "option": freight}],
                          "why": [{"demand": "t1-t2", "mode": "bus", "roi": 2.0}, {"demand": "iindustry:7-i9", "mode": "truck+freight_rail", "roi": 2.0}],
                          "routes_considered": 7}})
    }

    #[test]
    fn the_run_is_the_kernel_the_catalogue_matches_and_its_routes_enter_the_portfolio() {
        let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = asked.clone();
        let k = testing::kernels(move |k: &str, i: &Value| {
            log.borrow_mut().push((k.to_string(), i.clone()));
            if k == "transport-investment" { Ok(answer()) } else { Err(KernelFailure::Unset) }
        });
        let d = decide(&plan(), &engines(), &towns(), &k);
        assert_eq!(d["solver"], "algoglyph transport-investment over every cargo at once, then the portfolio knapsack");
        assert_eq!(d["runs"][0]["ok"], true);
        assert_eq!(d["runs"][0]["kernel"]["capability"], ROUTES_BY_RETURN);
        assert_eq!(d["runs"][0]["kernel"]["catalogue"]["commit"], "1b8e009059c299b29d7a829d2d51d4e057304d4d");
        assert_eq!((d["runs"][0]["routes_considered"].clone(), d["runs"][0]["chosen"].clone()), (json!(7), json!(2)));
        assert_eq!((d["candidates"].clone(), d["multi_leg"].clone()), (json!(2), json!(1)));
        assert_eq!(d["portfolio"]["chosen"][0]["mode"], "truck+freight_rail");
        assert_eq!(d["portfolio"]["chosen"][0]["why"]["by_score"]["score"], 800.0);
        assert_eq!(d["portfolio"]["fallback"]["reason"], "unset_dir", "the portfolio's own kernel call failed: the built-in decided");
        // The input went in under the adapter's keys, with the params riding along.
        let asked = asked.borrow();
        assert_eq!(asked[0].0, "transport-investment");
        let i = &asked[0].1;
        assert_eq!(i, &input(&plan(), &engines(), &towns()).unwrap());
        assert_eq!(i["params"]["max_legs"], 2);
        assert_eq!(asked[1].0, "knapsack-dp");
        let matched: Vec<Value> = k.record()["matches"].as_array().unwrap().iter().map(|m| m["chosen"].clone()).collect();
        assert_eq!(matched, vec![json!("transport-investment"), json!("knapsack-dp")]);
    }

    #[test]
    fn without_a_kernel_or_with_an_unchecked_answer_the_run_is_refused_with_its_reason() {
        let expired = testing::catalogue(|l| l.retain(|k| k["id"] != "transport-investment"));
        let d = decide(&plan(), &engines(), &towns(), &testing::on(expired, |k, _| match k {
            "knapsack-dp" => Err(KernelFailure::Unset),
            _ => panic!("an unmatched capability runs no kernel"),
        }));
        assert!(d["solver"].as_str().unwrap().starts_with("none: no kernel answered routes-by-return"));
        assert_eq!(d["runs"][0]["ok"], false);
        assert_eq!(d["runs"][0]["refused"]["refusal"]["code"], "kernel_expired");
        assert!(d["runs"][0]["error"].as_str().unwrap().starts_with("routes-by-return: kernel_expired ("));
        assert_eq!(d["candidates"], 0);
        // One route list for two demands: the answer fails its domain check and is not used.
        let mut short = answer();
        short["result"]["options"].as_array_mut().unwrap().pop();
        let d = decide(&plan(), &engines(), &towns(), &testing::kernels(move |k, _| if k == "knapsack-dp" { Err(KernelFailure::Unset) } else { Ok(short.clone()) }));
        assert_eq!(d["runs"][0]["refused"]["reason"], "domain_check_failed");
        assert_eq!(d["runs"][0]["refused"]["path"], "result.options");
        assert_eq!(d["candidates"], 0);
    }

    #[test]
    fn without_a_demand_nothing_runs_but_the_portfolio() {
        let k = testing::kernels(|_, _| panic!("no demand, no kernel run"));
        let d = decide(&json!({"params": {}}), &engines(), &towns(), &k);
        assert_eq!((d["solver"].clone(), d["candidates"].clone()), (json!("none"), json!(0)));
        assert_eq!(d["portfolio"]["solver"], "none", "no budget: no knapsack");
    }

    #[test]
    fn a_plan_becomes_one_kernel_input_with_vehicles_keyed_by_cargo() {
        let (plan, engines, towns) = (plan(), engines(), towns());
        let i = input(&plan, &engines, &towns).expect("an input");
        let d = i["demands"].as_array().unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["volume"], 50.0);
        assert_eq!(d[0]["two_way"], true);
        assert_eq!(d[1]["id"], "iindustry:7-i9");
        assert_eq!(d[1]["td1"], 7.0);
        assert_eq!(i["prebuilt"][0]["mode"], "bus");
        let modes = i["modes"].as_array().unwrap();
        let bus = modes.iter().find(|m| m["mode"] == "bus").unwrap();
        assert_eq!(bus["min_leg"], 12.0);
        assert_eq!(bus["vehicles"]["Passengers"]["name"], "Bus");
        let truck = modes.iter().find(|m| m["mode"] == "truck").unwrap();
        assert_eq!(truck["vehicles"]["Coal"]["name"], "Coal Truck");
        assert!(truck["vehicles"].get("Passengers").is_none());
        assert_eq!(i["places"].as_array().unwrap().len(), 4);
    }
    /// The kernel's routes get the planner's calibration: profit scaled, score and roi recomputed
    /// by the kernel's own formulas, the lowest leg calibration for a route of several legs.
    #[test]
    fn kernel_routes_take_the_calibration() {
        let p = json!({"calibration": {"air": 0.5, "bus": 0.8, "default": 1.0}, "payback_months": 24});
        let air = json!({"legs": [{"mode": "air"}], "monthly_profit": 1000.0, "investment": 6000.0, "score": 18000.0, "roi": 2.0});
        let a = super::calibrated(&air, &p);
        assert_eq!((a["monthly_profit"].as_f64(), a["score"].as_f64(), a["roi"].as_f64()), (Some(500.0), Some(6000.0), Some(1.0)));
        let two = json!({"legs": [{"mode": "bus"}, {"mode": "air"}], "monthly_profit": 100.0, "investment": 0.0});
        assert_eq!(super::calibrated(&two, &p)["calibration"], json!(0.5));
        let rail = json!({"legs": [{"mode": "rail"}], "monthly_profit": 100.0, "investment": 10.0, "score": 2390.0});
        assert_eq!(super::calibrated(&rail, &p), rail);
    }

}
