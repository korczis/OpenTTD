//! RESEARCH-ONLY: print a route plan for a running game as JSON (see src/plan.rs): town pairs
//! ranked by model v2 (the game's payment formula, speeds and base prices), each with its suggested mode of transport.
//!
//! Usage: mcp-plan [--panel http://127.0.0.1:8800 | --token <prismatic_mcp token file>] [--limit 20] [--params '{"cost_weight":1}']
//!        mcp-plan --invest ...  (also decide with AlgoGlyph's transport-investment, see src/invest.rs)
//!        mcp-plan --invest-inputs ... (print only the kernel input, to reproduce a case)
//!        mcp-plan --track N [--yards N] ...   (price the N best rail lines of each list on the real map,
//!                                               src/linecost.rs; --yards prices the N best freight yards)
//!        mcp-plan --calibrate ...  (real against predicted profit per line; the panel stores the reading)
//!        mcp-plan --outcome --for 0 ...  (does the plan match what the game pays, per company of --for:
//!                                      owner-split calibration, the plan re-chosen with the outcome
//!                                      prior, the fleet against the best other company; proposes only,
//!                                      src/outcome.rs)
//!        mcp-plan --for 0,6 ...   (plan for these companies: a freight line from a producer where any
//!                                  other company has a rail station is left out, as the bot refuses
//!                                  it; an empty list plans for a company not started yet; `claims`
//!                                  says which producers and why, src/cargo.rs `rail_claims`)
//!        mcp-plan --defaults   (print the default parameters)
//!        mcp-plan --capture DIR ...   (also write every openttd:// read, envelope and all, with its sha256,
//!                                      game tick and date, to DIR/capture.json; I0096) -- all at one
//!                                      game tick when the server offers openttd.resources.read_many
//!        mcp-plan --snapshot DIR ...  (replay: read every resource from a capture instead of a game,
//!                                      refusing a file whose sha256 differs from the capture's)
//!        mcp-plan --catalogue FILE ... (choose the kernels from this recorded AlgoGlyph catalogue,
//!                                      {kernels, commit} as battlebrain reads one, instead of the
//!                                      runner's own `flow -- kernels`)
//!
//! Every AlgoGlyph kernel the plan calls is chosen by capability from the catalogue (src/cargo.rs
//! `Kernels`, the matcher of battlebrain_contract.rs); the answer's `kernels` holds the catalogue
//! and the capability-match/v1 decision of every capability the plan resolved.
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
#[path = "../outcome.rs"]
mod outcome;
#[path = "../store_sql.rs"]
#[allow(dead_code)]
mod store_sql;
#[path = "../hub_algoglyph.rs"]
#[allow(dead_code)]
mod hub_algoglyph;
#[allow(dead_code)]
#[path = "../battlebrain_contract.rs"]
mod battlebrain_contract;
#[path = "../reads.rs"]
mod reads;
#[path = "../plan_pipeline.rs"]
mod plan_pipeline;
// The line-pricing record and its solver label live with the pipeline that writes them.
#[cfg(test)]
use plan_pipeline::{line_costs, solver_of};

/// Where resources come from: a panel's derived routes, an MCP server read directly through the
/// token file `prismatic_mcp start` writes ({"agent_token","endpoint"}), for a stack without a panel,
/// or a capture written by --capture (I0096), replayed only where every file is still what was read.
/// Or the answer of one read_many call (`Prefetched`, --capture's coherent path): every read comes
/// from that set, and `asked` keeps each name asked for, in order, repeats included.
enum Source {
    Panel(String),
    Mcp { endpoint: String, token: String },
    Recorded { dir: std::path::PathBuf, manifest: serde_json::Value },
    Prefetched { set: Vec<(String, Result<serde_json::Value, String>)>, asked: std::cell::RefCell<Vec<String>> },
}

/// The MCP tool that builds several resources in one main-thread call, so at one game tick
/// (research/prismatic-mcp/08-tool-catalog.md). Its name is all this planner assumes: whether the
/// server offers it, that it is read-only and how many uris one call takes come from the server's
/// own tool list (`read_many_offer`).
const READ_MANY: &str = "openttd.resources.read_many";
/// The server's own bound on one call (detail::READ_MANY_MAX_URIS); a lower maxItems in the tool's
/// inputSchema wins.
const READ_MANY_CAP: usize = 32;

/// How many uris one call of `tool` (its tools/list entry: inputSchema, annotations) takes, or why
/// --capture may not use it: absent, or not declared read-only.
fn read_many_offer(tool: Option<&serde_json::Value>) -> Result<usize, String> {
    let t = tool.ok_or_else(|| format!("read_many not offered: the server's tool list has no {READ_MANY}"))?;
    if t.pointer("/annotations/readOnlyHint") != Some(&serde_json::json!(true)) {
        return Err(format!("read_many not offered: {READ_MANY} is not declared readOnlyHint true"));
    }
    Ok(t.pointer("/inputSchema/properties/uris/maxItems").and_then(serde_json::Value::as_u64)
        .map_or(READ_MANY_CAP, |m| (m as usize).min(READ_MANY_CAP)))
}

/// One read_many answer as the set a `Prefetched` source reads from: for each name asked, the
/// envelope resources/read answers for it, or that resource's own error. A refused call, or an
/// answer without an entry for a name asked, fails the whole set.
fn prefetched(names: &[String], answer: &serde_json::Value) -> Result<Vec<(String, Result<serde_json::Value, String>)>, String> {
    if answer["ok"] != true {
        let why = answer.get("error").map_or(answer.to_string(), |e| e.as_str().map_or(e.to_string(), str::to_string));
        return Err(format!("read_many failed: {why}"));
    }
    let entries = answer["resources"].as_array().ok_or("read_many failed: the answer has no resources")?;
    names.iter().map(|n| {
        let uri = format!("openttd://{n}");
        let e = entries.iter().find(|e| e["uri"] == uri.as_str()).ok_or_else(|| format!("read_many failed: no entry for {uri}"))?;
        let r = if e["ok"] == true {
            e.get("data").cloned().ok_or_else(|| format!("{n}: the read_many entry has no data"))
        } else {
            Err(format!("{n}: {}", e["error"].as_str().unwrap_or("the read failed")))
        };
        Ok((n.clone(), r))
    }).collect()
}

/// --capture at one game tick. The set read_many prefetches is derived, not a second list of the
/// plan's reads: run the plan against what one call answered (the first time, nothing), note every
/// name it asked for outside that set, add those and run again, until a run asks for nothing
/// outside its set. That run took every read from one call, so from one tick, and its answer is the
/// answer (with the source it read, for the capture). Each round adds a name and the set never
/// exceeds `cap`, so this ends; in practice there is one round per required read the plan stops at,
/// plus one where only optional reads were missing, which runs the plan to its end on partial input
/// before it is discarded -- the price of deriving the set instead of listing it. Kernel calls in
/// such a round still count in the `algoglyph` summary when the one-serve switch is on.
fn coherent(read_many: &dyn Fn(&[String]) -> Result<serde_json::Value, String>, cap: usize,
            run: &dyn Fn(&Source) -> Result<serde_json::Value, String>) -> Result<(Result<serde_json::Value, String>, Source, usize), String> {
    let mut want: Vec<String> = Vec::new();
    for round in 1.. {
        if want.len() > cap {
            return Err(format!("read_many not used: the plan reads {} resources, more than the {cap} one call takes", want.len()));
        }
        let set = if want.is_empty() { Vec::new() } else {
            let uris: Vec<String> = want.iter().map(|n| format!("openttd://{n}")).collect();
            prefetched(&want, &read_many(&uris).map_err(|e| format!("read_many failed: {e}"))?)?
        };
        let src = Source::Prefetched { set, asked: std::cell::RefCell::new(Vec::new()) };
        let out = run(&src);
        let missed = src.missed();
        if missed.is_empty() {
            return Ok((out, src, round));
        }
        want.extend(missed);
    }
    unreachable!("the rounds above end")
}

/// The capture of this run (--capture): every read as it came, in order, first read of a name wins.
static CAPTURE: std::sync::Mutex<Option<(std::path::PathBuf, Vec<serde_json::Value>)>> = std::sync::Mutex::new(None);

/// The file a capture keeps resource openttd://<name> in.
fn capture_file(name: &str) -> String {
    format!("{}.json", name.replace('/', "__"))
}

/// Start writing a capture to `dir` (--capture): from here on every read is noted.
fn capture_start(dir: &str) {
    *CAPTURE.lock().unwrap() = Some((std::path::PathBuf::from(dir), Vec::new()));
}

/// Note one read in the capture, if one is being written; a name read twice keeps its first answer.
fn capture_note(name: &str, raw: &Result<serde_json::Value, String>) {
    let mut cap = CAPTURE.lock().unwrap();
    let Some((dir, entries)) = cap.as_mut() else { return };
    capture_note_in(dir, entries, name, raw);
}

/// `capture_note` on a given capture: the answer's file in `dir`, its entry in `entries`.
fn capture_note_in(dir: &std::path::Path, entries: &mut Vec<serde_json::Value>, name: &str, raw: &Result<serde_json::Value, String>) {
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

/// Write DIR/capture.json: every read, the game-time window they were taken in (coherent when
/// every read saw the same tick), and `coherence`: how the reads were taken, or why not at one tick.
fn capture_finish(source: &str, coherence: &str) {
    let cap = CAPTURE.lock().unwrap();
    let Some((dir, entries)) = cap.as_ref() else { return };
    let manifest = capture_manifest(entries, source, coherence);
    if let Err(e) = std::fs::write(dir.join("capture.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n") {
        eprintln!("mcp-plan: --capture: capture.json: {e}");
    }
}

/// The capture.json of `entries`.
fn capture_manifest(entries: &[serde_json::Value], source: &str, coherence: &str) -> serde_json::Value {
    let ticks: Vec<u64> = entries.iter().filter_map(|e| e["game_tick"].as_u64()).collect();
    let dates: Vec<&str> = entries.iter().filter_map(|e| e["game_date"].as_str()).collect();
    let (from, to) = (ticks.iter().min().copied(), ticks.iter().max().copied());
    serde_json::json!({
        "schema": "mcp-plan-capture/v1", "source": source, "resources": entries,
        "window": {"tick_from": from, "tick_to": to, "date_from": dates.iter().min(), "date_to": dates.iter().max(),
                   "coherent": from.is_some() && from == to},
        "coherence": coherence,
        "note": "RESEARCH-ONLY (I0096). Resource bodies only, no token. A name read twice replays its first answer."})
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

    /// The names a `Prefetched` run asked for that its set does not hold, once each, in order.
    fn missed(&self) -> Vec<String> {
        let Source::Prefetched { set, asked } = self else { return Vec::new() };
        let mut out: Vec<String> = Vec::new();
        for n in asked.borrow().iter() {
            if !set.iter().any(|(s, _)| s == n) && !out.contains(n) { out.push(n.clone()); }
        }
        out
    }

    /// Every read a `Prefetched` run took, in order, repeats included, each with its answer: what
    /// the capture notes once the run is the one kept.
    fn asked_reads(&self) -> Vec<(String, Result<serde_json::Value, String>)> {
        let Source::Prefetched { set, asked } = self else { return Vec::new() };
        asked.borrow().iter().filter_map(|n| set.iter().find(|(s, _)| s == n).cloned()).collect()
    }

    /// The server's read_many tool as a tools/list entry ({inputSchema, annotations}), None when it
    /// lists none. Through a panel, from the route it derives for the tool from the server's live
    /// tools/list (POST /api/tools/<name> in /openapi.json, the tool's inputSchema as its body);
    /// directly, from the MCP server's tools/list.
    fn read_many_tool(&self) -> Result<Option<serde_json::Value>, String> {
        match self {
            Source::Panel(panel) => {
                let doc: serde_json::Value = ureq::get(&format!("{panel}/openapi.json")).timeout(Duration::from_secs(30)).call()
                    .map_err(|e| format!("{panel}/openapi.json: {e}"))?.into_json().map_err(|e| format!("{panel}/openapi.json: {e}"))?;
                Ok(doc["paths"].get(format!("/api/tools/{READ_MANY}")).and_then(|p| p.get("post")).map(|op| serde_json::json!({
                    "name": READ_MANY, "annotations": op.get("x-mcp-annotations"),
                    "inputSchema": op.pointer("/requestBody/content/application~1json/schema")})))
            }
            Source::Mcp { endpoint, token } => {
                let out = mcp_rpc(endpoint, token, "tools/list", serde_json::json!({}))?;
                Ok(out.pointer("/result/tools").and_then(serde_json::Value::as_array).into_iter().flatten()
                    .find(|t| t["name"] == READ_MANY).cloned())
            }
            Source::Recorded { .. } | Source::Prefetched { .. } => Ok(None),
        }
    }

    /// One read_many call for `uris`: its answer {ok, game_tick, game_date, resources}. A read: the
    /// tool is read-only and needs no approval.
    fn read_many(&self, uris: &[String]) -> Result<serde_json::Value, String> {
        let args = serde_json::json!({"uris": uris});
        match self {
            Source::Panel(panel) => ureq::post(&format!("{panel}/api/tools/{READ_MANY}")).timeout(Duration::from_secs(120))
                .send_json(args).map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string()),
            Source::Mcp { endpoint, token } => {
                let out = mcp_rpc(endpoint, token, "tools/call", serde_json::json!({"name": READ_MANY, "arguments": args}))?;
                let text = out.pointer("/result/content/0/text").and_then(serde_json::Value::as_str)
                    .ok_or_else(|| out.get("error").cloned().unwrap_or(out.clone()).to_string())?;
                serde_json::from_str(text).map_err(|e| e.to_string())
            }
            Source::Recorded { .. } | Source::Prefetched { .. } => Err("no server to call".into()),
        }
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
            Source::Prefetched { set, asked } => {
                asked.borrow_mut().push(name.to_string());
                return set.iter().find(|(n, _)| n == name).map(|(_, r)| r.clone())
                    .unwrap_or_else(|| Err(format!("{name}: not in the read_many set")));
            }
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
                let out = mcp_rpc(endpoint, token, "resources/read", serde_json::json!({"uri": format!("openttd://{name}")}))
                    .map_err(|e| format!("{name}: {e}"))?;
                let text = out.pointer("/result/contents/0/text").and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("{name}: {}", out.get("error").cloned().unwrap_or(out.clone())))?;
                serde_json::from_str(text).map_err(|e| format!("{name}: {e}"))?
            }
        };
        Ok(v)
    }
}

/// One JSON-RPC call on an MCP server with the agent token.
fn mcp_rpc(endpoint: &str, token: &str, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    ureq::post(endpoint)
        .timeout(Duration::from_secs(if method == "tools/call" { 120 } else { 30 }))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json, text/event-stream")
        .send_json(body).map_err(|e| e.to_string())?
        .into_json().map_err(|e| e.to_string())
}

/// `--for`'s company ids: whole numbers separated by commas, none for a company not started yet.
fn companies(text: &str) -> Result<Vec<i64>, String> {
    text.split(',').map(str::trim).filter(|p| !p.is_empty())
        .map(|p| p.parse::<i64>().map_err(|_| format!("{p} is no company id (ids separated by commas)"))).collect()
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
    let mut outcome_too = false;
    let (mut capture_dir, mut snapshot_dir): (Option<String>, Option<String>) = (None, None);
    let mut catalogue: Option<String> = None;
    let mut plan_for: Option<Vec<i64>> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--capture" => capture_dir = args.next(),
            "--catalogue" => catalogue = args.next(),
            "--snapshot" => snapshot_dir = args.next(),
            "--panel" => panel = args.next().unwrap_or_default(),
            "--token" => token = args.next(),
            "--invest" => invest_too = true,
            "--invest-inputs" => invest_inputs = true,
            "--track" => track = args.next().and_then(|v| v.parse().ok()).unwrap_or(3),
            "--yards" => yards_n = args.next().and_then(|v| v.parse().ok()).unwrap_or(2),
            "--calibrate" => calibrate_too = true,
            "--outcome" => outcome_too = true,
            "--for" => match companies(&args.next().unwrap_or_default()) {
                Ok(c) => plan_for = Some(c),
                Err(e) => {
                    eprintln!("mcp-plan: --for: {e}");
                    std::process::exit(2);
                }
            },
            "--record" => {
                eprintln!("mcp-plan: --record is gone (I0082): POST /api/calibration on a panel stores the reading as plan.calibration");
                std::process::exit(2);
            }
            "--limit" => limit = args.next().and_then(|v| v.parse().ok()).unwrap_or(20),
            "-h" | "--help" => {
                println!("usage: mcp-plan [--panel URL | --token FILE] [--limit N] [--params JSON] [--invest | --invest-inputs] [--track N [--yards N]] [--calibrate] [--outcome] [--for IDS] [--capture DIR | --snapshot DIR] [--catalogue FILE] [--defaults]");
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
                eprintln!("usage: mcp-plan [--panel URL | --token FILE] [--limit N] [--params JSON] [--invest | --invest-inputs] [--track N [--yards N]] [--calibrate] [--outcome] [--for IDS] [--capture DIR | --snapshot DIR] [--catalogue FILE] [--defaults]");
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
    }
    let source_kind = if snapshot_dir.is_some() { "capture" } else if token.is_some() { "mcp" } else { "panel" };
    // The kernels of every run of the planner below: one catalogue, each capability matched once.
    let kernels = catalogue.as_deref().map_or_else(cargo::Kernels::runner, cargo::Kernels::from_file);
    // One run of the planner on `src`; --capture's coherent path runs it more than once (`coherent`).
    let run = |src: &Source| -> Result<serde_json::Value, String> {
        reads::clear();
        // A capture also keeps openttd://companies, which a decision's budget is read from; it is not
        // one of the plan's reads.
        if capture_dir.is_some() {
            capture_note("companies", &src.fetch("companies"));
        }
        let read = |name: &str| src.resource(name);
        let opts = plan_pipeline::Options { limit, params: params.clone(), track, yards: yards_n,
                                            calibrate: calibrate_too, outcome: outcome_too, invest: invest_too, invest_inputs, plan_for: plan_for.clone() };
        let mut out = plan_pipeline::run(&read, &opts, &kernels)?;
        if invest_inputs { return Ok(out); }
        // Which runner answered the kernel calls, only when the one-serve switch is on (I0086), so
        // the default answer stays as it was.
        if cargo::flow_serve::wanted() { out["algoglyph"] = cargo::flow_serve::summary(); }
        out["reads"] = reads::json();
        out["kernels"] = kernels.record();
        Ok(out)
    };
    // How a capture's reads were taken: at one tick through read_many, or why not.
    let mut coherence = String::new();
    let out = (|| -> Result<serde_json::Value, String> {
        let src = match (&snapshot_dir, &token) {
            (Some(d), _) => Source::recorded(d)?,
            (None, Some(t)) => Source::from_token_file(t)?,
            (None, None) => Source::Panel(panel.clone()),
        };
        let Some(dir) = &capture_dir else { return run(&src) };
        let tried = src.read_many_tool().map_err(|e| format!("read_many not offered: {e}"))
            .and_then(|tool| read_many_offer(tool.as_ref()))
            .and_then(|cap| coherent(&|uris| src.read_many(uris), cap, &run));
        capture_start(dir);
        match tried {
            Ok((out, kept, rounds)) => {
                let reads = kept.asked_reads();
                for (name, raw) in &reads { capture_note(name, raw); }
                coherence = format!("read_many: every read from one {READ_MANY} call, the set derived in {rounds} runs of the plan");
                out
            }
            Err(why) => {
                // Today's path: each resource read on its own while the game runs.
                coherence = why;
                run(&src)
            }
        }
    })();
    capture_finish(source_kind, &coherence);
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

    #[test]
    fn the_line_costs_name_the_kernel_the_catalogue_matched_or_that_none_did() {
        let k = cargo::testing::kernels(|_, _| Err(cargo::KernelFailure::Unset));
        let _ = linecost::route(&k, &serde_json::json!({}), &serde_json::json!({}), &serde_json::json!({}), &[]);
        let lc = line_costs(&k, vec![serde_json::json!({"candidate": 0})]);
        assert_eq!(lc["capability"], "rail-line-price");
        assert_eq!(lc["kernel"], "track-planning");
        assert_eq!(lc["solver"], "algoglyph track-planning on map, map/ways, map/terrain, economy/prices, bridges");
        assert_eq!(lc["priced"].as_array().unwrap().len(), 1);
        let expired = cargo::testing::catalogue(|l| l.retain(|k| k["id"] != "track-planning"));
        let k = cargo::testing::on(expired, |_, _| panic!("no kernel"));
        let _ = linecost::route(&k, &serde_json::json!({}), &serde_json::json!({}), &serde_json::json!({}), &[]);
        let lc = line_costs(&k, vec![]);
        assert_eq!(lc["kernel"], serde_json::Value::Null);
        assert_eq!(solver_of(&k, linecost::RAIL_LINE_PRICE), "no kernel (rail-line-price unmatched, see kernels)");
        assert_eq!(k.record()["matches"][0]["refusal"]["code"], "kernel_expired");
    }

    /// --for: company ids separated by commas; an empty list is a company not started yet.
    #[test]
    fn for_names_the_companies_a_plan_is_for() {
        assert_eq!(companies("0, 6,"), Ok(vec![0, 6]));
        assert_eq!(companies(""), Ok(vec![]));
        assert!(companies("0,x").unwrap_err().contains("x is no company id"));
    }

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

    /// A fake server's read_many: every resource it has at one tick, `engines` unknown to it, the
    /// uris of every call kept.
    fn fake_read_many(calls: &std::cell::RefCell<Vec<Vec<String>>>, uris: &[String]) -> Result<serde_json::Value, String> {
        calls.borrow_mut().push(uris.to_vec());
        let resources: Vec<serde_json::Value> = uris.iter().map(|u| match u.as_str() {
            "openttd://engines" => serde_json::json!({"uri": u, "ok": false, "error": "unknown resource uri"}),
            _ => serde_json::json!({"uri": u, "ok": true, "data": {"game_tick": 500, "game_date": "1953-02-09", "data": {"of": u}}}),
        }).collect();
        Ok(serde_json::json!({"ok": true, "game_tick": 500, "game_date": "1953-02-09", "resources": resources}))
    }

    /// A plan's shape: a direct fetch, required reads it stops at, optional ones it goes on without,
    /// and one name read twice.
    fn fake_plan(src: &Source) -> Result<serde_json::Value, String> {
        let _ = src.fetch("companies");
        let towns = src.resource("towns")?;
        let _map = src.resource("map")?;
        src.resource("stations")?;
        let engines = src.resource("engines").unwrap_or(serde_json::Value::Null);
        src.resource("map")?;
        let bridges = src.resource("bridges").ok();
        Ok(serde_json::json!({"towns": towns, "engines": engines, "bridges": bridges}))
    }

    /// The coherent --capture: the set read_many prefetches is derived from the plan's own reads,
    /// the run kept took all of them from one call, and its capture is one tick that replays.
    #[test]
    fn a_coherent_capture_derives_its_set_reads_one_tick_and_replays() {
        let calls = std::cell::RefCell::new(Vec::new());
        let (out, kept, rounds) = coherent(&|u| fake_read_many(&calls, u), READ_MANY_CAP, &fake_plan).unwrap();
        let out = out.unwrap();
        assert_eq!(out["towns"], serde_json::json!({"of": "openttd://towns"}), "the data, as a live read gives it");
        assert!(out["engines"].is_null() && out["bridges"]["of"] == "openttd://bridges");
        // Round 1 reads nothing; it stops at towns, the next at map, then stations; the fourth runs
        // to the end without engines and bridges; the fifth has everything.
        assert_eq!(rounds, 5);
        let calls = calls.into_inner();
        assert_eq!(calls.len(), 4, "no call for an empty set");
        assert_eq!(calls.last().unwrap(), &["companies", "towns", "map", "stations", "engines", "bridges"]
            .map(|n| format!("openttd://{n}")).to_vec(), "every name the plan asks for, once, in the order first asked");
        assert!(kept.missed().is_empty());
        // The capture of the kept run: one tick, coherent, and --snapshot replays it byte for byte.
        let d = temp("coherent");
        let mut entries = Vec::new();
        for (name, raw) in kept.asked_reads() { capture_note_in(&d, &mut entries, &name, &raw); }
        let m = capture_manifest(&entries, "panel", "read_many: test");
        assert_eq!(m["window"]["coherent"], true);
        assert_eq!((m["window"]["tick_from"].as_u64(), m["window"]["tick_to"].as_u64()), (Some(500), Some(500)));
        assert_eq!(m["coherence"], "read_many: test");
        let map = m["resources"].as_array().unwrap().iter().find(|e| e["uri"] == "openttd://map").unwrap();
        assert_eq!(map["reads"], 2, "a name read twice is noted once, with its count");
        std::fs::write(d.join("capture.json"), m.to_string()).unwrap();
        let replay = Source::recorded(d.to_str().unwrap()).unwrap();
        assert_eq!(fake_plan(&replay).unwrap(), out, "the replay answers what the coherent run answered");
        assert!(replay.resource("engines").unwrap_err().contains("unknown resource uri"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Why --capture falls back to one read at a time: the tool is absent or not read-only, the plan
    /// reads more than one call takes, or the call is refused; the cap is the server's, at most 32.
    #[test]
    fn a_capture_without_a_usable_read_many_says_why() {
        assert!(read_many_offer(None).unwrap_err().starts_with("read_many not offered:"));
        let tool = |ro: bool, max: Option<u64>| serde_json::json!({"name": READ_MANY, "annotations": {"readOnlyHint": ro},
            "inputSchema": {"properties": {"uris": {"maxItems": max}}}});
        assert!(read_many_offer(Some(&tool(false, Some(32)))).unwrap_err().contains("readOnlyHint"));
        assert_eq!(read_many_offer(Some(&tool(true, Some(8)))), Ok(8));
        assert_eq!(read_many_offer(Some(&tool(true, Some(100)))), Ok(READ_MANY_CAP));
        assert_eq!(read_many_offer(Some(&tool(true, None))), Ok(READ_MANY_CAP));
        assert!(Source::Panel(String::new()).missed().is_empty());
        let calls = std::cell::RefCell::new(Vec::new());
        let over = coherent(&|u| fake_read_many(&calls, u), 3, &fake_plan).err().unwrap();
        assert!(over.starts_with("read_many not used:") && over.contains("more than the 3"), "{over}");
        assert!(calls.borrow().iter().all(|c| c.len() <= 3), "no call over the cap");
        let refused = coherent(&|_| Ok(serde_json::json!({"ok": false, "error": "at most 2 uris per call"})), 32, &fake_plan);
        assert!(refused.err().unwrap().contains("at most 2 uris"));
        let short = prefetched(&["towns".into()], &serde_json::json!({"ok": true, "resources": []}));
        assert!(short.unwrap_err().contains("no entry for openttd://towns"));
        let m = capture_manifest(&[], "mcp", "read_many not offered: x");
        assert_eq!((m["window"]["coherent"].clone(), m["coherence"].clone()), (serde_json::json!(false), serde_json::json!("read_many not offered: x")));
    }
}
