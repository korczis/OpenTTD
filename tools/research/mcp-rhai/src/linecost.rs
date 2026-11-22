//! RESEARCH-ONLY: what a planned rail line costs on this map, from the AlgoGlyph kernel that
//! satisfies the capability `rail-line-price` in the discovered catalogue (`Kernels`; on the
//! catalogue of today, track-planning).
//!
//! The planner prices a line as distance × the rail tile cost, a straight line on flat land. The
//! track planner lays the real route on the game's map, ways, terrain, prices and bridges (the
//! input hub_algoglyph.rs `track_input` assembles, read once here and shared by every candidate),
//! and answers what it costs in game money, or that there is no route. For the best rail
//! candidates this replaces the estimate: the option's build cost becomes the track's money plus
//! the stations, its score and investment follow, and a candidate with no route loses its rail
//! option. The track input is the one the rail executor (mcp-human rail.rs `plan_track`) lays
//! from: each station site's end tile, leaving by its exit, arriving against the far one's.
//! A found plan is priced only when it passes its adapter's domain check; each priced option
//! names the kernel and catalogue it rests on, or why no kernel priced it.
use serde_json::{json, Value};

use crate::cargo::{Call, KernelFailure, Kernels};

/// The capability a line's price is (battlebrain_contract.rs CAPABILITIES).
pub const RAIL_LINE_PRICE: &str = "rail-line-price";

/// One plan between `from` and `to` on the game's resources `base` (what hub_algoglyph.rs
/// `track_input` assembled) with `blocked` tiles kept free, by the kernel that satisfies
/// `RAIL_LINE_PRICE`. The requirement's fields go in under the adapter's keys; the resources it
/// does not map (ways, terrain, bridges) ride along as read.
pub fn route(kernels: &Kernels, base: &Value, from: &Value, to: &Value, blocked: &[[i64; 2]]) -> Result<Call, KernelFailure> {
    kernels.call(RAIL_LINE_PRICE, |a| {
        let mut fields = vec![("from", from.clone()), ("to", to.clone())];
        fields.extend(["map", "prices"].into_iter().filter_map(|f| base.get(f).map(|v| (f, v.clone()))));
        if !blocked.is_empty() { fields.push(("blocked", json!(blocked))); }
        let mut input = base.clone();
        for (k, v) in a.input(&fields)?.as_object().into_iter().flatten() { input[k] = v.clone(); }
        Ok(input)
    })
}

/// What a plan says of a line: its status, its money, the kernel it rests on, and why it priced
/// nothing when it did not (`refused`). `per_tile` turns the kernel's tile cost into money when
/// the game publishes no prices (`in_money` false).
pub struct Track {
    pub status: String,
    pub money: Option<f64>,
    pub counts: Value,
    pub pieces: usize,
    pub runs: usize,
    pub kernel: Value,
    pub refused: Value,
}

/// Read a plan (`route`). A found plan that fails its domain check prices nothing.
pub fn track(answer: &Result<Call, KernelFailure>, in_money: bool, per_tile: f64) -> Track {
    let none = |status: String, kernel: Value, f: &KernelFailure| Track {
        status, money: None, counts: Value::Null, pieces: 0, runs: 0, kernel, refused: f.to_json(RAIL_LINE_PRICE)};
    let c = match answer {
        Ok(c) => c,
        Err(f) => return none(format!("error: {}", f.message(RAIL_LINE_PRICE)), Value::Null, f),
    };
    let status = c.get("status").as_str().unwrap_or("?").to_string();
    if status == "found" {
        if let Err(f) = c.check() {
            return none(format!("refused: {}", f.message(c.adapter.kernel)), c.kernel(), &f);
        }
    }
    // Without the game's prices the price is null and the kernel's cost counts plain-track tiles
    // (the adapter's note); `per_tile` makes money of it. The cost, counts and runs are evidence
    // the requirement does not name, read from the answer as it is.
    let result = c.answer.get("result").unwrap_or(&c.answer);
    let money = if in_money { c.get("price").as_f64() } else { result["cost"].as_f64().or_else(|| c.get("price").as_f64()).map(|x| x * per_tile) };
    Track { status, money, counts: result["counts"].clone(), pieces: c.get("pieces").as_array().map_or(0, Vec::len),
            runs: result["runs"].as_array().map_or(0, Vec::len), kernel: c.kernel(), refused: Value::Null }
}

fn num(v: &Value, k: &str) -> f64 { v.get(k).and_then(Value::as_f64).unwrap_or(0.0) }

fn reverse(dir: &str) -> &'static str {
    match dir { "NE" => "SW", "SW" => "NE", "SE" => "NW", _ => "SE" }
}

/// How many times `coherent_input` reads the track input before it leaves the terrain out.
pub const READ_TRIES: usize = 3;

/// The track input every line is priced on (hub_algoglyph `track_input`, without from/to), read so
/// that its map and its terrain describe the same land, and how it was read. track-planning checks
/// every tile's slope in openttd://map/terrain against openttd://map's heights and refuses the whole
/// input when one differs ("read map and terrain at the same moment"). The resources are read one
/// after another while the game runs, so one terraform between the map and the terrain read (a
/// town's road, any company's) left every line of the round without a price: seed 20261122, January
/// 1954, 0 of 21 (research/mechanisms/planner-and-algoglyph.md). The map is read once more after the
/// rest: when its heights are the first read's, the terrain was read on that land. Otherwise the
/// whole input is read again, up to `tries` times, and then the terrain is left out (the kernel
/// prices on the map's own types) rather than refused, which the record says.
pub fn coherent_input(read: &dyn Fn(&str) -> Result<Option<Value>, String>, tries: usize) -> Result<(Value, Value), String> {
    let ends = json!({"from": {}, "to": {}});
    let tries = tries.max(1);
    let mut last = Value::Null;
    for reads in 1..=tries {
        let mut input = crate::hub_algoglyph::track_input(&ends, read)?;
        if let Some(o) = input.as_object_mut() { o.remove("from"); o.remove("to"); }
        if input.get("terrain").is_none() {
            return Ok((input, json!({"reads": reads, "coherent": "no openttd://map/terrain to check against the map"})));
        }
        let again = read("map")?;
        if again.as_ref().and_then(|m| m.get("heights")) == input["map"].get("heights") {
            return Ok((input, json!({"reads": reads, "coherent": true})));
        }
        last = input;
    }
    if let Some(o) = last.as_object_mut() { o.remove("terrain"); }
    Ok((last, json!({"reads": tries, "coherent": false,
                     "terrain": format!("left out: the map's heights changed during each of {tries} reads, and track-planning refuses a terrain read on other land")})))
}

/// The track-planning endpoints between two station sites (src/sites.rs), as the executor uses them.
pub fn endpoints(a: &Value, b: &Value) -> Option<(Value, Value)> {
    let dir_a = a.pointer("/exit/dir")?.as_str()?;
    let dir_b = b.pointer("/exit/dir")?.as_str()?;
    Some((json!({"x": a.pointer("/end/x")?, "y": a.pointer("/end/y")?, "dir": dir_a}),
          json!({"x": b.pointer("/end/x")?, "y": b.pointer("/end/y")?, "dir": reverse(dir_b)})))
}

/// Price the line of every rail candidate in `candidates` (the `limit` best by score) with the
/// track planner. `base` is the track input without from/to (the game's resources), `rail_tile` the
/// planner's cost of one plain tile (used only when `base` has no prices). Each priced
/// option gets `track: {status, money, estimated_build, counts, pieces, runs}`; the candidate's mode
/// and score are those of its best option afterwards. Returns one log entry per line priced.
pub fn price(candidates: &mut [Value], base: &Value, station_cost: f64, rail_tile: f64, limit: usize, kernels: &Kernels) -> Vec<Value> {
    // With the game's prices in the input the planner answers in game money; without them its cost
    // is in plain-track tile units, which the planner's own rail tile cost turns into money.
    let in_money = base.get("prices").is_some_and(Value::is_object);
    let mut order: Vec<usize> = (0..candidates.len())
        .filter(|&i| candidates[i]["options"].as_array().is_some_and(|o| o.iter().any(|o| o["mode"] == "rail" || o["mode"] == "freight_rail")))
        .filter(|&i| candidates[i]["stations"]["from"].is_object() && candidates[i]["stations"]["to"].is_object())
        .collect();
    order.sort_by(|&a, &b| num(&candidates[b], "score").total_cmp(&num(&candidates[a], "score")));
    // At most two lines per producer: the portfolio admits one line per producer, so pricing the
    // same mine's lines over and over left the bot with three priced lines out of eight.
    let mut per_source: std::collections::HashMap<String, usize> = Default::default();
    order.retain(|&i| {
        let key = candidates[i].pointer("/producer/id").map(|v| v.to_string())
            .unwrap_or_else(|| format!("{}-{}", candidates[i]["a"]["id"], candidates[i]["b"]["id"]));
        let n = per_source.entry(key).or_insert(0);
        *n += 1;
        *n <= 2
    });
    let mut log = Vec::new();
    for i in order.into_iter().take(limit) {
        let c = &mut candidates[i];
        let Some((from, to)) = endpoints(&c["stations"]["from"], &c["stations"]["to"]) else { continue };
        let Track { status, money, counts, pieces, runs, kernel, refused } = track(&route(kernels, base, &from, &to, &[]), in_money, rail_tile);
        let found = status == "found" && money.is_some();
        for o in c["options"].as_array_mut().into_iter().flatten() {
            if o["mode"] != "rail" && o["mode"] != "freight_rail" { continue; }
            let estimated = num(o, "build_cost");
            if found {
                let build = money.unwrap_or(0.0) + station_cost;
                let delta = build - estimated;
                o["build_cost"] = json!(build.round());
                o["investment"] = json!((num(o, "investment") + delta).round());
                o["score"] = json!((num(o, "score") - delta).round());
            } else {
                // No route on this map: the option cannot be built as planned.
                o["score"] = json!(f64::MIN.round());
            }
            o["track"] = json!({"status": status, "money": money, "estimated_build": estimated, "stations": station_cost,
                                "money_from": if in_money { "the planner's money (openttd://economy/prices)" } else { "cost x rail_tile (the game publishes no openttd://economy/prices)" },
                                "counts": counts, "pieces": pieces, "runs": runs, "from": from, "to": to,
                                "kernel": kernel, "refused": refused});
        }
        // The candidate follows its best option now.
        if let Some(best) = c["options"].as_array().and_then(|o| o.iter().max_by(|a, b| num(a, "score").total_cmp(&num(b, "score")))).cloned() {
            c["mode"] = best["mode"].clone();
            c["score"] = best["score"].clone();
        }
        log.push(json!({"candidate": i, "status": status, "money": money, "pieces": pieces, "mode_now": c["mode"], "score_now": c["score"],
                        "kernel": kernel["id"], "refused": refused["reason"]}));
    }
    log
}

#[cfg(test)]
mod tests {
    use super::{coherent_input, endpoints, price, route, track, RAIL_LINE_PRICE};
    use crate::cargo::{testing, KernelFailure};
    use serde_json::{json, Value};
    use std::cell::Cell;

    /// A 6 x 4 tile map, flat at height 1, and its terrain: the input track-planning prices on.
    fn land() -> (Value, Value) {
        (json!({"width": 6, "height": 4, "heights": ["010101010101", "010101010101", "010101010101", "010101010101"],
                "types": ["CCCCCC", "CCCCCC", "CCCCCC", "CCCCCC"]}),
         json!({"slopes": ["0000000000", "0000000000", "0000000000"], "buildable": [".....", ".....", "....."]}))
    }

    /// The same map one terraform later: tile (2, 1)'s north corner one level up. With `land`'s
    /// terrain the kernel refuses it: "terrain.slopes[0] says tile (1, 0) has Slope 0, its heights
    /// give 2: read map and terrain at the same moment" (AlgoGlyph 1b8e009, flow kernel track-planning).
    fn terraformed() -> Value {
        let (mut map, _) = land();
        map["heights"][1] = json!("010102010101");
        map
    }

    /// A game whose n-th read of openttd://map (from 0) answers `maps(n)`; the other resources are fixed.
    fn game<'a>(maps: &'a dyn Fn(usize) -> Value, terrain: Option<Value>, n: &'a Cell<usize>)
        -> impl Fn(&str) -> Result<Option<Value>, String> + 'a {
        move |r: &str| Ok(match r {
            "map" => { let i = n.get(); n.set(i + 1); Some(maps(i)) }
            "map/terrain" => terrain.clone(),
            "economy/prices" => Some(json!({"prices": {"build_rail": 75}})),
            _ => None,
        })
    }

    #[test]
    fn a_map_unchanged_across_the_reads_is_priced_with_its_terrain_after_one_read() {
        let (map, terrain) = land();
        let n = Cell::new(0);
        let (input, how) = coherent_input(&game(&|_| map.clone(), Some(terrain.clone()), &n), 3).unwrap();
        assert_eq!(how, json!({"reads": 1, "coherent": true}));
        assert_eq!(n.get(), 2, "the map is read once more after the terrain");
        assert_eq!((input["map"].clone(), input["terrain"].clone()), (map, terrain));
        assert!(input.get("from").is_none() && input.get("to").is_none(), "every line adds its own ends");
        assert_eq!(input["prices"]["prices"]["build_rail"], 75);
    }

    #[test]
    fn a_terraform_between_the_map_and_the_terrain_read_reads_the_input_again() {
        // January 1954 on seed 20261122: the first map read is the land before a terraform, the
        // terrain and every later read the land after it. What is compared is the map's heights
        // before and after the terrain read, not the terrain itself.
        let (_, terrain) = land();
        let n = Cell::new(0);
        let maps = |i: usize| if i == 0 { land().0 } else { terraformed() };
        let (input, how) = coherent_input(&game(&maps, Some(terrain.clone()), &n), 3).unwrap();
        assert_eq!(how, json!({"reads": 2, "coherent": true}));
        assert_eq!(n.get(), 4);
        assert_eq!((input["map"].clone(), input["terrain"].clone()), (terraformed(), terrain));
    }

    #[test]
    fn land_that_changes_during_every_read_is_priced_without_its_terrain_and_says_so() {
        let (_, terrain) = land();
        let n = Cell::new(0);
        let maps = |i: usize| if i % 2 == 0 { land().0 } else { terraformed() };
        let (input, how) = coherent_input(&game(&maps, Some(terrain), &n), 2).unwrap();
        assert_eq!((how["reads"].clone(), how["coherent"].clone()), (json!(2), json!(false)));
        assert!(how["terrain"].as_str().unwrap().starts_with("left out: the map's heights changed during each of 2 reads"));
        assert_eq!(n.get(), 4);
        assert!(input.get("terrain").is_none(), "the kernel would refuse it");
        assert_eq!(input["map"], land().0, "the map of the last input read (each check read after it differed)");
        // Zero tries still reads once.
        let n = Cell::new(0);
        let (_, how) = coherent_input(&game(&maps, Some(land().1), &n), 0).unwrap();
        assert_eq!(how["reads"], 1);
    }

    #[test]
    fn without_a_terrain_there_is_nothing_to_check_and_without_a_map_nothing_to_price() {
        let n = Cell::new(0);
        let (input, how) = coherent_input(&game(&|_| land().0, None, &n), 3).unwrap();
        assert_eq!((how["reads"].clone(), how["coherent"].clone()), (json!(1), json!("no openttd://map/terrain to check against the map")));
        assert_eq!(n.get(), 1, "no second map read");
        assert!(input.get("terrain").is_none());
        let none = |_: &str| -> Result<Option<Value>, String> { Ok(None) };
        assert!(coherent_input(&none, 3).unwrap_err().contains("openttd://map"));
        let broken = |r: &str| -> Result<Option<Value>, String> { if r == "map" { Err("HTTP 503".into()) } else { Ok(None) } };
        assert_eq!(coherent_input(&broken, 3).unwrap_err(), "HTTP 503");
    }

    fn site(x: i64, dir: &str) -> Value {
        json!({"end": {"x": x, "y": 10}, "exit": {"x": x + 1, "y": 10, "dir": dir}})
    }
    fn rail(producer: i64, score: f64) -> Value {
        json!({"producer": {"id": producer}, "mode": "freight_rail", "score": score, "stations": {"from": site(12, "SW"), "to": site(40, "NE")},
               "options": [{"mode": "truck", "score": 1.0}, {"mode": "freight_rail", "build_cost": 5000.0, "investment": 20000.0, "score": score}]})
    }
    fn plan(status: &str, money: Value, pieces: Value) -> Value {
        json!({"ok": true, "kernel": {"id": "track-planning", "plate": "track"}, "work": {"expanded": 9},
               "result": {"status": status, "cost": 60.0, "money": money, "pieces": pieces, "counts": {"track": 2}, "runs": [1]}})
    }
    fn base() -> Value {
        json!({"map": {"width": 64}, "ways": {"w": 1}, "prices": {"build_rail": 100}})
    }

    #[test]
    fn a_line_is_priced_by_the_kernel_the_catalogue_matches_through_its_adapter() {
        let (asked, run) = testing::recording(plan("found", json!(9000.0), json!([{"x": 13, "y": 10}])));
        let k = testing::kernels(run);
        let mut cands = vec![rail(1, 100.0), rail(2, 50.0)];
        let log = price(&mut cands, &base(), 1480.0, 100.0, 1, &k);
        assert_eq!(log.len(), 1, "the limit");
        assert_eq!((log[0]["kernel"].clone(), log[0]["refused"].clone()), (json!("track-planning"), Value::Null));
        let o = &cands[0]["options"][1];
        assert_eq!(o["build_cost"], json!(10480.0));
        assert_eq!(o["investment"], json!(25480.0));
        assert_eq!(o["track"]["kernel"]["capability"], RAIL_LINE_PRICE);
        assert_eq!(o["track"]["kernel"]["catalogue"]["commit"], "1b8e009059c299b29d7a829d2d51d4e057304d4d");
        assert_eq!((o["track"]["pieces"].clone(), o["track"]["runs"].clone(), o["track"]["counts"].clone()), (json!(1), json!(1), json!({"track": 2})));
        assert!(cands[0]["options"][0].get("track").is_none(), "a truck option is not a rail line");
        assert!(cands[1]["options"][1].get("track").is_none(), "beyond the limit");
        // The runner ran the matched kernel on the adapter's keys; ways rode along as read.
        let (kernel, input) = asked.borrow()[0].clone();
        assert_eq!(kernel, "track-planning");
        assert_eq!(input, json!({"map": {"width": 64}, "ways": {"w": 1}, "prices": {"build_rail": 100},
                                 "from": {"x": 12, "y": 10, "dir": "SW"}, "to": {"x": 40, "y": 10, "dir": "SW"}}));
        assert_eq!(k.record()["matches"][0]["chosen"], "track-planning");
    }

    #[test]
    fn blocked_tiles_go_in_under_the_adapters_key_and_only_when_there_are_some() {
        let (asked, run) = testing::recording(plan("found", json!(1.0), json!([1])));
        let k = testing::kernels(run);
        let (from, to) = (json!({"x": 1}), json!({"x": 2}));
        route(&k, &json!({"map": {}}), &from, &to, &[[3, 4]]).ok().unwrap();
        route(&k, &json!({"map": {}}), &from, &to, &[]).ok().unwrap();
        assert_eq!(asked.borrow()[0].1["blocked"], json!([[3, 4]]));
        assert!(asked.borrow()[1].1.get("blocked").is_none());
        assert!(asked.borrow()[1].1.get("prices").is_none(), "a resource the game does not publish is left out");
    }

    #[test]
    fn a_line_no_kernel_priced_says_why_and_loses_its_rail_option() {
        // No kernel satisfies the capability: nothing runs, and the option is not built as planned.
        let expired = testing::catalogue(|l| l.retain(|k| k["id"] != "track-planning"));
        let k = testing::on(expired, |_, _| panic!("an unmatched capability runs no kernel"));
        let mut cands = vec![rail(1, 100.0)];
        let log = price(&mut cands, &base(), 1480.0, 100.0, 4, &k);
        let t = &cands[0]["options"][1]["track"];
        assert!(t["status"].as_str().unwrap().starts_with("error: rail-line-price: kernel_expired ("), "{}", t["status"]);
        assert_eq!(t["refused"]["refusal"]["code"], "kernel_expired");
        assert_eq!(t["kernel"], Value::Null);
        assert_eq!(cands[0]["options"][1]["score"], json!(f64::MIN.round()));
        assert_eq!(cands[0]["mode"], "truck", "the candidate follows its best option");
        assert_eq!(log[0]["refused"], "capability_unmatched");
        // A found plan that lays nothing fails its domain check: it prices nothing either.
        let k = testing::kernels(|_, _| Ok(plan("found", json!(9000.0), json!([]))));
        let mut cands = vec![rail(1, 100.0)];
        price(&mut cands, &base(), 1480.0, 100.0, 4, &k);
        let t = &cands[0]["options"][1]["track"];
        assert!(t["status"].as_str().unwrap().starts_with("refused: track-planning: the answer fails its domain check at result.pieces"), "{}", t["status"]);
        assert_eq!(t["refused"]["reason"], "domain_check_failed");
        assert_eq!(t["kernel"]["id"], "track-planning");
        assert_eq!(cands[0]["options"][1]["score"], json!(f64::MIN.round()));
    }

    #[test]
    fn a_plan_is_read_by_its_status_and_the_games_prices() {
        let k = testing::kernels(|_, _| Ok(plan("unreachable", Value::Null, json!([]))));
        let t = track(&route(&k, &json!({}), &json!({}), &json!({}), &[]), true, 100.0);
        assert_eq!((t.status.as_str(), t.money, t.refused.clone()), ("unreachable", None, Value::Null), "no route is an answer, not a fault");
        // Without the game's prices the kernel's tile cost is made money with the planner's tile cost.
        let k = testing::kernels(|_, _| Ok(plan("found", Value::Null, json!([1]))));
        let t = track(&route(&k, &json!({}), &json!({}), &json!({}), &[]), false, 100.0);
        assert_eq!((t.status.as_str(), t.money), ("found", Some(6000.0)));
        let t = track(&Err(KernelFailure::Unset), true, 100.0);
        assert_eq!((t.status.as_str(), t.refused["reason"].clone()), ("error: MCP_ALGOGLYPH_DIR is not set", json!("unset_dir")));
    }

    #[test]
    fn the_line_runs_from_one_station_end_to_the_other_arriving_against_its_exit() {
        let a = json!({"end": {"x": 12, "y": 10}, "exit": {"x": 13, "y": 10, "dir": "SW"}});
        let b = json!({"end": {"x": 40, "y": 10}, "exit": {"x": 39, "y": 10, "dir": "NE"}});
        let (from, to) = endpoints(&a, &b).unwrap();
        assert_eq!(from, json!({"x": 12, "y": 10, "dir": "SW"}));
        assert_eq!(to, json!({"x": 40, "y": 10, "dir": "SW"}));
    }
}
