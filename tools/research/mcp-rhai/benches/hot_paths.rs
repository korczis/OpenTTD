//! RESEARCH-ONLY: performance benchmarks of the hot paths the panel and the planner run, on the
//! real game capture the Battlebrain tests replay (tools/research/schema/fixtures/battlebrain/
//! captures/coplay-8800-1962-05-29: a 256 x 256 map, 25 towns, 1405 world-graph nodes) and the
//! AlgoGlyph catalogue fixture of the planner tests.
//!
//! The crate has no library target, so the benchmarked modules are compiled in here by path, as
//! every binary of the crate does (src/bin/mcp-plan.rs). Nothing is copied: each bench calls the
//! module's own function. An AlgoGlyph kernel is never run here: the planner's side of every
//! kernel call is measured with a runner that answers a straight plan at once, so the numbers are
//! the planner's own cost and not the runner's.
//!
//! Run, record and check: tools/research/rust-bench (research/rust-bench.md). Directly:
//! `cargo bench --manifest-path tools/research/mcp-rhai/Cargo.toml --bench hot_paths`.
#![allow(dead_code, unused_imports)]

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkGroup, Criterion, SamplingMode, Throughput};
use criterion::measurement::WallTime;
use serde_json::{json, Value};

#[path = "../src/battlebrain_contract.rs"]
mod battlebrain_contract;
#[path = "../src/bus_sinks.rs"]
mod bus_sinks;
#[path = "../src/calibrate.rs"]
mod calibrate;
#[path = "../src/cargo.rs"]
mod cargo;
#[path = "../src/graph.rs"]
mod graph;
#[path = "../src/hub_algoglyph.rs"]
mod hub_algoglyph;
#[path = "../src/invest.rs"]
mod invest;
#[path = "../src/linecost.rs"]
mod linecost;
#[path = "../src/outcome.rs"]
mod outcome;
#[path = "../src/plan.rs"]
mod plan;
#[path = "../src/plan_pipeline.rs"]
mod plan_pipeline;
#[path = "../src/sites.rs"]
mod sites;
#[path = "../src/store_sql.rs"]
mod store_sql;
#[path = "../src/yards.rs"]
mod yards;

use battlebrain_contract as bb;
use cargo::{KernelFailure, Kernels};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
const CAPTURE: &str = "../schema/fixtures/battlebrain/captures/coplay-8800-1962-05-29";

fn fixture_path(rel: &str) -> PathBuf {
    Path::new(MANIFEST_DIR).join(rel)
}

fn read_text(rel: &str) -> String {
    let p = fixture_path(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// One resource of the capture: the `data` of openttd://<name>, as mcp-plan --snapshot replays it.
fn resource(name: &str) -> Value {
    let file = format!("{CAPTURE}/{}.json", name.replace('/', "__"));
    let v: Value = serde_json::from_str(&read_text(&file)).expect("a capture file is JSON");
    v.get("data").cloned().unwrap_or(v)
}

/// Every resource the planner pipeline reads, read once.
struct Capture {
    resources: Vec<(&'static str, Value)>,
}

impl Capture {
    fn load() -> Capture {
        let names = ["towns", "map", "stations", "map/ways", "engines", "economy/prices", "world/graph", "map/terrain", "bridges"];
        Capture { resources: names.iter().map(|n| (*n, resource(n))).collect() }
    }
    fn get(&self, name: &str) -> Result<Value, String> {
        self.resources.iter().find(|(n, _)| *n == name).map(|(_, v)| v.clone()).ok_or_else(|| format!("{name}: not captured"))
    }
}

/// The real AlgoGlyph catalogue fixture (53 kernels), as the planner tests read it.
fn catalogue() -> bb::Catalogue {
    let f: Value = serde_json::from_str(&read_text("../schema/fixtures/battlebrain/catalogue.json")).expect("catalogue JSON");
    bb::Catalogue::from_runner(&f["kernels"], f["commit"].as_str().unwrap_or("")).expect("a valid catalogue")
}

/// A track plan from the input's `from` to its `to`: an L of plain track pieces, priced at 100 a
/// tile. It passes track-planning's domain check, so the planner prices and proves as on a found plan.
fn straight_plan(input: &Value) -> Value {
    let at = |k: &str, c: &str| input[k][c].as_i64().unwrap_or(0);
    let (fx, fy, tx, ty) = (at("from", "x"), at("from", "y"), at("to", "x"), at("to", "y"));
    let mut pieces = Vec::new();
    let step = |a: i64, b: i64| if b >= a { 1 } else { -1 };
    let mut x = fx;
    while x != tx { pieces.push(json!({"kind": "track", "x": x, "y": fy})); x += step(fx, tx); }
    let mut y = fy;
    while y != ty { pieces.push(json!({"kind": "track", "x": tx, "y": y})); y += step(fy, ty); }
    pieces.push(json!({"kind": "track", "x": tx, "y": ty}));
    let n = pieces.len() as f64;
    json!({"ok": true, "kernel": {"id": "track-planning", "plate": "track"}, "work": {"expanded": n},
           "result": {"status": "found", "cost": n, "money": n * 100.0, "pieces": pieces, "counts": {"track": n}, "runs": [1]}})
}

/// Kernels over the catalogue fixture whose runner answers `straight_plan`.
fn kernels(cat: &bb::Catalogue) -> Kernels {
    let cat = cat.clone();
    Kernels::new(move || Ok(cat.clone()), |_, input| Ok(straight_plan(input)))
}

/// Kernels without a catalogue: every capability falls back to the built-in solver.
fn no_kernels() -> Kernels {
    Kernels::new(|| Err(KernelFailure::NoCatalogue("benchmark: no AlgoGlyph catalogue".into())), |_, _| Err(KernelFailure::Unset))
}

/// The planner's parameters for the benches: a budget, so the portfolio solves its knapsack.
fn params() -> Value {
    json!({"budget": 2_000_000})
}

fn options(track: usize, yards: usize) -> plan_pipeline::Options {
    plan_pipeline::Options { limit: 20, params: params(), track, yards, ..Default::default() }
}

/// Short and flat: every sample runs the same number of iterations, so a bench of tens of
/// milliseconds finishes in its measurement time too, and the whole suite in about a minute.
fn quick(g: &mut BenchmarkGroup<'_, WallTime>, slow: bool) {
    g.warm_up_time(Duration::from_millis(500));
    g.measurement_time(Duration::from_secs(if slow { 3 } else { 2 }));
    g.sample_size(20);
    g.sampling_mode(SamplingMode::Flat);
}

fn json_parse(c: &mut Criterion) {
    let mut g = c.benchmark_group("json");
    quick(&mut g, false);
    for (name, file) in [("parse_world_graph", "world__graph.json"), ("parse_map_ways", "map__ways.json"), ("parse_map", "map.json")] {
        let text = read_text(&format!("{CAPTURE}/{file}"));
        g.throughput(Throughput::Bytes(text.len() as u64));
        g.bench_function(name, |b| b.iter(|| serde_json::from_str::<Value>(black_box(&text)).unwrap()));
    }
    g.finish();
}

fn planner(c: &mut Criterion) {
    let cap = Capture::load();
    let (towns, map, stations, ways, engines, prices) = (cap.get("towns").unwrap(), cap.get("map").unwrap(), cap.get("stations").unwrap(),
        cap.get("map/ways").unwrap(), cap.get("engines").unwrap(), cap.get("economy/prices").unwrap());
    let graph = graph::derive_with_ways(&map, &towns, &stations, &ways).expect("the capture's road graph");
    let p = plan::with_prices(&params(), &prices);
    let cat = catalogue();
    let read = |n: &str| cap.get(n);

    let mut g = c.benchmark_group("planner");
    quick(&mut g, true);
    g.bench_function("road_graph", |b| b.iter(|| graph::derive_with_ways(black_box(&map), &towns, &stations, &ways).unwrap()));
    g.bench_function("plan_passengers", |b| b.iter(|| plan::plan(black_box(&towns), &graph, 20, &p, &engines).unwrap()));
    g.bench_function("pipeline", |b| b.iter(|| plan_pipeline::run(&read, &options(0, 0), &no_kernels()).unwrap()));
    g.bench_function("pipeline_track_yards", |b| b.iter(|| plan_pipeline::run(&read, &options(8, 4), &kernels(&cat)).unwrap()));
    g.finish();
}

fn cargo_and_lines(c: &mut Criterion) {
    let cap = Capture::load();
    let read = |n: &str| cap.get(n);
    let out = plan_pipeline::run(&read, &options(0, 0), &no_kernels()).expect("a plan of the capture");
    let (world, engines, map) = (cap.get("world/graph").unwrap(), cap.get("engines").unwrap(), cap.get("map").unwrap());
    let mut candidates: Vec<Value> = Vec::new();
    for (_, ps) in out["by_mode"].as_object().cloned().unwrap_or_default() { candidates.extend(ps.as_array().cloned().unwrap_or_default()); }
    candidates.extend(out["freight"].as_array().cloned().unwrap_or_default());
    candidates.sort_by(|a, b| b["score"].as_f64().unwrap_or(0.0).total_cmp(&a["score"].as_f64().unwrap_or(0.0)));
    let rail: Vec<Value> = out["freight"].as_array().cloned().unwrap_or_default().into_iter().filter(|c| c["stations"]["from"].is_object()).collect();
    assert!(!rail.is_empty(), "the capture's plan has no freight rail line with station sites");
    let opt = |r: &str| -> Result<Option<Value>, String> { Ok(cap.get(r).ok()) };
    let (base, _) = linecost::coherent_input(&opt, linecost::READ_TRIES).expect("the capture's track input");
    let cat = catalogue();
    let ends: Vec<(i64, i64, i64, i64)> = candidates.iter().take(20).map(|c| {
        let (a, b) = if c.get("producer").is_some() { (&c["producer"], &c["acceptor"]) } else { (&c["a"], &c["b"]) };
        let n = |v: &Value, k: &str| v[k].as_i64().unwrap_or(0);
        (n(a, "x"), n(a, "y"), n(b, "x"), n(b, "y"))
    }).collect();

    let mut g = c.benchmark_group("cargo");
    quick(&mut g, false);
    g.bench_function("freight_pairs", |b| b.iter(|| cargo::freight_pairs(black_box(&world), &engines, &out["params"], 20)));
    g.throughput(Throughput::Elements(candidates.len() as u64));
    g.bench_function("portfolio_builtin", |b| b.iter(|| cargo::portfolio(black_box(&candidates), Some(2_000_000.0), &no_kernels())));
    g.finish();

    let mut g = c.benchmark_group("lines");
    quick(&mut g, false);
    g.throughput(Throughput::Elements(ends.len() as u64));
    g.bench_function("station_sites", |b| b.iter(|| {
        ends.iter().map(|&(ox, oy, tx, ty)| sites::site(black_box(&map), ox, oy, tx, ty, 3)).filter(Option::is_some).count()
    }));
    g.throughput(Throughput::Elements(rail.len().min(8) as u64));
    g.bench_function("price_rail_lines", |b| b.iter_batched(|| (rail.clone(), kernels(&cat)),
        |(mut lines, k)| linecost::price(&mut lines, &base, 1480.0, 100.0, 8, &k), criterion::BatchSize::SmallInput));
    let mut priced = rail.clone();
    let log = linecost::price(&mut priced, &base, 1480.0, 100.0, 8, &kernels(&cat));
    assert!(log.iter().any(|l| l["status"] == "found"), "no line was priced: {log:?}");
    assert!(!yards::prove(&priced, &base, 4, &kernels(&cat)).is_empty(), "the priced lines share no end: no yard to prove");
    g.throughput(Throughput::Elements(priced.len() as u64));
    g.bench_function("prove_yards", |b| b.iter(|| yards::prove(black_box(&priced), &base, 4, &kernels(&cat))));
    g.finish();
}

/// An engine that resolves `import` from mcp-panel/rhai, as the panel's does (main.rs lib_engine).
fn lib_engine() -> rhai::Engine {
    let mut e = rhai::Engine::new();
    e.set_module_resolver(rhai::module_resolvers::FileModuleResolver::new_with_path(fixture_path("../mcp-panel/rhai")));
    e
}

/// 120 bot line records over 40 slots of two companies, 40 stations, read by rail_lib as the
/// steward reads a live game (the record format of main.rs RAIL_FIXTURE).
const RAIL_SCRIPT: &str = r#"
    import "rail_lib" as rl;
    let ev = [];
    let stations = [];
    for s in 0..40 {
        stations.push(#{ id: s, owner: s % 2, tile: s * 300 + 2, x: (s * 300 + 2) % 256, y: (s * 300 + 2) / 256,
                         catchment_area: #{ x: (s * 300) % 256, y: (s * 300) / 256, w: 9, h: 7 } });
    }
    for i in 0..120 {
        let slot = i % 40;
        let a = (slot * 300 + 2);
        let b = ((slot + 1) % 40) * 300 + 2;
        ev.push(#{ kind: "ai.log", seq: i, data: #{ company: slot % 2,
            text: `PrismaticAI: rail line ${slot}: label=COAL ${slot}|a=${a}|b=${b}|cargo=1|platforms=2|len=${3 + i % 3}|depot=${a + 1}|trains_max=2|wagons_fit=4|tiles=60|state=open` } });
    }
    let recs = rl::records_from_events(ev);
    let lines = rl::lines_of(0, recs, [], stations, 256);
    [recs.len(), lines.len()]
"#;

/// The director's coalescing over 120 events (main.rs the_director_acts_once_per_run_of_one_kind).
const DIRECTOR_SCRIPT: &str = r#"
    import "director_lib" as d;
    let ev = [];
    for i in 0..119 { ev.push(#{ kind: "ai.log", data: #{ text: `PrismaticAI: built rail at tile ${1000 + i}` } }); }
    ev.push(#{ kind: "ai.log", data: #{ text: "PrismaticAI: built rail station in Runinghall at tile 9866" } });
    ev.push(#{ kind: "game.new_world", data: () });
    d::coalesce(ev).len()
"#;

fn rhai_engine(c: &mut Criterion) {
    let mut g = c.benchmark_group("rhai");
    quick(&mut g, false);
    for (name, file) in [("compile_rail_lib", "rail_lib.rhai"), ("compile_ttd", "ttd.rhai"), ("compile_replan", "replan.rhai")] {
        let text = read_text(&format!("../mcp-panel/rhai/{file}"));
        let e = rhai::Engine::new();
        g.throughput(Throughput::Bytes(text.len() as u64));
        g.bench_function(name, |b| b.iter(|| e.compile(black_box(&text)).unwrap()));
    }
    let e = lib_engine();
    let out: rhai::Array = e.eval(RAIL_SCRIPT).expect("rail_lib runs on the bench records");
    assert_eq!(out[0].as_int().unwrap_or(0) > 0, true, "rail_lib read no record");
    g.throughput(Throughput::Elements(120));
    g.bench_function("rail_lib_lines", |b| b.iter(|| e.eval::<rhai::Array>(black_box(RAIL_SCRIPT)).unwrap()));
    g.bench_function("director_coalesce", |b| b.iter(|| e.eval::<i64>(black_box(DIRECTOR_SCRIPT)).unwrap()));
    g.finish();
}

/// A store as one panel run fills it: 18 companies sampled over 400 game days, 5000 bus events.
fn filled_store(dir: &Path) -> bus_sinks::db::Store {
    let _ = std::fs::remove_dir_all(dir);
    let mut store = bus_sinks::db::Store::open(&dir.join("bench.duckdb"), "panel-bench").expect("a DuckDB store");
    store.insert_run(&json!("bench"), Some(1), 1_000, (Some(8800), Some(8731), Some(3979))).expect("the run row");
    let kinds = ["game.month", "news.shown", "plan.portfolio", "ai.log", "plan.duel"];
    let events: Vec<Value> = (0..5000).map(|i| json!({"instance": "panel-bench", "seq": i, "ts_ms": 2_000 + i, "kind": kinds[i as usize % kinds.len()],
        "game_tick": i * 74, "game_date": format!("1962-{:02}-{:02}", 1 + (i / 28) % 12, 1 + i % 28),
        "data": {"date": "1962-05-29", "text": format!("event {i}"), "inputs": {"live": {"record": format!("r{i}")}}}})).collect();
    store.insert_events(&events).expect("the events");
    let rows: Vec<bus_sinks::SampleRow> = (0..400).flat_map(|day| (0..18).map(move |c| bus_sinks::SampleRow {
        ts_ms: 2_000 + day * 10, game_tick: Some(day * 74), game_date: Some(format!("1962-{:02}-{:02}", 1 + (day / 28) % 12, 1 + day % 28)),
        company: Some(c), name: Some(format!("Company {c}")), money: 1e5 + (day * c) as f64, loan: 3e5, value: 2e5 + (day * 100 + c) as f64,
        net: 1000.0, vehicles: Some(c + day / 40) })).collect();
    store.insert_samples(&rows).expect("the samples");
    store
}

fn store(c: &mut Criterion) {
    let dir = std::env::temp_dir().join(format!("mcp-rhai-bench-{}", std::process::id()));
    let store = filled_store(&dir);
    let named = |name: &'static str, p: Value| store_sql::Named::new(name, p);
    let mut g = c.benchmark_group("store");
    quick(&mut g, false);
    for (bench, q) in [("newest_events", named("newest_events", json!({}))),
                       ("event_field", named("event_field", json!({"kind": "plan.duel", "path": "$.inputs.live.record"}))),
                       ("company_trend", named("company_trend", json!({"company": 3, "months": 12}))),
                       ("kernel_window", named("kernel_window", json!({"instance": "panel-bench", "days": 30})))] {
        let first = store.run(&q).unwrap_or_else(|e| panic!("{bench}: {e}"));
        assert!(first["count"].as_u64().unwrap_or(0) > 0, "{bench} answered no row: {first}");
        g.bench_function(bench, |b| b.iter(|| store.run(black_box(&q)).unwrap()));
    }
    g.finish();
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

criterion_group!(benches, json_parse, planner, cargo_and_lines, rhai_engine, store);
criterion_main!(benches);
