//! RESEARCH-ONLY: `battlebrain`, the home of the single strategic authority of the freight-line
//! decision (I0095, research/battlebrain/README.md). Phase 02 holds only its contracts; it decides
//! nothing and reaches no game.
//!
//!   battlebrain schema <record|invocation|decision>   print a contract's JSON Schema (generated)
//!   battlebrain validate <file>...                    check records, invocations or decision blocks;
//!                                                     exit 1 with structured refusals on any failure
//!   battlebrain id <file>                             print a record's id (the digest of its body)
//!   battlebrain transition <type> <from> <to>         whether a status transition is legal
//!   battlebrain check <kernel> <input> <output>       a kernel output's domain check against its input
//!   battlebrain snapshot --capture <dir> --instance <name> --company <id> [--max-age-days 31]
//!                                                     the snapshot record of an `mcp-plan --capture`
//!   battlebrain shadow --snapshot <record> --plan <mcp-plan answer> --companies <openttd://companies>
//!                      --company <id> [--margin 0.2] [--reserve 25000] [--allow-unpriced]
//!                                                     policy freight-margin (battlebrain_policy.rs) in SHADOW (I0096): the
//!                                                     planner's candidate, battlebrain's candidate, a
//!                                                     decision and a directive block in mode shadow,
//!                                                     and the comparison; reads files only, never a game
//!   battlebrain live ... [--max-window-ticks n]       the same decision LIVE (I0097): refused unless the
//!                                                     snapshot window fits, the policy requires priced lines
//!                                                     and the companies were read after the snapshot; writes
//!                                                     the directive (planned_freight_only=1 + one fr_* slot)
//!   battlebrain serve --directive <file> --port <p>   answer ONE loopback GET with it, then exit
//!   battlebrain report <result.json>                  the derived summary (.md) of a result record
//!   battlebrain rollback --company <id>               the directive back to the baseline (planned_freight_only=0)
//!   battlebrain catalogue --commit <sha> [--kernel <id> --engine <e>] < kernels.json
//!                                                     the discovered catalogue's digest, from the
//!                                                     runner's own `flow -- kernels` answer
//!
//! A file holds one JSON value or {"value": ..., "catalogue": {"kernels": [...], "commit": ...}}
//! (an invocation is checked against the catalogue it carries; there is no built-in list).

#[path = "../battlebrain_contract.rs"]
mod battlebrain_contract;
#[path = "../battlebrain_policy.rs"]
mod battlebrain_policy;

use battlebrain_contract as bb;
use battlebrain_policy as policy;
use serde_json::{json, Value};
use std::io::Read;

/// The value after `--name` in the arguments.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Print refusals as the structured answer every subcommand gives, and the exit code 1.
fn refused(rs: &[bb::Refusal]) -> i32 {
    println!("{}", json!({"ok": false, "refusals": rs.iter().map(bb::Refusal::to_json).collect::<Vec<_>>()}));
    1
}

fn read_json(path: &str) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("{path}: not JSON: {e}"))
}

fn validate(path: &str) -> Value {
    let f = match read_json(path) {
        Ok(v) => v,
        Err(e) => return json!({"file": path, "ok": false, "refusals": [{"code": "unreadable", "path": "", "message": e}]}),
    };
    let v = f.get("value").unwrap_or(&f);
    let res = match v.get("schema").and_then(Value::as_str) {
        Some(s) if s.starts_with("algoglyph-invocation/") => match f.get("catalogue") {
            None => Err(vec![bb::Refusal::new("no_catalogue", "catalogue", "an invocation is checked against the catalogue the runner reported; none given")]),
            Some(c) => match bb::Catalogue::from_runner(&c["kernels"], c["commit"].as_str().unwrap_or_default()) {
                Ok(cat) => bb::validate_invocation(v, &cat).map(|_| "invocation"),
                Err(r) => Err(vec![r]),
            },
        },
        Some(s) if s.starts_with("battlebrain-decision/") => bb::validate_decision_block(v).map(|_| "decision-block"),
        _ => bb::validate_record(v),
    };
    match res {
        Ok(kind) => json!({"file": path, "ok": true, "kind": kind, "mesh_kind": bb::mesh_kind(kind)}),
        Err(rs) => json!({"file": path, "ok": false, "refusals": rs.iter().map(bb::Refusal::to_json).collect::<Vec<_>>()}),
    }
}


/// Days since 1970-01-01 of a civil date, and back (Howard Hinnant's algorithm): the game calendar
/// is the proleptic Gregorian one.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn add_days(date: &str, days: i64) -> Option<String> {
    let mut it = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    let (y, m, d) = civil_from_days(days_from_civil(y, m, d) + days);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// A snapshot record from a capture `mcp-plan --capture` wrote: every captured file is read back
/// and must still have its digest; the record's sources are those digests, its time the first read.
pub fn snapshot_from_capture(dir: &std::path::Path, instance: &str, company: i64, max_age_days: u64) -> Result<Value, Vec<bb::Refusal>> {
    let refuse = |code: &'static str, msg: String| vec![bb::Refusal::new(code, "", msg)];
    let m: Value = std::fs::read_to_string(dir.join("capture.json")).map_err(|e| refuse("unreadable", format!("capture.json: {e}")))
        .and_then(|t| serde_json::from_str(&t).map_err(|e| refuse("unreadable", format!("capture.json: {e}"))))?;
    if m["schema"] != "mcp-plan-capture/v1" {
        return Err(refuse("unknown_schema", format!("capture.json: schema {} is not mcp-plan-capture/v1", m["schema"])));
    }
    let mut sources = Vec::new();
    for e in m["resources"].as_array().into_iter().flatten().filter(|e| e["ok"] == true) {
        let file = dir.join(e["file"].as_str().unwrap_or_default());
        let v: Value = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok())
            .ok_or_else(|| refuse("unreadable", format!("{}", file.display())))?;
        if bb::digest(&v).as_str() != e["digest"].as_str().unwrap_or_default() {
            return Err(refuse("digest_mismatch", format!("{} is not what was captured", file.display())));
        }
        sources.push(json!({"uri": e["uri"], "digest": e["digest"]}));
    }
    sources.sort_by(|a, b| a["uri"].as_str().cmp(&b["uri"].as_str()));
    let w = &m["window"];
    let r = bb::record("snapshot", "current", &json!({"instance": instance, "company": company}),
        &json!({"tick": w["tick_from"], "date": w["date_from"]}), ("mcp-plan", "capture/v1"), &[], json!({
            "digest": bb::digest(&json!(sources)), "sources": sources, "max_age_days": max_age_days,
            "window": {"tick_from": w["tick_from"], "tick_to": w["tick_to"], "coherent": w["coherent"]}}));
    bb::validate_record(&r).map(|_| r)
}

/// The shadow run: every record it writes is validated first, and nothing is written if one fails.
/// The checks a live decision must pass before it is written (I0097; research/battlebrain/README.md,
/// "Cutover thresholds"). Every one is a parameter recorded in the decision, never a hidden default.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveGate {
    /// The widest game-tick window a snapshot may span (0: one tick, coherent).
    pub max_window_ticks: u64,
}

pub fn shadow(snapshot: &Value, plan: &Value, companies: &Value, company: i64, p: &policy::Params) -> Result<Value, Vec<bb::Refusal>> {
    run(snapshot, plan, companies, company, p, None)
}

/// One decision on one snapshot: in shadow (no gate) or live (`gate`), the records, the directive
/// block, and in live mode the directive itself. Every record is validated before anything is
/// returned, and nothing is returned if one fails.
pub fn run(snapshot: &Value, plan: &Value, companies: &Value, company: i64, p: &policy::Params, gate: Option<&LiveGate>) -> Result<Value, Vec<bb::Refusal>> {
    let refuse = |code: &'static str, msg: String| vec![bb::Refusal::new(code, "", msg)];
    match bb::validate_record(snapshot) {
        Ok("snapshot") => {}
        Ok(other) => return Err(refuse("not_a_snapshot", format!("--snapshot is a {other} record"))),
        Err(e) => return Err(e),
    }
    if snapshot["scope"]["company"].as_i64() != Some(company) {
        return Err(refuse("scope_mismatch", format!("the snapshot is scoped to company {}, not {company}", snapshot["scope"]["company"])));
    }
    let list = companies.pointer("/data/companies").or_else(|| companies.get("companies")).or_else(|| companies.get("data")).unwrap_or(companies);
    let Some(co) = list.as_array().and_then(|a| a.iter().find(|c| c["id"].as_i64() == Some(company))) else {
        return Err(refuse("unknown_company", format!("company {company} is not in the companies record")));
    };
    let snap_id = snapshot["id"].as_str().map(str::to_string).unwrap_or_else(|| bb::record_id(snapshot));
    let plan_digest = bb::digest(plan);
    let choice = policy::decide(&plan["freight"], co, p).map_err(|e| refuse("invalid_input", e))?;
    let base = policy::baseline(plan);
    let at = snapshot["at"].clone();
    let scope = snapshot["scope"].clone();
    let solver = plan.pointer("/portfolio/solver").and_then(Value::as_str).unwrap_or("none").to_string();
    let fallback = solver.starts_with("builtin");
    let model = plan["model"].as_str().unwrap_or("unknown").split(':').next().unwrap_or("unknown").to_string();
    let from_snapshot = [json!(snap_id)];
    let line_candidate = |l: &policy::Line, producer: (&str, &str), assumptions: Vec<String>| -> Value {
        bb::record("candidate", "proposed", &scope, &at, producer, &from_snapshot, json!({
            "action": {"kind": "freight_line", "settings": {"fr_from": l.producer, "fr_to": l.acceptor, "fr_cargo": l.cargo_id}},
            "estimate": {"cost": {"value": l.investment, "unit": "money"}, "profit": {"value": l.monthly_profit * 12.0, "unit": "money_per_year"}},
            "inputs_digest": plan_digest, "invocations": [], "assumptions": assumptions}))
    };
    let base_rec = base.as_ref().map(|b| line_candidate(b, ("mcp-plan", &model), vec![
        "planner v4 portfolio: its highest-scored freight_rail line".into(),
        format!("portfolio solver: {solver}"),
        "the bot's slot order (panel_bot.rs settings_from_plan) may differ when a line is unbuildable".into()]));
    let ours_rec = match &choice.selected {
        Some(l) => line_candidate(l, (bb::AUTHORITY, env!("CARGO_PKG_VERSION")), vec![format!("policy {}@{}", policy::POLICY_ID, policy::POLICY_VERSION), "estimates are the planner's (plan.rs model), not re-derived".into()]),
        None => bb::record("candidate", "proposed", &scope, &at, (bb::AUTHORITY, env!("CARGO_PKG_VERSION")), &from_snapshot,
            json!({"action": {"kind": "hold"}, "assumptions": ["no freight rail line clears the margin and the reserve"]})),
    };
    let (class, why) = policy::divergence(base.as_ref(), choice.selected.as_ref(), choice.budget, p);
    let mut derived = vec![json!(snap_id), ours_rec["id"].clone()];
    let mut rejected = Vec::new();
    if let Some(b) = &base_rec {
        if b["id"] != ours_rec["id"] {
            derived.push(b["id"].clone());
            rejected.push(json!({"candidate": b["id"], "reason": format!("{class}: {why}")}));
        }
    }
    let max_age = snapshot.pointer("/body/max_age_days").and_then(Value::as_i64).unwrap_or(1);
    let date = at["date"].as_str().unwrap_or_default();
    let valid_until = add_days(date, max_age).ok_or_else(|| refuse("invalid_input", format!("the snapshot date {date:?} is not a date")))?;
    let key = format!("{}:{}:{}:freight-line", scope["instance"].as_str().unwrap_or_default(), company, date);
    let decision = bb::record("decision", "decided", &scope, &at, (bb::AUTHORITY, env!("CARGO_PKG_VERSION")), &derived, json!({
        "policy": {"id": policy::POLICY_ID, "version": policy::POLICY_VERSION, "params": p.to_json()},
        "goal": "next freight rail line (one fr_* slot) or HOLD", "snapshot": snap_id, "selected": ours_rec["id"],
        "rejected": rejected, "idempotency_key": key, "approval": "operator", "valid_until": valid_until}));
    let block = json!({"schema": bb::DECISION_SCHEMA, "decision": decision["id"], "snapshot_digest": snapshot["body"]["digest"],
        "idempotency_key": key, "policy": format!("{}@{}", policy::POLICY_ID, policy::POLICY_VERSION), "valid_until": valid_until,
        "mode": if gate.is_some() { "live" } else { "shadow" }});
    let mut records: Vec<Value> = base_rec.iter().cloned().collect();
    if base_rec.as_ref().map(|b| &b["id"]) != Some(&ours_rec["id"]) {
        records.push(ours_rec.clone());
    }
    records.push(decision.clone());
    let mut errs = Vec::new();
    for r in &records {
        if let Err(e) = bb::validate_record(r) {
            errs.extend(e);
        }
    }
    if let Err(e) = bb::validate_decision_block(&block) {
        errs.extend(e);
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    // Live: the gates of the cutover, checked here, before a directive exists at all.
    let directive = match gate {
        None => Value::Null,
        Some(g) => {
            let mut why = Vec::new();
            let w = snapshot.pointer("/body/window").cloned().unwrap_or(Value::Null);
            match (w["tick_from"].as_u64(), w["tick_to"].as_u64()) {
                (Some(f), Some(t)) if t - f <= g.max_window_ticks => {}
                (Some(f), Some(t)) => why.push(bb::Refusal::new("incoherent_snapshot", "body.window", format!("the snapshot spans {} ticks, more than the {} this run accepts", t - f, g.max_window_ticks))),
                _ => why.push(bb::Refusal::new("incoherent_snapshot", "body.window", "the snapshot does not say which ticks it was read between")),
            }
            if !p.require_priced {
                why.push(bb::Refusal::new("unpriced_policy", "params.require_priced", "a live decision needs require_priced: only a line track-planning priced may go live"));
            }
            let fresh = companies.get("game_tick").and_then(Value::as_u64);
            if fresh.map_or(true, |t| t < w["tick_to"].as_u64().unwrap_or(u64::MAX)) {
                why.push(bb::Refusal::new("stale_companies", "companies.game_tick", "the companies must be read again, after the snapshot, so affordability is checked against money now"));
            }
            if !why.is_empty() {
                return Err(why);
            }
            match &choice.selected {
                None => Value::Null,
                // planned_freight_only=1 with the slot: the bot builds this line and plans none itself,
                // so the line has one author (one approval batch for the operator).
                Some(l) => json!({"company_id": company, "settings": format!("planned_freight_only=1,fr_from={},fr_to={},fr_cargo={}", l.producer, l.acceptor, l.cargo_id), "decision": block}),
            }
        }
    };
    Ok(json!({
        "mode": if gate.is_some() { "live" } else { "shadow" },
        "directive": directive,
        "live_gate": gate.map(|g| json!({"max_window_ticks": g.max_window_ticks})),
        "records": records,
        "directive_block": block,
        "comparison": {
            "snapshot": snap_id, "snapshot_digest": snapshot["body"]["digest"], "plan_digest": plan_digest,
            "policy": format!("{}@{}", policy::POLICY_ID, policy::POLICY_VERSION), "params": p.to_json(), "budget": choice.budget,
            "baseline": base.as_ref().map(policy::Line::to_json), "battlebrain": choice.selected.as_ref().map(policy::Line::to_json),
            "same": class == "same", "divergence": class, "reason": why,
            "rejected": choice.rejected.iter().map(|(l, r)| json!({"line": l.to_json(), "reason": r})).collect::<Vec<_>>(),
            "metrics": {"portfolio_solver": solver, "fallback_solver": fallback,
                        "missing_kernel": plan.pointer("/portfolio/kernel").map_or(true, Value::is_null),
                        "coherent_snapshot": snapshot.pointer("/body/window/coherent").cloned().unwrap_or(Value::Null),
                        "snapshot_window": snapshot.pointer("/body/window").cloned().unwrap_or(Value::Null),
                        "unpriced_lines": policy::lines(&plan["freight"]).iter().filter(|l| l.track.is_none()).count(),
                        "unroutable_lines": policy::lines(&plan["freight"]).iter().filter(|l| l.track.as_deref().is_some_and(|t| t != "found")).count()}
        }
    }))
}

/// The human summary of a shadow or live result record (`battlebrain-shadow-result/v1`), derived
/// from it alone: research/results/<id>.md is this function's output for <id>.json, and a test
/// refuses one that is not.
pub fn report(rec: &Value) -> String {
    let c = &rec["comparison"];
    let pv = &rec["provenance"];
    let w = &pv["capture"]["window"];
    let money = |v: &Value| v.as_f64().map_or("-".to_string(), |x| {
        let s = format!("{}", x.round() as i64);
        let (neg, d) = s.strip_prefix('-').map_or((false, s.as_str()), |r| (true, r));
        let mut out = String::new();
        for (i, ch) in d.chars().enumerate() {
            if i > 0 && (d.len() - i) % 3 == 0 { out.push(','); }
            out.push(ch);
        }
        if neg { format!("-{out}") } else { out }
    });
    let row = |who: &str, l: &Value| -> String {
        if l.is_null() {
            return format!("| {who} | HOLD (no line) | - | - | - | - | - | - |");
        }
        format!("| {who} | {} → {} (cargo {}) | {} | {} | {} | {} | {} | {} |", l["producer"], l["acceptor"], l["cargo_id"],
            money(&l["investment"]), money(&l["monthly_profit"]), l["annual_return"], l["payback_months"], money(&l["planner_score"]),
            l["track"].as_str().unwrap_or("not priced"))
    };
    let coherent = w["coherent"].as_bool().unwrap_or(false);
    let mut out = vec![
        format!("# {}", rec["id"].as_str().unwrap_or("battlebrain result")),
        String::new(),
        "Derived from its .json by `battlebrain report` (tools/research/mcp-rhai/src/bin/battlebrain.rs); do not edit.".into(),
        String::new(),
        format!("RESEARCH-ONLY. Mode **{}**: {}. One capture, one company and these parameters only; it does not generalise.",
            rec["mode"].as_str().unwrap_or("?"),
            if rec["mode"] == "shadow" { "the decision was never submitted; its directive block says `mode: shadow`, which the executor refuses (`panel_decision.rs` `decision_gate`)" } else { "see the outcome record for what the game did" }),
        String::new(),
        format!("- **Capture:** {}, ticks {}-{}: {}.", pv["capture"]["game"].as_str().unwrap_or("a game"), w["tick_from"], w["tick_to"],
            if coherent { "coherent (one tick)".to_string() } else { "**not coherent**: the game ran while it was read".to_string() }),
        format!("- **Policy:** `{}` with {}; company {}; budget {} (money + loan headroom - reserve).", c["policy"].as_str().unwrap_or("?"),
            c["params"], pv["company"], money(&c["budget"])),
        format!("- **Portfolio solver:** {}; AlgoGlyph commit `{}`.", c["metrics"]["portfolio_solver"].as_str().unwrap_or("?"),
            pv["algoglyph_commit"].as_str().map_or("?", |a| &a[..a.len().min(10)])),
        format!("- **Lines:** {} unpriced, {} unroutable, {} rejected with their reasons (see the .json).",
            c["metrics"]["unpriced_lines"], c["metrics"]["unroutable_lines"], c["rejected"].as_array().map_or(0, Vec::len)),
        String::new(),
        "| | line | investment | monthly profit | annual return | payback (months) | planner score | track |".into(),
        "|---|---|---|---|---|---|---|---|".into(),
        row("planner v4 (baseline)", &c["baseline"]),
        row("battlebrain", &c["battlebrain"]),
        String::new(),
        format!("**Divergence:** `{}`. {}.", c["divergence"].as_str().unwrap_or("?"), c["reason"].as_str().unwrap_or("")),
    ];
    if let Some(f) = rec["findings"].as_array() {
        out.push(String::new());
        for x in f {
            out.push(format!("**Finding:** {}", x.as_str().unwrap_or("")));
        }
    }
    out.join("\n") + "\n"
}

/// Answer exactly one GET on a loopback port with `body`, then return (`serve --once`): the
/// directive leaves this process once, whoever asks.
fn serve_once(port: u16, body: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
    let (mut stream, peer) = listener.accept().map_err(|e| e.to_string())?;
    if !peer.ip().is_loopback() {
        return Err(format!("refused a non-loopback peer {peer}"));
    }
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let ok = line.starts_with("GET ");
    let (status, out) = if ok { ("200 OK", body.to_string()) } else { ("405 Method Not Allowed", "{\"error\":\"GET only\"}".to_string()) };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).map_err(|e| e.to_string())?;
    if ok { Ok(line.trim().to_string()) } else { Err(format!("refused {}", line.trim())) }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("schema") => match args.get(1).and_then(|n| bb::schema(n)) {
            Some(s) => {
                println!("{}", serde_json::to_string_pretty(&s).expect("a schema serialises"));
                0
            }
            None => {
                eprintln!("battlebrain schema <record|invocation|decision>");
                2
            }
        },
        Some("validate") if args.len() > 1 => {
            let answers: Vec<Value> = args[1..].iter().map(|p| validate(p)).collect();
            let ok = answers.iter().all(|a| a["ok"] == true);
            for a in &answers {
                println!("{a}");
            }
            if ok { 0 } else { 1 }
        }
        Some("id") if args.len() == 2 => match read_json(&args[1]) {
            Ok(f) => {
                println!("{}", bb::record_id(f.get("value").unwrap_or(&f)));
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        Some("transition") if args.len() == 4 => match bb::transition(&args[1], &args[2], &args[3]) {
            Ok(()) => {
                println!("{}", json!({"ok": true}));
                0
            }
            Err(r) => refused(&[r]),
        },
        Some("check") if args.len() == 4 => match (read_json(&args[2]), read_json(&args[3])) {
            (Ok(i), Ok(o)) => match bb::domain_check(&args[1], &i, &o) {
                Ok(()) => {
                    println!("{}", json!({"ok": true}));
                    0
                }
                Err(r) => refused(&[r]),
            },
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("{e}");
                1
            }
        },
        Some("snapshot") => {
            let flag = |name: &str| flag(&args, name);
            match (flag("--capture"), flag("--instance"), flag("--company").and_then(|c| c.parse::<i64>().ok())) {
                (Some(dir), Some(instance), Some(company)) => {
                    let age = flag("--max-age-days").and_then(|a| a.parse().ok()).unwrap_or(31);
                    match snapshot_from_capture(std::path::Path::new(&dir), &instance, company, age) {
                        Ok(r) => {
                            println!("{}", serde_json::to_string_pretty(&r).expect("serialises"));
                            0
                        }
                        Err(rs) => refused(&rs),
                    }
                }
                _ => {
                    eprintln!("battlebrain snapshot --capture <dir> --instance <name> --company <id> [--max-age-days 31]");
                    2
                }
            }
        }
        Some("report") if args.len() == 2 => match read_json(&args[1]) {
            Ok(r) => {
                print!("{}", report(&r));
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        Some("live") => {
            let flag = |name: &str| flag(&args, name);
            let p = policy::Params {
                margin: flag("--margin").and_then(|m| m.parse().ok()).unwrap_or(0.2),
                reserve: flag("--reserve").and_then(|r| r.parse().ok()).unwrap_or(25_000),
                require_priced: true,
            };
            let gate = LiveGate { max_window_ticks: flag("--max-window-ticks").and_then(|t| t.parse().ok()).unwrap_or(0) };
            match (flag("--snapshot"), flag("--plan"), flag("--companies"), flag("--company").and_then(|c| c.parse::<i64>().ok())) {
                (Some(s), Some(pl), Some(c), Some(id)) => match (read_json(&s), read_json(&pl), read_json(&c)) {
                    (Ok(s), Ok(pl), Ok(c)) => match run(&s, &pl, &c, id, &p, Some(&gate)) {
                        Ok(out) => {
                            println!("{}", serde_json::to_string_pretty(&out).expect("serialises"));
                            0
                        }
                        Err(rs) => refused(&rs),
                    },
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                        eprintln!("{e}");
                        1
                    }
                },
                _ => {
                    eprintln!("battlebrain live --snapshot <record> --plan <file> --companies <fresh read> --company <id> [--margin x] [--reserve n] [--max-window-ticks n]");
                    2
                }
            }
        }
        Some("serve") => {
            let flag = |name: &str| flag(&args, name);
            match (flag("--directive").map(|f| read_json(&f)), flag("--port").and_then(|p| p.parse::<u16>().ok())) {
                (Some(Ok(d)), Some(port)) if d.get("company_id").is_some() => match serve_once(port, &d.to_string()) {
                    Ok(req) => {
                        println!("{}", json!({"ok": true, "served": req, "port": port}));
                        0
                    }
                    Err(e) => {
                        eprintln!("serve: {e}");
                        1
                    }
                },
                (Some(Err(e)), _) => {
                    eprintln!("{e}");
                    1
                }
                _ => {
                    eprintln!("battlebrain serve --directive <file with a company_id> --port <p>   (answers one GET on 127.0.0.1, then exits)");
                    2
                }
            }
        }
        Some("rollback") => match flag(&args, "--company").and_then(|c| c.parse::<i64>().ok()) {
            // Back to the verified baseline: the bot plans its own freight lines again. No decision
            // block: it is an ordinary directive, approved by the operator like every change.
            Some(id) => {
                println!("{}", json!({"company_id": id, "settings": "planned_freight_only=0"}));
                0
            }
            None => {
                eprintln!("battlebrain rollback --company <id>");
                2
            }
        },
        Some("shadow") => {
            let flag = |name: &str| flag(&args, name);
            let files = (flag("--snapshot"), flag("--plan"), flag("--companies"), flag("--company").and_then(|c| c.parse::<i64>().ok()));
            let p = policy::Params {
                margin: flag("--margin").and_then(|m| m.parse().ok()).unwrap_or(0.2),
                reserve: flag("--reserve").and_then(|r| r.parse().ok()).unwrap_or(25_000),
                require_priced: !args.iter().any(|a| a == "--allow-unpriced"),
            };
            match files {
                (Some(s), Some(pl), Some(c), Some(id)) => match (read_json(&s), read_json(&pl), read_json(&c)) {
                    (Ok(s), Ok(pl), Ok(c)) => match shadow(&s, &pl, &c, id, &p) {
                        Ok(out) => {
                            println!("{}", serde_json::to_string_pretty(&out).expect("serialises"));
                            0
                        }
                        Err(rs) => refused(&rs),
                    },
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                        eprintln!("{e}");
                        1
                    }
                },
                _ => {
                    eprintln!("battlebrain shadow --snapshot <record> --plan <file> --companies <file> --company <id> [--margin x] [--reserve n]");
                    2
                }
            }
        }
        Some("catalogue") => {
            let flag = |name: &str| flag(&args, name);
            let mut text = String::new();
            let _ = std::io::stdin().read_to_string(&mut text);
            match serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()).and_then(|v| {
                bb::Catalogue::from_runner(&v, &flag("--commit").unwrap_or_default()).map_err(|r| format!("{}: {}", r.code, r.message))
            }) {
                Ok(cat) => {
                    let kernel = flag("--kernel");
                    let found = kernel.as_deref().map(|k| cat.engine(k).map(str::to_string));
                    let engine = found.clone().flatten();
                    println!("{}", json!({"digest": cat.digest, "commit": cat.commit, "kernels": cat.entries.len(),
                        "kernel": kernel, "engine": engine, "known": found.map(|f| f.is_some())}));
                    if flag("--engine").is_some() && engine != flag("--engine") { 1 } else { 0 }
                }
                Err(e) => {
                    eprintln!("catalogue: {e}");
                    1
                }
            }
        }
        _ => {
            eprintln!("battlebrain <schema|validate|id|transition|check|snapshot|shadow|live|serve|report|rollback|catalogue> ... (see the header of src/bin/battlebrain.rs)");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn fx(rel: &str) -> Value {
        let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../schema/fixtures/battlebrain").join(rel);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap();
        v.get("value").cloned().unwrap_or(v)
    }
    fn shadow_on(plan: &Value) -> Result<Value, Vec<bb::Refusal>> {
        shadow(&fx("valid/snapshot.json"), plan, &fx("shadow/companies.json"), 1, &policy::Params { margin: 0.2, reserve: 25_000, require_priced: true })
    }

    #[test]
    fn a_shadow_run_writes_valid_records_and_a_shadow_block_only() {
        let out = shadow_on(&fx("shadow/plan.json")).unwrap();
        assert_eq!(out["mode"], "shadow");
        assert_eq!(out["directive_block"]["mode"], "shadow");
        assert!(bb::validate_decision_block(&out["directive_block"]).is_ok());
        let recs = out["records"].as_array().unwrap();
        let kinds: Vec<(String, String)> = recs.iter().map(|r| (r["type"].as_str().unwrap().into(), r["producer"]["id"].as_str().unwrap().into())).collect();
        assert_eq!(kinds, vec![("candidate".into(), "mcp-plan".into()), ("candidate".into(), "battlebrain".into()), ("decision".into(), "battlebrain".into())]);
        for r in recs {
            assert!(bb::validate_record(r).is_ok(), "{r}");
        }
        let dec = &recs[2];
        assert_eq!(dec["body"]["selected"], recs[1]["id"]);
        assert_eq!(dec["body"]["rejected"][0]["candidate"], recs[0]["id"]);
        assert_eq!(out["directive_block"]["decision"], dec["id"]);
        assert_eq!(out["comparison"]["divergence"], "input_model_discrepancy");
        assert_eq!(out["comparison"]["battlebrain"]["producer"], 13);
    }

    #[test]
    fn replay_is_a_function_of_the_recorded_input_only() {
        let a = shadow_on(&fx("shadow/plan.json")).unwrap();
        let b = shadow_on(&fx("shadow/plan.json")).unwrap();
        assert_eq!(a, b, "same recorded input, same records and ids");
        let mut changed = fx("shadow/plan.json");
        changed["freight"][1]["options"][1]["monthly_profit"] = json!(3001);
        let c = shadow_on(&changed).unwrap();
        assert_ne!(a["comparison"]["plan_digest"], c["comparison"]["plan_digest"]);
        assert_ne!(a["records"][1]["id"], c["records"][1]["id"], "a candidate's id follows its input digest");
        assert_eq!(c["records"][1]["body"]["inputs_digest"], c["comparison"]["plan_digest"]);
    }

    #[test]
    fn a_shadow_run_refuses_inputs_it_cannot_trust() {
        let mut snap = fx("valid/snapshot.json");
        snap["body"]["digest"] = json!("not-a-digest");
        let e = shadow(&snap, &fx("shadow/plan.json"), &fx("shadow/companies.json"), 1, &policy::Params { margin: 0.2, reserve: 0, require_priced: true }).unwrap_err();
        assert!(e.iter().any(|r| r.code == "invalid_field" || r.code == "id_mismatch"), "{e:?}");
        let e = shadow(&fx("valid/snapshot.json"), &fx("shadow/plan.json"), &fx("shadow/companies.json"), 2, &policy::Params { margin: 0.2, reserve: 0, require_priced: true }).unwrap_err();
        assert_eq!(e[0].code, "scope_mismatch");
        let e = shadow(&fx("valid/decision.json"), &fx("shadow/plan.json"), &fx("shadow/companies.json"), 1, &policy::Params { margin: 0.2, reserve: 0, require_priced: true }).unwrap_err();
        assert_eq!(e[0].code, "not_a_snapshot");
    }

    #[test]
    fn a_hold_is_recorded_as_a_hold_candidate() {
        let out = shadow(&fx("valid/snapshot.json"), &fx("shadow/plan.json"), &fx("shadow/companies.json"), 1, &policy::Params { margin: 5.0, reserve: 0, require_priced: true }).unwrap();
        let ours = &out["records"][1];
        assert_eq!(ours["body"]["action"]["kind"], "hold");
        assert_eq!(out["comparison"]["divergence"], "input_model_discrepancy");
        assert!(out["comparison"]["battlebrain"].is_null());
    }

    #[test]
    fn a_snapshot_from_a_capture_names_every_source_and_refuses_a_changed_file() {
        let d = std::env::temp_dir().join(format!("battlebrain-capture-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let towns = json!({"game_tick": 100, "game_date": "1953-02-09", "data": []});
        let map = json!({"game_tick": 143, "game_date": "1953-02-10", "data": {"x": 64}});
        std::fs::write(d.join("towns.json"), towns.to_string()).unwrap();
        std::fs::write(d.join("map.json"), map.to_string()).unwrap();
        let manifest = json!({"schema": "mcp-plan-capture/v1", "resources": [
            {"uri": "openttd://towns", "ok": true, "file": "towns.json", "digest": bb::digest(&towns)},
            {"uri": "openttd://map", "ok": true, "file": "map.json", "digest": bb::digest(&map)},
            {"uri": "openttd://engines", "ok": false, "error": "not offered"}],
            "window": {"tick_from": 100, "tick_to": 143, "date_from": "1953-02-09", "date_to": "1953-02-10", "coherent": false}});
        std::fs::write(d.join("capture.json"), manifest.to_string()).unwrap();
        let r = snapshot_from_capture(&d, "fixture", 3, 31).unwrap();
        assert_eq!(r["body"]["sources"].as_array().unwrap().len(), 2, "a failed read is no source");
        assert_eq!(r["body"]["window"]["coherent"], false);
        assert_eq!(r["at"], json!({"tick": 100, "date": "1953-02-09"}));
        assert_eq!(snapshot_from_capture(&d, "fixture", 3, 31).unwrap(), r, "deterministic");
        std::fs::write(d.join("map.json"), json!({"game_tick": 143, "data": {"x": 65}}).to_string()).unwrap();
        assert_eq!(snapshot_from_capture(&d, "fixture", 3, 31).unwrap_err()[0].code, "digest_mismatch");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_live_decision_passes_only_its_gates_and_carries_one_slot() {
        let p = policy::Params { margin: 0.2, reserve: 25_000, require_priced: true };
        let mut snap = fx("valid/snapshot.json");
        snap["body"]["window"] = json!({"tick_from": 74000, "tick_to": 74000, "coherent": true});
        snap.as_object_mut().unwrap().remove("id");
        snap["id"] = json!(bb::record_id(&snap));
        let mut fresh = fx("shadow/companies.json");
        fresh["game_tick"] = json!(74100);
        let out = run(&snap, &fx("shadow/plan.json"), &fresh, 1, &p, Some(&LiveGate { max_window_ticks: 0 })).unwrap();
        assert_eq!(out["mode"], "live");
        assert_eq!(out["directive_block"]["mode"], "live");
        assert_eq!(out["directive"]["company_id"], 1);
        assert_eq!(out["directive"]["settings"], "planned_freight_only=1,fr_from=13,fr_to=32,fr_cargo=1");
        assert_eq!(out["directive"]["decision"], out["directive_block"]);
        assert!(bb::validate_decision_block(&out["directive"]["decision"]).is_ok());
        // Each gate refuses on its own.
        let wide = run(&fx("valid/snapshot.json"), &fx("shadow/plan.json"), &fresh, 1, &p, Some(&LiveGate { max_window_ticks: 0 })).unwrap_err();
        assert!(wide.iter().any(|r| r.code == "incoherent_snapshot"), "{wide:?}");
        let stale = run(&snap, &fx("shadow/plan.json"), &fx("shadow/companies.json"), 1, &p, Some(&LiveGate { max_window_ticks: 0 })).unwrap_err();
        assert!(stale.iter().any(|r| r.code == "stale_companies"));
        let loose = policy::Params { require_priced: false, ..p.clone() };
        let unpriced = run(&snap, &fx("shadow/plan.json"), &fresh, 1, &loose, Some(&LiveGate { max_window_ticks: 0 })).unwrap_err();
        assert!(unpriced.iter().any(|r| r.code == "unpriced_policy"));
        // Shadow mode never writes a directive.
        assert!(run(&snap, &fx("shadow/plan.json"), &fresh, 1, &p, None).unwrap()["directive"].is_null());
    }

    #[test]
    fn a_directive_is_served_once_on_loopback_and_never_twice() {
        let port = 39_000 + (std::process::id() % 1000) as u16;
        let t = std::thread::spawn(move || serve_once(port, "{\"company_id\":1}"));
        let body = (0..50).find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            ureq::get(&format!("http://127.0.0.1:{port}/")).call().ok().and_then(|r| r.into_string().ok())
        });
        assert_eq!(body.as_deref(), Some("{\"company_id\":1}"));
        assert!(t.join().unwrap().unwrap().starts_with("GET "));
        assert!(ureq::get(&format!("http://127.0.0.1:{port}/")).timeout(std::time::Duration::from_millis(300)).call().is_err(), "served once, then gone");
    }

    #[test]
    fn every_committed_result_summary_is_its_records_report() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../research/results");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap().filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("battlebrain-") && name.ends_with(".json")) {
                continue;
            }
            let rec: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap();
            let md = std::fs::read_to_string(e.path().with_extension("md")).unwrap_or_else(|_| panic!("{name}: no .md beside it"));
            assert_eq!(md, report(&rec), "{name}: regenerate with `battlebrain report {name} > ….md`");
            n += 1;
        }
        assert!(n > 0, "no battlebrain result under research/results");
    }

    #[test]
    fn game_dates_add_days_on_the_gregorian_calendar() {
        assert_eq!(add_days("1961-01-01", 31).as_deref(), Some("1961-02-01"));
        assert_eq!(add_days("1960-02-28", 1).as_deref(), Some("1960-02-29"));
        assert_eq!(add_days("1961-12-31", 1).as_deref(), Some("1962-01-01"));
        assert_eq!(add_days("x", 1), None);
    }
}
