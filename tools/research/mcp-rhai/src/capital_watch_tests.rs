//! Tests of the capital watch, `tools/research/mcp-panel/rhai/capital_watch.rhai` (RESEARCH-ONLY;
//! research/mechanisms/rhai-scripting.md "The capital watch"). The service runs against fake host
//! functions that answer what a live panel answers: the game date, openttd://companies, the
//! engine's quarterly income and expenses getters, the store's capital_months rows and the shared
//! blackboard. Two fixtures replay the post-mortem's live cases
//! (research/analysis-2026-11-23-why-prismatic-loses.md): game 12609's reinvestment collapse (62 %,
//! 44 %, 12 % against AAAHogEx's 98 %) and game 10487's hoard (165M+ in the bank, 42 vehicles, for
//! 160 game years). The fake panel registers no tool, setting, approval, chat or news function:
//! a call to one would fail the round, and every round is checked to have run without an error.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rhai::{Dynamic, Engine, Map};
use serde_json::{json, Value};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel/rhai/capital_watch.rhai");

fn script() -> String { std::fs::read_to_string(SCRIPT).unwrap() }

/// Months since year 0, January = 0 (the script's mi_of).
fn mi(y: i64, m: i64) -> i64 { y * 12 + m - 1 }

fn month_text(mi: i64) -> String { format!("{}-{:02}", mi / 12, mi % 12 + 1) }

/// One company of a fixture: its quarters by the month each ended in (income, expenses), its net
/// at the end of every month, its vehicles in every month.
#[derive(Clone, Default)]
struct Co {
    id: i64,
    name: &'static str,
    value: i64,
    loan: i64,
    quarters: BTreeMap<i64, (i64, i64)>,
    net: BTreeMap<i64, f64>,
    vehicles: BTreeMap<i64, i64>,
}

impl Co {
    /// Quarters from the one ending at `first_end` on, each (income, expenses, reinvestment): the
    /// net is `net0` at the end of the quarter before, and each quarter keeps (1 - r) of its
    /// operating cash flow, spread evenly over its three months. Vehicles: `v0` plus `per_month`.
    fn new(id: i64, name: &'static str, value: i64, loan: i64, first_end: i64, net0: f64,
           quarters: &[(i64, i64, f64)], v0: i64, per_month: i64) -> Co {
        let mut c = Co { id, name, value, loan, ..Co::default() };
        let mut net = net0;
        c.net.insert(first_end - 3, net0);
        for (k, (income, expenses, r)) in quarters.iter().enumerate() {
            let end = first_end + 3 * k as i64;
            c.quarters.insert(end, (*income, *expenses));
            let kept = (income + expenses) as f64 * (1.0 - r);
            for i in 1..=3 {
                c.net.insert(end - 3 + i, net + kept * i as f64 / 3.0);
            }
            net += kept;
        }
        for m in (first_end - 3)..=(first_end + 3 * quarters.len() as i64 + 2) {
            c.vehicles.insert(m, v0 + per_month * (m - (first_end - 3)));
        }
        c
    }
}

/// What the fake panel saw the script do.
#[derive(Default)]
struct Seen {
    looks: usize,
    printed: Vec<String>,
    shared: BTreeMap<String, Value>,
    store_calls: usize,
    getter_calls: usize,
}

/// The fake game: its companies, the date each look sees, whether the store answers, and a game
/// date read that fails on the listed looks.
struct World {
    cos: Vec<Co>,
    dates: Vec<&'static str>,
    store_on: bool,
    read_fails: Vec<usize>,
    /// Looks on which the store query itself fails (a panel that does not answer).
    store_throws: Vec<usize>,
}

fn date_mi(d: &str) -> i64 { mi(d[0..4].parse().unwrap(), d[5..7].parse().unwrap()) }

/// Runs the service for as many looks as `w.dates`, with `preset` on the blackboard.
fn run(w: World, preset: &[(&str, Value)]) -> Seen {
    let seen = Arc::new(Mutex::new(Seen::default()));
    {
        let mut s = seen.lock().unwrap();
        for (k, v) in preset { s.shared.insert(k.to_string(), v.clone()); }
    }
    let w = Arc::new(w);
    let mut e = Engine::new();
    let rounds = w.dates.len().to_string();
    e.register_fn("env", move |name: &str| -> String { if name == "CAPITAL_WATCH_ROUNDS" { rounds.clone() } else { String::new() } });
    let s = seen.clone();
    e.register_fn("sleep", move |ms: i64| { assert_eq!(ms, 20_000); s.lock().unwrap().looks += 1; });
    let s = seen.clone();
    e.on_print(move |text| s.lock().unwrap().printed.push(text.to_string()));
    let (s, ww) = (seen.clone(), w.clone());
    e.register_fn("read", move |uri: &str| -> super::R<Dynamic> {
        let n = s.lock().unwrap().looks;
        let date = ww.dates[n];
        let now = if date.len() >= 7 && date.as_bytes()[4] == b'-' { date_mi(date) } else { 0 };
        match uri {
            "openttd://game/state" if ww.read_fails.contains(&n) => Err("panel: connection refused".into()),
            "openttd://game/state" => super::to_dyn(json!({"game_tick": 1, "game_date": date})),
            "openttd://companies" => super::to_dyn(json!({"data": {"companies": ww.cos.iter().map(|c| {
                // Live money: the net of the newest month the fixture has at or before now, plus the loan.
                let net = c.net.range(..=now).next_back().map(|(_, v)| *v).unwrap_or(0.0);
                let v = c.vehicles.range(..=now).next_back().map(|(_, v)| *v).unwrap_or(0);
                json!({"id": c.id, "name": c.name, "money": (net as i64) + c.loan, "loan": c.loan,
                       "company_value": c.value, "fleet": {"train": v, "road": 0, "ship": 0, "aircraft": 0}})
            }).collect::<Vec<_>>()}})),
            other => Err(format!("unexpected read {other}").into()),
        }
    });
    for (getter, expenses) in [("openttd_api_ScriptCompany_GetQuarterlyIncome", false),
                               ("openttd_api_ScriptCompany_GetQuarterlyExpenses", true)] {
        let (s, ww) = (seen.clone(), w.clone());
        e.register_fn(getter, move |m: Map| -> super::R<Dynamic> {
            let args = super::from_dyn(Dynamic::from_map(m))?;
            let (company, q) = (args["args"][0].as_i64().unwrap(), args["args"][1].as_i64().unwrap());
            let mut s = s.lock().unwrap();
            s.getter_calls += 1;
            let now = date_mi(ww.dates[s.looks]);
            // Quarter q ended the month before q-1 quarters before the running one's first month.
            let end = now - now % 3 - 3 * (q - 1) - 1;
            let c = ww.cos.iter().find(|c| c.id == company).unwrap();
            // The engine's -1: no such quarter (as for an invalid company).
            let v = c.quarters.get(&end).map(|(i, x)| if expenses { *x } else { *i }).unwrap_or(-1);
            super::to_dyn(json!({"ok": true, "result": v}))
        });
    }
    let (s, ww) = (seen.clone(), w.clone());
    e.register_fn("api_dbNamedRun", move |m: Map| -> super::R<Dynamic> {
        let q = super::from_dyn(Dynamic::from_map(m))?;
        assert_eq!(q, json!({"name": "capital_months", "params": {"months": 18}}));
        let mut s = s.lock().unwrap();
        s.store_calls += 1;
        if ww.store_throws.contains(&s.looks) { return Err("POST /api/db/named: connection refused".into()); }
        if !ww.store_on { return super::to_dyn(json!({"error": "the store is off"})); }
        let now = date_mi(ww.dates[s.looks]);
        let mut rows = vec![];
        for c in &ww.cos {
            for (m, net) in c.net.range((now - 17)..=now) {
                let v = c.vehicles.get(m).copied().unwrap_or(0);
                rows.push(json!([c.id, c.name, month_text(*m), net + c.loan as f64, c.loan, net, v, c.value]));
            }
        }
        super::to_dyn(json!({"columns": ["company", "name", "month", "money", "loan", "net", "vehicles", "value"], "rows": rows}))
    });
    let s = seen.clone();
    e.register_fn("shared_get", move |k: &str| -> super::R<Dynamic> {
        s.lock().unwrap().shared.get(k).cloned().map_or(Ok(Dynamic::UNIT), super::to_dyn)
    });
    let s = seen.clone();
    e.register_fn("shared_set", move |k: &str, v: Dynamic| -> super::R<i64> {
        s.lock().unwrap().shared.insert(k.to_string(), super::from_dyn(v)?);
        Ok(1)
    });
    e.run(&script()).unwrap();
    drop(e);
    Arc::try_unwrap(seen).ok().unwrap().into_inner().unwrap()
}

/// Game 12609 (seed 12609, 1952-1954): PrismaticAI reinvests 62 % of its operating cash flow in
/// 1952, 44 % in 1953 and 12 % in 1954 (the post-mortem's 61.9 / 43.6 / 12.4 %); AAAHogEx reinvests
/// 98 % throughout and grows its fleet by 5 vehicles a month, while PrismaticAI's stays at 33.
fn game_12609() -> Vec<Co> {
    let ours: Vec<(i64, i64, f64)> = (0..11).map(|k| (120_000, -40_000, if k < 4 { 0.62 } else if k < 8 { 0.44 } else { 0.12 })).collect();
    let theirs: Vec<(i64, i64, f64)> = (0..11).map(|_| (300_000, -100_000, 0.98)).collect();
    vec![Co::new(0, "Prismatic", 1_536_394, 0, mi(1952, 3), 100_000.0, &ours, 33, 0),
         Co::new(1, "AAAHogEx", 2_631_237, 300_000, mi(1952, 3), -250_000.0, &theirs, 60, 5)]
}

fn errors(seen: &Seen) -> Vec<&String> {
    seen.printed.iter().filter(|l| l.starts_with("capital_watch: ") && !l.starts_with("capital_watch: a look")).collect()
}

/// The collapse is seen from the second quarter under the floor, said once a quarter, and never
/// for the leader; every figure is the engine's and the store's, each threshold names its source.
#[test]
fn capital_watch_flags_game_12609s_reinvestment_collapse_once_a_quarter() {
    let seen = run(World { cos: game_12609(), dates: vec!["1953-01-05", "1953-01-25", "1953-07-04", "1953-08-04", "1954-10-04"],
                           store_on: true, read_fails: vec![], store_throws: vec![] }, &[]);
    assert!(errors(&seen).is_empty(), "{:?}", seen.printed);
    assert_eq!(seen.looks, 4, "five looks, a sleep between each");
    assert_eq!(seen.store_calls, 4, "one store query per game month judged: 1953-01, 07, 08 and 1954-10");
    assert_eq!(seen.getter_calls, 4 * 2 * 4 * 2, "two getters, four quarters, two companies, four months");
    let said: Vec<&String> = seen.printed.iter().filter(|l| l.contains("under_reinvesting")).collect();
    assert_eq!(said.len(), 2, "said at 1953-07 and again at 1954-10, not at 1953-08: {:?}", seen.printed);
    assert_eq!(said[0].as_str(), "capital_watch 1953-07 company 1 (Prismatic), 1953Q2: under_reinvesting: reinvested 1953Q2 44 %; \
1953Q1 44 % of its operating cash flow, under the floor 50 % for 2 quarter(s); AAAHogEx 98 % in 1953Q2 \
[floor from REINVEST_FLOOR (cold start), idle from IDLE_YEARS (cold start)]");
    assert!(said[1].starts_with("capital_watch 1954-10 company 1 (Prismatic), 1954Q3: hoarding: 612000 net in the bank is 1.3 year(s) of income (threshold 1.0); \
vehicles 0 in 12 months against AAAHogEx's 60; under_reinvesting: reinvested 1954Q3 12 %; 1954Q2 12 %; 1954Q1 12 %; 1953Q4 44 %"), "{}", said[1]);
    assert!(seen.printed.iter().any(|l| l == "capital_watch 1954-10-04: 2 company(ies) judged; flagged: [1]"), "{:?}", seen.printed);
    assert_eq!(seen.printed.last().unwrap(), "capital_watch look 5: month 1954-10");

    let c0 = &seen.shared["capital.0"];
    // The ADR 0042 envelope.
    assert_eq!((c0["company"].clone(), c0["period"].clone(), c0["game_date"].clone(), c0["writer"].clone(), c0["version"].clone()),
               (json!(0), json!("1954-10"), json!("1954-10-04"), json!("capital_watch"), json!(1)));
    assert_eq!(c0["reads"], json!([
        {"source": "openttd://game/state", "rows": 1}, {"source": "openttd://companies", "rows": 2},
        {"source": "ScriptCompany.GetQuarterlyIncome/GetQuarterlyExpenses", "quarters": 4, "rows": 16},
        {"source": "store:capital_months", "params": {"months": 18}, "rows": 34},
        {"source": "derived.capital_watch.reinvestment_floor", "rows": 0},
        {"source": "derived.capital_watch.idle_years", "rows": 0}]));
    assert_eq!(c0["unknown"], json!([]));
    let q = &c0["quarters"];
    assert_eq!(q.as_array().unwrap().iter().map(|r| r["quarter"].clone()).collect::<Vec<_>>(),
               vec![json!("1954Q3"), json!("1954Q2"), json!("1954Q1"), json!("1953Q4")]);
    assert_eq!((q[0]["ocf"].clone(), q[0]["implied_capex"].clone(), q[3]["implied_capex"].clone()),
               (json!(80000.0), json!(9600.0), json!(35200.0)));
    assert!((q[0]["reinvestment"].as_f64().unwrap() - 0.12).abs() < 1e-9);
    assert_eq!((c0["streak"].clone(), c0["vehicles"].clone(), c0["vehicles_growth"].clone(), c0["said_quarter"].clone()),
               (json!(4), json!(33), json!(0.0), json!("1954Q3")));
    assert!((c0["trailing"]["reinvestment"].as_f64().unwrap() - 0.2).abs() < 1e-9, "(3 x 0.12 + 0.44) / 4: {}", c0["trailing"]);
    assert!((c0["idle_ratio"].as_f64().unwrap() - 612_000.0 / 480_000.0).abs() < 1e-9);
    assert_eq!(c0["thresholds"]["reinvestment_floor"], json!({"value": 0.5, "source": "REINVEST_FLOOR (cold start)", "bootstrap": true,
        "why": "derived.capital_watch.reinvestment_floor is not published: the declared cold start 0.5"}));
    assert_eq!(c0["leader"]["company"], json!(1));
    assert!((c0["leader"]["reinvestment"].as_f64().unwrap() - 0.98).abs() < 1e-9);
    assert_eq!(c0["leader"]["vehicles_growth"], json!(60.0));
    let c1 = &seen.shared["capital.1"];
    assert_eq!((c1["flags"].clone(), c1["streak"].clone(), c1["leader"]["company"].clone()), (json!({}), json!(0), json!(0)),
               "the leader reinvests and is judged against the runner-up: {c1}");
    assert!(c1["idle_ratio"].as_f64().unwrap() < 0.0, "AAAHogEx borrows: its net is below zero");
}

/// A restart in the same quarter says nothing again; a derived floor replaces the cold start, and
/// a derived value out of range falls back and says why.
#[test]
fn capital_watch_reads_derived_thresholds_and_does_not_repeat_after_a_restart() {
    let preset = [("capital.0", json!({"said_quarter": "1954Q3"})),
                  ("derived.capital_watch.reinvestment_floor", json!({"value": 0.3, "method": "p25", "n": 40})),
                  ("derived.capital_watch.idle_years", json!(500))];
    let seen = run(World { cos: game_12609(), dates: vec!["1954-10-04"], store_on: true, read_fails: vec![], store_throws: vec![] }, &preset);
    assert!(errors(&seen).is_empty(), "{:?}", seen.printed);
    assert!(!seen.printed.iter().any(|l| l.contains("company 1 (Prismatic)")), "said before the restart: {:?}", seen.printed);
    let c0 = &seen.shared["capital.0"];
    assert_eq!(c0["thresholds"]["reinvestment_floor"], json!({"value": 0.3, "source": "derived.capital_watch.reinvestment_floor",
        "bootstrap": false, "why": "derived.capital_watch.reinvestment_floor = 0.3"}));
    assert_eq!(c0["thresholds"]["idle_years"]["source"], json!("IDLE_YEARS (cold start)"));
    assert_eq!(c0["thresholds"]["idle_years"]["why"],
               json!("derived.capital_watch.idle_years = 500 is outside 0.25..50.0: the declared cold start 1.0"));
    assert_eq!(c0["streak"], json!(3), "0.44 is above the derived floor 0.3: three quarters of 12 %");
    assert_eq!(c0["said_quarter"], json!("1954Q3"), "kept from before the restart");
    assert_eq!(c0["reads"][4], json!({"source": "derived.capital_watch.reinvestment_floor", "rows": 1}));
}

/// Game 10487 (2131): company 1 holds 185,657,267 with no loan and 42 vehicles that have not
/// changed for years, earning 4M a year and keeping almost all of it, while AAAHogEx, worth
/// 4.87 billion, adds ten vehicles a month and reinvests 97 %.
#[test]
fn capital_watch_flags_game_10487s_hoard() {
    let ours: Vec<(i64, i64, f64)> = (0..5).map(|_| (1_000_000, -300_000, 0.05)).collect();
    let theirs: Vec<(i64, i64, f64)> = (0..5).map(|_| (90_000_000, -40_000_000, 0.97)).collect();
    let hoarder = Co::new(0, "Prismatic", 190_000_000, 0, mi(2129, 12), 182_332_267.0, &ours, 42, 0);
    let leader = Co::new(1, "AAAHogEx", 4_866_411_199, 2_000_000, mi(2129, 12), 1_000_000.0, &theirs, 1_000, 10);
    let seen = run(World { cos: vec![hoarder, leader], dates: vec!["2131-01-23"], store_on: true, read_fails: vec![], store_throws: vec![] }, &[]);
    assert!(errors(&seen).is_empty(), "{:?}", seen.printed);
    let c0 = &seen.shared["capital.0"];
    assert_eq!(c0["money"], json!(185_657_267.0), "the post-mortem's 2131-01 money: {c0}");
    assert_eq!(c0["vehicles"], json!(42));
    assert!((c0["idle_ratio"].as_f64().unwrap() - 185_657_267.0 / 4_000_000.0).abs() < 1e-6, "46 years of income in the bank");
    assert!(c0["flags"]["hoarding"].as_str().unwrap().starts_with("185657267 net in the bank is 46.4 year(s) of income (threshold 1.0); vehicles 0 in 12 months against AAAHogEx's 120"),
            "{}", c0["flags"]["hoarding"]);
    assert_eq!(c0["streak"], json!(4));
    let line = seen.printed.iter().find(|l| l.contains("company 1 (Prismatic)")).unwrap();
    assert!(line.starts_with("capital_watch 2131-01 company 1 (Prismatic), 2130Q4: hoarding: "), "{line}");
    assert_eq!(seen.shared["capital.1"]["flags"], json!({}));
}

/// Without the store the engine's figures are still recorded, every quarter and the growth are
/// unknown, never zero, and nothing is flagged; a failing read and a game without a date cost a
/// look, not the service.
#[test]
fn capital_watch_with_the_store_off_records_unknowns_and_flags_nothing() {
    let seen = run(World { cos: game_12609(), dates: vec!["", "1954-10-04", "1954-10-05", "1954-11-02"], store_on: false,
                           read_fails: vec![2], store_throws: vec![3] }, &[]);
    assert_eq!(seen.printed[1], "capital_watch: no game date yet ('')");
    assert_eq!(seen.printed[2], "capital_watch look 1: month not judged yet");
    assert!(seen.printed.iter().any(|l| l == "capital_watch: panel: connection refused"), "{:?}", seen.printed);
    assert!(seen.printed.iter().any(|l| l == "capital_watch: store: POST /api/db/named: connection refused"), "{:?}", seen.printed);
    assert_eq!(seen.store_calls, 2, "1954-10 and 1954-11");
    let c0 = &seen.shared["capital.0"];
    assert_eq!(c0["flags"], json!({}));
    assert_eq!(c0["reads"][3]["rows"], json!(null));
    let unknown: Vec<String> = c0["unknown"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    assert_eq!(unknown[0], "store: capital_months did not answer");
    assert_eq!(unknown[1], "vehicles growth: the store holds no month 12 months back");
    assert_eq!(unknown[2], "1954Q3: the store holds no month-end net for 1954-06");
    assert_eq!(unknown.len(), 6);
    assert_eq!((c0["quarters"][0]["ocf"].clone(), c0["quarters"][0]["reinvestment"].clone()), (json!(80000.0), json!(null)));
    assert_eq!(c0["trailing"]["unknown"], json!("1954Q3: the store holds no month-end net for 1954-06"));
    assert!((c0["idle_ratio"].as_f64().unwrap() - 612_000.0 / 480_000.0).abs() < 1e-9, "the engine alone gives the idle ratio");
}

/// The early game: quarters before the company existed (the engine's -1 or zeros), a cash flow
/// under MIN_OCF, a selling quarter whose implied capex is negative, and a game of one company.
#[test]
fn capital_watch_keeps_small_missing_and_negative_quarters_unknown_or_as_they_are() {
    // Two quarters of economy only; the first earned 4,000, the second sold vehicles (r = -0.5).
    let c = Co::new(0, "Solo", 50_000, 100_000, mi(1950, 6), 20_000.0, &[(6_000, -2_000, 0.5), (60_000, -10_000, -0.5)], 5, 0);
    let seen = run(World { cos: vec![c], dates: vec!["1950-10-02"], store_on: true, read_fails: vec![], store_throws: vec![] }, &[]);
    assert!(errors(&seen).is_empty(), "{:?}", seen.printed);
    let c0 = &seen.shared["capital.0"];
    let q = &c0["quarters"];
    assert_eq!((q[0]["quarter"].clone(), q[0]["implied_capex"].clone()), (json!("1950Q3"), json!(-25000.0)));
    assert!((q[0]["reinvestment"].as_f64().unwrap() + 0.5).abs() < 1e-9, "kept, not clipped");
    assert_eq!((q[1]["ocf"].clone(), q[1]["reinvestment"].clone(), q[1]["unknown"].clone()),
               (json!(4000.0), json!(null), json!("operating cash flow 4000 is under 10000")));
    assert_eq!((q[2]["income"].clone(), q[2]["unknown"].clone()), (json!(null), json!("the engine gave no economy for the quarter")));
    assert_eq!(c0["trailing"]["income"], json!(null));
    assert_eq!(c0["unknown"][0], json!("idle ratio: the engine gave no income for a quarter"));
    assert_eq!((c0["leader"].clone(), c0["streak"].clone(), c0["flags"].clone()), (json!(null), json!(1), json!({})), "1950Q3 sold more than it bought: one quarter under the floor, then an unknown one");
}

/// The pure helpers on their edges.
#[test]
fn capital_watch_helpers_are_pure() {
    let s = script();
    let fns = s.split("// --- the loop").next().unwrap();
    let mut e = Engine::new();
    e.register_fn("env", |_: &str| String::new());
    let out: rhai::Array = e.eval(&format!(r#"{fns}
        [mi_of("1954-10-03"), mi_of("1954-13"), mi_of("1954"), mi_of(()), month_text(mi_of("1954-01")), quarter_text(mi_of("1954-12")),
         pct(0.444), pct(()), econ_value(#{{ ok: false }}), econ_value(#{{ ok: true, result: -1 }}), econ_value(#{{ ok: true, result: 7 }}), econ_value(()),
         by_company(()), by_company(#{{ error: "x" }}), by_company(#{{ columns: ["company", "month", "net"], rows: [[1.0, "1950-01", 5]] }})["1"]["1950-01"].net,
         threshold_of(0.2, "k", 0.5, "F", [0.0, 1.0]).value, threshold_of(#{{ p75: 0.2 }}, "k", 0.5, "F", [0.0, 1.0]).why,
         trailing_of([]).reinvestment, trailing_of([#{{ quarter: "q", income: 100.0, ocf: 100.0, implied_capex: 50.0 }}]).unknown,
         growth_of(#{{}}, "1954-10"), growth_of(#{{}}, "x"), quarters_of([], #{{}}, "x").len(), fleet_of(()), fleet_of(#{{ train: 2.0, road: 3 }}),
         leader_of([#{{ id: 0, company_value: 5 }}], 0), rounds(), joined([])]
    "#)).unwrap();
    // () prints as the empty text.
    let got: Vec<String> = out.iter().map(|x| x.to_string()).collect();
    assert_eq!(got, vec!["23457", "", "", "", "1954-01", "1954Q4", "44", "", "", "", "7.0", "",
                         "", "", "5", "0.2", "k = #{\"p75\": 0.2} is outside 0.0..1.0: the declared cold start 0.5",
                         "", "operating cash flow 100 over the year is under 10000 a quarter",
                         "", "", "0", "", "5", "", "0", ""]);
}
