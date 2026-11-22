//! RESEARCH-ONLY / UNSTABLE: the panel's background strategist and its blackboard
//! (research/mechanisms/strategist-and-blackboard.md).
//!
//! The blackboard is the shared working state of one panel instance: keys with a JSON value, who
//! wrote it, a revision, the wall time and the game date of the write, and the last `HISTORY`
//! values of each key. Panel routes, Rhai scripts (`api_blackboard`, `api_blackboardWrite`,
//! tools/research/mcp-panel/rhai/strategy_lib.rhai) and pages read it; scripts write any key except
//! the reserved namespaces `strategy.*` (the strategist's) and `mesh.*` (what it read from the
//! Majordomus mesh). It lives in the panel's memory: a restarted panel starts empty.
//!
//! The strategist is one thread started with the panel (opt out: `PRISMATIC_STRATEGIST=0`). While
//! a game runs it recomputes the long-term plan once per game month it observes (at most every
//! `PRISMATIC_STRATEGY_MIN_GAP` wall seconds) and at least every `PRISMATIC_STRATEGY_EVERY` wall
//! seconds. A cycle runs its parts on a scoped pool of three workers: the planner (src/plan_pipeline.rs
//! in this process: passenger and freight candidates, the portfolio through AlgoGlyph knapsack-dp or
//! the built-in DP, then AlgoGlyph transport-investment), the rail lines priced on the map (AlgoGlyph
//! track-planning, `PRISMATIC_STRATEGY_TRACK` lines), and the town advice for every rated company
//! (src/panel_town.rs). AlgoGlyph is found by hub_algoglyph::dir(); its kernels go to the process's
//! one long-lived `flow serve` (src/flow_serve.rs), which only the strategist uses in this process.
//! It also reads BattleBrain's tuning proposals (`brain.json`, read-only).
//!
//! It reads and computes; it never changes the game: no tool call, no setting, no bot start. Each
//! cycle writes `strategy.*` keys and publishes one `plan.strategy` event (ADR 0034). An error is
//! recorded on `strategy.status` and never ends the thread. Optionally it writes its result to the
//! Majordomus mesh journal (typed records, src/mesh_records.rs), announces it on the board
//! (src/board.rs) when it cannot, and reads the peers' plans back into `mesh.peers`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::cargo::flow_serve;
use crate::panel::{Game, Input};
use crate::panel_events;
use crate::rhai_host::{board, mesh_records};

/// The strategist's rules; part of every plan.strategy event's version.
pub const VERSION: &str = "strategist/v1";
/// Earlier values kept per key.
pub const HISTORY: usize = 8;
/// Namespaces only the strategist writes.
pub const RESERVED: &[&str] = &["strategy.", "mesh."];
/// The most keys scripts may add.
pub const MAX_KEYS: usize = 1024;
/// The largest value a script writes, as JSON text (the shared memory's limit, rhai_shared.rs).
pub const MAX_VALUE: usize = crate::rhai_shared::MAX_VALUE;
/// The workers of one cycle: planner, track, towns.
pub const WORKERS: usize = 3;

// ---------------------------------------------------------------------- the blackboard

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub value: Value,
    pub by: String,
    pub revision: u64,
    /// Unix seconds of the write.
    pub written_at: f64,
    /// The game date of the write, or null before the first sample.
    pub game_date: Value,
}

struct Slot {
    current: Entry,
    /// Earlier values, newest first, at most HISTORY.
    history: VecDeque<Entry>,
}

#[derive(Default)]
pub struct Blackboard {
    slots: RwLock<BTreeMap<String, Slot>>,
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// The reserved namespace `key` is in, if any.
pub fn reserved(key: &str) -> Option<&'static str> {
    RESERVED.iter().copied().find(|p| key.starts_with(p))
}

impl Blackboard {
    /// Write any key (the strategist's own writes): the new revision.
    pub fn put(&self, key: &str, value: Value, by: &str, game_date: Value) -> Result<u64, String> {
        if !crate::rhai_shared::valid_key(key) { return Err(format!("key {key:?}: 1-128 characters of letters, digits and _ . : / -")); }
        let mut m = self.slots.write().map_err(|_| "the blackboard's lock is poisoned".to_string())?;
        let e = Entry { key: key.into(), value, by: by.into(), revision: 1, written_at: unix_now(), game_date };
        Ok(match m.get_mut(key) {
            Some(s) => {
                let mut e = e;
                e.revision = s.current.revision + 1;
                let old = std::mem::replace(&mut s.current, e);
                s.history.push_front(old);
                s.history.truncate(HISTORY);
                s.current.revision
            }
            None => { m.insert(key.into(), Slot { current: e, history: VecDeque::new() }); 1 }
        })
    }

    /// A script's write: refused (403) in a reserved namespace, (400) when the key, the value or the
    /// writer is not acceptable or the board is full.
    pub fn write_external(&self, key: &str, value: Value, by: &str, game_date: Value) -> Result<u64, (u16, String)> {
        if let Some(ns) = reserved(key) {
            return Err((403, format!("{ns}* is reserved: the strategist writes {}", RESERVED.join("*, ") + "*")));
        }
        let size = value.to_string().len();
        if size > MAX_VALUE { return Err((400, format!("value: {size} bytes as JSON, at most {MAX_VALUE}"))); }
        let by = if by.trim().is_empty() { "unnamed" } else { by.trim() };
        if by.len() > 64 || by.chars().any(char::is_control) { return Err((400, "by: at most 64 printable characters".into())); }
        {
            let m = self.slots.read().map_err(|_| (500, "the blackboard's lock is poisoned".to_string()))?;
            let external = m.keys().filter(|k| reserved(k).is_none()).count();
            if !m.contains_key(key) && external >= MAX_KEYS { return Err((400, format!("at most {MAX_KEYS} keys"))); }
        }
        self.put(key, value, by, game_date).map_err(|e| (400, e))
    }

    /// One key with its history, newest first.
    pub fn get(&self, key: &str) -> Option<(Entry, Vec<Entry>)> {
        let m = self.slots.read().ok()?;
        m.get(key).map(|s| (s.current.clone(), s.history.iter().cloned().collect()))
    }

    /// Every key under `prefix`, in key order.
    pub fn list(&self, prefix: &str) -> Vec<(Entry, Vec<Entry>)> {
        let Ok(m) = self.slots.read() else { return vec![] };
        m.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix))
            .map(|(_, s)| (s.current.clone(), s.history.iter().cloned().collect())).collect()
    }

    /// The current value of `key`, or null.
    pub fn value(&self, key: &str) -> Value {
        self.get(key).map(|(e, _)| e.value).unwrap_or(Value::Null)
    }
}

pub fn entry_json(e: &Entry) -> Value {
    json!({"key": e.key, "value": e.value, "by": e.by, "revision": e.revision, "written_at": e.written_at, "game_date": e.game_date})
}

fn slot_json(e: &Entry, history: &[Entry], with_history: bool) -> Value {
    let mut v = entry_json(e);
    v["history_len"] = json!(history.len());
    if with_history { v["history"] = Value::Array(history.iter().map(entry_json).collect()); }
    v
}

/// The one blackboard of this panel process.
pub fn board() -> &'static Blackboard {
    static B: OnceLock<Blackboard> = OnceLock::new();
    B.get_or_init(Blackboard::default)
}

// ---------------------------------------------------------------------- configuration

/// What the environment sets; `config_from` reads it, so a test passes its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub enabled: bool,
    /// At least this often while a game runs.
    pub every: Duration,
    /// At most this often on a new game month.
    pub min_gap: Duration,
    pub limit: usize,
    pub track: usize,
    /// Use the one long-lived flow serve (unless MCP_ALGOGLYPH_SERVE is explicitly off).
    pub serve: bool,
    /// Cooperate with the Majordomus mesh at all.
    pub mesh: bool,
    pub majordomus: String,
    pub session: Option<String>,
    pub mesh_task: String,
    pub mesh_scope: Vec<String>,
    pub brain: PathBuf,
}

fn off(v: &str) -> bool {
    matches!(v.trim(), "0" | "off" | "false" | "no")
}

pub fn config_from(get: &dyn Fn(&str) -> Option<String>) -> Config {
    let secs = |name: &str, default: u64, min: u64| Duration::from_secs(get(name).and_then(|v| v.trim().parse().ok()).unwrap_or(default).max(min));
    let n = |name: &str, default: usize, max: usize| get(name).and_then(|v| v.trim().parse().ok()).unwrap_or(default).min(max);
    let home = get("HOME").unwrap_or_default();
    Config {
        enabled: !get("PRISMATIC_STRATEGIST").is_some_and(|v| off(&v)),
        every: secs("PRISMATIC_STRATEGY_EVERY", 300, 10),
        min_gap: secs("PRISMATIC_STRATEGY_MIN_GAP", 30, 5),
        limit: n("PRISMATIC_STRATEGY_LIMIT", 20, 200).max(1),
        track: n("PRISMATIC_STRATEGY_TRACK", 3, 10),
        serve: !get("MCP_ALGOGLYPH_SERVE").is_some_and(|v| off(&v)),
        mesh: !get("PRISMATIC_STRATEGY_MESH").is_some_and(|v| off(&v)),
        majordomus: get("MAJORDOMUS_URL").filter(|v| !v.is_empty())
            .or_else(|| get("PRISMATIC_MAJORDOMUS_URL").filter(|v| !v.is_empty()))
            .unwrap_or_else(|| "http://127.0.0.1:8741".into()),
        session: get("MAJORDOMUS_SESSION").filter(|v| !v.is_empty()),
        mesh_task: get("PRISMATIC_STRATEGY_MESH_TASK").filter(|v| !v.trim().is_empty()).unwrap_or_else(|| "openttd-strategy".into()),
        mesh_scope: get("PRISMATIC_STRATEGY_MESH_SCOPE").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| vec!["research/mechanisms/strategist-and-blackboard.md".into()]),
        brain: get("PRISMATIC_STRATEGY_BRAIN").filter(|v| !v.is_empty()).map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join("ttd-battle/results/brain.json")),
    }
}

fn config_json(c: &Config) -> Value {
    json!({"enabled": c.enabled, "every_s": c.every.as_secs(), "min_gap_s": c.min_gap.as_secs(), "limit": c.limit, "track": c.track,
           "serve": c.serve, "mesh": c.mesh, "majordomus": c.majordomus, "session": c.session, "mesh_task": c.mesh_task,
           "mesh_scope": c.mesh_scope, "brain": c.brain.display().to_string(), "workers": WORKERS})
}

/// Why a cycle is due now, if it is: the first one, a new game month (no sooner than `min_gap`
/// after the last), or `every` since the last.
pub fn due(since_last: Option<Duration>, month: Option<&str>, last_month: Option<&str>, every: Duration, min_gap: Duration) -> Option<&'static str> {
    let Some(since) = since_last else { return Some("first") };
    if month.is_some() && month != last_month && since >= min_gap { return Some("month"); }
    if since >= every { return Some("interval"); }
    None
}

/// The month of a game date ("1950-03-14" -> "1950-03").
pub fn month_of(date: &Value) -> Option<String> {
    date.as_str().filter(|d| d.len() >= 7).map(|d| d[..7].to_string())
}

// ---------------------------------------------------------------------- the strategist's state

#[derive(Default)]
struct Mesh {
    /// Our typed records: (key, revision) of the plan and the observation.
    plan: Option<(String, u64)>,
    observation: Option<(String, u64)>,
    plan_title: Option<String>,
    observation_title: Option<String>,
    board: Option<board::Board>,
    announced: Option<String>,
}

#[derive(Default)]
struct Run {
    cycles: u64,
    running: bool,
    last_at: Option<Instant>,
    last_month: Option<String>,
    last: Option<Value>,
    thread: bool,
    mesh: Mesh,
}

pub struct Strategist {
    pub cfg: Config,
    run: Mutex<Run>,
    cycle: Mutex<()>,
}

pub fn strategist() -> &'static Strategist {
    static S: OnceLock<Strategist> = OnceLock::new();
    S.get_or_init(|| Strategist { cfg: config_from(&|k| std::env::var(k).ok()), run: Mutex::new(Run::default()), cycle: Mutex::new(()) })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The game date: the newest daily sample's, else openttd://game/state's (before the first sample),
/// else null.
fn game_date(g: &Game) -> Value {
    if let Some(d) = g.latest_sample().map(|s| s["date"].clone()).filter(|d| !d.is_null()) { return d; }
    if !g.mcp_up.load(Ordering::SeqCst) { return Value::Null; }
    g.resource("openttd://game/state").ok().and_then(|v| v.get("game_date").cloned()).unwrap_or(Value::Null)
}

const BY: &str = "strategist";

fn put(key: &str, value: Value, date: &Value) {
    let _ = board().put(key, value, BY, date.clone());
}

/// Record the status unless it is already in that state with the same reason (no revision a second).
fn status(state: &str, reason: &str, extra: Value, date: &Value) {
    let cur = board().value("strategy.status");
    if cur["state"] == state && cur["reason"] == reason && extra.is_null() { return; }
    let mut v = json!({"state": state, "reason": reason, "at": unix_now()});
    if let (Some(o), Some(x)) = (v.as_object_mut(), extra.as_object()) { o.extend(x.clone()); }
    put("strategy.status", v, date);
}

static STOPPING: AtomicBool = AtomicBool::new(false);

/// Start the strategist thread for `g`, unless PRISMATIC_STRATEGIST is off (said on strategy.status).
pub fn start(g: Arc<Game>) {
    let s = strategist();
    if !s.cfg.enabled {
        status("off", "PRISMATIC_STRATEGIST is off: no background cycles (POST /api/strategy/run runs one)", Value::Null, &Value::Null);
        return;
    }
    status("waiting", "started; waiting for a game", Value::Null, &Value::Null);
    lock(&s.run).thread = true;
    std::thread::spawn(move || {
        while !STOPPING.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_secs(1));
            if !(g.mcp_up.load(Ordering::SeqCst) && g.server_running()) {
                status("waiting", "no game: the MCP server is not up", Value::Null, &Value::Null);
                continue;
            }
            let date = game_date(&g);
            let why = {
                let r = lock(&s.run);
                due(r.last_at.map(|t| t.elapsed()), month_of(&date).as_deref(), r.last_month.as_deref(), s.cfg.every, s.cfg.min_gap)
            };
            if let Some(why) = why { cycle(&g, why); }
        }
    });
}

/// Stop at the panel's stop: no further cycle, the flow serve ends, the board session closes (the
/// board keeps its named claim).
pub fn stop() {
    STOPPING.store(true, Ordering::SeqCst);
    flow_serve::shutdown();
    if let Some(b) = lock(&strategist().run).mesh.board.as_mut() { b.close(); }
}

// ---------------------------------------------------------------------- one cycle

/// The game's resources, read once per cycle and shared by the workers. Reads are serialised: the
/// game answers them one at a time anyway, and the map is read once, not once per worker.
struct Reads<'a> {
    g: &'a Game,
    cache: Mutex<HashMap<String, Result<Value, String>>>,
    order: Mutex<Vec<String>>,
}

impl Reads<'_> {
    fn read(&self, name: &str) -> Result<Value, String> {
        let mut c = lock(&self.cache);
        if let Some(r) = c.get(name) { return r.clone(); }
        let r = self.g.resource(&format!("openttd://{name}")).and_then(|v| {
            if v.get("error").is_some() && v.get("data").is_none() || v.get("http_error").is_some() {
                Err(format!("openttd://{name}: {}", board::clip(&v.to_string(), 200)))
            } else {
                Ok(v.get("data").cloned().unwrap_or(v))
            }
        });
        lock(&self.order).push(name.to_string());
        c.insert(name.to_string(), r.clone());
        r
    }

    fn json(&self) -> Value {
        let c = lock(&self.cache);
        Value::Array(lock(&self.order).iter().map(|n| json!({"uri": format!("openttd://{n}"), "ok": c.get(n).is_some_and(|r| r.is_ok())})).collect())
    }
}

/// Where AlgoGlyph is and how its kernels are called this cycle.
fn algoglyph(cfg: &Config) -> Value {
    let dir = crate::hub_algoglyph::dir();
    let from = if std::env::var_os("MCP_ALGOGLYPH_DIR").is_some() { "MCP_ALGOGLYPH_DIR" } else { "a default checkout (hub_algoglyph::dir)" };
    let available = dir.join("package.json").is_file();
    let mut retried = Value::Null;
    if available {
        crate::cargo::use_checkout(&dir);
        if cfg.serve {
            flow_serve::prefer();
            retried = flow_serve::retry().map(Value::String).unwrap_or(Value::Null);
        }
    }
    json!({"available": available, "dir": dir.display().to_string(), "from": from,
           "runner": if !available { "none: the built-in DP decides, and investment and track are skipped" }
                     else if cfg.serve { "flow serve, one process for every kernel call of the panel" }
                     else { "npm run flow -- kernel, one process per call (MCP_ALGOGLYPH_SERVE is off)" },
           "serve_restarted_after": retried})
}

/// The time and outcome of one part.
fn timed<T>(f: impl FnOnce() -> Result<T, String>) -> (Result<T, String>, u128) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed().as_millis())
}

fn part(r: &Result<Value, String>, ms: u128) -> Value {
    match r {
        Ok(v) if v.get("skipped").is_some() => json!({"ok": true, "ms": ms, "skipped": v["skipped"]}),
        Ok(_) => json!({"ok": true, "ms": ms}),
        Err(e) => json!({"ok": false, "ms": ms, "error": e}),
    }
}

/// A candidate as the board shows it: its ends, mode, score, investment and profit.
pub fn line_of(c: &Value) -> Value {
    let end = |v: &Value| v.get("name").cloned().unwrap_or(Value::Null);
    let best = c["options"].as_array().and_then(|o| o.iter().find(|o| o["mode"] == c["mode"])).cloned().unwrap_or(json!({}));
    let (from, to) = if c.get("producer").is_some() { (end(&c["producer"]), end(&c["acceptor"])) }
                     else if c.get("a").is_some() { (end(&c["a"]), end(&c["b"])) }
                     else { (c["from"].clone(), c.get("to").cloned().unwrap_or(Value::Null)) };
    json!({"kind": c.get("kind"), "cargo": c.get("cargo"), "from": from, "to": to, "mode": c.get("mode"), "score": c.get("score"),
           "distance": c.get("distance"), "investment": best.get("investment"), "monthly_profit": best.get("monthly_profit"),
           "roi": c.get("roi").or(best.get("roi")), "track": best.pointer("/track/status")})
}

/// The portfolio of a planner answer as the board keeps it.
pub fn portfolio_of(out: &Value) -> Value {
    let p = &out["portfolio"];
    let chosen: Vec<Value> = p["chosen"].as_array().into_iter().flatten().map(line_of).collect();
    json!({"solver": p.get("solver"), "kernel": p.pointer("/kernel/id"), "total_score": p.get("total_score"),
           "investment_k": p.get("investment_k"), "budget_k": p.get("budget_k"), "chosen": chosen,
           "shadowed": p["shadowed"].as_array().map_or(0, Vec::len)})
}

/// The planner's answer as `strategy.plan`.
pub fn plan_of(out: &Value) -> Value {
    let modes: serde_json::Map<String, Value> = out["by_mode"].as_object().into_iter().flatten()
        .map(|(k, v)| (k.clone(), json!(v.as_array().map_or(0, Vec::len)))).collect();
    json!({"portfolio": portfolio_of(out), "by_mode": modes, "freight": out["freight"].as_array().map_or(0, Vec::len),
           "pairs": out["pairs"].as_array().map_or(0, Vec::len), "model": out.get("model"),
           "budget": out.pointer("/params/budget"), "params_digest": panel_events::digest(&out["params"])})
}

/// Town warnings of one company's live answer (panel_town::live_from): every town in a poor band
/// or below the new-station gate, with its first two pieces of advice.
pub fn warnings_of(live: &Value) -> Vec<Value> {
    live["towns"].as_array().into_iter().flatten().filter_map(|t| {
        let band = t["band"].as_str().unwrap_or("");
        let floor = t["rating_floor"].as_i64();
        let poor = band.contains("POOR") || band.contains("APPALLING");
        let refuses = floor.is_some_and(|f| f <= -200);
        (poor || refuses).then(|| json!({"town": t["name"], "id": t["id"], "rating": t["rating"], "band": t["band"],
            "refuses_stations": refuses, "advice": t["advice"].as_array().map(|a| a.iter().take(2).cloned().collect::<Vec<_>>()).unwrap_or_default()}))
    }).collect()
}

fn towns_part(g: &Game, reads: &Reads) -> Result<Value, String> {
    let payload = reads.read("towns")?;
    let towns: Vec<Value> = payload["towns"].as_array().cloned().unwrap_or_default();
    let companies = crate::panel_town::rated_companies(&towns);
    let read = if g.server_running() {
        let mark = g.log_seq();
        g.console("setting difficulty.town_council_tolerance").ok().and_then(|_| {
            std::thread::sleep(Duration::from_millis(600));
            g.log_after(mark).iter().find_map(|l| crate::panel_town::tolerance_of(l))
        })
    } else { None };
    let tolerance = read.unwrap_or("lenient");
    let lib = crate::rhai_host::rhai_dir();
    let mut per = Vec::new();
    let mut total = 0;
    for c in &companies {
        let live = crate::panel_town::live_from(&lib, &payload, Some(*c), tolerance);
        if let Some(e) = live["advice_error"].as_str() { return Err(format!("town_lib: {e}")); }
        let w = warnings_of(&live);
        total += w.len();
        per.push(json!({"company": c, "towns": live["count"], "warning_count": w.len(), "warnings": w}));
    }
    Ok(json!({"tolerance": tolerance, "tolerance_source": if read.is_some() { "console" } else { "default: lenient" },
              "towns": towns.len(), "rated_companies": companies, "companies": per, "warning_count": total}))
}

/// BattleBrain's proposals (`brain.json`, schema battle-brain/v1) as `strategy.tuning`.
pub fn tuning_of(v: &Value, mtime: f64, path: &str) -> Value {
    let proposals: Vec<Value> = v["proposals"].as_array().into_iter().flatten().map(|p| json!({
        "kind": p["kind"], "predicted_share": p["predicted_share"], "spread": p.get("uncertainty").or(p.get("spread")),
        "settings": p["settings"]})).collect();
    json!({"schema": v["schema"], "file": path, "mtime": mtime, "rows": v["rows"], "distinct_settings": v["distinct_settings"],
           "trust_region": v["trust_region"], "enough_data": v["enough_data"], "proposals": proposals,
           "note": "read-only: the strategist never writes brain.json nor calls the GA"})
}

fn tuning_part(cfg: &Config) -> Result<Value, String> {
    let p = &cfg.brain;
    let meta = match std::fs::metadata(p) { Ok(m) => m, Err(e) => return Ok(json!({"skipped": format!("{}: {e}", p.display())})) };
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64());
    let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))?;
    if v["schema"] != "battle-brain/v1" { return Err(format!("{}: schema {} is not battle-brain/v1", p.display(), v["schema"])); }
    Ok(tuning_of(&v, mtime, &p.display().to_string()))
}

/// The short solver name of a portfolio's `solver`.
pub fn solver_short(solver: &Value) -> &'static str {
    match solver.as_str() {
        Some(s) if s.starts_with("algoglyph") => "AlgoGlyph knapsack-dp",
        Some(_) => "built-in DP",
        None => "no solver",
    }
}

/// One cycle now; `trigger` says why. Never panics into the caller: every failure is a part's error.
pub fn cycle(g: &Arc<Game>, trigger: &str) -> Value {
    let s = strategist();
    let _one = lock(&s.cycle);
    let cfg = &s.cfg;
    let date = game_date(g);
    let started = unix_now();
    let t0 = Instant::now();
    { let mut r = lock(&s.run); r.running = true; }
    status("running", &format!("cycle started ({trigger})"), Value::Null, &date);
    let algo = algoglyph(cfg);
    let available = algo["available"] == true;
    let reads = Reads { g, cache: Mutex::new(HashMap::new()), order: Mutex::new(Vec::new()) };
    let mut params = json!({});
    if let Some(ml) = g.max_loan().filter(|m| *m > 0) { params["budget"] = json!(ml); }
    let read = |n: &str| reads.read(n);
    let opts = crate::plan_pipeline::Options { limit: cfg.limit, params: params.clone(), invest: available, ..Default::default() };
    let topts = crate::plan_pipeline::Options { track: cfg.track, ..opts.clone() };
    let topts = crate::plan_pipeline::Options { invest: false, ..topts };
    // The pool: three workers, each a part; the planner's investment runs after its plan, in its worker.
    let ((plan_r, plan_ms), (track_r, track_ms), (towns_r, towns_ms)) = std::thread::scope(|sc| {
        let plan = sc.spawn(|| timed(|| crate::plan_pipeline::run(&read, &opts)));
        let track = sc.spawn(|| timed(|| {
            if !available { return Ok(json!({"skipped": "no AlgoGlyph checkout: track-planning needs one"})); }
            if cfg.track == 0 { return Ok(json!({"skipped": "PRISMATIC_STRATEGY_TRACK is 0"})); }
            crate::plan_pipeline::run(&read, &topts)
        }));
        let towns = sc.spawn(|| timed(|| towns_part(g, &reads)));
        let join = |h: std::thread::ScopedJoinHandle<'_, (Result<Value, String>, u128)>| h.join().unwrap_or((Err("the worker panicked".into()), 0));
        (join(plan), join(track), join(towns))
    });
    let (tuning_r, tuning_ms) = timed(|| tuning_part(cfg));
    let mut errors = serde_json::Map::new();
    let mut parts = serde_json::Map::new();
    parts.insert("plan".into(), part(&plan_r, plan_ms));
    parts.insert("track".into(), part(&track_r, track_ms));
    parts.insert("towns".into(), part(&towns_r, towns_ms));
    parts.insert("tuning".into(), part(&tuning_r, tuning_ms));
    for (k, v) in &parts { if let Some(e) = v.get("error") { errors.insert(k.clone(), e.clone()); } }

    // The board: what each part found.
    let mut summary = json!({"cycle": lock(&s.run).cycles + 1, "game_date": date, "trigger": trigger, "algoglyph": algo});
    match &plan_r {
        Ok(out) => {
            let plan = plan_of(out);
            summary["solver"] = plan["portfolio"]["solver"].clone();
            summary["lines"] = json!(plan["portfolio"]["chosen"].as_array().map_or(0, Vec::len));
            summary["total_score"] = plan["portfolio"]["total_score"].clone();
            put("strategy.plan", plan, &date);
            let inv = match out.get("investment") {
                Some(i) => json!({"solver": i.get("solver"), "candidates": i.get("candidates"), "multi_leg": i.get("multi_leg"),
                                  "runs": i.get("runs"), "portfolio": portfolio_of(i)}),
                None => json!({"skipped": format!("no AlgoGlyph checkout at {}: transport-investment needs one", summary["algoglyph"]["dir"].as_str().unwrap_or(""))}),
            };
            summary["investment_lines"] = json!(inv.pointer("/portfolio/chosen").and_then(Value::as_array).map_or(0, Vec::len));
            put("strategy.investment", inv, &date);
            let p = crate::panel_bot::settings_from_plan(out, &mut |_| {});
            let (settings, text) = crate::panel_bot::settings_json(&p.settings);
            put("strategy.bot_settings_suggestion", json!({"settings": settings, "text": text, "lines": p.bot_settings,
                "data_only": "a suggestion: the strategist never applies settings; the bot start and POST /api/bots/planned do"}), &date);
        }
        Err(e) => { put("strategy.plan", json!({"error": e}), &date); }
    }
    let track = match &track_r {
        Ok(v) if v.get("skipped").is_some() => v.clone(),
        Ok(v) => json!({"line_costs": v["line_costs"], "portfolio": portfolio_of(v)}),
        Err(e) => json!({"error": e}),
    };
    put("strategy.track", track, &date);
    let towns = towns_r.clone().unwrap_or_else(|e| json!({"error": e}));
    summary["town_warnings"] = towns.get("warning_count").cloned().unwrap_or(Value::Null);
    put("strategy.towns", towns.clone(), &date);
    let tuning = tuning_r.clone().unwrap_or_else(|e| json!({"error": e}));
    summary["tuning_proposals"] = json!(tuning["proposals"].as_array().map_or(0, Vec::len));
    put("strategy.tuning", tuning.clone(), &date);

    // The mesh: what we planned goes out, what peers plan comes in. Never fails the cycle.
    let solver = summary["solver"].clone();
    let fallback = solver_short(&solver) != "AlgoGlyph knapsack-dp";
    let url = g.panel_url();
    let title = format!("OpenTTD strategy {}: {} lines by {}, score {}; full data GET {url}/api/strategy",
                        month_of(&date).unwrap_or_else(|| "?".into()), summary["lines"], solver_short(&solver), summary["total_score"]);
    let observation = observation_text(&towns, fallback, &summary["algoglyph"], &url);
    let instance = format!("panel-{}", g.args.panel_port);
    let mesh = { let mut r = lock(&s.run); mesh_step(&mut r.mesh, cfg, &instance, &title, observation.as_deref()) };
    if let Some(peers) = mesh.get("peers").cloned() { put("mesh.peers", peers, &date); }
    let mesh_status = { let mut m = mesh.clone(); if let Some(o) = m.as_object_mut() { o.remove("peers"); } m };
    summary["text"] = json!(title);
    summary["duration_ms"] = json!(t0.elapsed().as_millis());
    put("strategy.summary", summary.clone(), &date);

    // ADR 0034: one structured event per decision, beside its sentence.
    let stored = panel_events::Stored::Fallback(format!("live only: the strategist reads no recorded history yet (store {})",
        crate::panel::py_str(Some(&crate::panel_store::store_status()))));
    let fallbacks: Vec<String> = [
        (!available).then(|| format!("no AlgoGlyph at {}: built-in DP, no investment, no track", summary["algoglyph"]["dir"].as_str().unwrap_or(""))),
        (available && fallback).then(|| "AlgoGlyph was found but the built-in DP decided (the runner refused)".to_string()),
    ].into_iter().flatten().collect();
    let event = panel_events::plan("strategy.cycle",
        json!({"strategist": VERSION, "limit": cfg.limit, "track": cfg.track, "every_s": cfg.every.as_secs(), "budget": params.get("budget")}),
        json!({"reads": reads.json(), "algoglyph": summary["algoglyph"], "trigger": trigger, "fallback": fallbacks,
               "brain": cfg.brain.display().to_string(), "board": "GET /api/blackboard?prefix=strategy."}),
        &stored,
        json!({"solver": solver, "lines": summary["lines"], "total_score": summary["total_score"], "investment_lines": summary["investment_lines"],
               "town_warnings": summary["town_warnings"], "tuning_proposals": summary["tuning_proposals"],
               "chosen": board().value("strategy.plan").pointer("/portfolio/chosen").cloned().unwrap_or(Value::Null)}));
    let seq = g.plan("plan.strategy", event);
    g.event("plan", &format!("strategy: {} lines by {}, score {}, {} town warning(s), {} tuning proposal(s) ({} ms)",
                             summary["lines"], solver_short(&solver), crate::panel::py_str(summary.get("total_score")),
                             crate::panel::py_str(summary.get("town_warnings")), summary["tuning_proposals"], summary["duration_ms"]));

    let record = json!({"cycle": summary["cycle"], "trigger": trigger, "started_at": started, "finished_at": unix_now(),
                        "duration_ms": summary["duration_ms"], "parts": parts, "errors": errors, "event_seq": seq, "mesh": mesh_status});
    {
        let mut r = lock(&s.run);
        r.cycles += 1;
        r.running = false;
        r.last_at = Some(Instant::now());
        r.last_month = month_of(&date);
        r.last = Some(record.clone());
    }
    let mut notes = Vec::new();
    if tuning.get("skipped").is_some() { notes.push(format!("BattleBrain: {}", tuning["skipped"].as_str().unwrap_or(""))); }
    let state = if errors.is_empty() { "ok" } else { "error" };
    status(state, if errors.is_empty() { "the last cycle completed" } else { "the last cycle had errors (see errors)" },
           json!({"cycle": summary["cycle"], "errors": errors, "notes": notes, "mesh": mesh_status, "last": record}), &date);
    record
}

/// The observation a cycle has for its peers: town warnings, or the solver's fallback; None when
/// there is nothing to observe.
pub fn observation_text(towns: &Value, fallback: bool, algo: &Value, url: &str) -> Option<String> {
    let mut parts = Vec::new();
    let n = towns["warning_count"].as_u64().unwrap_or(0);
    if n > 0 {
        let first = towns["companies"].as_array().into_iter().flatten()
            .find_map(|c| c["warnings"].as_array().and_then(|w| w.first()).map(|w| (c["company"].clone(), w.clone())));
        let detail = first.map(|(c, w)| format!(" (company {c}: {} rating {})", w["town"].as_str().unwrap_or("?"),
                                                 crate::panel::py_str(w.get("rating")))).unwrap_or_default();
        parts.push(format!("{n} town warning(s){detail}"));
    }
    if fallback {
        parts.push(if algo["available"] == true { "solver fell back to the built-in DP".to_string() }
                   else { "solver fallback: built-in DP, no AlgoGlyph checkout".to_string() });
    }
    (!parts.is_empty()).then(|| format!("OpenTTD strategy: {}; GET {url}/api/strategy", parts.join("; ")))
}

// ---------------------------------------------------------------------- the mesh

/// How a refused journal write is handled (mesh_records.rs: Majordomus' admission rules).
#[derive(Debug, PartialEq)]
pub enum Admission {
    /// No live claim of the session covers the record's scope: announce instead.
    OutOfScope,
    /// The record is another session's: write a new one of our own.
    NotOwn,
    /// The record moved on since we read it: write a new one.
    StaleRevision,
    /// Typed records are not served here (Majordomus before 1.2.0), or nothing answers.
    Unavailable(String),
    Other(String),
}

pub fn classify(e: &mesh_records::Error) -> Admission {
    use mesh_records::Error as E;
    match e {
        E::Refused { status: 404, .. } => Admission::Unavailable(format!("{e} (typed records need Majordomus 1.2.0)")),
        E::Refused { code, message, .. } => {
            let text = format!("{code} {message}");
            if text.contains("out_of_scope") { Admission::OutOfScope }
            else if text.contains("not_own") { Admission::NotOwn }
            else if text.contains("stale_revision") { Admission::StaleRevision }
            else { Admission::Other(e.to_string()) }
        }
        E::Unreachable(_) => Admission::Unavailable(e.to_string()),
        _ => Admission::Other(e.to_string()),
    }
}

/// Write `title` as our record of `kind` (revising the one we hold), with one fresh write when the
/// held one is stale or not ours. Ok(why not written) when there was nothing to do.
fn write_record(c: &mesh_records::Client, held: &mut Option<(String, u64)>, last: &mut Option<String>, kind: &str,
                title: &str, cfg: &Config) -> Result<Value, Admission> {
    if held.is_some() && last.as_deref() == Some(title) { return Ok(json!({"kind": kind, "unchanged": true, "record": held.as_ref().map(|h| &h.0)})); }
    let fresh = json!({"kind": kind, "task": cfg.mesh_task, "scope": cfg.mesh_scope, "title": title,
                       "sources": [{"type": "tool", "command": "GET /api/strategy", "result": "the strategist's full data on the panel"}]});
    let first = match held { Some((k, rev)) => json!({"record": k, "revision": rev, "title": title}), None => fresh.clone() };
    let r = c.write(first).or_else(|e| match classify(&e) {
        Admission::StaleRevision | Admission::NotOwn if held.is_some() => { *held = None; c.write(fresh) }
        _ => Err(e),
    });
    match r {
        Ok(v) => {
            *held = Some((v["key"].as_str().unwrap_or("").to_string(), v["revision"].as_u64().unwrap_or(1)));
            *last = Some(title.to_string());
            Ok(json!({"kind": kind, "written": v["key"], "revision": v["revision"]}))
        }
        Err(e) => Err(classify(&e)),
    }
}

fn mesh_step(m: &mut Mesh, cfg: &Config, instance: &str, title: &str, observation: Option<&str>) -> Value {
    if !cfg.mesh { return json!({"skipped": "PRISMATIC_STRATEGY_MESH is off"}); }
    let client = match mesh_records::Client::new(&cfg.majordomus, cfg.session.clone(), Duration::from_secs(5)) {
        Ok(c) => c,
        Err(e) => return json!({"skipped": e.to_string()}),
    };
    let mut out = json!({"majordomus": cfg.majordomus});
    // 1. Typed records, when the session is named and the journal admits them.
    let mut announce_why: Option<String> = None;
    match &cfg.session {
        None => announce_why = Some("no MAJORDOMUS_SESSION: typed records need a session with live claims".into()),
        Some(_) => {
            let mut written = Vec::new();
            let (plan, plan_title) = (&mut m.plan, &mut m.plan_title);
            match write_record(&client, plan, plan_title, "plan", title, cfg) {
                Ok(v) => written.push(v),
                Err(a) => announce_why = Some(format!("plan record: {a:?}")),
            }
            if let (Some(obs), None) = (observation, &announce_why) {
                match write_record(&client, &mut m.observation, &mut m.observation_title, "observation", obs, cfg) {
                    Ok(v) => written.push(v),
                    Err(a) => announce_why = Some(format!("observation record: {a:?}")),
                }
            }
            out["records"] = Value::Array(written);
        }
    }
    // 2. Else a named claim on the board (board.rs).
    if let Some(why) = announce_why {
        out["records_skipped"] = json!(why);
        let text = board::clip(&match observation { Some(o) => format!("{title} | {o}"), None => title.to_string() }, 480);
        let claim = format!("openttd:{instance}:strategy");
        if m.announced.as_deref() == Some(text.as_str()) {
            out["announce"] = json!({"claim": claim, "unchanged": true});
        } else {
            let b = m.board.get_or_insert_with(|| board::Board::new(&cfg.majordomus, false, "openttd-strategist", "OpenTTD panel strategist"));
            match b.announce(&claim, &text) {
                Ok(()) => { m.announced = Some(text); out["announce"] = json!({"claim": claim, "announced": true}); }
                Err(e) => { out["announce"] = json!({"claim": claim, "skipped": e}); }
            }
        }
    }
    if let Some(b) = m.board.as_mut() { b.keep_alive(); }
    // 3. What the peers plan: typed records, else the cooperation journal's events.
    out["peers"] = read_peers(&client, cfg);
    out
}

fn read_peers(c: &mesh_records::Client, cfg: &Config) -> Value {
    let own = cfg.session.as_deref();
    let plans = c.records(&json!({"kind": "plan"}));
    let decisions = c.records(&json!({"kind": "decision"}));
    match (plans, decisions) {
        (Ok(p), Ok(d)) if p["active"] == true => peers_from_records(&p, &d, own),
        (Ok(p), _) if p["active"] == false => json!({"source": "mesh.records", "records": [], "intents": [], "reason": p["reason"], "read_at": unix_now()}),
        (Err(e), _) | (_, Err(e)) if !matches!(classify(&e), Admission::Unavailable(_)) =>
            json!({"source": "mesh.records", "records": [], "intents": [], "reason": e.to_string(), "read_at": unix_now()}),
        _ => {
            let url = format!("{}/api/v1/mesh/events?newest=true&limit=200", cfg.majordomus.trim_end_matches('/'));
            match ureq::get(&url).timeout(Duration::from_secs(5)).call().map_err(|e| e.to_string())
                .and_then(|r| r.into_json::<Value>().map_err(|e| e.to_string())) {
                Ok(v) => peers_from_events(&v, own),
                Err(e) => json!({"source": "none", "records": [], "intents": [], "reason": format!("mesh.records unavailable and {url}: {e}"), "read_at": unix_now()}),
            }
        }
    }
}

fn own_author(author: &str, own: Option<&str>) -> bool {
    own.is_some_and(|s| author == s || author.ends_with(&format!("/{s}")))
}

fn trim_record(r: &Value) -> Value {
    json!({"key": r["key"], "author": r["author"], "kind": r["kind"], "task": r["task"], "title": r["title"],
           "status": r.get("status"), "revision": r["revision"], "scope": r.get("scope")})
}

/// The newest plan and decision records of other authors (at most 10 of each, as listed).
pub fn peers_from_records(plans: &Value, decisions: &Value, own: Option<&str>) -> Value {
    let pick = |v: &Value| -> Vec<Value> {
        let rs: Vec<Value> = v["records"].as_array().into_iter().flatten()
            .filter(|r| !own_author(r["author"].as_str().unwrap_or(""), own)).map(trim_record).collect();
        rs[rs.len().saturating_sub(10)..].to_vec()
    };
    let mut records = pick(plans);
    records.extend(pick(decisions));
    json!({"source": "mesh.records", "records": records, "intents": [], "reason": Value::Null, "read_at": unix_now()})
}

/// Peers from the cooperation journal (`GET /api/v1/mesh/events`, Majordomus before 1.2.0): every
/// written plan or decision record, and the newest intent of each other session (opened or claimed).
pub fn peers_from_events(v: &Value, own: Option<&str>) -> Value {
    if v["active"] == false {
        return json!({"source": "mesh.events", "records": [], "intents": [], "reason": v.get("reason").cloned().unwrap_or(json!("the mesh is not active")), "read_at": unix_now()});
    }
    let mut records = Vec::new();
    let mut intents: BTreeMap<String, Value> = BTreeMap::new();
    for e in v["events"].as_array().into_iter().flatten() {
        let b = &e["body"];
        match b["kind"].as_str().unwrap_or("") {
            "record_written" => {
                let r = b.get("record").unwrap_or(b);
                let kind = r.get("record_kind").or(r.get("kind")).and_then(Value::as_str).unwrap_or("");
                if matches!(kind, "plan" | "decision") && !own_author(r["author"].as_str().or(r["session"].as_str()).unwrap_or(""), own) {
                    let mut t = trim_record(r);
                    t["kind"] = json!(kind);
                    records.push(t);
                }
            }
            "session_opened" | "claim_acquired" => {
                let session = b["session"].as_str().or(b["info"]["session"].as_str()).unwrap_or("");
                let intent = b["intent"].as_str().or(b["info"]["intent"].as_str()).unwrap_or("");
                if session.is_empty() || intent.is_empty() || own == Some(session) { continue; }
                intents.insert(session.to_string(), json!({"session": session, "intent": intent, "via": b["kind"], "at": e["at"], "scope": b.get("scope")}));
            }
            "session_closed" => { if let Some(s) = b["session"].as_str() { intents.remove(s); } }
            _ => {}
        }
    }
    let mut intents: Vec<Value> = intents.into_values().collect();
    intents.sort_by(|a, b| b["at"].as_f64().unwrap_or(0.0).total_cmp(&a["at"].as_f64().unwrap_or(0.0)));
    intents.truncate(20);
    let records = records[records.len().saturating_sub(20)..].to_vec();
    json!({"source": "mesh.events", "records": records, "intents": intents,
           "reason": "typed records are not served here (Majordomus before 1.2.0): the newest 200 journal events instead", "read_at": unix_now()})
}

// ---------------------------------------------------------------------- routes

/// GET /api/strategy: status, summary, the last cycle with its durations, the next run.
pub fn h_strategy(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    let s = strategist();
    let r = lock(&s.run);
    let next = if !s.cfg.enabled { Value::Null } else {
        match r.last_at {
            None => json!(0),
            Some(t) => json!(s.cfg.every.saturating_sub(t.elapsed()).as_secs()),
        }
    };
    let keys: Vec<Value> = board().list("strategy.").iter().map(|(e, _)| json!({"key": e.key, "revision": e.revision, "written_at": e.written_at})).collect();
    Ok((200, json!({
        "enabled": s.cfg.enabled, "thread": r.thread, "running": r.running, "cycles": r.cycles, "game": g.mcp_up.load(Ordering::SeqCst),
        "config": config_json(&s.cfg), "version": VERSION, "revision": panel_events::REVISION,
        "status": board().value("strategy.status"), "summary": board().value("strategy.summary"), "last": r.last,
        "next_run_in_s": next, "next_run_note": "at the latest; a new game month runs one sooner (no sooner than min_gap_s after the last)",
        "plan": board().value("strategy.plan"), "investment": board().value("strategy.investment"),
        "track": board().value("strategy.track"), "towns": board().value("strategy.towns"),
        "tuning": board().value("strategy.tuning"), "bot_settings_suggestion": board().value("strategy.bot_settings_suggestion"),
        "peers": board().value("mesh.peers"), "keys": keys,
        "algoglyph": flow_serve::summary(),
    })))
}

/// POST /api/strategy/run {wait?}: one cycle now; with wait the answer is the cycle's record.
pub fn h_strategy_run(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    if !g.mcp_up.load(Ordering::SeqCst) {
        return Ok((409, json!({"error": "no game: the MCP server is not up", "status": board().value("strategy.status")})));
    }
    if i.body.get("wait").and_then(Value::as_bool).unwrap_or(false) {
        return Ok((200, cycle(g, "run")));
    }
    if strategist().cycle.try_lock().is_err() {
        return Ok((200, json!({"started": false, "running": true, "note": "a cycle is running; GET /api/strategy shows it"})));
    }
    let g = g.clone();
    std::thread::spawn(move || { cycle(&g, "run"); });
    Ok((202, json!({"started": true, "note": "GET /api/strategy shows the cycle when it ends"})))
}

/// GET /api/blackboard: one key (?key=, 404 when absent) or every key under ?prefix=, each with its
/// history (?history=0 leaves it out).
pub fn h_blackboard(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let with = i.query.get("history") != Some("0");
    if let Some(key) = i.query.get("key") {
        return Ok(match board().get(key) {
            Some((e, h)) => (200, slot_json(&e, &h, with)),
            None => (404, json!({"error": format!("no blackboard key {key:?}")})),
        });
    }
    let prefix = i.query.get("prefix").unwrap_or("");
    let entries: Vec<Value> = board().list(prefix).iter().map(|(e, h)| slot_json(e, h, with)).collect();
    Ok((200, json!({"entries": entries, "count": entries.len(), "reserved": RESERVED, "history_kept": HISTORY,
                    "limits": {"keys": MAX_KEYS, "value_bytes": MAX_VALUE}})))
}

/// POST /api/blackboard {key, value, by?}: a script's write, published as blackboard.write.
pub fn h_blackboard_write(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let key = i.body.get("key").and_then(Value::as_str).unwrap_or("");
    let value = i.body.get("value").cloned().unwrap_or(Value::Null);
    let by = i.body.get("by").and_then(Value::as_str).unwrap_or("");
    Ok(match board().write_external(key, value, by, game_date(g)) {
        Ok(rev) => {
            let (e, h) = board().get(key).ok_or("the key went away")?;
            g.bus.publish("blackboard.write", json!({"key": key, "revision": rev, "by": e.by}));
            (200, slot_json(&e, &h, false))
        }
        Err((code, why)) => (code, json!({"error": why})),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_keeps_its_writer_revision_and_a_short_history() {
        let b = Blackboard::default();
        for n in 0..12 { b.put("strategy.plan", json!(n), "strategist", json!("1950-01-01")).unwrap(); }
        let (e, h) = b.get("strategy.plan").unwrap();
        assert_eq!((e.value.clone(), e.revision, e.by.as_str()), (json!(11), 12, "strategist"));
        assert_eq!(h.len(), HISTORY, "only the last few values are kept");
        assert_eq!(h[0].value, json!(10), "newest first");
        assert_eq!(e.game_date, json!("1950-01-01"));
        assert!(e.written_at > 0.0);
    }

    #[test]
    fn scripts_cannot_write_the_reserved_namespaces() {
        let b = Blackboard::default();
        assert_eq!(b.write_external("strategy.plan", json!(1), "rhai", Value::Null).unwrap_err().0, 403);
        assert_eq!(b.write_external("mesh.peers", json!(1), "rhai", Value::Null).unwrap_err().0, 403);
        assert_eq!(b.write_external("rail.hint", json!({"x": 1}), "  ", Value::Null), Ok(1));
        assert_eq!(b.get("rail.hint").unwrap().0.by, "unnamed");
        assert_eq!(b.write_external("bad key", json!(1), "w", Value::Null).unwrap_err().0, 400);
        assert_eq!(b.write_external("big", json!("x".repeat(MAX_VALUE)), "w", Value::Null).unwrap_err().0, 400);
        assert_eq!(b.write_external("k", json!(1), "a\nb", Value::Null).unwrap_err().0, 400);
        assert!(b.put("strategy.summary", json!({}), "strategist", Value::Null).is_ok(), "the strategist writes its own");
        let keys: Vec<String> = b.list("").iter().map(|(e, _)| e.key.clone()).collect();
        assert_eq!(keys, vec!["rail.hint", "strategy.summary"]);
        assert_eq!(b.list("strategy.").len(), 1);
    }

    #[test]
    fn the_environment_configures_the_strategist() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        let d = config_from(&env(&[("HOME", "/h")]));
        assert!(d.enabled && d.serve && d.mesh);
        assert_eq!((d.every.as_secs(), d.min_gap.as_secs(), d.limit, d.track), (300, 30, 20, 3));
        assert_eq!(d.majordomus, "http://127.0.0.1:8741");
        assert_eq!(d.brain, PathBuf::from("/h/ttd-battle/results/brain.json"));
        assert_eq!(d.mesh_scope, vec!["research/mechanisms/strategist-and-blackboard.md"]);
        let c = config_from(&env(&[("PRISMATIC_STRATEGIST", "0"), ("PRISMATIC_STRATEGY_EVERY", "2"), ("MCP_ALGOGLYPH_SERVE", "off"),
                                   ("PRISMATIC_MAJORDOMUS_URL", "http://127.0.0.1:9"), ("PRISMATIC_STRATEGY_MESH_SCOPE", "a, b,"),
                                   ("PRISMATIC_STRATEGY_TRACK", "99"), ("MAJORDOMUS_SESSION", "s1")]));
        assert!(!c.enabled && !c.serve);
        assert_eq!(c.every.as_secs(), 10, "at least 10 s");
        assert_eq!(c.track, 10);
        assert_eq!(c.majordomus, "http://127.0.0.1:9");
        assert_eq!(c.mesh_scope, vec!["a", "b"]);
        assert_eq!(c.session.as_deref(), Some("s1"));
        let m = config_from(&env(&[("MAJORDOMUS_URL", "http://127.0.0.1:1"), ("PRISMATIC_MAJORDOMUS_URL", "http://127.0.0.1:9")]));
        assert_eq!(m.majordomus, "http://127.0.0.1:1", "MAJORDOMUS_URL first");
    }

    #[test]
    fn a_cycle_is_due_first_on_a_new_month_and_at_least_every_interval() {
        let (every, gap) = (Duration::from_secs(300), Duration::from_secs(30));
        let s = Duration::from_secs;
        assert_eq!(due(None, None, None, every, gap), Some("first"));
        assert_eq!(due(Some(s(5)), Some("1950-02"), Some("1950-01"), every, gap), None, "not sooner than the gap");
        assert_eq!(due(Some(s(31)), Some("1950-02"), Some("1950-01"), every, gap), Some("month"));
        assert_eq!(due(Some(s(100)), Some("1950-01"), Some("1950-01"), every, gap), None);
        assert_eq!(due(Some(s(301)), Some("1950-01"), Some("1950-01"), every, gap), Some("interval"));
        assert_eq!(month_of(&json!("1950-03-14")), Some("1950-03".into()));
        assert_eq!(month_of(&Value::Null), None);
    }

    #[test]
    fn the_portfolio_is_kept_as_lines_with_their_solver() {
        let out = json!({"params": {"budget": 300000}, "by_mode": {"bus": [{}, {}], "rail": [{}]}, "freight": [{}], "pairs": [],
            "portfolio": {"solver": "builtin knapsack (MCP_ALGOGLYPH_DIR not set or the runner refused)", "kernel": null, "total_score": 5000.0,
                          "investment_k": 90, "budget_k": 300, "shadowed": [{}],
                          "chosen": [{"kind": "freight", "cargo": "Coal", "producer": {"name": "Mine"}, "acceptor": {"name": "Plant"},
                                      "mode": "freight_rail", "score": 4000, "distance": 60,
                                      "options": [{"mode": "truck", "investment": 1}, {"mode": "freight_rail", "investment": 80000, "monthly_profit": 9000}]}]}});
        let p = plan_of(&out);
        assert_eq!(p["portfolio"]["chosen"][0], json!({"kind": "freight", "cargo": "Coal", "from": "Mine", "to": "Plant", "mode": "freight_rail",
            "score": 4000, "distance": 60, "investment": 80000, "monthly_profit": 9000, "roi": null, "track": null}));
        assert_eq!(p["by_mode"], json!({"bus": 2, "rail": 1}));
        assert_eq!(p["portfolio"]["shadowed"], 1);
        assert_eq!(p["budget"], 300000);
        assert_eq!(solver_short(&p["portfolio"]["solver"]), "built-in DP");
        assert_eq!(solver_short(&json!("algoglyph knapsack-dp")), "AlgoGlyph knapsack-dp");
    }

    #[test]
    fn town_warnings_are_the_poor_bands_and_the_refusing_towns() {
        let live = json!({"towns": [
            {"name": "Aville", "id": 0, "rating": -260, "band": "TOWN_RATING_VERY_POOR", "rating_floor": -260,
             "advice": [{"what": "a"}, {"what": "b"}, {"what": "c"}]},
            {"name": "Btown", "id": 1, "rating": null, "band": null, "rating_floor": 500, "advice": []},
            {"name": "Ctown", "id": 2, "rating": 700, "band": "TOWN_RATING_GOOD", "rating_floor": 700, "advice": []}]});
        let w = warnings_of(&live);
        assert_eq!(w.len(), 1);
        assert_eq!((w[0]["town"].clone(), w[0]["refuses_stations"].clone()), (json!("Aville"), json!(true)));
        assert_eq!(w[0]["advice"].as_array().unwrap().len(), 2);
        let towns = json!({"warning_count": 1, "companies": [{"company": 0, "warnings": w}]});
        let o = observation_text(&towns, false, &json!({}), "http://127.0.0.1:1").unwrap();
        assert!(o.contains("1 town warning(s) (company 0: Aville rating -260)") && o.ends_with("/api/strategy"), "{o}");
        assert!(observation_text(&json!({"warning_count": 0}), true, &json!({"available": false}), "u").unwrap().contains("no AlgoGlyph"));
        assert_eq!(observation_text(&json!({}), false, &json!({}), "u"), None);
    }

    #[test]
    fn battle_brains_proposals_are_summarised() {
        let v = json!({"schema": "battle-brain/v1", "rows": 218, "distinct_settings": 40, "trust_region": {"radius": 2}, "enough_data": true,
                       "proposals": [{"kind": "safe", "settings": "a=1", "predicted_share": 1.29, "uncertainty": 0.05},
                                     {"kind": "explore", "settings": "a=2", "predicted_share": 1.2, "uncertainty": 0.28}]});
        let t = tuning_of(&v, 12.5, "/x/brain.json");
        assert_eq!(t["proposals"][1], json!({"kind": "explore", "predicted_share": 1.2, "spread": 0.28, "settings": "a=2"}));
        assert_eq!((t["rows"].clone(), t["mtime"].clone()), (json!(218), json!(12.5)));
        let cfg = Config { brain: PathBuf::from("/nonexistent/brain.json"), ..config_from(&|_| None) };
        assert!(tuning_part(&cfg).unwrap()["skipped"].as_str().unwrap().contains("/nonexistent/brain.json"));
    }

    #[test]
    fn journal_refusals_are_typed() {
        use mesh_records::Error as E;
        let r = |code: &str, message: &str| E::Refused { status: 422, code: code.into(), message: message.into() };
        assert_eq!(classify(&r("refused", "stale_revision: the current revision is 3")), Admission::StaleRevision);
        assert_eq!(classify(&r("out_of_scope", "no live claim covers research/x.md")), Admission::OutOfScope);
        assert_eq!(classify(&r("refused", "not_own: s2/r1 is another session's")), Admission::NotOwn);
        assert!(matches!(classify(&E::Refused { status: 404, code: "not_found".into(), message: "no route".into() }), Admission::Unavailable(_)));
        assert!(matches!(classify(&E::Unreachable("x".into())), Admission::Unavailable(_)));
        assert!(matches!(classify(&r("invalid_input", "x")), Admission::Other(_)));
    }

    #[test]
    fn peers_come_from_records_or_the_journals_events_without_our_own() {
        let plans = json!({"active": true, "records": [
            {"key": "s1/r1", "author": "st/openttd-0f", "kind": "plan", "task": "t", "title": "theirs", "revision": 1},
            {"key": "s1/r2", "author": "st/me", "kind": "plan", "task": "t", "title": "ours", "revision": 2}], "conflicts": [], "ignored": []});
        let decisions = json!({"active": true, "records": [], "conflicts": [], "ignored": []});
        let p = peers_from_records(&plans, &decisions, Some("me"));
        assert_eq!(p["records"].as_array().unwrap().len(), 1);
        assert_eq!(p["records"][0]["title"], "theirs");
        let events = json!({"active": true, "events": [
            {"at": 10, "body": {"kind": "session_opened", "info": {"session": "openttd-1e", "intent": "live arena"}}},
            {"at": 11, "body": {"kind": "claim_acquired", "session": "openttd-7e", "intent": "battle", "scope": ["x"]}},
            {"at": 12, "body": {"kind": "session_opened", "info": {"session": "me", "intent": "mine"}}},
            {"at": 13, "body": {"kind": "session_closed", "session": "openttd-1e"}},
            {"at": 14, "body": {"kind": "record_written", "record": {"key": "a/b", "author": "st/openttd-7e", "kind": "decision", "title": "go", "revision": 1}}}]});
        let e = peers_from_events(&events, Some("me"));
        assert_eq!(e["source"], "mesh.events");
        assert_eq!(e["intents"].as_array().unwrap().iter().map(|i| i["session"].as_str().unwrap()).collect::<Vec<_>>(), vec!["openttd-7e"]);
        assert_eq!(e["records"][0]["title"], "go");
        assert_eq!(peers_from_events(&json!({"active": false, "reason": "off"}), None)["reason"], "off");
    }
}
