//! RESEARCH-ONLY: prove a transfer yard on the map before any bot builds one.
//!
//! Two platforms side by side where two planned lines meet (a yard) were tried twice in the bot and
//! reverted (knowledge the-bots-rail-pathfinder-lays-plain-track-only: one yard walled in by the
//! first line, −16% and −48% against no yards), with the lesson "before another try, prove the
//! corridor offline on openttd://map with the first line's path". This is that proof, by the same
//! track planner that prices the lines (the kernel that satisfies `rail-line-price`, src/linecost.rs):
//!
//! 1. For two priced rail lines A (the better) and B whose station sites coincide at one end (the
//!    shared industry or town), lay A's route.
//! 2. Block every tile A takes: its pieces, the span of each bridge and tunnel, both stations.
//! 3. Put B's platform beside A's at the shared end (one tile to either side, the same axis and exit
//!    direction, buildable and flat by the station-site rule of src/sites.rs) and lay B's route from
//!    there with A's tiles blocked.
//!
//! A yard is proven when B still has a route; the answer says on which side, and what B's track costs
//! with the yard against B on its own, and carries both lines as candidates an executor builds
//! (`a_candidate`, and `b_candidate` with its shared end on the yard platform). Nothing here builds anything: the bot gets a yard only for a
//! proven pair, in a later step, and the duel decides whether it pays.
use serde_json::{json, Value};

use crate::cargo::{Call, KernelFailure, Kernels};
use crate::linecost::{route, track};

/// Every tile a plan's `pieces` take: each track piece's tile, and for a bridge or tunnel both
/// heads and every tile between them along its axis.
pub fn tiles_of(pieces: &Value) -> Vec<[i64; 2]> {
    let xy = |v: &Value| Some([v.get("x")?.as_i64()?, v.get("y")?.as_i64()?]);
    let mut out = Vec::new();
    for p in pieces.as_array().into_iter().flatten() {
        if let Some(t) = xy(p) { out.push(t); continue; }
        if let (Some(a), Some(b)) = (p.get("from").and_then(xy), p.get("to").and_then(xy)) {
            let (dx, dy) = ((b[0] - a[0]).signum(), (b[1] - a[1]).signum());
            let mut t = a;
            out.push(t);
            while t != b && (dx != 0 || dy != 0) {
                t = [t[0] + dx, t[1] + dy];
                out.push(t);
            }
        }
    }
    out
}

/// The platform tiles of a station site (src/sites.rs: origin x, y, length, axis).
pub fn platform(site: &Value) -> Vec<[i64; 2]> {
    let (Some(x), Some(y)) = (site["x"].as_i64(), site["y"].as_i64()) else { return vec![] };
    let len = site["length"].as_i64().unwrap_or(3);
    let along_x = site["axis"] == "NE_SW";
    (0..len).map(|i| if along_x { [x + i, y] } else { [x, y + i] }).collect()
}

fn free_flat(map: &Value, x: i64, y: i64) -> bool {
    let (Some(w), Some(h)) = (map["width"].as_i64(), map["height"].as_i64()) else { return false };
    if x < 1 || y < 1 || x + 1 >= w || y + 1 >= h { return false; }
    let row = |k: &str, yy: i64| map[k].get(yy as usize).and_then(Value::as_str).map(str::as_bytes);
    let Some(types) = row("types", y) else { return false };
    if !matches!(types.get(x as usize), Some(b'C') | Some(b'T')) { return false; }
    let z = |xx: i64, yy: i64| -> Option<u32> {
        let r = row("heights", yy)?;
        u32::from_str_radix(std::str::from_utf8(r.get(2 * xx as usize..2 * xx as usize + 2)?).ok()?, 16).ok()
    };
    let z0 = z(x, y);
    z0.is_some() && z0 == z(x + 1, y) && z0 == z(x, y + 1) && z0 == z(x + 1, y + 1)
}

/// The same station site moved `side` tiles across its axis (a second platform beside the first):
/// the platform and its exit tile must be buildable and flat and not among `taken`.
pub fn beside(map: &Value, site: &Value, side: i64, taken: &[[i64; 2]]) -> Option<Value> {
    let along_x = site["axis"] == "NE_SW";
    let (dx, dy) = if along_x { (0, side) } else { (side, 0) };
    let mut s = site.clone();
    for (k, d) in [("x", dx), ("y", dy)] { s[k] = json!(site[k].as_i64()? + d); }
    for part in ["end", "exit"] {
        s[part]["x"] = json!(site[part]["x"].as_i64()? + dx);
        s[part]["y"] = json!(site[part]["y"].as_i64()? + dy);
    }
    s["depot"] = Value::Null;
    if let Some(w) = map["width"].as_i64() { s["tile"] = json!(s["y"].as_i64()? * w + s["x"].as_i64()?); }
    let exit = [s["exit"]["x"].as_i64()?, s["exit"]["y"].as_i64()?];
    let tiles: Vec<[i64; 2]> = platform(&s).into_iter().chain(std::iter::once(exit)).collect();
    tiles.iter().all(|t| free_flat(map, t[0], t[1]) && !taken.contains(t)).then_some(s)
}

/// Pairs of priced lines (indices into `lines`, better first) whose station sites coincide at one
/// end, with that end: "from" or "to" of each.
pub fn shared_ends(lines: &[Value]) -> Vec<(usize, usize, &'static str, &'static str)> {
    let tile = |c: &Value, end: &str| c["stations"][end]["tile"].as_i64();
    let mut out = Vec::new();
    for a in 0..lines.len() {
        for b in a + 1..lines.len() {
            'ends: for ea in ["from", "to"] {
                for eb in ["from", "to"] {
                    if tile(&lines[a], ea).is_some() && tile(&lines[a], ea) == tile(&lines[b], eb) {
                        out.push((a, b, ea, eb));
                        break 'ends;
                    }
                }
            }
        }
    }
    out
}

/// The money of a found plan that passed its domain check: game money when `base` carries the
/// game's prices, else the kernel's tile cost.
fn found_money(answer: &Result<Call, KernelFailure>, base: &Value) -> Option<f64> {
    let t = track(answer, base.get("prices").is_some_and(Value::is_object), 1.0);
    (t.status == "found").then_some(t.money).flatten()
}

/// Prove the yards of the best priced rail lines in `candidates`: at most `limit` pairs. Each entry
/// names both lines, the shared end, whether B still has a route from a platform beside A's with A's
/// tiles blocked, on which side, and B's track money with the yard against B alone.
pub fn prove(candidates: &[Value], base: &Value, limit: usize, kernels: &Kernels) -> Vec<Value> {
    let map = &base["map"];
    let mut lines: Vec<&Value> = candidates.iter()
        .filter(|c| c["options"].as_array().is_some_and(|o| o.iter().any(|o| o["track"]["status"] == "found")))
        .collect();
    lines.sort_by(|a, b| b["score"].as_f64().unwrap_or(0.0).total_cmp(&a["score"].as_f64().unwrap_or(0.0)));
    let owned: Vec<Value> = lines.iter().map(|c| (*c).clone()).collect();
    let name = |c: &Value| {
        let n = |k: &str| c[k]["name"].as_str().map(str::to_string);
        format!("{} -> {}", n("producer").or_else(|| n("a")).unwrap_or_default(), n("acceptor").or_else(|| n("b")).unwrap_or_default())
    };
    let mut out = Vec::new();
    for (a, b, ea, eb) in shared_ends(&owned).into_iter().take(limit) {
        let (la, lb) = (&owned[a], &owned[b]);
        let Some((fa, ta)) = crate::linecost::endpoints(&la["stations"]["from"], &la["stations"]["to"]) else { continue };
        let answer_a = route(kernels, base, &fa, &ta, &[]);
        let mut taken = answer_a.as_ref().map_or(vec![], |c| tiles_of(c.get("pieces")));
        for end in ["from", "to"] { taken.extend(platform(&la["stations"][end])); }
        let alone = lb.pointer("/options").and_then(Value::as_array)
            .and_then(|o| o.iter().find_map(|o| o["track"]["money"].as_f64()));
        let mut entry = json!({"a": name(la), "b": name(lb), "shared": {"a": ea, "b": eb, "end": la["stations"][ea]["end"]},
                               "a_route": found_money(&answer_a, base).is_some(),
                               "kernel": answer_a.as_ref().map_or_else(|f| f.to_json(crate::linecost::RAIL_LINE_PRICE), |c| c.kernel()), "a_tiles": taken.len(), "b_alone": alone,
                               "proven": false, "tried": []});
        for side in [1, -1] {
            let Some(yard) = beside(map, &la["stations"][ea], side, &taken) else {
                entry["tried"].as_array_mut().unwrap().push(json!({"side": side, "result": "no buildable flat platform beside"}));
                continue;
            };
            let other = &lb["stations"][if eb == "from" { "to" } else { "from" }];
            let (from_site, to_site) = if eb == "from" { (&yard, other) } else { (other, &yard) };
            let Some((f, t)) = crate::linecost::endpoints(from_site, to_site) else { continue };
            let answer_b = route(kernels, base, &f, &t, &taken);
            let money = found_money(&answer_b, base);
            entry["tried"].as_array_mut().unwrap().push(json!({"side": side, "result": if money.is_some() { "route" } else { "no route" }, "money": money}));
            if let Some(m) = money {
                entry["proven"] = json!(true);
                entry["side"] = json!(side);
                entry["yard"] = yard.clone();
                entry["b_with_yard"] = json!(m.round());
                entry["extra"] = alone.map_or(Value::Null, |x| json!((m - x).round()));
                // Both lines as an executor builds them (mcp-human `yard`): A as planned, B with its
                // shared end moved onto the yard platform beside A's.
                let mut b_yard = lb.clone();
                b_yard["stations"][eb] = yard.clone();
                entry["a_candidate"] = la.clone();
                entry["b_candidate"] = b_yard;
                break;
            }
        }
        out.push(entry);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_map(w: usize, h: usize) -> Value {
        json!({"width": w, "height": h, "types": vec!["C".repeat(w); h], "heights": vec!["00".repeat(w); h]})
    }

    #[test]
    fn a_bridge_takes_every_tile_between_its_heads() {
        let a = json!({"pieces": [{"kind": "track", "x": 5, "y": 5}, {"kind": "bridge", "from": {"x": 6, "y": 5}, "to": {"x": 9, "y": 5}}]});
        assert_eq!(tiles_of(&a["pieces"]), vec![[5, 5], [6, 5], [7, 5], [8, 5], [9, 5]]);
    }

    #[test]
    fn the_second_platform_lies_beside_the_first_on_free_flat_land_away_from_the_first_line() {
        let map = flat_map(30, 30);
        let site = json!({"x": 10, "y": 10, "length": 3, "axis": "NE_SW", "end": {"x": 12, "y": 10}, "exit": {"x": 13, "y": 10, "dir": "SW"}, "tile": 310});
        let s = beside(&map, &site, 1, &[]).unwrap();
        assert_eq!((s["y"].clone(), s["end"]["y"].clone(), s["exit"]["y"].clone()), (json!(11), json!(11), json!(11)));
        assert_eq!(s["tile"], json!(11 * 30 + 10));
        assert_eq!(platform(&s), vec![[10, 11], [11, 11], [12, 11]]);
        assert!(beside(&map, &site, 1, &[[13, 11]]).is_none(), "the first line runs over the exit");
        let mut hilly = map.clone();
        hilly["types"][9] = json!(format!("{}H{}", "C".repeat(11), "C".repeat(18)));
        assert!(beside(&hilly, &site, -1, &[]).is_none(), "a house beside the platform");
    }

    fn site(x: i64, y: i64, dir: &str, tile: i64) -> Value {
        json!({"x": x, "y": y, "length": 3, "axis": "NE_SW", "end": {"x": x + 2, "y": y}, "exit": {"x": x + 3, "y": y, "dir": dir}, "tile": tile})
    }
    fn line(to: Value, score: f64, money: f64) -> Value {
        json!({"producer": {"name": "Mine"}, "acceptor": {"name": "Plant"}, "score": score,
               "stations": {"from": site(10, 10, "SW", 310), "to": to}, "options": [{"track": {"status": "found", "money": money}}]})
    }

    #[test]
    fn a_yard_is_proven_by_the_kernel_the_catalogue_matches() {
        let lines = vec![line(site(10, 20, "NE", 610), 10.0, 5000.0), line(site(20, 5, "NE", 170), 5.0, 5000.0)];
        let base = json!({"map": flat_map(30, 30), "prices": {"build_rail": 100}});
        let answer = json!({"ok": true, "kernel": {"id": "track-planning"}, "result": {"status": "found", "cost": 70.0, "money": 7000.0, "pieces": [{"x": 14, "y": 10}]}});
        let (asked, run) = crate::cargo::testing::recording(answer);
        let out = prove(&lines, &base, 2, &crate::cargo::testing::kernels(run));
        assert_eq!(out.len(), 1);
        let e = &out[0];
        assert_eq!((e["a_route"].clone(), e["proven"].clone(), e["side"].clone()), (json!(true), json!(true), json!(1)));
        assert_eq!((e["b_with_yard"].clone(), e["extra"].clone()), (json!(7000.0), json!(2000.0)));
        assert_eq!(e["kernel"]["id"], "track-planning");
        assert_eq!(e["b_candidate"]["stations"]["from"]["y"], 11, "B leaves from the platform beside A's");
        // B's plan keeps A's route and both A stations free, under the adapter's key.
        let asked = asked.borrow();
        assert_eq!(asked[0].1.get("blocked"), None);
        let blocked = asked[1].1["blocked"].as_array().unwrap();
        assert!(blocked.contains(&json!([14, 10])) && blocked.contains(&json!([10, 20])), "{blocked:?}");
    }

    #[test]
    fn without_a_matching_kernel_no_yard_is_proven_and_the_refusal_is_kept() {
        let lines = vec![line(site(10, 20, "NE", 610), 10.0, 5000.0), line(site(20, 5, "NE", 170), 5.0, 5000.0)];
        let expired = crate::cargo::testing::catalogue(|l| l.retain(|k| k["id"] != "track-planning"));
        let k = crate::cargo::testing::on(expired, |_, _| panic!("an unmatched capability runs no kernel"));
        let out = prove(&lines, &json!({"map": flat_map(30, 30)}), 2, &k);
        assert_eq!((out[0]["a_route"].clone(), out[0]["proven"].clone()), (json!(false), json!(false)));
        assert_eq!(out[0]["kernel"]["refusal"]["code"], "kernel_expired");
        assert_eq!(out[0]["tried"][0]["result"], "no route");
    }

    #[test]
    fn lines_meet_where_their_station_sites_coincide() {
        let l = |f: i64, t: i64| json!({"stations": {"from": {"tile": f}, "to": {"tile": t}}});
        let lines = vec![l(1, 2), l(3, 2), l(4, 5), l(1, 6)];
        assert_eq!(shared_ends(&lines), vec![(0, 1, "to", "to"), (0, 3, "from", "from")]);
    }
}
