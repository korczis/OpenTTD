//! RESEARCH-ONLY: what a planned rail line costs on this map, from AlgoGlyph's track-planning.
//!
//! The planner prices a line as distance × the rail tile cost, a straight line on flat land. The
//! track planner lays the real route on the game's map, ways, terrain, prices and bridges (the
//! input hub_algoglyph.rs `track_input` assembles, read once here and shared by every candidate),
//! and answers what it costs in game money, or that there is no route. For the best rail
//! candidates this replaces the estimate: the option's build cost becomes the track's money plus
//! the stations, its score and investment follow, and a candidate with no route loses its rail
//! option. The track input is the one the rail executor (mcp-human rail.rs `plan_track`) lays
//! from: each station site's end tile, leaving by its exit, arriving against the far one's.
use serde_json::{json, Value};

use crate::cargo::algoglyph;

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
pub fn price(candidates: &mut [Value], base: &Value, station_cost: f64, rail_tile: f64, limit: usize) -> Vec<Value> {
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
        let mut input = base.clone();
        input["from"] = from.clone();
        input["to"] = to.clone();
        let answer = algoglyph("track-planning", &input);
        let (status, money, counts, pieces, runs) = match &answer {
            Ok(v) => {
                let r = v.get("result").unwrap_or(v);
                let money = if in_money { r["money"].as_f64() } else { r["cost"].as_f64().or_else(|| r["money"].as_f64()).map(|c| c * rail_tile) };
                (r["status"].as_str().unwrap_or("?").to_string(), money,
                 r["counts"].clone(), r["pieces"].as_array().map_or(0, Vec::len), r["runs"].as_array().map_or(0, Vec::len))
            }
            Err(e) => (format!("error: {e}"), None, Value::Null, 0, 0),
        };
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
                                "counts": counts, "pieces": pieces, "runs": runs, "from": from, "to": to});
        }
        // The candidate follows its best option now.
        if let Some(best) = c["options"].as_array().and_then(|o| o.iter().max_by(|a, b| num(a, "score").total_cmp(&num(b, "score")))).cloned() {
            c["mode"] = best["mode"].clone();
            c["score"] = best["score"].clone();
        }
        log.push(json!({"candidate": i, "status": status, "money": money, "pieces": pieces, "mode_now": c["mode"], "score_now": c["score"]}));
    }
    log
}

#[cfg(test)]
mod tests {
    use super::{coherent_input, endpoints};
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

    #[test]
    fn the_line_runs_from_one_station_end_to_the_other_arriving_against_its_exit() {
        let a = json!({"end": {"x": 12, "y": 10}, "exit": {"x": 13, "y": 10, "dir": "SW"}});
        let b = json!({"end": {"x": 40, "y": 10}, "exit": {"x": 39, "y": 10, "dir": "NE"}});
        let (from, to) = endpoints(&a, &b).unwrap();
        assert_eq!(from, json!({"x": 12, "y": 10, "dir": "SW"}));
        assert_eq!(to, json!({"x": 40, "y": 10, "dir": "SW"}));
    }
}
