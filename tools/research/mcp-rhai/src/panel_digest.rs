//! RESEARCH-ONLY. The strategy digest in the panel (research/mechanisms/strategy-digest.md): what the
//! digest service (tools/research/mcp-panel/rhai/digest.rhai) keeps in the panel's shared memory,
//! answered under one route, `GET /api/digest`, for the /digest page, OpenAPI and Rhai
//! (`api_digest()`).
//!
//! The service writes `digest.<company>` (the latest digest), `digest.<company>.<week|month|quarter>`
//! (a row per digest of that cadence) and `digest.status` (its heartbeat, every round). This module
//! only reads them: it names the companies that have a digest, picks one (`?company=`, else the
//! owner's, else the first), joins its histories, lists every active alert across the companies,
//! and says whether the service is alive: a heartbeat older than the stall limit digest.rhai declares
//! on its first line (`// stall: <seconds>`, read through `panel_scripts::stall_of`, the supervisor's
//! own reading) is reported as stale, because a dead digest must not read as a quiet one.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::panel::{Game, Input};
use crate::rhai_shared::{shared, Shared};

/// The service's script in the library, whose `// stall:` line is the heartbeat's limit.
pub const SCRIPT_FILE: &str = "digest.rhai";

/// The stall limit digest.rhai declares, in seconds: the same reading the supervisor makes, so the
/// page calls the service stale exactly when the supervisor would restart it. None when the script
/// is not in the library or declares no limit.
pub fn stale_after_s() -> Option<u64> { stale_after_in(&crate::rhai_host::rhai_dir()) }

/// `stale_after_s` for a library directory.
pub fn stale_after_in(dir: &std::path::Path) -> Option<u64> {
    let text = std::fs::read_to_string(dir.join(SCRIPT_FILE)).ok()?;
    crate::panel_scripts::stall_of(&text).map(|d| d.as_secs())
}
/// The cadences a digest history is kept for, in the order the page shows them.
pub const PERIODS: [&str; 3] = ["week", "month", "quarter"];

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The company id of a key `digest.<company>`, None for every other key (histories, status).
pub fn company_of_key(key: &str) -> Option<i64> {
    key.strip_prefix("digest.")?.parse::<i64>().ok()
}

/// The answer of `GET /api/digest` from a shared memory at wall time `now` (pure but for `store`).
pub fn assemble(store: &Shared, company: Option<i64>, now: u64, stale_after: Option<u64>) -> Value {
    let status = store.get("digest.status");
    let heartbeat = match &status {
        Some(e) => {
            let age = now.saturating_sub(e.at);
            match stale_after {
                Some(limit) => json!({"value": e.value, "at": e.at, "age_s": age, "alive": age <= limit}),
                None => json!({"value": e.value, "at": e.at, "age_s": age, "alive": false,
                               "why": format!("{SCRIPT_FILE} is not in the script library or declares no `// stall:` limit, so its heartbeat cannot be judged")}),
            }
        }
        None => json!({"value": null, "at": null, "age_s": null, "alive": false,
                       "why": "no digest.status: the digest service has not run (POST /api/services/start {\"name\": \"digest\"})"}),
    };
    let mut companies = Vec::new();
    let mut alerts = Vec::new();
    let mut digests = std::collections::BTreeMap::new();
    for (key, e) in store.list("digest.") {
        let Some(c) = company_of_key(&key) else { continue };
        let d = &e.value;
        companies.push(json!({"company": c, "name": d["name"], "level": d["level"], "label": d["label"],
                              "period": d["period"], "date": d["date"], "owner": d["owner"], "written_at": e.at}));
        for a in d["alerts"].as_array().into_iter().flatten() {
            alerts.push(json!({"company": c, "name": d["name"], "key": a["key"], "level": a["level"], "text": a["text"]}));
        }
        digests.insert(c, d.clone());
    }
    let owner = status.as_ref().and_then(|e| e.value["owner"].as_i64());
    let chosen = company.filter(|c| digests.contains_key(c))
        .or_else(|| owner.filter(|c| digests.contains_key(c)))
        .or_else(|| digests.keys().next().copied());
    let history: serde_json::Map<String, Value> = PERIODS.iter().map(|p| {
        let rows = chosen.and_then(|c| store.get(&format!("digest.{c}.{p}"))).map(|e| e.value).unwrap_or_else(|| json!([]));
        (p.to_string(), rows)
    }).collect();
    let missing = company.filter(|c| !digests.contains_key(c));
    json!({
        "company": chosen,
        "requested_missing": missing,
        "companies": companies,
        "digest": chosen.and_then(|c| digests.get(&c).cloned()),
        "history": history,
        "alerts": alerts,
        "heartbeat": heartbeat,
        "stale_after_s": stale_after,
    })
}

/// GET /api/digest[?company=N]: the digest of one company (the requested one, else the owner's,
/// else the first), its histories, every company with a digest and every active alert, and the
/// service's heartbeat. Read-only.
pub fn h_digest(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let company = match i.query.get("company") {
        None | Some("") => None,
        Some(s) => match s.parse::<i64>() {
            Ok(c) => Some(c),
            Err(_) => return Ok((400, json!({"error": format!("company {s:?}: a 0-based company id")}))),
        },
    };
    Ok((200, assemble(shared(), company, now(), stale_after_s())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    fn digest(name: &str, level: &str, alerts: Value, owner: bool) -> Value {
        json!({"name": name, "level": level, "label": "1954-Q1", "period": "quarter", "date": "1954-04-01", "owner": owner, "alerts": alerts})
    }

    #[test]
    fn the_route_joins_the_digests_their_histories_and_the_heartbeat() {
        let s = Shared::default();
        let empty = assemble(&s, None, 1000, Some(120));
        assert_eq!((empty["company"].clone(), empty["digest"].clone(), empty["heartbeat"]["alive"].clone()), (Value::Null, Value::Null, json!(false)));
        assert!(empty["heartbeat"]["why"].as_str().unwrap().contains("has not run"));

        s.set("digest.status", json!({"owner": 2, "round": 7}), "digest", None).unwrap();
        s.set("digest.0", digest("Rival AI", "ok", json!([]), false), "digest", None).unwrap();
        s.set("digest.2", digest("Ours", "alert", json!([{"key": "fleet_gap", "level": "alert", "text": "1 vehicles against X's 140"}]), true), "digest", None).unwrap();
        s.set("digest.2.quarter", json!([{"label": "1954-Q1", "share": 0.3}]), "digest", None).unwrap();
        s.set("digest.2.month", json!([{"label": "1954-03"}, {"label": "1954-02"}]), "digest", None).unwrap();
        s.set("digest.x", json!(1), "digest", None).unwrap();
        let at = s.get("digest.status").unwrap().at;

        let a = assemble(&s, None, at + 5, Some(120));
        assert_eq!(a["company"], json!(2), "the owner's by default");
        assert_eq!(a["digest"]["name"], "Ours");
        assert_eq!(a["companies"].as_array().unwrap().len(), 2, "digest.x and the histories are no company");
        assert_eq!(a["history"]["month"].as_array().unwrap().len(), 2);
        assert_eq!(a["history"]["week"], json!([]));
        assert_eq!(a["alerts"], json!([{"company": 2, "name": "Ours", "key": "fleet_gap", "level": "alert", "text": "1 vehicles against X's 140"}]));
        assert_eq!((a["heartbeat"]["alive"].clone(), a["heartbeat"]["age_s"].clone()), (json!(true), json!(5)));

        let b = assemble(&s, Some(0), at + 121, Some(120));
        assert_eq!((b["company"].clone(), b["digest"]["name"].clone(), b["history"]["quarter"].clone()), (json!(0), json!("Rival AI"), json!([])));
        assert_eq!(b["heartbeat"]["alive"], json!(false), "a heartbeat older than the declared limit is stale");
        let c = assemble(&s, Some(9), at, Some(120));
        let u = assemble(&s, None, at, None);
        assert_eq!(u["heartbeat"]["alive"], json!(false), "no declared limit: never called alive");
        assert!(u["heartbeat"]["why"].as_str().unwrap().contains("declares no `// stall:` limit"));
        // The limit is the script's own first-line declaration, as the supervisor reads it.
        let lib = std::path::Path::new(SCRIPT).parent().unwrap();
        let declared = crate::panel_scripts::stall_of(&std::fs::read_to_string(SCRIPT).unwrap()).unwrap().as_secs();
        assert_eq!(stale_after_in(lib), Some(declared));
        assert_eq!(stale_after_in(&std::env::temp_dir().join("no-such-digest-library")), None);
        assert_eq!((c["company"].clone(), c["requested_missing"].clone()), (json!(2), json!(9)), "a company without a digest falls back, and says so");
        assert_eq!((company_of_key("digest.12"), company_of_key("digest.1.month"), company_of_key("digest.status")), (Some(12), None, None));
    }

    // ------------------------------------------- the service (tools/research/mcp-panel/rhai/digest.rhai)

    const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel/rhai/digest.rhai");

    fn to_dyn(v: Value) -> rhai::Dynamic { rhai::serde::to_dynamic(v).unwrap() }

    /// The script's functions and constants without its main loop, with `extra` evaluated after them.
    fn eval_pure(extra: &str) -> Value {
        let text = std::fs::read_to_string(SCRIPT).unwrap();
        let head = text.split("// ---- main").next().unwrap();
        let mut e = rhai::Engine::new();
        e.set_max_expr_depths(256, 256);
        e.register_fn("env", |_: &str| String::new());
        let out: rhai::Dynamic = e.eval(&format!("{head}\n{extra}")).unwrap_or_else(|err| panic!("{err}"));
        rhai::serde::from_dynamic(&out).unwrap()
    }

    #[test]
    fn cadence_standing_series_and_approvals_are_pure() {
        let v = eval_pure(r#"
            let a = period_of("1954-01-10", 74 * 1470);
            let b = period_of("1954-01-18", 74 * 1478);
            let c = period_of("1954-02-01", 74 * 1492);
            let q = period_of("1954-04-01", 74 * 1551);
            let back = period_of("1950-01-05", 74 * 4);
            [crossed((), a), crossed(a, a), crossed(a, b), crossed(b, c), crossed(c, q), crossed(q, back),
             reported("quarter", c, q).label, reported("month", b, c).label, reported("first", (), a).period,
             months_between("1953-10-27", "1954-02-10"), quarter_of_month("1954-12")]
        "#);
        assert_eq!(v, json!(["first", null, "week", "month", "quarter", "rollback", "1954-Q1", "1954-01", "start", 4, "1954-Q4"]));

        let s = eval_pure(r#"
            let cs = [#{ id: 0, name: "Ours", company_value: 300, fleet: #{ total: 1 } },
                      #{ id: 1, name: "Big", company_value: 1000, fleet: #{ total: 140 } },
                      #{ id: 2, name: "Small", company_value: 100, fleet: #{ total: 5 } }];
            let st = standing(cs, 0);
            let lead = standing(cs, 1);
            let rows = #{ "0": [["1953-12", 400.0], ["1953-09", 500.0]], "1": [["1953-12", 1000.0], ["1953-09", 1000.0]], "2": [["1953-12", 100.0]] };
            let ser = series_of(rows, #{ "0": "Ours", "1": "Big", "2": "Small" }, 0);
            [st.rank, st.of, st.leader, st.share, st.largest_fleet_name, st.fleet_share > 0.007 && st.fleet_share < 0.008,
             lead.we_lead, lead.share, lead.runner_up, lead.lead_margin > 3.3, type_of(standing(cs, 7)),
             ser.len(), ser[0].month, ser[0].share, ser[1].of, ser[1].leader,
             quarters_of(ser, "1954-Q1").len(), quarters_of(ser, "1953-Q4").len(), pct(0.555), short(1234567), short(-45000)]
        "#);
        assert_eq!(s, json!([2, 3, "Big", 0.3, "Big", true, true, 1.0, "Ours", true, "()",
                             2, "1953-09", 0.5, 3, "Big", 2, 1, "56%", "1.2M", "-45k"]));

        let a = eval_pure(r#"
            let doc = #{ deadline: #{ verdict: "keep", days: 30 }, approvals: [
                #{ id: 5, state: "pending", tool: "openttd.ai.set_setting", args: #{ company: 0, name: "fr_from", value: 3 }, created_tick: 0, deadline_tick: 740 },
                #{ id: 6, state: "pending", tool: "openttd.ai.set_setting", args: #{ company: 0, name: "rail_a", value: 9 }, created_tick: 74, deadline_tick: 296, verdict: "approve" },
                #{ id: 7, state: "pending", tool: "openttd.ai.set_setting", args: #{ company: 1, name: "x", value: 1 }, created_tick: 0, deadline_tick: 100 },
                #{ id: 8, state: "denied", settled_by: "deadline keep", args: #{ company: 0 }, created_tick: 150 },
                #{ id: 9, state: "approved", args: #{ company: 0 }, created_tick: 10 },
                #{ id: 10, state: "pending", tool: "openttd.game.set_pause", args: #{}, created_tick: 0 }] };
            let r = approvals_of(doc, 0, 148, 100);
            [r.pending, r.items[0].id, r.items[0].countdown_days, r.items[0].verdict, r.items[1].verdict, r.items[1].what,
             r.soonest_days, r.oldest_days, r.since.denied, r.since.by_deadline, r.since.approved]
        "#);
        assert_eq!(a, json!([2, 6, 2, "approve", "keep", "ai.set_setting fr_from=3", 2, 2, 1, 1, 0]),
                   "each item's own verdict, else the queue's; the soonest countdown first; only this company's");
    }

    /// Escalation follows the data: grace before two complete quarters, hysteresis on far_behind,
    /// approvals without a deadline, idle cash, the levels and the chat rule.
    #[test]
    fn escalation_has_grace_hysteresis_and_unknown() {
        let v = eval_pure(r#"
            let st = #{ rank: 3, of: 3, share: 0.2, leader: "Big", fleet_share: 0.5, vehicles: 5, largest_fleet: 10, largest_fleet_name: "Big" };
            let money = #{ cash: 0.0, loan: 0.0, loan_step: 10000.0 };
            let appr = #{ pending: 0, items: [], soonest_days: (), oldest_days: 0 };
            let ser = [#{ month: "1953-11", share: 0.2 }, #{ month: "1953-12", share: 0.2 }];
            let d = #{ standing: st, quarters: [#{ share: 0.2 }], series: ser, approvals: appr };
            let early = alerts_of(d, [], (), money, #{}, 0).map(|a| a.key);
            d.quarters = [#{ share: 0.2 }, #{ share: 0.2 }];
            let late = alerts_of(d, [], (), money, #{}, 0).map(|a| a.key);
            d.series = [#{ month: "1953-11", share: 0.28 }, #{ month: "1953-12", share: 0.28 }];
            d.standing.share = 0.28;
            let held = alerts_of(d, ["far_behind"], (), money, #{}, 0).map(|a| a.key);
            let released = alerts_of(d, [], (), money, #{}, 0).map(|a| a.key);
            d.approvals = #{ pending: 2, items: [#{ verdict: () }], soonest_days: (), oldest_days: 45 };
            let waiting = alerts_of(d, [], (), money, #{}, 0).filter(|a| a.key == "approvals_waiting").len();
            let wd = #{ stalled: false, quiet_days: 90 };
            let idle = alerts_of(d, [], wd, #{ cash: 80000.0, loan: 0.0, loan_step: 10000.0 }, #{}, 30000).filter(|a| a.key == "idle_cash").len();
            let dd = #{ period: "week", level: "watch", standing: #{ rank: 2 }, new_alerts: [], alerts: [], cleared: [] };
            let pd = #{ level: "watch", standing: #{ rank: 2 } };
            [early, late, held, released, waiting, idle,
             level_of([], []), level_of([], ["series"]), level_of([#{ level: "watch" }], ["series"]),
             should_say(dd, pd, 1000, 1000, 0), should_say(dd, #{ level: "ok", standing: #{ rank: 2 } }, 1000, 1000, 0),
             should_say(dd, #{ level: "ok", standing: #{ rank: 2 } }, 10, 10, 300),
             should_say(#{ period: "month", new_alerts: ["x"], alerts: [], cleared: [] }, pd, 0, 0, 300)]
        "#);
        assert_eq!(v, json!([[], ["far_behind", "last_place"], ["far_behind", "last_place"], ["behind", "last_place"],
                             1, 1, "ok", "unknown", "watch", false, true, false, true]));
    }

    /// A signal key nobody wrote (gap.<company>, outcome.<company>) is unknown in the digest, never
    /// silently "ok": the keys the digest reads are the ones gap_watch.rhai and panel_bot.rs write.
    #[test]
    fn a_missing_signal_key_is_unknown() {
        let v = eval_pure(r#"
            let input = #{ company: 0, companies: [#{ id: 0, name: "Ours", company_value: 10, money: 0, loan: 0, fleet: #{ total: 1 } }],
                           now: period_of("1954-01-10", 74 * 1470), report: #{ period: "start", label: "1954-01-10" }, prev: (),
                           series: [], replan: (), watchdog: (), signals: #{}, services: [],
                           approvals: #{ pending: 0, items: [], soonest_days: (), oldest_days: 0, since: #{ by_deadline: 0 } }, line_budget: 0 };
            let d = compose(input);
            input.signals = #{ gap: #{ flags: #{} }, outcome: #{ verdict: "consistent", signals: [] } };
            let e = compose(input);
            say("no chat here");
            [d.unknown, d.level, e.unknown, SIGNALS, line_of(d), type_of(row_of(d).profit)]
        "#);
        assert_eq!(v, json!([["by_type", "series", "watchdog", "gap", "outcome", "capital", "eval"], "unknown", ["by_type", "series", "watchdog", "capital", "eval"], ["gap", "outcome", "capital"],
                             "1954-01-10: rank 1/1, leading nobody, 1 vs 0 vehicles; unknown", "()"]),
                   "without replan the line and the history row still form (no profit); without con_say the round goes on");
    }

    /// The service evaluation as service_eval keeps it: poor entries in either form, coverage under
    /// either field name, and only a service with enough episodes and too few reactions alerts.
    #[test]
    fn the_service_evaluation_is_read_with_its_caveat() {
        let v = eval_pure(r#"
            let ev = evaluation_of(["steward.close", #{ key: "replan.propose", streak: 3 }, #{ service: "rail", action: "add", verdict: "unclear" }],
                                   #{ services: #{ replan: #{ deterioration_episodes: 40, reacted_to: 2 }, steward: #{ episodes: 40, reacted: 30 }, x: 1 } });
            let a = evaluation_alerts(ev);
            [type_of(evaluation_of((), ())), ev.poor.map(|x| x.entry), ev.coverage.len(), a.map(|x| x.key), a[1].text, a[3].text,
             evaluation_alerts(()).len()]
        "#);
        assert_eq!(v, json!(["()", ["steward.close", "replan.propose", "rail.add"], 2,
                             ["eval.poor.steward.close", "eval.poor.replan.propose", "eval.poor.rail.add", "eval.coverage.replan"],
                             "replan.propose is persistently poor (streak 3); associative, not causal",
                             "replan reacted to 2 of 40 deterioration episodes (5.0%)", 0]));
    }

    /// A governance change is a line naming who and from where, and an alert when it turns
    /// auto-approve on; only the period's changes count.
    #[test]
    fn governance_changes_are_named_and_escalate() {
        let v = eval_pure(r#"
            let on = #{ setting: "auto_approve", from: false, to: true, via: "--auto-approve at start", credential: "none", game_tick: 500 };
            let off = #{ setting: "auto_approve", from: true, to: false, via: "POST /api/auto-approve", origin: "", game_tick: 100 };
            let d = #{ standing: (), quarters: [], series: [], approvals: #{ pending: 0 }, governance: [off] };
            let money = #{ cash: 0.0, loan: 0.0, loan_step: 1.0 };
            let watch = alerts_of(d, [], (), money, #{}, 0);
            d.governance = [off, on];
            let grave = alerts_of(d, [], (), money, #{}, 0);
            [governance_line(on), governance_line(#{ kind: "governance.changed" }), watch[0].level, grave[0].level, grave[0].text,
             governance_since([off, on, #{ setting: "x" }], 200).len(), governance_grave(off), governance_grave(on)]
        "#);
        assert_eq!(v, json!(["auto_approve false -> true via --auto-approve at start (credential none)", "governance.changed ? -> ? via ?",
                             "watch", "alert", "2 governance change(s): auto_approve false -> true via --auto-approve at start (credential none)",
                             2, false, true]));
    }

    /// The service over stubbed host reads, six rounds: a first digest that alerts at once (share
    /// falling, fleet gap, stall), a round inside the same week (nothing), a quiet week (kept, not
    /// said), a month (said), a quarter (said, news, mesh) and a stopped bot (said once).
    #[test]
    fn the_service_digests_on_the_cadence_and_escalates() {
        let round = Arc::new(Mutex::new(0usize));
        let shared: Arc<Mutex<BTreeMap<String, Value>>> = Arc::new(Mutex::new(BTreeMap::from([
            ("watchdog.0".to_string(), json!({"stalled": true, "quiet_days": 250, "threshold_days": 100, "reasons": ["the bot itself chose to build nothing"]})),
            ("replan.0".to_string(), json!({"date": "1954-03-20", "competition": [{"type": "train", "leader": "Big", "leader_per_vehicle": 30000.0, "ours_per_vehicle": 900.0, "ours_vehicles": 1, "we_lead": false}],
                "finances": {"quarters": [{"income": 50000, "expenses": -20000}, {"income": 40000, "expenses": -20000}]},
                "model": {"adopt": false, "why": "no line in the record pays"}, "proposed": ["fr_from"], "portfolio": ["rail A -> B"], "held": null, "refused": []})),
            // gap.<company> as gap_watch.rhai writes it, outcome.<company> as panel_bot.rs writes it.
            ("gap.0".to_string(), json!({"month": "1954-01", "level": 2, "idle_cash": 60000.0, "unused_credit": 0.0,
                "flags": {"idle_capital": "60000 cash above the reserve is 2x the line budget", "losing_ground": "30 % of Big's value"},
                "streaks": {"idle_capital": 2, "losing_ground": 1}, "builds": {"refused": 3, "failed": 1, "removed": 0}, "held": "no lever applies"})),
            ("outcome.0".to_string(), json!({"company": 0, "verdict": "mismatch", "why": "bus earns 6 % of its prediction",
                "signals": [{"kind": "overpredicted_mode_planned", "severity": "mismatch", "why": "bus prior 0.16 on 6 lines"},
                            {"kind": "leader_out_earns", "severity": "watch", "why": "Big earns 1.5x ours per aircraft"}],
                "game": {"seed": 1, "date": "1954-01-01", "tick": 0}})),
            // capital.<company> as capital_watch.rhai writes it.
            ("capital.0".to_string(), json!({"company": 0, "period": "1954-01", "flags": {"under_reinvesting": "reinvested 12 % of its operating cash flow"},
                "trailing": {"reinvestment": 0.12}, "idle_ratio": 1.5, "vehicles_growth": 0.0})),
            // eval.poor and eval.coverage as service_eval writes them.
            ("eval.poor".to_string(), json!([{"service": "replan", "action": "propose", "streak": 4, "verdict": "poor"}])),
            ("eval.coverage".to_string(), json!({"replan": {"episodes": 255, "reacted": 7}, "watchdog": {"episodes": 10, "reacted": 0}})),
        ])));
        let said = Arc::new(Mutex::new(Vec::<String>::new()));
        let news = Arc::new(Mutex::new(Vec::<String>::new()));
        let mesh = Arc::new(Mutex::new(Vec::<String>::new()));
        let printed = Arc::new(Mutex::new(Vec::<String>::new()));
        // (date, day) of each round.
        const DATES: [(&str, i64); 6] = [("1954-01-10", 1470), ("1954-01-11", 1471), ("1954-01-18", 1478),
                                          ("1954-02-01", 1492), ("1954-04-01", 1551), ("1954-04-20", 1570)];
        type R = Result<rhai::Dynamic, Box<rhai::EvalAltResult>>;
        let mut e = rhai::Engine::new();
        e.set_max_expr_depths(256, 256);
        e.register_fn("env", |n: &str| -> String { match n { "DIGEST_ROUNDS" => "6".into(), "DIGEST_COOLDOWN_S" => "0".into(), _ => String::new() } });
        let r = round.clone();
        e.register_fn("sleep", move |_ms: i64| { *r.lock().unwrap() += 1; });
        let r = round.clone();
        e.register_fn("read", move |uri: &str| -> R {
            let (date, day) = DATES[*r.lock().unwrap()];
            Ok(to_dyn(match uri {
                "openttd://game/state" => json!({"game_tick": day * 74, "game_date": date}),
                "openttd://companies" => json!({"data": {"companies": [
                    {"id": 0, "name": "Ours", "company_value": 300, "money": 90000, "loan": 300000, "max_loan": 300000, "loan_step": 10000, "fleet": {"total": 1, "train": 1}},
                    {"id": 1, "name": "Big", "company_value": 1000, "money": 5000, "loan": 0, "fleet": {"total": 140, "train": 140}},
                    {"id": 2, "name": "Small", "company_value": 100, "money": 5000, "loan": 0, "fleet": {"total": 5}}]}}),
                "openttd://approvals" => json!({"data": {"deadline": {"verdict": "keep", "days": 30}, "approvals": [
                    {"id": 5, "state": "pending", "tool": "openttd.ai.set_setting", "args": {"company": 0, "name": "fr_from", "value": 3}, "created_tick": 74 * 1460, "deadline_tick": 74 * 1475}]}}),
                other => return Err(format!("unexpected read {other}").into()),
            }))
        });
        let r = round.clone();
        e.register_fn("api_bots", move || -> rhai::Dynamic {
            // The bot stops in the last round.
            to_dyn(if *r.lock().unwrap() < 5 { json!({"running": [{"id": 0, "ai": "PrismaticAI", "settings": [{"name": "line_budget", "value": 30000}]}, {"id": 1, "ai": "AAAHogEx"}]}) }
                   else { json!({"running": [{"id": 1, "ai": "AAAHogEx"}]}) })
        });
        e.register_fn("api_services", || -> rhai::Dynamic { to_dyn(json!({"services": [{"name": "replan", "stalled": false}, {"name": "arena", "gave_up": true}]})) });
        let r = round.clone();
        e.register_fn("api_dbNamedRun", move |m: rhai::Map| -> rhai::Dynamic {
            // Each company's months up to the round's month, newest first, as company_trend has them.
            let (date, _) = DATES[*r.lock().unwrap()];
            let c = m["params"].clone_cast::<rhai::Map>()["company"].as_int().unwrap() as usize;
            let months = [("1953-06", [600, 1000, 100]), ("1953-09", [500, 1000, 100]), ("1953-12", [400, 1000, 100]),
                          ("1954-01", [350, 1000, 100]), ("1954-02", [320, 1000, 100]), ("1954-03", [300, 1000, 100])];
            let rows: Vec<Value> = months.iter().rev().filter(|(mo, _)| *mo <= &date[..7]).map(|(mo, v)| json!([mo, v[c] as f64, null, 0, 0, 1])).collect();
            to_dyn(json!({"rows": rows}))
        });
        let s = shared.clone();
        e.register_fn("shared_get", move |k: &str| -> rhai::Dynamic { s.lock().unwrap().get(k).cloned().map_or(rhai::Dynamic::UNIT, to_dyn) });
        let s = shared.clone();
        e.register_fn("shared_set", move |k: &str, v: rhai::Dynamic| -> i64 { s.lock().unwrap().insert(k.to_string(), rhai::serde::from_dynamic(&v).unwrap()); 1 });
        let t = said.clone();
        e.register_fn("con_say", move |text: &str| { t.lock().unwrap().push(text.to_string()); });
        let t = news.clone();
        e.register_fn("api_newsShow", move |m: rhai::Map| -> rhai::Dynamic { t.lock().unwrap().push(m["text"].to_string()); rhai::Dynamic::UNIT });
        let t = mesh.clone();
        e.register_fn("mesh_record_write", move |m: rhai::Map| -> rhai::Dynamic {
            assert_eq!(m["kind"].to_string(), "observation");
            t.lock().unwrap().push(format!("{} | {}", m["task"], m["title"]));
            rhai::Dynamic::UNIT
        });
        let r = round.clone();
        e.register_fn("events", move |since: i64, kinds: &str| -> rhai::Dynamic {
            // One governance change, in the round the bot stops: auto-approve turned on through the API.
            assert_eq!(kinds, "governance");
            let ev = if *r.lock().unwrap() == 5 && since < 7 { json!([{"seq": 7, "kind": "governance.changed", "data": {"setting": "auto_approve", "from": false, "to": true,
                "via": "POST /api/auto-approve", "credential": "none", "origin": "http://evil.example", "user_agent": "curl/8", "peer": "127.0.0.1", "game_date": "1954-04-20", "game_tick": 1570 * 74}}]) } else { json!([]) };
            to_dyn(json!({"events": ev, "last_seq": if since >= 7 || *r.lock().unwrap() == 5 { 7 } else { since }}))
        });
        e.register_fn("to_json", |v: rhai::Dynamic| -> String { serde_json::to_string(&rhai::serde::from_dynamic::<Value>(&v).unwrap()).unwrap() });
        let t = printed.clone();
        e.on_print(move |s| t.lock().unwrap().push(s.to_string()));
        e.run(&std::fs::read_to_string(SCRIPT).unwrap()).unwrap();

        let printed = printed.lock().unwrap();
        assert!(!printed.iter().any(|p| p.starts_with("digest: ") && !p.contains("a look every")), "no round failed: {printed:?}");
        let shared = shared.lock().unwrap();
        let d = &shared["digest.0"];
        assert_eq!((d["period"].clone(), d["label"].clone(), d["level"].clone(), d["owner"].clone()), (json!("quarter"), json!("1954-Q1"), json!("alert"), json!(true)));
        let keys: Vec<&str> = d["alerts"].as_array().unwrap().iter().map(|a| a["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["share_falling", "behind", "fleet_gap", "stalled", "approvals_due", "gap.idle_capital", "gap.losing_ground",
                          "capital.under_reinvesting", "outcome.overpredicted_mode_planned", "outcome.leader_out_earns", "eval.poor.replan.propose", "eval.coverage.replan"], "{d}");
        assert_eq!(d["alerts"][11]["text"], "replan reacted to 7 of 255 deterioration episodes (2.7%)");
        assert!(d["evaluation"]["caveat"].as_str().unwrap().starts_with("associative, not causal"));
        let level = |k: &str| d["alerts"].as_array().unwrap().iter().find(|a| a["key"] == k).unwrap()["level"].clone();
        assert_eq!((level("gap.idle_capital"), level("gap.losing_ground"), level("outcome.overpredicted_mode_planned"), level("outcome.leader_out_earns")),
                   (json!("alert"), json!("watch"), json!("alert"), json!("watch")), "a gap flag held two months, and a mismatch, are alerts");
        assert_eq!(d["unknown"], json!([]), "every source was there");
        assert_eq!((d["money"]["idle_cash"].clone(), d["money"]["unused_credit"].clone()), (json!(60000.0), json!(0.0)));
        assert_eq!((d["money"]["reinvestment"].clone(), d["money"]["idle_ratio"].clone()), (json!(0.12), json!(1.5)));
        assert!(d["failed"].as_array().unwrap().iter().any(|f| f == "gap watch 1954-01: 3 build(s) refused, 1 failed, 0 removed"), "{}", d["failed"]);
        assert!(d["failed"].as_array().unwrap().iter().any(|f| f == "planner outcome mismatch: bus earns 6 % of its prediction"), "{}", d["failed"]);
        assert!(d["alerts"][0]["text"].as_str().unwrap().ends_with("50% -> 40% -> 30%"), "{}", d["alerts"][0]);
        assert_eq!(d["quarters"].as_array().unwrap().len(), 4);
        assert_eq!(d["approvals"]["items"][0]["verdict"], "keep", "the queue's verdict, read from the record");
        assert_eq!(d["profit"]["quarter"], json!(30000.0));
        assert_eq!(d["by_type_source"], "replan");
        assert!(d["failed"].as_array().unwrap().iter().any(|f| f == "service arena was given up"), "{}", d["failed"]);
        assert!(d["failed"].as_array().unwrap().iter().any(|f| f == "replan did not switch: no line in the record pays"));
        assert_eq!(shared["digest.0.week"].as_array().unwrap().len(), 1, "the quiet week is kept");
        assert_eq!(shared["digest.0.month"].as_array().unwrap().len(), 1);
        assert_eq!(shared["digest.0.quarter"][0]["label"], "1954-Q1");
        assert!(!shared.contains_key("digest.1"), "only the AI's companies get a digest");
        assert_eq!((shared["digest.status"]["stopped"].clone(), shared["digest.status"]["said_stopped"].clone()), (json!([0]), json!([0])));

        let said = said.lock().unwrap();
        assert_eq!(said.len(), 5, "start (new alerts), month, quarter, governance, bot stopped; not the quiet week: {said:?}");
        assert!(said[0].starts_with("\"[digest] 1954-01-10: rank 2/3, 30% of Big, 1 vs 140 vehicles, op. profit 30k/q, 1 approval(s): keep in"), "{}", said[0]);
        assert!(said[0].contains("; ALERT share_falling fleet_gap stalled"), "{}", said[0]);
        assert!(said[1].starts_with("\"[digest] 1954-01: "), "{}", said[1]);
        assert!(said[2].starts_with("\"[digest] 1954-Q1: "), "{}", said[2]);
        assert!(said[3].contains("ALERT bot_stopped: PrismaticAI company 1 no longer runs"), "{}", said[3]);
        assert_eq!(said[4], "\"[digest] ALERT governance: auto_approve false -> true via POST /api/auto-approve on 1954-04-20 (credential none, Origin http://evil.example, UA curl/8, peer 127.0.0.1)\"");
        assert_eq!(shared["digest.governance"].as_array().unwrap().len(), 1);
        assert_eq!(shared["digest.status"]["gov_seq"], json!(7));
        let news = news.lock().unwrap();
        assert_eq!(news.len(), 4, "{news:?}");
        assert!(news[2].starts_with("Strategy alert: PrismaticAI company 1 stopped running"), "{}", news[2]);
        assert!(news[3].starts_with("Strategy alert: auto_approve false -> true"), "{}", news[3]);
        assert!(news[0].starts_with("Strategy alert: share of the leader fell"), "{}", news[0]);
        assert!(news[1].starts_with("Strategy digest 1954-Q1: "), "{}", news[1]);
        let mesh = mesh.lock().unwrap();
        assert_eq!(mesh.len(), 4, "{mesh:?}");
        assert!(mesh[3].starts_with("openttd:digest:governance | ALERT governance: auto_approve false -> true"), "{}", mesh[3]);
        assert_eq!(mesh[0], "openttd:digest:0 | ALERT company 1 1954-01-10: alert");
        assert_eq!(mesh[1], "openttd:digest:0 | digest company 1 1954-Q1: alert");
    }
}
