//! RESEARCH-ONLY: print a route plan for a running game as JSON (see src/plan.rs): town pairs
//! ranked by model v2 (the game's payment formula, speeds and base prices), each with its suggested mode of transport.
//!
//! Usage: mcp-plan [--panel http://127.0.0.1:8800 | --token <prismatic_mcp token file>] [--limit 20] [--params '{"cost_weight":1}']
//!        mcp-plan --invest ...  (also decide with AlgoGlyph's transport-investment, see src/invest.rs)
//!        mcp-plan --invest-inputs ... (print only the kernel input, to reproduce a case)
//!        mcp-plan --track N [--yards N] ...   (price the N best rail lines of each list on the real map,
//!                                               src/linecost.rs; --yards prices the N best freight yards)
//!        mcp-plan --calibrate ...  (real against predicted profit per line; the panel stores the reading)
//!        mcp-plan --defaults   (print the default parameters)
//!        mcp-plan --capture DIR ...   (also write every openttd:// read, envelope and all, with its sha256,
//!                                      game tick and date, to DIR/capture.json; I0096)
//!        mcp-plan --snapshot DIR ...  (replay: read every resource from a capture instead of a game,
//!                                      refusing a file whose sha256 differs from the capture's)
use std::time::Duration;

#[path = "../graph.rs"]
mod graph;
#[path = "../plan.rs"]
mod plan;
#[path = "../cargo.rs"]
mod cargo;
#[path = "../sites.rs"]
mod sites;
#[path = "../invest.rs"]
mod invest;
#[path = "../linecost.rs"]
mod linecost;
#[path = "../yards.rs"]
mod yards;
#[path = "../calibrate.rs"]
#[allow(dead_code)]
mod calibrate;
#[path = "../hub_algoglyph.rs"]
#[allow(dead_code)]
mod hub_algoglyph;
#[allow(dead_code)]
#[path = "../battlebrain_contract.rs"]
mod battlebrain_contract;
#[path = "../reads.rs"]
mod reads;

/// The game's resources a track plan reads, as hub_algoglyph assembles them (from/to added later).
fn linecost_input(read: &dyn Fn(&str) -> Result<Option<serde_json::Value>, String>) -> Result<serde_json::Value, String> {
    let ends = serde_json::json!({"from": {}, "to": {}});
    hub_algoglyph::track_input(&ends, read)
}

/// Where resources come from: a panel's derived routes, an MCP server read directly through the
/// token file `prismatic_mcp start` writes ({"agent_token","endpoint"}), for a stack without a panel,
/// or a capture written by --capture (I0096), replayed only where every file is still what was read.
enum Source {
    Panel(String),
    Mcp { endpoint: String, token: String },
    Recorded { dir: std::path::PathBuf, manifest: serde_json::Value },
}

/// The capture of this run (--capture): every read as it came, in order, first read of a name wins.
static CAPTURE: std::sync::Mutex<Option<(std::path::PathBuf, Vec<serde_json::Value>)>> = std::sync::Mutex::new(None);

/// The file a capture keeps resource openttd://<name> in.
fn capture_file(name: &str) -> String {
    format!("{}.json", name.replace('/', "__"))
}

/// Note one read in the capture, if one is being written; a name read twice keeps its first answer.
fn capture_note(name: &str, raw: &Result<serde_json::Value, String>) {
    let mut cap = CAPTURE.lock().unwrap();
    let Some((dir, entries)) = cap.as_mut() else { return };
    let uri = format!("openttd://{name}");
    if let Some(e) = entries.iter_mut().find(|e| e["uri"] == uri.as_str()) {
        e["reads"] = serde_json::json!(e["reads"].as_u64().unwrap_or(1) + 1);
        return;
    }
    let entry = match raw {
        Ok(v) => {
            let file = capture_file(name);
            if let Err(e) = std::fs::write(dir.join(&file), serde_json::to_string_pretty(v).unwrap_or_default() + "\n") {
                eprintln!("mcp-plan: --capture: {file}: {e}");
            }
            serde_json::json!({"uri": uri, "ok": true, "file": file, "digest": battlebrain_contract::digest(v),
                               "game_tick": v.get("game_tick"), "game_date": v.get("game_date"), "reads": 1})
        }
        Err(e) => serde_json::json!({"uri": uri, "ok": false, "error": e, "reads": 1}),
    };
    entries.push(entry);
}

/// Write DIR/capture.json: every read, and the game-time window they were taken in (coherent when
/// every read saw the same tick).
fn capture_finish(source: &str) {
    let cap = CAPTURE.lock().unwrap();
    let Some((dir, entries)) = cap.as_ref() else { return };
    let ticks: Vec<u64> = entries.iter().filter_map(|e| e["game_tick"].as_u64()).collect();
    let dates: Vec<&str> = entries.iter().filter_map(|e| e["game_date"].as_str()).collect();
    let (from, to) = (ticks.iter().min().copied(), ticks.iter().max().copied());
    let manifest = serde_json::json!({
        "schema": "mcp-plan-capture/v1", "source": source, "resources": entries,
        "window": {"tick_from": from, "tick_to": to, "date_from": dates.iter().min(), "date_to": dates.iter().max(),
                   "coherent": from.is_some() && from == to},
        "note": "RESEARCH-ONLY (I0096). Resource bodies only, no token. A name read twice replays its first answer."});
    if let Err(e) = std::fs::write(dir.join("capture.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n") {
        eprintln!("mcp-plan: --capture: capture.json: {e}");
    }
}

impl Source {
    /// The data of resource openttd://<name>, the read noted in `reads` (and in the capture).
    fn resource(&self, name: &str) -> Result<serde_json::Value, String> {
        let raw = self.fetch(name);
        capture_note(name, &raw);
        let r = raw.map(|v| v.get("data").cloned().unwrap_or(v));
        reads::note(name, r.is_ok());
        r
    }

    /// A capture as a source: its manifest must be one this planner writes.
    fn recorded(dir: &str) -> Result<Source, String> {
        let dir = std::path::PathBuf::from(dir);
        let text = std::fs::read_to_string(dir.join("capture.json")).map_err(|e| format!("{}: {e}", dir.join("capture.json").display()))?;
        let manifest: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("capture.json: {e}"))?;
        if manifest["schema"] != "mcp-plan-capture/v1" {
            return Err(format!("capture.json: schema {} is not mcp-plan-capture/v1", manifest["schema"]));
        }
        Ok(Source::Recorded { dir, manifest })
    }

    fn from_token_file(path: &str) -> Result<Source, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        let field = |k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::to_string).ok_or(format!("{path}: no {k}"));
        Ok(Source::Mcp { endpoint: field("endpoint")?, token: field("agent_token")? })
    }

    /// The whole answer for openttd://<name>, envelope included.
    fn fetch(&self, name: &str) -> Result<serde_json::Value, String> {
        let v: serde_json::Value = match self {
            Source::Recorded { dir, manifest } => {
                let uri = format!("openttd://{name}");
                let e = manifest["resources"].as_array().into_iter().flatten().find(|e| e["uri"] == uri.as_str())
                    .ok_or_else(|| format!("{name}: not in the capture"))?;
                if e["ok"] != true {
                    return Err(format!("{name}: {}", e["error"].as_str().unwrap_or("the captured read failed")));
                }
                let file = dir.join(e["file"].as_str().unwrap_or_default());
                let text = std::fs::read_to_string(&file).map_err(|err| format!("{}: {err}", file.display()))?;
                let v: serde_json::Value = serde_json::from_str(&text).map_err(|err| format!("{}: {err}", file.display()))?;
                if battlebrain_contract::digest(&v).as_str() != e["digest"].as_str().unwrap_or_default() {
                    return Err(format!("{name}: {} is not what was captured (sha256 differs)", file.display()));
                }
                v
            }
            Source::Panel(panel) => ureq::get(&format!("{panel}/api/resources/{name}"))
                .timeout(Duration::from_secs(30)).call().map_err(|e| format!("{name}: {e}"))?
                .into_json().map_err(|e| format!("{name}: {e}"))?,
            Source::Mcp { endpoint, token } => {
                let body = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "resources/read",
                                              "params": {"uri": format!("openttd://{name}")}});
                let out: serde_json::Value = ureq::post(endpoint)
                    .timeout(Duration::from_secs(30))
                    .set("Authorization", &format!("Bearer {token}"))
                    .set("Accept", "application/json, text/event-stream")
                    .send_json(body).map_err(|e| format!("{name}: {e}"))?
                    .into_json().map_err(|e| format!("{name}: {e}"))?;
                let text = out.pointer("/result/contents/0/text").and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("{name}: {}", out.get("error").cloned().unwrap_or(out.clone())))?;
                serde_json::from_str(text).map_err(|e| format!("{name}: {e}"))?
            }
        };
        Ok(v)
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut panel, mut limit) = ("http://127.0.0.1:8800".to_string(), 20usize);
    let mut params = serde_json::Value::Null;
    let mut token: Option<String> = None;
    let mut invest_too = false;
    let mut invest_inputs = false;
    let mut track = 0usize;
    let mut yards_n = 0usize;
    let mut calibrate_too = false;
    let (mut capture_dir, mut snapshot_dir): (Option<String>, Option<String>) = (None, None);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--capture" => capture_dir = args.next(),
            "--snapshot" => snapshot_dir = args.next(),
            "--panel" => panel = args.next().unwrap_or_default(),
            "--token" => token = args.next(),
            "--invest" => invest_too = true,
            "--invest-inputs" => invest_inputs = true,
            "--track" => track = args.next().and_then(|v| v.parse().ok()).unwrap_or(3),
            "--yards" => yards_n = args.next().and_then(|v| v.parse().ok()).unwrap_or(2),
            "--calibrate" => calibrate_too = true,
            "--record" => {
                eprintln!("mcp-plan: --record is gone (I0082): POST /api/calibration on a panel stores the reading as plan.calibration");
                std::process::exit(2);
            }
            "--limit" => limit = args.next().and_then(|v| v.parse().ok()).unwrap_or(20),
            "-h" | "--help" => {
                println!("usage: mcp-plan [--panel URL | --token FILE] [--limit N] [--params JSON] [--invest | --invest-inputs] [--track N [--yards N]] [--calibrate] [--capture DIR | --snapshot DIR] [--defaults]");
                return;
            }
            "--defaults" => {
                println!("{}", plan::defaults());
                return;
            }
            "--params" => match serde_json::from_str(&args.next().unwrap_or_default()) {
                Ok(v) => params = v,
                Err(e) => {
                    eprintln!("mcp-plan: --params is not JSON: {e}");
                    std::process::exit(2);
                }
            },
            _ => {
                eprintln!("usage: mcp-plan [--panel URL | --token FILE] [--limit N] [--params JSON] [--invest | --invest-inputs] [--track N [--yards N]] [--calibrate] [--capture DIR | --snapshot DIR] [--defaults]");
                std::process::exit(2);
            }
        }
    }
    let panel = panel.trim_end_matches('/').to_string();
    if capture_dir.is_some() && snapshot_dir.is_some() {
        eprintln!("mcp-plan: --capture and --snapshot exclude each other");
        std::process::exit(2);
    }
    if let Some(d) = &capture_dir {
        if let Err(e) = std::fs::create_dir_all(d) {
            eprintln!("mcp-plan: --capture {d}: {e}");
            std::process::exit(2);
        }
        *CAPTURE.lock().unwrap() = Some((std::path::PathBuf::from(d), Vec::new()));
    }
    let source_kind = if snapshot_dir.is_some() { "capture" } else if token.is_some() { "mcp" } else { "panel" };
    let out = (|| -> Result<serde_json::Value, String> {
        let src = match (&snapshot_dir, &token) {
            (Some(d), _) => Source::recorded(d)?,
            (None, Some(t)) => Source::from_token_file(t)?,
            (None, None) => Source::Panel(panel.clone()),
        };
        // A capture also keeps openttd://companies, which a decision's budget is read from; it is not
        // one of the plan's reads.
        if capture_dir.is_some() {
            capture_note("companies", &src.fetch("companies"));
        }
        let towns = src.resource("towns")?;
        let g = graph::derive_with_ways(&src.resource("map")?, &towns, &src.resource("stations")?, &src.resource("map/ways").unwrap_or(serde_json::Value::Null))?;
        // Live vehicles when the game offers openttd://engines; the plan says so when it does not.
        let engines = src.resource("engines").unwrap_or(serde_json::Value::Null);
        // Construction costs follow the game's prices now (openttd://economy/prices) where it offers
        // them; a parameter the caller set wins.
        let prices = src.resource("economy/prices").unwrap_or(serde_json::Value::Null);
        let params = plan::with_prices(&params, &prices);
        let mut out = plan::plan(&towns, &g, limit, &params, &engines)?;
        // Freight from the world graph's industries, and the portfolio over passengers and freight.
        let world = src.resource("world/graph").unwrap_or(serde_json::Value::Null);
        let freight = cargo::freight_pairs(&world, &engines, &out["params"], limit.max(10));
        out["freight"] = serde_json::Value::Array(freight);
        // Station sites for every rail candidate (freight and passengers), by the bot's own rule
        // (src/sites.rs), so an executor builds the plan without a rule of its own.
        let map = src.resource("map")?;
        let pos = |v: &serde_json::Value| (v["x"].as_i64().unwrap_or(0), v["y"].as_i64().unwrap_or(0));
        let add_sites = |c: &mut serde_json::Value| {
            let (a, b) = if c.get("producer").is_some() { (pos(&c["producer"]), pos(&c["acceptor"])) } else { (pos(&c["a"]), pos(&c["b"])) };
            c["stations"] = serde_json::json!({
                "from": sites::site(&map, a.0, a.1, b.0, b.1, 3), "to": sites::site(&map, b.0, b.1, a.0, a.1, 3),
                "rule": "axis toward the other end, buildable flat rectangle and exit tile, nearest site (as PrismaticAI rail.nut)"});
        };
        if let Some(arr) = out["freight"].as_array_mut() {
            for c in arr.iter_mut().filter(|c| c["mode"] == "freight_rail") { add_sites(c); }
        }
        if let Some(arr) = out["by_mode"]["rail"].as_array_mut() { for c in arr.iter_mut() { add_sites(c); } }
        // --calibrate: what the running lines really earn against what this plan predicts for the
        // same towns or industries, per mode (src/calibrate.rs); `calibration.params` goes back in
        // as --params for the next plan. Read before track pricing changes any score.
        if calibrate_too {
            let mut cands: Vec<serde_json::Value> = out["pairs"].as_array().cloned().unwrap_or_default();
            cands.extend(out["freight"].as_array().cloned().unwrap_or_default());
            out["calibration"] = calibrate::calibrate(&src.resource("vehicles")?, &src.resource("stations")?, &cands, calibrate::REACH_TILES);
        }
        // --track N: the N best rail lines of each list priced on the real map by AlgoGlyph's
        // track-planning (src/linecost.rs) instead of distance x tile cost, before the portfolio.
        if track > 0 {
            let read = |r: &str| -> Result<Option<serde_json::Value>, String> { Ok(src.resource(r).ok()) };
            let mut base = linecost_input(&read)?;
            if let Some(o) = base.as_object_mut() { o.remove("from"); o.remove("to"); }
            let stations = out["params"]["costs"]["rail_stations"].as_f64().unwrap_or(1480.0);
            let rail_tile = out["params"]["costs"]["rail_tile"].as_f64().unwrap_or(100.0);
            let mut log = Vec::new();
            for list in ["freight", "rail"] {
                let arr = if list == "freight" { out["freight"].as_array_mut() } else { out["by_mode"]["rail"].as_array_mut() };
                if let Some(arr) = arr { log.extend(linecost::price(arr, &base, stations, rail_tile, track)); }
            }
            out["line_costs"] = serde_json::json!({"solver": "algoglyph track-planning on map, map/ways, map/terrain, economy/prices, bridges",
                                                   "priced": log});
            // --yards N: prove up to N transfer yards offline (src/yards.rs): the better line's route
            // blocked, can the other still leave from a platform beside it?
            if yards_n > 0 {
                let mut lines: Vec<serde_json::Value> = out["freight"].as_array().cloned().unwrap_or_default();
                lines.extend(out["by_mode"]["rail"].as_array().cloned().unwrap_or_default());
                out["yards"] = serde_json::json!({"solver": "algoglyph track-planning, the better line's tiles blocked (src/yards.rs)",
                                                  "pairs": yards::prove(&lines, &base, yards_n)});
            }
        }
        let mut candidates: Vec<serde_json::Value> = Vec::new();
        for (_, ps) in out["by_mode"].as_object().cloned().unwrap_or_default() {
            for p in ps.as_array().cloned().unwrap_or_default() { candidates.push(p); }
        }
        candidates.extend(out["freight"].as_array().cloned().unwrap_or_default());
        candidates.sort_by(|a, b| b["score"].as_f64().unwrap_or(0.0).total_cmp(&a["score"].as_f64().unwrap_or(0.0)));
        out["portfolio"] = cargo::portfolio(&candidates, out["params"]["budget"].as_f64());
        if invest_inputs { return Ok(invest::input(&out, &engines, &towns).unwrap_or(serde_json::Value::Null)); }
        if invest_too { out["investment"] = invest::decide(&out, &engines, &towns); }
        out["model"] = serde_json::json!("v4: v3 for passengers, freight pairs from the world graph's industries priced with each cargo's own payment rate and transit periods, and a portfolio chosen by knapsack within the budget");
        // Which runner answered the kernel calls, only when the one-serve switch is on (I0086), so
        // the default answer stays as it was.
        if cargo::flow_serve::wanted() { out["algoglyph"] = cargo::flow_serve::summary(); }
        out["reads"] = reads::json();
        Ok(out)
    })();
    capture_finish(source_kind);
    match out {
        Ok(v) => println!("{v}"),
        Err(e) => {
            eprintln!("mcp-plan: {e}");
            std::process::exit(3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mcp-plan-capture-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// I0096: a replay reads what was captured, and only that.
    #[test]
    fn a_capture_replays_its_reads_and_refuses_a_changed_file() {
        let d = temp("replay");
        let towns = serde_json::json!({"game_tick": 85325, "game_date": "1953-02-09", "data": [{"id": 1, "name": "A"}]});
        std::fs::write(d.join(capture_file("towns")), towns.to_string()).unwrap();
        let manifest = serde_json::json!({"schema": "mcp-plan-capture/v1", "resources": [
            {"uri": "openttd://towns", "ok": true, "file": capture_file("towns"), "digest": battlebrain_contract::digest(&towns)},
            {"uri": "openttd://engines", "ok": false, "error": "engines: not offered"}]});
        std::fs::write(d.join("capture.json"), manifest.to_string()).unwrap();
        let src = Source::recorded(d.to_str().unwrap()).unwrap();
        assert_eq!(src.resource("towns").unwrap(), serde_json::json!([{"id": 1, "name": "A"}]), "the data, as a live read gives it");
        assert!(src.resource("engines").unwrap_err().contains("not offered"), "a read that failed when captured fails again");
        assert!(src.resource("map").unwrap_err().contains("not in the capture"), "nothing is invented");
        let changed = serde_json::json!({"game_tick": 85325, "game_date": "1953-02-09", "data": [{"id": 1, "name": "B"}]});
        std::fs::write(d.join(capture_file("towns")), changed.to_string()).unwrap();
        assert!(src.resource("towns").unwrap_err().contains("sha256 differs"));
        assert_eq!(capture_file("map/ways"), "map__ways.json");
        let mut wrong = manifest.clone();
        wrong["schema"] = serde_json::json!("mcp-plan-capture/v0");
        std::fs::write(d.join("capture.json"), wrong.to_string()).unwrap();
        assert!(Source::recorded(d.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
