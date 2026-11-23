//! Tests of the leader-gap watch, `tools/research/mcp-panel/rhai/gap_watch.rhai` (RESEARCH-ONLY;
//! research/control-loops-audit.md). The service runs against fake host functions that answer
//! what a live panel answers (shapes copied from the owner's game of 2026-11-22): replan's and
//! watchdog's blackboard keys, the store's company_samples rows, bus events with the bot's own log
//! lines, filed approvals and executed decisions, the running bot's declared settings, and the
//! pending approvals. Every call the script makes that could reach a game is recorded and checked.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rhai::{Dynamic, Engine, Map};
use serde_json::{json, Value};

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel/rhai/gap_watch.rhai");

fn script() -> String { std::fs::read_to_string(SCRIPT).unwrap() }

fn engine() -> Engine {
    let mut e = Engine::new();
    e.set_module_resolver(rhai::module_resolvers::FileModuleResolver::new_with_path(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel/rhai")));
    e
}

/// The script's functions and constants without its loop, followed by `tail`.
fn functions_then(tail: &str) -> Vec<String> {
    let s = script();
    let fns = s.split("// --- the loop").next().unwrap();
    let mut e = engine();
    e.register_fn("env", |_: &str| String::new());
    let out: rhai::Array = e.eval(&format!("{fns}\n{tail}")).unwrap();
    out.iter().map(|x| x.to_string()).collect()
}

/// One row of the named query company_samples: instance, seed, run_ms, game_tick, game_date, value,
/// money, loan, net, vehicles, aggressiveness, directive_outcome.
fn row(run_ms: i64, date: &str, value: i64, money: i64, loan: i64, vehicles: i64) -> Value {
    json!(["panel-9", 3, run_ms, 0, date, value, money, loan, money - loan, vehicles, null, null])
}

fn setting(name: &str, value: i64, min: i64, max: i64, default: i64) -> Value {
    json!({"name": name, "value": value, "min": min, "max": max, "default": default, "step": 1, "editable": true})
}

fn bot_settings() -> Value {
    json!([setting("invest_borrow", 0, 0, 1, 0), setting("air_grow_money", 0, 0, 10000, 2500),
           setting("air_routes_max", 2, 0, 30, 15), setting("invest_road_cap", 12, 1, 30, 12),
           setting("max_routes", 6, 1, 30, 4), setting("cash_reserve", 5000, 0, 500000, 25000),
           setting("line_budget", 50000, 10000, 1000000, 50000)])
}

/// What the fake panel saw the script do.
#[derive(Default)]
struct Seen {
    looks: i64,
    cursors: Vec<i64>,
    said: Vec<String>,
    news: Vec<String>,
    proposals: Vec<Value>,
    approvals_read: i64,
    shared: BTreeMap<String, Value>,
}

/// Runs the service for `rounds` looks. Look n shows bus events of game month 1953-(n+1); the first
/// look also the month's bot lines, approvals and a decision. `pending(n)` answers status().pending
/// at look n, `env` the environment.
fn run(rounds: i64, env: &'static [(&'static str, &'static str)], pending: fn(i64) -> Value,
       preset: &[(&str, Value)]) -> Seen { run_with(rounds, env, pending, preset, false) }

/// As `run`; `text_answer` makes the set_settings tool answer as older servers did, in text only.
fn run_with(rounds: i64, env: &'static [(&'static str, &'static str)], pending: fn(i64) -> Value,
            preset: &[(&str, Value)], text_answer: bool) -> Seen {
    let seen = Arc::new(Mutex::new(Seen::default()));
    {
        let mut s = seen.lock().unwrap();
        for (k, v) in preset { s.shared.insert(k.to_string(), v.clone()); }
    }
    let mut e = engine();
    let rounds_text = rounds.to_string();
    e.register_fn("env", move |name: &str| -> String {
        if name == "GAP_WATCH_ROUNDS" { return rounds_text.clone(); }
        env.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string()).unwrap_or_default()
    });
    let s = seen.clone();
    e.register_fn("sleep", move |_ms: i64| {
        let mut s = s.lock().unwrap();
        s.looks += 1;
        // replan writes its key once a game month: keep its date in the month the bus shows.
        let date = format!("1953-{:02}-05", s.looks + 1);
        if let Some(r) = s.shared.get_mut("replan.0") { r["date"] = json!(date); }
    });
    let s = seen.clone();
    e.register_fn("events", move |since: i64, kinds: &str| -> super::R<Dynamic> {
        assert_eq!(kinds, "ai.log,panel.mcp,decision.recorded,game.month");
        let mut s = s.lock().unwrap();
        s.cursors.push(since);
        let n = s.looks;
        // The panel's /api/events envelopes carry no game date: game.month opens each month.
        let at = |seq: i64, _day: i64, kind: &str, data: Value| json!({"seq": seq, "kind": kind, "data": data});
        let base = 100 * (n + 1);
        let mut evs = vec![
            at(base - 1, 1, "game.month", json!({"date": format!("1953-{:02}-01", n + 1), "vehicles": 30})),
            at(base, 2, "ai.log", json!({"company": 0, "level": "info",
                "text": "PrismaticAI heartbeat: date=19530102 money=300000 vehicles=30"}))];
        if n == 0 {
            evs.extend([
                at(base + 1, 6, "ai.log", json!({"company": 0, "text": "PrismaticAI: levelled tile 47120 for 8735 (estimated 9707) and the airport was still refused: ERR_FLAT_LAND_REQUIRED (bank 280389)"})),
                at(base + 2, 7, "ai.log", json!({"company": 0, "text": "PrismaticAI: repaid 20000 of the loan (now 0)"})),
                at(base + 3, 8, "ai.log", json!({"company": 0, "text": "PrismaticAI: no rail path between the stations; removing them"})),
                at(base + 4, 8, "ai.log", json!({"company": 0, "text": "PrismaticAI: no own freight line: 48 producers on land, 9 ours, 19 claimed by another company's rail station, cargoes: 11 under 40 a month"})),
                at(base + 5, 9, "ai.log", json!({"company": 1, "text": "1953-1-9 not PrismaticAI's line"})),
                at(base + 6, 10, "panel.mcp", json!({"text": "openttd.ai.set_setting({\"company\": 0, \"name\": \"air_planes\", \"value\": 5}) -> pending approval #7"})),
                at(base + 7, 10, "panel.mcp", json!({"text": "openttd.ai.set_setting({\"company\": 0, \"name\": \"own_lines\", \"value\": 4}) -> pending approval #8"})),
                at(base + 8, 10, "panel.mcp", json!({"text": "MCP server up at http://127.0.0.1:1/mcp"})),
                at(base + 9, 11, "decision.recorded", json!({"approval_id": 7, "command_result": "succeeded"})),
                at(base + 10, 12, "decision.recorded", json!({"approval_id": 99, "command_result": "failed"})),
            ]);
        }
        let last = evs.last().unwrap()["seq"].clone();
        super::to_dyn(json!({"events": evs, "last_seq": last}))
    });
    let s = seen.clone();
    e.register_fn("status", move || -> super::R<Dynamic> {
        let n = s.lock().unwrap().looks;
        super::to_dyn(json!({"pending": pending(n)}))
    });
    e.register_fn("api_bots", || -> super::R<Dynamic> {
        super::to_dyn(json!({"running": [
            {"id": 0, "ai": "PrismaticAI", "name": "Owner Co", "settings": bot_settings()},
            {"id": 1, "ai": "AAAHogEx", "name": "Leader", "settings": []}]}))
    });
    e.register_fn("api_dbNamedRun", |m: Map| -> super::R<Dynamic> {
        let q = super::from_dyn(Dynamic::from_map(m))?;
        assert_eq!(q["name"], "company_samples");
        let rows: Vec<Value> = match q["params"]["company"].as_i64() {
            // An older run of the same store first: only the newest run counts.
            Some(0) => [row(1, "1952-01-01", 1, 1, 1, 1)].into_iter().chain((0..7).map(|i| {
                let date = if i < 2 { format!("1952-{:02}-01", 11 + i) } else { format!("1953-{:02}-01", i - 1) };
                row(100, &date, 500_000 + 3_000 * i, 300_000, 0, 30)
            })).collect(),
            Some(1) => (0..7).map(|i| {
                let date = if i < 2 { format!("1952-{:02}-01", 11 + i) } else { format!("1953-{:02}-01", i - 1) };
                row(100, &date, 900_000 + 50_000 * i, 40_000, if i == 0 { 300_000 } else { 290_000 }, 40 + 3 * i)
            }).collect(),
            other => panic!("unexpected company {other:?}"),
        };
        super::to_dyn(json!({"columns": [], "rows": rows}))
    });
    let s = seen.clone();
    e.register_fn("read", move |uri: &str| -> super::R<Dynamic> {
        assert_eq!(uri, "openttd://approvals", "the only game read: the deadline policy when filing");
        s.lock().unwrap().approvals_read += 1;
        super::to_dyn(json!({"game_tick": 9000, "data": {"approvals": [], "deadline": {"ticks": 2220, "days": 30, "verdict": "keep"}}}))
    });
    let s = seen.clone();
    e.register_fn("api_tool_openttd_ai_set_settings", move |m: Map| -> super::R<Dynamic> {
        s.lock().unwrap().proposals.push(super::from_dyn(Dynamic::from_map(m))?);
        super::to_dyn(if text_answer { json!({"content": [{"type": "text", "text": "pending approval #900"}]}) }
                      else { json!({"approval_id": 900, "status": "pending_approval",
                                    "message": "Approval required in OpenTTD: prismatic_mcp approve 900"}) })
    });
    let s = seen.clone();
    e.register_fn("api_newsShow", move |m: Map| -> super::R<Dynamic> {
        let v = super::from_dyn(Dynamic::from_map(m))?;
        s.lock().unwrap().news.push(v["text"].as_str().unwrap().to_string());
        super::to_dyn(json!({"ok": true}))
    });
    let s = seen.clone();
    e.register_fn("con_say", move |t: &str| { s.lock().unwrap().said.push(t.to_string()); });
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
    drop(e); // its functions hold the other references to `seen`
    Arc::try_unwrap(seen).ok().unwrap().into_inner().unwrap()
}

fn replan_key() -> Value {
    json!({"date": "1953-01-05", "value": 518000, "rank": 2,
           "competitors": [{"company": 1, "name": "Leader", "value": 1_200_000}, {"company": 2, "name": "Small", "value": 10}],
           "competition": [
               {"type": "aircraft", "leader": "Leader", "leader_per_vehicle": 29871.0, "ours_per_vehicle": 16900.0, "ours_vehicles": 8, "we_lead": false},
               {"type": "road", "leader": "Owner Co", "leader_per_vehicle": 3000.0, "ours_per_vehicle": 3000.0, "ours_vehicles": 10, "we_lead": true}]})
}

fn watchdog_key() -> Value { json!({"stalled": false, "quiet_days": 12, "threshold_days": 60, "reasons": []}) }

const PROPOSE: &[(&str, &str)] = &[("GAP_WATCH_PROPOSE", "1")];

/// Month by month on a game that falls behind: the flags build up, the owner is told once a flag
/// has held two months, the third month files ONE set_settings with exactly the levers that apply
/// (bounded at the declared defaults; road is skipped where we lead), the next month holds while it
/// waits, and the month after records that it was not applied and holds for the cooldown.
#[test]
fn gap_watch_flags_tells_once_proposes_one_bounded_plan_and_tracks_its_outcome() {
    fn pending(look: i64) -> Value {
        match look {
            // A replan row on a setting that is not a lever does not hold the proposal.
            3 => json!([{"id": 31, "tool": "openttd.ai.set_setting", "args": {"company": 0, "name": "aggressiveness", "value": 80}}]),
            4 => json!([{"id": 900, "tool": "openttd.ai.set_settings", "args": {"company": 0, "settings": {}}}]),
            _ => json!([]),
        }
    }
    let seen = run(6, PROPOSE, pending, &[("replan.0", replan_key()), ("watchdog.0", watchdog_key())]);
    assert_eq!(seen.looks, 5, "six looks, a sleep between each");
    assert_eq!(seen.cursors, vec![0, 110, 200, 300, 400, 500], "each look continues after the newest event seen");
    assert_eq!(seen.shared["gap.0"]["proposal"]["deadline"], json!({"days": 30, "ticks": 2220, "verdict": "keep"}),
               "the policy as the server states it, the tick taken out");

    // One proposal, through the one atomic channel, with exactly the applicable levers.
    assert_eq!(seen.proposals.len(), 1, "{:?}", seen.proposals);
    let p = &seen.proposals[0];
    assert_eq!(p["company"], json!(0));
    assert_eq!(p["settings"], json!({"air_grow_money": 2500, "air_routes_max": 15, "invest_borrow": 1}));
    assert_eq!(p["expires_tick"], json!(9000 + 30 * 74), "the game's tick (openttd://approvals) plus EXPIRE_DAYS");
    assert_eq!(seen.approvals_read, 1, "the deadline policy is read when filing, never assumed");

    let g = &seen.shared["gap.0"];
    assert_eq!(g["month"], json!("1953-05"));
    assert_eq!(g["proposal"]["id"], json!(900));
    assert_eq!(g["proposal"]["deadline"]["verdict"], json!("keep"));
    assert_eq!(g["proposal"]["outcome"], json!("not_applied"), "{g}");
    assert_eq!(g["declined"], json!(1));
    assert_eq!(g["held"], json!("the last proposal (1953-03) is within 3 month(s)"));
    assert_eq!(g["level"], json!(3));
    assert_eq!(g["streaks"], json!({"idle_capital": 5, "losing_ground": 5, "unused_credit": 5}));
    assert_eq!(g["leader"], json!({"company": 1, "name": "Leader", "value": 1_200_000}));
    assert_eq!(g["months"], json!(6), "the newest run only, WINDOW months");
    assert_eq!(g["reads"].as_array().unwrap().len(), 7);
    assert_eq!(g["thresholds"], json!({"share_drop": 0.05, "source": "SHARE_DROP (cold start)"}));
    assert_eq!(g["proposing"], json!(true));

    // The first month's tally and its collateral damage, from the bot's own lines.
    let said = &seen.said;
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[0].starts_with("\"[gap-watch] Owner Co: idle_capital (2 mo): ") && said[0].contains("losing_ground (2 mo): 43 % of Leader's value (down 10 points)"), "{}", said[0]);
    assert!(said[1].starts_with("\"[gap-watch] Owner Co: 3 setting(s) proposed as one approval #900 for the owner at the game's console (settled at its deadline as 'keep'): invest_borrow 0 -> 1; air_grow_money 0 -> 2500; air_routes_max 2 -> 15"), "{}", said[1]);
    assert_eq!(seen.news.len(), 2, "every owner line also goes to the game window");
    assert!(seen.news[0].starts_with("[gap-watch] Owner Co: "));
    assert_eq!(seen.shared["gap_watch.owner"]["service"], json!("gap_watch"));
}

/// The first judged month: every tally of the bot's lines, the partial plan, the flags with their
/// reasons, and level 1 (nothing said yet).
#[test]
fn gap_watch_first_month_tallies_the_bots_lines_and_finds_collateral() {
    let seen = run(2, PROPOSE, |_| json!([]), &[("derived.gap_watch.share_drop", json!({"p75": 0.0118, "p90": 0.0428})),
                                                ("replan.0", replan_key()), ("watchdog.0", json!({"stalled": true, "quiet_days": 70, "threshold_days": 60, "reasons": ["money 9000 is below the line budget 30000"]}))]);
    let g = &seen.shared["gap.0"];
    assert_eq!(g["month"], json!("1953-01"));
    assert_eq!(g["thresholds"], json!({"share_drop": 0.0118, "source": "derived.gap_watch.share_drop"}),
               "the derived threshold wins over the cold-start constant");
    assert_eq!(g["builds"], json!({"claimed_by_others": 19, "executed": [], "failed": 0, "filed": [7, 8], "frozen": 0,
                                   "no_route": 0, "refused": 1, "removed": 1, "repaid": 20000, "wasted_levelling": 8735}));
    assert_eq!(g["plans"], json!({"executed": 1, "filed": 2, "partial": ["1953-01: 1 of 2"], "partial_months": ["1953-01"]}));
    assert_eq!(g["partial_seen"], json!(["1953-01"]), "a partial plan is reported once");
    let flags = g["flags"].as_object().unwrap();
    assert_eq!(flags.keys().cloned().collect::<Vec<_>>(),
               vec!["collateral", "idle_capital", "losing_ground", "stalled", "unused_credit"]);
    assert_eq!(flags["collateral"], json!("8735 spent levelling sites that were still refused; 1 build(s) removed again; plans applied in part: 1953-01: 1 of 2"));
    assert_eq!(flags["stalled"], json!("watchdog: nothing built for 70 day(s): money 9000 is below the line budget 30000"));
    assert!(flags["unused_credit"].as_str().unwrap().contains("while Leader borrows 290000"), "{}", flags["unused_credit"]);
    assert_eq!(g["level"], json!(1));
    assert_eq!(g["levers"], json!([]));
    assert_eq!(g["held"], json!("no lever applies"));
    assert!(seen.said.is_empty() && seen.proposals.is_empty());
}

/// Observation only without the owner's switch: the levers are recorded, nothing is filed. Paused:
/// nothing is said either. Stopped: the loop ends before its first look.
#[test]
fn gap_watch_observes_without_the_switch_is_silent_when_paused_and_stops_on_its_key() {
    let seen = run(4, &[], |_| json!([]), &[("replan.0", replan_key()), ("watchdog.0", watchdog_key())]);
    assert!(seen.proposals.is_empty());
    let g = &seen.shared["gap.0"];
    assert_eq!(g["levers"].as_array().unwrap().len(), 3, "{g}");
    assert!(g["held"].as_str().unwrap().starts_with("observation only: proposals need GAP_WATCH_PROPOSE=1"), "{g}");
    assert_eq!(seen.said.len(), 2, "the owner is still told: {:?}", seen.said);

    let paused = run(4, PROPOSE, |_| json!([]), &[("replan.0", replan_key()), ("watchdog.0", watchdog_key()), ("gap_watch.pause", json!(true))]);
    assert!(paused.proposals.is_empty() && paused.said.is_empty() && paused.news.is_empty());
    assert_eq!(paused.shared["gap.0"]["held"], json!("paused (gap_watch.pause)"));
    assert_eq!(paused.shared["gap.0"]["paused"], json!(true));

    let stopped = run(4, PROPOSE, |_| json!([]), &[("gap_watch.stop", json!(true))]);
    assert_eq!((stopped.looks, stopped.cursors.len()), (0, 0));
}

/// Without replan's key the month is unknown (and replan silent), never "on track"; the store's
/// rows of the leader are not even asked for.
#[test]
fn gap_watch_without_inputs_reports_unknown() {
    let seen = run(2, PROPOSE, |_| json!([]), &[]);
    let g = &seen.shared["gap.0"];
    let flags = g["flags"].as_object().unwrap();
    assert!(flags.contains_key("unknown") && flags.contains_key("replan_silent"), "{g}");
    assert_eq!(flags["unknown"], json!("inputs missing: 6 month(s) of our samples, 0 of the leader's, replan never"));
    assert_eq!(g["leader"], Value::Null);
}

/// The pure parts: lever conditions (sites, from_zero, declared bounds, a type we lead), the hold
/// rules (another set_settings, a set_setting on a lever, the same proposal declined twice, another
/// planner), the outcome of a proposal, and the text parsers.
#[test]
fn gap_watch_levers_holds_and_outcomes_are_pure() {
    let v = functions_then(r#"
        let bot = #{ settings: [
            #{ name: "invest_borrow", value: 0, min: 0, max: 1, "default": 0, editable: true },
            #{ name: "air_grow_money", value: 250, min: 0, max: 10000, "default": 2500, editable: true },
            #{ name: "air_routes_max", value: 2, min: 0, max: 10, "default": 15, editable: true },
            #{ name: "max_routes", value: 6, min: 1, max: 30, "default": 4, editable: true },
            #{ name: "invest_road_cap", value: 1, min: 1, max: 30, "default": 12, editable: false } ] };
        let comp = [ #{ type: "aircraft", we_lead: false, ours_per_vehicle: 100.0 },
                     #{ type: "road", we_lead: false, ours_per_vehicle: 50.0 } ];
        let streaks = #{ losing_ground: 3, unused_credit: 2 };
        let quiet = #{ wasted_levelling: 0 };
        let a = levers_of(bot, streaks, comp, quiet);
        let b = levers_of(bot, streaks, comp, #{ wasted_levelling: 10 });
        let c = levers_of(bot, streaks, [#{ type: "aircraft", we_lead: false, ours_per_vehicle: 0.0 }], quiet);
        let d = levers_of(bot, #{ unused_credit: 3 }, comp, quiet);
        let lv = [#{ name: "air_routes_max", to: 10 }];
        let p = #{ month: "1953-01", signature: signature_of(lv), id: 5, settings: #{ air_routes_max: 10 } };
        let refusal = #{ why: "planner.0 is held by battlebrain" };
        [a.map(|l| `${l.name}=${l.to}`), b.len(), c.len(), d.map(|l| l.name),
         hold_of(0, "1953-09", lv, [#{ id: 4, tool: "openttd.ai.set_setting", args: #{ company: 0, name: "air_routes_max" } }], (), false, (), true, 0),
         hold_of(0, "1953-09", lv, [#{ id: 4, tool: "openttd.ai.set_setting", args: #{ company: 1, name: "air_routes_max" } }], (), false, (), true, 0),
         hold_of(0, "1953-09", lv, [], p, false, (), true, 2),
         hold_of(0, "1953-09", lv, [], p, false, refusal, true, 0),
         hold_of(0, "1953-09", lv, [], (), false, (), true, 0),
         outcome_of(p, bot, [5]), outcome_of(p, bot, []), outcome_of(#{ id: (), settings: #{ air_routes_max: 2 } }, bot, []),
         level_of(#{}, []), level_of(#{ a: 1 }, []), level_of(#{ a: 2 }, []), level_of(#{}, lv),
         int_after("no digits here", "here"), int_before("x claimed", " claimed"), int_after("abc", "zz"),
         month_of("1953"), pct(0.555), rounds(), proposing(), id_of(2.0),
         type_of(tell_of("Co", #{ a: "x" }, #{ a: 2 }, 2, #{ said: ["a"], level: 2 })),
         type_of(tell_of("Co", #{ a: "x" }, #{ a: 1 }, 1, #{})),
         series_of([[0, 0, 7, 0, "1950-01-01", 1, 2, 3, 0, 4]], 6).len(),
         tally_event(#{}, #{ kind: "ai.log", data: #{ company: 0, text: "PrismaticAI: x" } }, "").len(),
         tally_event(#{}, #{ kind: "panel.mcp", data: #{ text: "openttd.ai.set_setting({}) -> pending approval #3" } }, "1953-01").len(),
         tally_event(#{}, #{ kind: "other.kind", game_date: "1953-01-01", data: #{} }, "").len(),
         tally_event(#{}, #{ kind: "ai.log", game_date: "1953-02-03", data: #{ company: 0, text: "PrismaticAI: x" } }, "1953-01")["0"].keys(),
         month_text(()), month_text("1953-04-01"),
         thresholds_of(0.02).share_drop, thresholds_of(1).source, thresholds_of(#{ value: 0.03 }).share_drop,
         thresholds_of(3.0).source, thresholds_of("x").share_drop, thresholds_of(()).source]
    "#);
    assert_eq!(v, vec![
        "[\"air_routes_max=10\"]", // air_grow_money is positive (a step size), max_routes is above its default,
                                   // invest_road_cap is not editable, invest_borrow's flag has not held long enough;
                                   // air_routes_max is clamped to its declared max
        "0", "0", "[\"invest_borrow\"]",
        "approval #4 (openttd.ai.set_setting air_routes_max) still waits for the owner",
        "", // () prints empty: nothing holds
        "the same proposal went unapplied 2 time(s)",
        "planner.0 is held by battlebrain",
        "",
        "pending", "not_applied", "applied",
        "0", "1", "2", "3",
        "", "", "",
        "0", "56", "0", "false", "2",
        "()", "()",
        "1", "0", "0", "0",
        "[\"1953-02\"]", // an envelope's own date wins over the bus's month
        "", "1953-04",
        "0.02", "derived.gap_watch.share_drop", "0.03",
        "SHARE_DROP (cold start)", "0.05", "SHARE_DROP (cold start)",
    ]);
}

/// The script has no path to approve, and its only game-acting call is the one atomic, owner-only
/// set_settings. That it names no freight-line slot is boundary B2's scan
/// (authority_boundaries.rs), which reads every .rhai file.
#[test]
fn gap_watch_names_no_approval_path() {
    let s = script();
    for bad in ["approve(", "auto_approve", "api_approve", "api_autoApprove", "api_botSetting",
                "api_tool_openttd_ai_set_setting(", "api_botStart", "console("] {
        assert!(!s.contains(bad), "gap_watch.rhai names {bad}");
    }
    assert_eq!(s.matches("api_tool_openttd_ai_set_settings(").count(), 1);
    assert!(s.starts_with("// stall: "), "the supervisor's stall limit is declared on the first line");
}

/// After a restart the bus replays months already judged: they are skipped, and the first new month
/// continues the stored streaks (so a proposal can follow at once). An older server's text-only
/// answer still yields the approval id.
#[test]
fn gap_watch_skips_replayed_months_and_reads_a_text_answer() {
    let prev = json!({"month": "1953-02", "level": 2, "said": ["idle_capital", "losing_ground", "unused_credit"],
                      "partial_seen": ["1953-01"],
                      "streaks": {"idle_capital": 2, "losing_ground": 2, "unused_credit": 2}});
    let seen = run_with(4, PROPOSE, |_| json!([]), &[("replan.0", replan_key()), ("watchdog.0", watchdog_key()), ("gap.0", prev)], true);
    let g = &seen.shared["gap.0"];
    assert_eq!(g["month"], json!("1953-03"), "1953-01 and 1953-02 were judged before the restart");
    assert_eq!(g["streaks"], json!({"idle_capital": 3, "losing_ground": 3, "unused_credit": 3}));
    assert_eq!(seen.proposals.len(), 1);
    assert_eq!(g["proposal"]["id"], json!(900));
    assert_eq!(g["proposal"]["outcome"], json!("pending"));
}
