//! RESEARCH-ONLY: the construction rules every audit of a game judges by, one implementation for
//! the hub's `hub.audit` (hub.rs) and the Rhai built-in `build_audit` (main.rs) that the panel's
//! `build_audit.rhai` service runs once a game month (research/mcp-hub.md, "No duplicate
//! buildings"; research/mechanisms/rhai-scripting.md).
//!
//! Pure functions over the game's own resources, no I/O: `openttd://stations` (catchments),
//! `openttd://depots` (tile, type, entrance, the service reach), `openttd://map/ways` (road bits and
//! rail TrackBits per tile) and `openttd://map` (tile types and owners). Nothing is hand-listed
//! that the game publishes; the two tables below are the game's own (src/station_type.h,
//! src/table/track_data.h), and a test reads the second one from the source.
//!
//! The rules:
//! - duplicate stations: one company owner, a shared facility, overlapping catchments;
//! - duplicate depots: one owner and type, the second within the game's service reach (along the
//!   road network for road depots when the ways are known);
//! - a depot's exit: `open` when a vehicle leaving it is joined to its entrance tile and can go on
//!   from there, `dead_end` when the entrance leads nowhere else, `no_exit` when the entrance does
//!   not lead into the depot at all, `unknown` for a ship depot or without ways. Signals, one-way
//!   roads, rail and road types and the owner of the entrance tile are not judged;
//! - road stops: a station tile with road bits is a drive-through stop when its bits are an axis
//!   (NW+SE or SW+NE), a bay when one direction; a drive-through road waypoint counts as
//!   drive-through.

use serde_json::{json, Value};

/// The catchment radius of each facility, used only for a station whose instance does not publish
/// `catchment_radius` (an older binary). Taken from src/station_type.h (CA_BUS, CA_TRUCK, CA_TRAIN,
/// CA_DOCK, and CA_UNMODIFIED for an airport, whose real radius comes from its spec).
const FALLBACK_RADIUS: [(&str, i64); 5] = [("bus", 3), ("truck", 3), ("train", 4), ("dock", 5), ("airport", 4)];

fn facilities(s: &Value) -> Vec<String> {
    s.get("facilities").and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The station's catchment as (x0, y0, x1, y1), inclusive: the game's own area when published,
/// else a square of its radius around its sign tile.
pub fn catchment(s: &Value) -> Option<(i64, i64, i64, i64)> {
    if let Some(a) = s.get("catchment_area").filter(|a| a.is_object()) {
        let (x, y) = (a.get("x")?.as_i64()?, a.get("y")?.as_i64()?);
        return Some((x, y, x + a.get("w")?.as_i64()? - 1, y + a.get("h")?.as_i64()? - 1));
    }
    let (x, y) = (s.get("x")?.as_i64()?, s.get("y")?.as_i64()?);
    let r = s.get("catchment_radius").and_then(Value::as_i64).unwrap_or_else(|| {
        facilities(s).iter().filter_map(|f| FALLBACK_RADIUS.iter().find(|(n, _)| n == f).map(|(_, r)| *r)).max().unwrap_or(0)
    });
    Some((x - r, y - r, x + r, y + r))
}

/// The game's company slots (MAX_COMPANIES, src/company_type.h). An owner at or above it is the
/// game's own: 15 a town, 16 nobody (an oil rig's heliport and dock), 17 water.
pub const MAX_COMPANIES: i64 = 15;

/// The company that built a station or depot, None when the game itself owns it. Only a company
/// can build a duplicate: two oil rigs 7 tiles apart, each with its own unowned airport and dock,
/// are the map, not a build (seeds 24455 and 2468, 2026-11-16).
pub fn company_owner(v: &Value) -> Option<i64> {
    v.get("owner").and_then(Value::as_i64).filter(|o| (0..MAX_COMPANIES).contains(o))
}

/// Pairs of stations that duplicate each other: one company owner, at least one facility in common,
/// and catchments that overlap, so both serve the same tiles with the same kind of transport. Two
/// stations of one owner far apart, a bus stop beside its own airport, or stations the game owns
/// (company_owner) are not duplicates.
pub fn duplicate_stations(stations: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for (i, a) in stations.iter().enumerate() {
        for b in &stations[i + 1..] {
            let (Some(oa), Some(ob)) = (company_owner(a), company_owner(b)) else { continue };
            if oa != ob { continue; }
            let fb = facilities(b);
            let shared: Vec<String> = facilities(a).into_iter().filter(|f| fb.contains(f)).collect();
            if shared.is_empty() { continue; }
            let (Some(ca), Some(cb)) = (catchment(a), catchment(b)) else { continue };
            if ca.0 > cb.2 || cb.0 > ca.2 || ca.1 > cb.3 || cb.1 > ca.3 { continue; }
            let pick = |s: &Value| json!({"id": s["id"], "name": s["name"], "x": s["x"], "y": s["y"], "tile": s["tile"], "town": s["town"]});
            let d = (a["x"].as_i64().unwrap_or(0) - b["x"].as_i64().unwrap_or(0)).abs()
                .max((a["y"].as_i64().unwrap_or(0) - b["y"].as_i64().unwrap_or(0)).abs());
            out.push(json!({"owner": oa, "facilities": shared, "a": pick(a), "b": pick(b), "distance": d}));
        }
    }
    out
}

/// The station list inside whatever an instance answered: an MCP resources/read result, a panel's
/// `{availability, data}` envelope, or a bare `{stations}`.
pub fn stations_in(v: &Value) -> Vec<Value> {
    unwrap_data(v).get("stations").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The ways as openttd://map/ways publishes them. Roads: per tile the directions a road vehicle can
/// leave it (NW=1 y-1, SW=2 x+1, SE=4 y+1, NE=8 x-1); two tiles are joined when each points at the
/// other. Rails: per tile its TrackBits (X=1, Y=2, upper=4, lower=8, left=16, right=32), absent
/// from an older binary (then every rail judgement is `unknown`).
pub struct Ways { pub width: i64, pub height: i64, roads: Vec<Vec<u8>>, rails: Vec<Vec<u8>> }

/// One row of hex digits, `digits` per tile.
fn hex_row(row: &Value, digits: usize) -> Vec<u8> {
    let s = row.as_str().unwrap_or("").as_bytes();
    s.chunks(digits).map(|c| std::str::from_utf8(c).ok().and_then(|t| u8::from_str_radix(t, 16).ok()).unwrap_or(0)).collect()
}

/// The four directions by DiagDirection (NE 0, SE 1, SW 2, NW 3, src/direction_type.h): the road
/// bit that leaves a tile that way and the step it makes.
const DIRS: [(u8, i64, i64); 4] = [(8, -1, 0), (4, 0, 1), (2, 1, 0), (1, 0, -1)];

/// The tracks a train can take entering a tile while moving in each direction (DiagdirReachesTracks,
/// the table _exitdir_reaches_trackdirs of src/table/track_data.h), as TrackBits.
pub const REACHES: [u8; 4] = [1 | 8 | 16, 2 | 16 | 4, 1 | 4 | 32, 2 | 32 | 8];

/// The direction opposite to `d`.
fn reverse(d: usize) -> usize { (d + 2) % 4 }

impl Ways {
    pub fn from_ways(v: &Value) -> Option<Ways> {
        let d = unwrap_data(v);
        let width = d.get("width")?.as_i64()?;
        let roads: Vec<Vec<u8>> = d.get("roads")?.as_array()?.iter().map(|r| hex_row(r, 1)).collect();
        let rails = d.get("rails").and_then(Value::as_array).map(|a| a.iter().map(|r| hex_row(r, 2)).collect()).unwrap_or_default();
        Some(Ways { width, height: roads.len() as i64, roads, rails })
    }

    fn at(&self, rows: &[Vec<u8>], x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width || y >= self.height { return 0; }
        rows.get(y as usize).and_then(|r| r.get(x as usize)).copied().unwrap_or(0)
    }

    fn bits(&self, x: i64, y: i64) -> u8 { self.at(&self.roads, x, y) }

    fn rail(&self, x: i64, y: i64) -> u8 { self.at(&self.rails, x, y) }

    /// Whether the rails layer was published.
    pub fn has_rails(&self) -> bool { !self.rails.is_empty() }

    /// The number of road steps from one tile to another over joined tiles, if at most `limit`.
    pub fn distance(&self, from: (i64, i64), to: (i64, i64), limit: i64) -> Option<i64> {
        let mut seen = std::collections::HashSet::from([from]);
        let mut q = std::collections::VecDeque::from([(from, 0i64)]);
        while let Some(((x, y), d)) = q.pop_front() {
            if (x, y) == to { return Some(d); }
            if d >= limit { continue; }
            for (k, &(bit, dx, dy)) in DIRS.iter().enumerate() {
                let n = (x + dx, y + dy);
                if self.bits(x, y) & bit != 0 && self.bits(n.0, n.1) & DIRS[reverse(k)].0 != 0 && seen.insert(n) {
                    q.push_back((n, d + 1));
                }
            }
        }
        None
    }
}

/// The payload inside whatever an instance answered: an MCP resources/read result, a panel's
/// `{availability, data}` envelope, or the bare document.
pub fn unwrap_data(v: &Value) -> Value {
    let mut v = v.clone();
    if let Some(text) = v.pointer("/contents/0/text").and_then(Value::as_str) {
        v = serde_json::from_str(text).unwrap_or(Value::Null);
    }
    v.get("data").filter(|d| d.is_object()).cloned().unwrap_or(v)
}

/// Pairs of depots that duplicate each other: one owner, one type, and the second within the reach a
/// vehicle searches for a depot to be serviced at (`service_reach_tiles`, the game's own setting),
/// measured along the road network when its ways are known, else as a straight distance (and said so).
pub fn duplicate_depots(depots: &[Value], reach: i64, roads: Option<&Ways>) -> Vec<Value> {
    let mut out = Vec::new();
    let xy = |d: &Value| (d["x"].as_i64().unwrap_or(0), d["y"].as_i64().unwrap_or(0));
    for (i, a) in depots.iter().enumerate() {
        for b in &depots[i + 1..] {
            let (Some(oa), Some(ob)) = (company_owner(a), company_owner(b)) else { continue };
            if oa != ob || a["type"] != b["type"] { continue; }
            let (pa, pb) = (xy(a), xy(b));
            let straight = (pa.0 - pb.0).abs().max((pa.1 - pb.1).abs());
            let (dist, by) = match (a["type"].as_str(), roads) {
                (Some("road"), Some(r)) => match r.distance(pa, pb, reach) { Some(d) => (d, "road network"), None => continue },
                _ => (straight, "straight distance"),
            };
            if dist > reach { continue; }
            let pick = |d: &Value| json!({"id": d["id"], "x": d["x"], "y": d["y"], "tile": d["tile"], "town": d["town"]});
            out.push(json!({"owner": a["owner"], "type": a["type"], "a": pick(a), "b": pick(b), "distance": dist, "by": by, "reach": reach}));
        }
    }
    out
}

/// The direction of the step from one tile to its neighbour, as a DiagDirection index.
fn dir_between(from: (i64, i64), to: (i64, i64)) -> Option<usize> {
    DIRS.iter().position(|&(_, dx, dy)| (from.0 + dx, from.1 + dy) == to)
}

fn xy(v: &Value) -> Option<(i64, i64)> {
    Some((v.get("x")?.as_i64()?, v.get("y")?.as_i64()?))
}

/// Whether a vehicle leaving a depot can drive on: `open`, `dead_end`, `no_exit` or `unknown` (see
/// the module comment). A road depot's entrance must point back at the depot and on to a tile joined
/// to it; a rail depot's entrance must hold a track a train reaches moving out of the depot, and one
/// such track must run on into a neighbour whose tracks a train reaches moving that way.
pub fn depot_exit(depot: &Value, ways: &Ways) -> &'static str {
    let (Some(at), Some(front)) = (xy(depot), depot.get("entrance").and_then(xy)) else { return "unknown" };
    let Some(k) = dir_between(at, front) else { return "unknown" };
    let step = |d: usize| (front.0 + DIRS[d].1, front.1 + DIRS[d].2);
    let onward = match depot.get("type").and_then(Value::as_str) {
        Some("road") => {
            let bits = ways.bits(front.0, front.1);
            if bits & DIRS[reverse(k)].0 == 0 { return "no_exit"; }
            (0..4).filter(|&d| d != reverse(k) && bits & DIRS[d].0 != 0)
                .any(|d| { let n = step(d); ways.bits(n.0, n.1) & DIRS[reverse(d)].0 != 0 })
        }
        Some("rail") if ways.has_rails() => {
            let reached = ways.rail(front.0, front.1) & REACHES[k];
            if reached == 0 { return "no_exit"; }
            // A track entered moving k leaves through its other side: the other direction m it is
            // entered by (REACHES[m] holds it) is the way back, so the way on is reverse(m).
            (0..8).map(|b| 1u8 << b).filter(|t| reached & t != 0)
                .filter_map(|t| (0..4).find(|&m| m != k && REACHES[m] & t != 0).map(reverse))
                .any(|d| { let n = step(d); ways.rail(n.0, n.1) & REACHES[d] != 0 })
        }
        _ => return "unknown",
    };
    if onward { "open" } else { "dead_end" }
}

/// A company id from openttd://map's owner character (0-9 then a-e), None for a town, nobody,
/// water or a tile no one can own.
fn owner_of(c: char) -> Option<i64> {
    c.to_digit(16).map(i64::from).filter(|o| (0..MAX_COMPANIES).contains(o))
}

/// Every road stop tile with its owner and kind: a station tile (openttd://map type `S`) with road
/// bits and no rail is drive-through when its bits are an axis, a bay when they are one direction.
pub fn road_stops(map: &Value, ways: &Ways) -> Vec<(i64, i64, i64, &'static str)> {
    let rows = |k: &str| map.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let (types, owners) = (rows("types"), rows("owners"));
    let mut out = Vec::new();
    for (y, row) in types.iter().enumerate() {
        let own: Vec<char> = owners.get(y).and_then(Value::as_str).unwrap_or("").chars().collect();
        for (x, t) in row.as_str().unwrap_or("").chars().enumerate() {
            let (x, y) = (x as i64, y as i64);
            if t != 'S' || ways.rail(x, y) != 0 { continue; }
            let Some(o) = own.get(x as usize).copied().and_then(owner_of) else { continue };
            match ways.bits(x, y) {
                5 | 10 => out.push((o, x, y, "drive_through")),
                1 | 2 | 4 | 8 => out.push((o, x, y, "bay")),
                _ => {}
            }
        }
    }
    out
}

/// One finding of the audit: a stable key (what it is, not its numbers), its kind, where, and one
/// sentence.
fn defect(key: String, kind: &str, at: (i64, i64), what: String) -> Value {
    json!({"key": key, "kind": kind, "x": at.0, "y": at.1, "what": what})
}

/// The whole construction audit of one game, per company, from `{stations, depots, ways, map}`
/// (each a resource as `read()` answers it, its payload, or null when not published): depots by
/// type and how many can be left, duplicate depots and stations, road stops drive-through against
/// bay, and every defect with a key that stays the same while the defect stays.
pub fn audit(input: &Value) -> Value {
    use std::collections::BTreeMap;
    let stations = stations_in(&input["stations"]);
    let doc = unwrap_data(&input["depots"]);
    let depots = doc.get("depots").and_then(Value::as_array).cloned().unwrap_or_default();
    let reach = doc.get("service_reach_tiles").and_then(Value::as_i64).unwrap_or(20);
    let ways = Ways::from_ways(&input["ways"]);
    let map = unwrap_data(&input["map"]);
    let stops = match (&ways, map.get("types").is_some()) {
        (Some(w), true) => Some(road_stops(&map, w)),
        _ => None,
    };
    // Each company's record, created at its first building; `bump` counts one more at a path in it.
    let mut by: BTreeMap<i64, Value> = BTreeMap::new();
    fn company(by: &mut BTreeMap<i64, Value>, c: i64) -> &mut Value {
        by.entry(c).or_insert_with(|| json!({"company": c,
            "depots": {"road": 0, "rail": 0, "ship": 0, "other": 0},
            "exits": {"open": 0, "dead_end": 0, "no_exit": 0, "unknown": 0},
            "stops": {"drive_through": 0, "bay": 0, "drive_through_share": null},
            "duplicate_depots": 0, "duplicate_stations": 0, "defects": []}))
    }
    fn bump(v: &mut Value, path: &[&str]) {
        let mut cur = v;
        for p in path { cur = &mut cur[*p]; }
        *cur = json!(cur.as_i64().unwrap_or(0) + 1);
    }
    let push = |by: &mut BTreeMap<i64, Value>, c: i64, d: Value| {
        if let Some(a) = company(by, c)["defects"].as_array_mut() { a.push(d); }
    };
    for d in &depots {
        let Some(c) = company_owner(d) else { continue };
        let ty = d.get("type").and_then(Value::as_str).unwrap_or("other");
        let ty = if ["road", "rail", "ship"].contains(&ty) { ty } else { "other" };
        bump(company(&mut by, c), &["depots", ty]);
        let exit = ways.as_ref().map_or("unknown", |w| depot_exit(d, w));
        bump(company(&mut by, c), &["exits", exit]);
        let at = xy(d).unwrap_or((0, 0));
        let front = d.get("entrance").and_then(xy).unwrap_or((0, 0));
        let what = match exit {
            "no_exit" => format!("{ty} depot at {},{}: its entrance {},{} does not lead into it", at.0, at.1, front.0, front.1),
            "dead_end" => format!("{ty} depot at {},{}: its entrance {},{} leads nowhere else", at.0, at.1, front.0, front.1),
            _ => continue,
        };
        push(&mut by, c, defect(format!("{exit}:{ty}:{}", d["tile"]), exit, at, what));
    }
    for p in duplicate_depots(&depots, reach, ways.as_ref()) {
        let Some(c) = company_owner(&p) else { continue };
        bump(company(&mut by, c), &["duplicate_depots"]);
        let (a, b) = (xy(&p["a"]).unwrap_or((0, 0)), xy(&p["b"]).unwrap_or((0, 0)));
        let (ta, tb) = (p["a"]["tile"].as_i64().unwrap_or(0), p["b"]["tile"].as_i64().unwrap_or(0));
        let what = format!("{} depots at {},{} and {},{} are {} tiles apart by {} (service reach {reach})",
                           p["type"].as_str().unwrap_or("?"), a.0, a.1, b.0, b.1, p["distance"], p["by"].as_str().unwrap_or("?"));
        push(&mut by, c, defect(format!("duplicate_depot:{}:{}-{}", p["type"].as_str().unwrap_or("?"), ta.min(tb), ta.max(tb)), "duplicate_depot", a, what));
    }
    for p in duplicate_stations(&stations) {
        let Some(c) = company_owner(&p) else { continue };
        bump(company(&mut by, c), &["duplicate_stations"]);
        let (ia, ib) = (p["a"]["id"].as_i64().unwrap_or(0), p["b"]["id"].as_i64().unwrap_or(0));
        let shared: Vec<&str> = p["facilities"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let name = |s: &Value| s["name"].as_str().map(str::to_string).unwrap_or_else(|| format!("#{}", s["id"]));
        let what = format!("stations {} and {} share {} and their catchments overlap", name(&p["a"]), name(&p["b"]), shared.join(", "));
        push(&mut by, c, defect(format!("duplicate_station:{}-{}", ia.min(ib), ia.max(ib)), "duplicate_station", xy(&p["a"]).unwrap_or((0, 0)), what));
    }
    for &(c, x, y, kind) in stops.iter().flatten() {
        bump(company(&mut by, c), &["stops", kind]);
        if kind == "bay" {
            push(&mut by, c, defect(format!("bay_stop:{x},{y}"), "bay_stop", (x, y), format!("road stop at {x},{y} is a bay, not drive-through")));
        }
    }
    for v in by.values_mut() {
        let (dt, bay) = (v["stops"]["drive_through"].as_i64().unwrap_or(0), v["stops"]["bay"].as_i64().unwrap_or(0));
        if stops.is_some() && dt + bay > 0 { v["stops"]["drive_through_share"] = json!(dt as f64 / (dt + bay) as f64); }
        if let Some(a) = v["defects"].as_array_mut() { a.sort_by(|x, y| x["key"].as_str().cmp(&y["key"].as_str())); }
    }
    json!({"reach": reach, "network": ways.is_some(), "rails": ways.as_ref().is_some_and(Ways::has_rails), "stops_known": stops.is_some(),
           "catchment": if stations.iter().any(|s| s.get("catchment_area").is_some()) { "game" } else { "fallback radius" },
           "companies": by.into_values().collect::<Vec<_>>()})
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 10 x 5 world. Company 0: road depots D1 (1,1) and D4 (2,0) joined by road 2 tiles apart, D2
    /// (1,3) facing a tile without road, a ship depot, a drive-through stop (3,0), a bay (4,0), two bus
    /// stations with overlapping catchments. Company 1: road depot D3 (5,3) whose entrance (5,2)
    /// points back and nowhere else, rail depot R1 (6,0) on a line, R2 (6,4) facing a Y track, a
    /// drive-through stop (2,4) and a rail station tile (6,2).
    /// A depot as openttd://depots lists it on the 10-wide map of `world`.
    pub(crate) fn depot(id: i64, owner: i64, ty: &str, at: (i64, i64), front: Option<(i64, i64)>) -> Value {
        let mut d = json!({"id": id, "owner": owner, "type": ty, "x": at.0, "y": at.1, "tile": at.1 * 10 + at.0, "town": 0});
        if let Some(f) = front { d["entrance"] = json!({"x": f.0, "y": f.1, "tile": f.1 * 10 + f.0}); }
        d
    }

    pub(crate) fn world() -> Value {
        let station = |id: i64, owner: i64, x: i64| json!({"id": id, "name": format!("S{id}"), "owner": owner, "x": x, "y": 0,
            "tile": x, "facilities": ["bus"], "catchment_area": {"x": x - 3, "y": 0, "w": 7, "h": 4}});
        json!({
            "stations": {"availability": "available", "data": {"stations": [station(1, 0, 3), station(2, 0, 4), station(3, 1, 9)]}},
            "depots": {"data": {"service_reach_tiles": 20, "depots": [
                depot(1, 0, "road", (1, 1), Some((2, 1))), depot(2, 0, "road", (1, 3), Some((2, 3))),
                depot(4, 0, "road", (2, 0), Some((2, 1))), depot(9, 0, "ship", (0, 4), None),
                depot(3, 1, "road", (5, 3), Some((5, 2))), depot(5, 1, "rail", (6, 0), Some((7, 0))),
                depot(6, 1, "rail", (6, 4), Some((7, 4))), depot(7, 16, "road", (9, 4), Some((8, 4)))]}},
            "ways": {"data": {"width": 10,
                "roads": ["004a200000", "02b8000000", "0000040000", "0200010000", "0050000000"],
                "rails": ["00000000000001010100", "00000000000000000000", "00000000000001000000",
                          "00000000000000000000", "00000000000001020000"]}},
            "map": {"data": {
                "types": ["CCrSSCRRRC", "CrrrCCCCCC", "CCCCCrSCCC", "CrCCCrCCCC", "WCSCCCRRCC"],
                "owners": ["NN000N111N", "N00NNNNNNN", "NNNNN11NNN", "N0NNN1NNNN", "0N1NNN11NN"]}},
        })
    }

    fn company(a: &Value, c: i64) -> Value {
        a["companies"].as_array().unwrap().iter().find(|v| v["company"] == c).cloned().unwrap()
    }

    #[test]
    fn a_depot_is_open_only_when_its_entrance_leads_into_it_and_on() {
        let w = world();
        let ways = Ways::from_ways(&w["ways"]).unwrap();
        let exit = |i: usize| depot_exit(&w["depots"]["data"]["depots"][i], &ways);
        assert_eq!(exit(0), "open", "D1: the entrance points back and on to (3,1)");
        assert_eq!(exit(1), "no_exit", "D2: no road at the entrance");
        assert_eq!(exit(2), "open", "D4: entered from the NW side, on along the road");
        assert_eq!(exit(3), "unknown", "a ship depot has no entrance tile");
        assert_eq!(exit(4), "dead_end", "D3: the entrance points only back");
        assert_eq!(exit(5), "open", "R1: X track on into another X track");
        assert_eq!(exit(6), "no_exit", "R2: a Y track at the entrance is not reached moving SW");
        let off = json!({"type": "rail", "x": 7, "y": 0, "entrance": {"x": 8, "y": 0}});
        assert_eq!(depot_exit(&off, &ways), "dead_end", "an X track whose way on leaves the map");
        let far = json!({"type": "road", "x": 1, "y": 1, "entrance": {"x": 5, "y": 1}});
        assert_eq!(depot_exit(&far, &ways), "unknown", "an entrance that is not a neighbour");
        let no_rails = Ways::from_ways(&json!({"width": 10, "roads": w["ways"]["data"]["roads"]})).unwrap();
        assert!(!no_rails.has_rails());
        assert_eq!(depot_exit(&w["depots"]["data"]["depots"][5], &no_rails), "unknown", "rails not published");
    }

    #[test]
    fn the_track_table_is_the_games_own() {
        // _exitdir_reaches_trackdirs in src/table/track_data.h, one row per DiagDirection.
        let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../src/table/track_data.h"));
        let at = src.find("_exitdir_reaches_trackdirs{{{").expect("the table");
        let bit = |name: &str| match name.split('_').next().unwrap() {
            "X" => 1, "Y" => 2, "Upper" => 4, "Lower" => 8, "Left" => 16, "Right" => 32, other => panic!("track {other}"),
        };
        let rows: Vec<u8> = src[at..].lines().skip(1).take(4).map(|l| {
            l.split("Trackdir::").skip(1).map(|t| bit(t.split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap())).sum()
        }).collect();
        assert_eq!(rows, REACHES.to_vec());
    }

    #[test]
    fn the_audit_judges_every_company_by_the_same_rules() {
        let a = audit(&world());
        assert_eq!((a["reach"].clone(), a["network"].clone(), a["rails"].clone(), a["stops_known"].clone(), a["catchment"].clone()),
                   (json!(20), json!(true), json!(true), json!(true), json!("game")));
        assert_eq!(a["companies"].as_array().unwrap().len(), 2, "owner 16 (nobody) is the game's, not a company");
        let c0 = company(&a, 0);
        assert_eq!(c0["depots"], json!({"road": 3, "rail": 0, "ship": 1, "other": 0}));
        assert_eq!(c0["exits"], json!({"open": 2, "dead_end": 0, "no_exit": 1, "unknown": 1}));
        assert_eq!(c0["stops"], json!({"drive_through": 1, "bay": 1, "drive_through_share": 0.5}));
        assert_eq!((c0["duplicate_depots"].clone(), c0["duplicate_stations"].clone()), (json!(1), json!(1)));
        let keys: Vec<&str> = c0["defects"].as_array().unwrap().iter().map(|d| d["key"].as_str().unwrap()).collect();
        assert_eq!(keys, vec!["bay_stop:4,0", "duplicate_depot:road:2-11", "duplicate_station:1-2", "no_exit:road:31"]);
        let texts: Vec<&str> = c0["defects"].as_array().unwrap().iter().map(|d| d["what"].as_str().unwrap()).collect();
        assert_eq!(texts, vec!["road stop at 4,0 is a bay, not drive-through",
                               "road depots at 1,1 and 2,0 are 2 tiles apart by road network (service reach 20)",
                               "stations S1 and S2 share bus and their catchments overlap",
                               "road depot at 1,3: its entrance 2,3 does not lead into it"]);
        let c1 = company(&a, 1);
        assert_eq!(c1["exits"], json!({"open": 1, "dead_end": 1, "no_exit": 1, "unknown": 0}));
        assert_eq!(c1["stops"], json!({"drive_through": 1, "bay": 0, "drive_through_share": 1.0}), "the rail station tile is no road stop");
        let kinds: Vec<&str> = c1["defects"].as_array().unwrap().iter().map(|d| d["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["dead_end", "duplicate_depot", "no_exit"], "R1 and R2 are 4 tiles apart in a straight line");
        assert!(c1["defects"][0]["what"].as_str().unwrap().contains("leads nowhere else"));
    }

    #[test]
    fn without_ways_or_map_the_audit_says_what_it_could_not_judge() {
        let mut w = world();
        w["ways"] = Value::Null;
        let a = audit(&w);
        assert_eq!((a["network"].clone(), a["rails"].clone(), a["stops_known"].clone()), (json!(false), json!(false), json!(false)));
        let c0 = company(&a, 0);
        assert_eq!(c0["exits"]["unknown"], json!(4));
        assert_eq!(c0["stops"]["drive_through_share"], Value::Null);
        assert_eq!(c0["duplicate_depots"], json!(3), "the straight distance stands in for the road network");
        let mut w = world();
        w["map"] = Value::Null;
        for s in w["stations"]["data"]["stations"].as_array_mut().unwrap() { s.as_object_mut().unwrap().remove("catchment_area"); }
        let a = audit(&w);
        assert_eq!((a["stops_known"].clone(), a["catchment"].clone()), (json!(false), json!("fallback radius")));
        assert_eq!(company(&a, 0)["duplicate_stations"], json!(1), "bus radius 3 + 3 overlaps one tile apart");
        let nameless = audit(&json!({"stations": {"stations": [
            {"id": 1, "owner": 2, "x": 0, "y": 0, "facilities": ["truck"]}, {"id": 2, "owner": 2, "x": 1, "y": 0, "facilities": ["truck"]}]}}));
        assert_eq!(company(&nameless, 2)["defects"][0]["what"], json!("stations #1 and #2 share truck and their catchments overlap"));
        assert_eq!(audit(&json!({}))["companies"], json!([]));
    }
}
