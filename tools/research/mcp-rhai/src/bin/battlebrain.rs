//! RESEARCH-ONLY: `battlebrain`, the home of the single strategic authority of the freight-line
//! decision (I0095, research/battlebrain/README.md). Phase 02 holds only its contracts; it decides
//! nothing and reaches no game.
//!
//!   battlebrain schema <record|invocation|decision|match> print a contract's JSON Schema (generated)
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
//!   battlebrain capabilities --catalogue <file> [--json | --snapshot <record>]
//!                                                     which kernel of that catalogue satisfies each
//!                                                     capability (I0098): the match table, the
//!                                                     capability-match/v1 decisions, or their
//!                                                     inference records; exit 1 if one is refused
//!
//! `shadow` and `live` take `--catalogue <file>` too: the run then also writes one capability-match
//! inference per capability, and a live run is refused when a capability has no kernel or the
//! planner computed it with another one (or with none).
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
use std::fmt::Write as _;
use std::io::Read;

/// The value after `--name` in the arguments.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Write refusals as the structured answer every subcommand gives, and the exit code 1.
fn refused(out: &mut String, rs: &[bb::Refusal]) -> i32 {
    let _ = writeln!(out, "{}", json!({"ok": false, "refusals": rs.iter().map(bb::Refusal::to_json).collect::<Vec<_>>()}));
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

/// Where the planner's answer names the kernel it computed a capability with. A capability absent
/// here is one the planner's answer does not attribute (rail-line-price: the track answers carry no
/// kernel id).
const PLAN_KERNELS: &[(&str, &str)] = &[("portfolio-within-budget", "/portfolio/kernel/id")];

/// The catalogue a file holds: `{"kernels": [...], "commit": ...}`, as tools/research/schema/
/// fixtures/battlebrain/catalogue.json records the runner's answer.
fn catalogue_file(path: &str) -> Result<bb::Catalogue, Vec<bb::Refusal>> {
    let v = read_json(path).map_err(|e| vec![bb::Refusal::new("unreadable", "", e)])?;
    let v = v.get("value").cloned().unwrap_or(v);
    bb::Catalogue::from_runner(&v["kernels"], v["commit"].as_str().unwrap_or_default()).map_err(|r| vec![r])
}

/// Add to a shadow or live run one capability-match inference per capability (derived from the
/// run's snapshot, stale with its decision) and the summary `capabilities`. A live run is refused
/// when a capability resolves to no kernel, or the planner computed it with another kernel or none.
pub fn attach_capabilities(out: &mut Value, plan: &Value, cat: &bb::Catalogue) -> Result<(), Vec<bb::Refusal>> {
    let snap = out["comparison"]["snapshot"].clone();
    let decision = out["records"].as_array().and_then(|r| r.iter().find(|r| r["type"] == "decision")).cloned().unwrap_or(Value::Null);
    let valid_until = out["directive_block"]["valid_until"].as_str().unwrap_or_default().to_string();
    let (mut records, mut summary, mut why) = (Vec::new(), Vec::new(), Vec::new());
    for c in bb::CAPABILITIES {
        let m = bb::match_capability(c.id, cat, bb::ADAPTERS);
        let r = bb::match_record(&m, &decision["scope"], &decision["at"], &[snap.clone()], &valid_until);
        bb::validate_record(&r)?;
        let used = PLAN_KERNELS.iter().find(|p| p.0 == c.id).map(|p| plan.pointer(p.1).cloned().unwrap_or(Value::Null));
        let agrees = used.as_ref().map(|u| m.chosen.is_some() && u.as_str() == m.chosen.as_deref());
        match (&m.refusal, agrees) {
            (Some(r), _) => why.push(bb::Refusal::new("capability_unmatched", c.id, format!("{}: {}", r.code, r.message))),
            (None, Some(false)) => why.push(bb::Refusal::new("capability_mismatch", c.id, format!("the planner computed {} with {}, the catalogue's match is {}",
                c.id, used.as_ref().map_or("?".to_string(), |u| if u.is_null() { "no kernel (a built-in fallback)".into() } else { u.to_string() }), m.chosen.as_deref().unwrap_or("?")))),
            _ => {}
        }
        summary.push(json!({"capability": c.id, "chosen": m.chosen, "refusal": m.refusal.as_ref().map(bb::Refusal::to_json), "record": r["id"],
            "planner_kernel": used, "agrees": agrees}));
        records.push(r);
    }
    if out["mode"] == "live" && !why.is_empty() {
        return Err(why);
    }
    out["records"].as_array_mut().expect("a run has records").extend(records);
    out["capabilities"] = json!({"catalogue": {"digest": cat.digest, "commit": cat.commit}, "matches": summary});
    Ok(())
}

/// The match table of every capability against a catalogue (Markdown), and whether every
/// capability resolved to a kernel. research/battlebrain/README.md carries it for the fixture.
pub fn match_table(cat: &bb::Catalogue, adapters: &[bb::Adapter]) -> (String, bool) {
    let mut ok = true;
    let fields = |fs: &[bb::Field]| fs.iter().map(|f| match f.unit {
        Some(u) => format!("`{}`: {} ({u})", f.name, f.kind),
        None => format!("`{}`: {}", f.name, f.kind),
    }).collect::<Vec<_>>().join(", ");
    let mut out = vec!["| capability | goal | inputs | outputs |".to_string(), "|---|---|---|---|".to_string()];
    for c in bb::CAPABILITIES {
        out.push(format!("| `{}` | {} | {} | {} |", c.id, c.goal, fields(c.inputs), fields(c.outputs)));
    }
    out.extend([
        String::new(),
        format!("Catalogue `{}` at AlgoGlyph `{}`: {} kernels (`{}`).", cat.digest, &cat.commit[..10], cat.kernels.len(), bb::CATALOGUE_SOURCE),
        String::new(),
        "| capability | kernel | engine | input shape | exact | verdict |".to_string(),
        "|---|---|---|---|---|---|".to_string(),
    ]);
    for c in bb::CAPABILITIES {
        let m = bb::match_capability(c.id, cat, adapters);
        let d = &m.decision;
        for k in d["candidates"].as_array().into_iter().flatten() {
            let verdict = if m.chosen.as_deref() == k["kernel"].as_str() {
                format!("**chosen**: {}", d["why"].as_str().unwrap_or(""))
            } else if k["compatible"] == true {
                "compatible, not chosen".to_string()
            } else {
                format!("rejected: {}", k["rejected"].as_array().into_iter().flatten().filter_map(|r| r["code"].as_str()).collect::<Vec<_>>().join(", "))
            };
            out.push(format!("| `{}` | `{}` | {} | {} | {} | {} |", c.id, k["kernel"].as_str().unwrap_or("?"), k["engine"].as_str().unwrap_or("absent"),
                k["shape"].as_str().unwrap_or("-"), if k["exact"] == true { "yes" } else { "no" }, verdict));
        }
        if let Some(r) = &m.refusal {
            ok = false;
            out.push(format!("| `{}` | - | - | - | - | **refused** `{}`: {} |", c.id, r.code, r.message));
        }
    }
    out.push(String::new());
    out.push(format!("Every other kernel of the catalogue was considered and has no adapter for that capability ({}).",
        bb::CAPABILITIES.iter().map(|c| format!("`{}`: {}", c.id, bb::match_capability(c.id, cat, adapters).decision["unadapted"].as_array().map_or(0, Vec::len))).collect::<Vec<_>>().join(", ")));
    (out.join("\n") + "\n", ok)
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
    // The planner's candidate and battlebrain's are records of two producers, so their ids always
    // differ, also when both name one line: the planner's is rejected with the divergence class
    // (`same` included), and both are written.
    let mut derived = vec![json!(snap_id), ours_rec["id"].clone()];
    let mut rejected = Vec::new();
    if let Some(b) = &base_rec {
        derived.push(b["id"].clone());
        rejected.push(json!({"candidate": b["id"], "reason": format!("{class}: {why}")}));
    }
    let max_age = snapshot.pointer("/body/max_age_days").and_then(Value::as_i64).unwrap_or(1);
    let date = at["date"].as_str().unwrap_or_default();
    let valid_until = bb::add_days(date, max_age).ok_or_else(|| refuse("invalid_input", format!("the snapshot date {date:?} is not a date")))?;
    let key = format!("{}:{}:{}:freight-line", scope["instance"].as_str().unwrap_or_default(), company, date);
    let decision = bb::record("decision", "decided", &scope, &at, (bb::AUTHORITY, env!("CARGO_PKG_VERSION")), &derived, json!({
        "policy": {"id": policy::POLICY_ID, "version": policy::POLICY_VERSION, "params": p.to_json()},
        "goal": "next freight rail line (one fr_* slot) or HOLD", "snapshot": snap_id, "selected": ours_rec["id"],
        "rejected": rejected, "idempotency_key": key, "approval": "operator", "valid_until": valid_until}));
    // The exact settings this decision authorises: the chosen line in slot 1, every other slot
    // cleared (-1), and planned_freight_only=1 so the bot builds this line and plans none itself.
    // The block names their digest; the executor recomputes it (I0117).
    let settings = choice.selected.as_ref().map_or(String::new(), |l| {
        let mut s = format!("planned_freight_only=1,{p}from={},{p}to={},{p}cargo={}", l.producer, l.acceptor, l.cargo_id, p = bb::freight_slot_prefix(1));
        for i in 2..=bb::FREIGHT_SLOTS {
            s.push_str(&format!(",{}from=-1", bb::freight_slot_prefix(i)));
        }
        s
    });
    let block = json!({"schema": bb::DECISION_SCHEMA, "decision": decision["id"], "snapshot_digest": snapshot["body"]["digest"],
        "idempotency_key": key, "policy": format!("{}@{}", policy::POLICY_ID, policy::POLICY_VERSION), "valid_until": valid_until,
        "settings_digest": bb::settings_digest(&settings), "mode": if gate.is_some() { "live" } else { "shadow" }});
    let mut records: Vec<Value> = base_rec.iter().cloned().collect();
    records.push(ours_rec.clone());
    records.push(decision.clone());
    let errs: Vec<bb::Refusal> = records.iter().filter_map(|r| bb::validate_record(r).err()).flatten()
        .chain(bb::validate_decision_block(&block).err().into_iter().flatten()).collect();
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
            if fallback {
                why.push(bb::Refusal::new("fallback_solver", "metrics.portfolio_solver", format!("the planner's portfolio came from {solver:?}, not an AlgoGlyph kernel")));
            }
            if plan.pointer("/portfolio/kernel").map_or(true, Value::is_null) {
                why.push(bb::Refusal::new("missing_kernel", "portfolio.kernel", "the planner names no kernel for its portfolio"));
            }
            if !p.require_priced {
                why.push(bb::Refusal::new("unpriced_policy", "params.require_priced", "a live decision needs require_priced: only a line track-planning priced may go live"));
            }
            // The snapshot is at most its own max_age_days old in game time, counted to the fresh read.
            let age = companies.get("game_date").and_then(Value::as_str).and_then(|now| bb::days_between(snapshot["at"]["date"].as_str().unwrap_or_default(), now));
            let max_age = snapshot.pointer("/body/max_age_days").and_then(Value::as_i64).unwrap_or(0);
            if age.map_or(true, |a| a > max_age) {
                why.push(bb::Refusal::new("stale_snapshot", "at.date", format!("the snapshot is {age:?} game days old at the fresh read, more than its {max_age}")));
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
                Some(_) => json!({"company_id": company, "settings": settings, "decision": block}),
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
/// The ledger of idempotency keys `serve` has handed out, one per line, kept across runs so the
/// same live decision is never served twice (`$BATTLEBRAIN_STATE_DIR`, else
/// ~/.local/state/battlebrain/served-keys).
fn served_ledger() -> std::path::PathBuf {
    served_ledger_in(std::env::var_os("BATTLEBRAIN_STATE_DIR"), std::env::var_os("HOME"))
}

/// The ledger's path for a state directory, else for a home directory.
fn served_ledger_in(state_dir: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> std::path::PathBuf {
    let dir = state_dir.map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(home.unwrap_or_default()).join(".local/state/battlebrain"));
    dir.join("served-keys")
}

/// Refuse a key already served; otherwise record it first, then serve.
fn claim_key(ledger: &std::path::Path, key: &str) -> Result<(), String> {
    use std::io::Write;
    if key.is_empty() || key.contains('\n') {
        return Err("a live directive needs a one-line idempotency key".into());
    }
    let seen = std::fs::read_to_string(ledger).unwrap_or_default();
    if seen.lines().any(|l| l == key) {
        return Err(format!("key {key:?} was served before ({}); a decision is served once", ledger.display()));
    }
    if let Some(d) = ledger.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(ledger).map_err(|e| e.to_string())?;
    writeln!(f, "{key}").map_err(|e| e.to_string())
}

/// Only a loopback peer is answered. The listener binds 127.0.0.1, so this holds already; it is
/// checked again so that a change of the bind address cannot hand a decision to the network.
fn loopback_only(peer: std::net::SocketAddr) -> Result<(), String> {
    if peer.ip().is_loopback() { Ok(()) } else { Err(format!("refused a non-loopback peer {peer}")) }
}

fn serve_once(port: u16, body: &str) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
    let (mut stream, peer) = listener.accept().map_err(|e| e.to_string())?;
    loopback_only(peer)?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let ok = line.starts_with("GET ");
    let (status, out) = if ok { ("200 OK", body.to_string()) } else { ("405 Method Not Allowed", "{\"error\":\"GET only\"}".to_string()) };
    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len()).map_err(|e| e.to_string())?;
    if ok { Ok(line.trim().to_string()) } else { Err(format!("refused {}", line.trim())) }
}

/// `--catalogue <file>` on a run: attach the capability matches, or nothing without the flag.
fn with_catalogue(out: &mut Value, plan: &Value, file: Option<String>) -> Result<(), Vec<bb::Refusal>> {
    match file {
        None => Ok(()),
        Some(f) => attach_capabilities(out, plan, &catalogue_file(&f)?),
    }
}

/// The command line: every subcommand, its output and its exit code. `main` only hands it the
/// process's arguments and stdin and prints what it wrote, so the tests run every subcommand.
fn cli(args: &[String], stdin: &mut dyn Read, out: &mut String, err: &mut String) -> i32 {
    let code = match args.first().map(String::as_str) {
        Some("schema") => match args.get(1).and_then(|n| bb::schema(n)) {
            Some(s) => {
                let _ = writeln!(out, "{}", serde_json::to_string_pretty(&s).expect("a schema serialises"));
                0
            }
            None => {
                let _ = writeln!(err, "battlebrain schema <record|invocation|decision|match>");
                2
            }
        },
        Some("validate") if args.len() > 1 => {
            let answers: Vec<Value> = args[1..].iter().map(|p| validate(p)).collect();
            let ok = answers.iter().all(|a| a["ok"] == true);
            for a in &answers {
                let _ = writeln!(out, "{a}");
            }
            if ok { 0 } else { 1 }
        }
        Some("id") if args.len() == 2 => match read_json(&args[1]) {
            Ok(f) => {
                let _ = writeln!(out, "{}", bb::record_id(f.get("value").unwrap_or(&f)));
                0
            }
            Err(e) => {
                let _ = writeln!(err, "{e}");
                1
            }
        },
        Some("transition") if args.len() == 4 => match bb::transition(&args[1], &args[2], &args[3]) {
            Ok(()) => {
                let _ = writeln!(out, "{}", json!({"ok": true}));
                0
            }
            Err(r) => refused(out, &[r]),
        },
        Some("check") if args.len() == 4 => match (read_json(&args[2]), read_json(&args[3])) {
            (Ok(i), Ok(o)) => match bb::domain_check(&args[1], &i, &o) {
                Ok(()) => {
                    let _ = writeln!(out, "{}", json!({"ok": true}));
                    0
                }
                Err(r) => refused(out, &[r]),
            },
            (Err(e), _) | (_, Err(e)) => {
                let _ = writeln!(err, "{e}");
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
                            let _ = writeln!(out, "{}", serde_json::to_string_pretty(&r).expect("serialises"));
                            0
                        }
                        Err(rs) => refused(out, &rs),
                    }
                }
                _ => {
                    let _ = writeln!(err, "battlebrain snapshot --capture <dir> --instance <name> --company <id> [--max-age-days 31]");
                    2
                }
            }
        }
        Some("report") if args.len() == 2 => match read_json(&args[1]) {
            Ok(r) => {
                let _ = write!(out, "{}", report(&r));
                0
            }
            Err(e) => {
                let _ = writeln!(err, "{e}");
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
                    (Ok(s), Ok(pl), Ok(c)) => match run(&s, &pl, &c, id, &p, Some(&gate)).and_then(|mut r| with_catalogue(&mut r, &pl, flag("--catalogue")).map(|_| r)) {
                        Ok(r) => {
                            let _ = writeln!(out, "{}", serde_json::to_string_pretty(&r).expect("serialises"));
                            0
                        }
                        Err(rs) => refused(out, &rs),
                    },
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                        let _ = writeln!(err, "{e}");
                        1
                    }
                },
                _ => {
                    let _ = writeln!(err, "battlebrain live --snapshot <record> --plan <file> --companies <fresh read> --company <id> [--margin x] [--reserve n] [--max-window-ticks n]");
                    2
                }
            }
        }
        Some("serve") => {
            let flag = |name: &str| flag(&args, name);
            match (flag("--directive").map(|f| read_json(&f)), flag("--port").and_then(|p| p.parse::<u16>().ok())) {
                (Some(Ok(d)), Some(port)) if d.get("company_id").is_some() => match claim_key(&served_ledger(), d.pointer("/decision/idempotency_key").and_then(Value::as_str).unwrap_or_default())
                    .and_then(|_| serve_once(port, &d.to_string())) {
                    Ok(req) => {
                        let _ = writeln!(out, "{}", json!({"ok": true, "served": req, "port": port}));
                        0
                    }
                    Err(e) => {
                        let _ = writeln!(err, "serve: {e}");
                        1
                    }
                },
                (Some(Err(e)), _) => {
                    let _ = writeln!(err, "{e}");
                    1
                }
                _ => {
                    let _ = writeln!(err, "battlebrain serve --directive <file with a company_id> --port <p>   (answers one GET on 127.0.0.1, then exits)");
                    2
                }
            }
        }
        Some("rollback") => match flag(&args, "--company").and_then(|c| c.parse::<i64>().ok()) {
            // Back to the verified baseline: the bot plans its own freight lines again. No decision
            // block: it is an ordinary directive, approved by the operator like every change.
            Some(id) => {
                let _ = writeln!(out, "{}", json!({"company_id": id, "settings": "planned_freight_only=0"}));
                0
            }
            None => {
                let _ = writeln!(err, "battlebrain rollback --company <id>");
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
                    (Ok(s), Ok(pl), Ok(c)) => match shadow(&s, &pl, &c, id, &p).and_then(|mut r| with_catalogue(&mut r, &pl, flag("--catalogue")).map(|_| r)) {
                        Ok(r) => {
                            let _ = writeln!(out, "{}", serde_json::to_string_pretty(&r).expect("serialises"));
                            0
                        }
                        Err(rs) => refused(out, &rs),
                    },
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                        let _ = writeln!(err, "{e}");
                        1
                    }
                },
                _ => {
                    let _ = writeln!(err, "battlebrain shadow --snapshot <record> --plan <file> --companies <file> --company <id> [--margin x] [--reserve n]");
                    2
                }
            }
        }
        Some("catalogue") => {
            let flag = |name: &str| flag(&args, name);
            let mut text = String::new();
            let _ = stdin.read_to_string(&mut text);
            match serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()).and_then(|v| {
                bb::Catalogue::from_runner(&v, &flag("--commit").unwrap_or_default()).map_err(|r| format!("{}: {}", r.code, r.message))
            }) {
                Ok(cat) => {
                    let kernel = flag("--kernel");
                    let found = kernel.as_deref().map(|k| cat.engine(k).map(str::to_string));
                    let engine = found.clone().flatten();
                    let _ = writeln!(out, "{}", json!({"digest": cat.digest, "commit": cat.commit, "kernels": cat.kernels.len(),
                        "kernel": kernel, "engine": engine, "known": found.map(|f| f.is_some())}));
                    if flag("--engine").is_some() && engine != flag("--engine") { 1 } else { 0 }
                }
                Err(e) => {
                    let _ = writeln!(err, "catalogue: {e}");
                    1
                }
            }
        }
        Some("capabilities") => match flag(&args, "--catalogue").map(|f| catalogue_file(&f)) {
            Some(Ok(cat)) => {
                let matches: Vec<bb::Match> = bb::CAPABILITIES.iter().map(|c| bb::match_capability(c.id, &cat, bb::ADAPTERS)).collect();
                let ok = matches.iter().all(|m| m.chosen.is_some());
                if args.iter().any(|a| a == "--json") {
                    let _ = writeln!(out, "{}", serde_json::to_string_pretty(&matches.iter().map(|m| &m.decision).collect::<Vec<_>>()).expect("serialises"));
                } else if let Some(s) = flag(&args, "--snapshot") {
                    let snap = match read_json(&s) {
                        Ok(v) => v.get("value").cloned().unwrap_or(v),
                        Err(e) => {
                            let _ = writeln!(err, "{e}");
                            return 1;
                        }
                    };
                    if let Err(rs) = bb::validate_record(&snap).and_then(|k| if k == "snapshot" { Ok(()) } else { Err(vec![bb::Refusal::new("not_a_snapshot", "type", format!("--snapshot is a {k} record"))]) }) {
                        return refused(out, &rs);
                    }
                    let until = bb::add_days(snap["at"]["date"].as_str().unwrap_or_default(), snap["body"]["max_age_days"].as_i64().unwrap_or(1)).unwrap_or_default();
                    let recs: Vec<Value> = matches.iter().map(|m| bb::match_record(m, &snap["scope"], &snap["at"], &[snap["id"].clone()], &until)).collect();
                    let errs: Vec<bb::Refusal> = recs.iter().filter_map(|r| bb::validate_record(r).err()).flatten().collect();
                    if !errs.is_empty() {
                        return refused(out, &errs);
                    }
                    let _ = writeln!(out, "{}", serde_json::to_string_pretty(&recs).expect("serialises"));
                } else {
                    let _ = write!(out, "{}", match_table(&cat, bb::ADAPTERS).0);
                }
                if ok { 0 } else { 1 }
            }
            Some(Err(rs)) => refused(out, &rs),
            None => {
                let _ = writeln!(err, "battlebrain capabilities --catalogue <file with kernels and commit> [--json | --snapshot <record>]");
                2
            }
        },
        _ => {
            let _ = writeln!(err, "battlebrain <schema|validate|id|transition|check|snapshot|shadow|live|serve|report|rollback|catalogue|capabilities> ... (see the header of src/bin/battlebrain.rs)");
            2
        }
    };
    code
}

fn main() {
    let (mut out, mut err) = (String::new(), String::new());
    let code = cli(&std::env::args().skip(1).collect::<Vec<_>>(), &mut std::io::stdin(), &mut out, &mut err);
    print!("{out}");
    eprint!("{err}");
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
        fresh["game_date"] = json!("1961-01-02");
        let mut old = fresh.clone();
        old["game_date"] = json!("1961-03-01");
        let e = run(&snap, &fx("shadow/plan.json"), &old, 1, &p, Some(&LiveGate { max_window_ticks: 0 })).unwrap_err();
        assert!(e.iter().any(|r| r.code == "stale_snapshot"), "{e:?}");
        let out = run(&snap, &fx("shadow/plan.json"), &fresh, 1, &p, Some(&LiveGate { max_window_ticks: 0 })).unwrap();
        assert_eq!(out["mode"], "live");
        assert_eq!(out["directive_block"]["mode"], "live");
        assert_eq!(out["directive"]["company_id"], 1);
        let settings = out["directive"]["settings"].as_str().unwrap();
        assert!(settings.starts_with("planned_freight_only=1,fr_from=13,fr_to=32,fr_cargo=1,fr2_from=-1,"), "{settings}");
        assert!(settings.ends_with(",fr16_from=-1"));
        assert_eq!(out["directive"]["decision"]["settings_digest"], json!(bb::settings_digest(settings)));
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
    fn a_served_key_is_never_served_again() {
        let dir = std::env::temp_dir().join(format!("battlebrain-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ledger = dir.join("served-keys");
        assert!(claim_key(&ledger, "coplay:0:1963-03-01:freight-line").is_ok());
        assert!(claim_key(&ledger, "coplay:0:1963-03-01:freight-line").unwrap_err().contains("served before"));
        assert!(claim_key(&ledger, "another").is_ok());
        assert!(claim_key(&ledger, "").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_live_decision_refuses_a_fallback_solver_or_no_kernel() {
        let p = policy::Params { margin: 0.2, reserve: 25_000, require_priced: true };
        let mut snap = fx("valid/snapshot.json");
        snap["body"]["window"] = json!({"tick_from": 74000, "tick_to": 74000, "coherent": true});
        snap.as_object_mut().unwrap().remove("id");
        snap["id"] = json!(bb::record_id(&snap));
        let mut fresh = fx("shadow/companies.json");
        fresh["game_tick"] = json!(74100);
        fresh["game_date"] = json!("1961-01-02");
        let mut plan = fx("shadow/plan.json");
        plan["portfolio"]["solver"] = json!("builtin knapsack (MCP_ALGOGLYPH_DIR not set or the runner refused)");
        plan["portfolio"]["kernel"] = Value::Null;
        let e = run(&snap, &plan, &fresh, 1, &p, Some(&LiveGate { max_window_ticks: 0 })).unwrap_err();
        assert!(e.iter().any(|r| r.code == "fallback_solver") && e.iter().any(|r| r.code == "missing_kernel"), "{e:?}");
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

    fn real_catalogue() -> bb::Catalogue {
        catalogue_file(Path::new(env!("CARGO_MANIFEST_DIR")).join("../schema/fixtures/battlebrain/catalogue.json").to_str().unwrap()).unwrap()
    }

    fn live_inputs() -> (Value, Value) {
        let mut snap = fx("valid/snapshot.json");
        snap["body"]["window"] = json!({"tick_from": 74000, "tick_to": 74000, "coherent": true});
        snap.as_object_mut().unwrap().remove("id");
        snap["id"] = json!(bb::record_id(&snap));
        let mut fresh = fx("shadow/companies.json");
        fresh["game_tick"] = json!(74100);
        fresh["game_date"] = json!("1961-01-02");
        (snap, fresh)
    }

    #[test]
    fn a_run_with_the_catalogue_records_one_capability_match_per_capability() {
        let plan = fx("shadow/plan.json");
        let mut out = shadow_on(&plan).unwrap();
        let before = out.clone();
        attach_capabilities(&mut out, &plan, &real_catalogue()).unwrap();
        let recs = out["records"].as_array().unwrap();
        assert_eq!(recs.len(), 3 + bb::CAPABILITIES.len());
        assert_eq!(&recs[..3], before["records"].as_array().unwrap().as_slice(), "the decision's records are unchanged");
        assert_eq!(out["directive_block"], before["directive_block"]);
        for r in &recs[3..] {
            assert_eq!(bb::validate_record(r), Ok("inference"), "{r}");
            assert_eq!(r["body"]["method"]["id"], bb::MATCH_METHOD);
            assert_eq!(r["derived_from"], json!([out["comparison"]["snapshot"]]));
            assert_eq!(r["body"]["valid_until"], out["directive_block"]["valid_until"]);
        }
        let m = &out["capabilities"]["matches"];
        assert_eq!(m[0], json!({"capability": "portfolio-within-budget", "chosen": "knapsack-dp", "refusal": null, "record": recs[3]["id"],
            "planner_kernel": "knapsack-dp", "agrees": true}));
        assert_eq!(m[1]["chosen"], "track-planning");
        assert!(m[1]["planner_kernel"].is_null(), "the planner's track answers name no kernel");
        assert_eq!(out["capabilities"]["catalogue"]["commit"], "1b8e009059c299b29d7a829d2d51d4e057304d4d");
    }

    #[test]
    fn a_live_run_is_refused_when_a_capability_has_no_kernel_or_another_one() {
        let (snap, fresh) = live_inputs();
        let p = policy::Params { margin: 0.2, reserve: 25_000, require_priced: true };
        let gate = LiveGate { max_window_ticks: 0 };
        let plan = fx("shadow/plan.json");
        let live = || run(&snap, &plan, &fresh, 1, &p, Some(&gate)).unwrap();
        let mut ok = live();
        attach_capabilities(&mut ok, &plan, &real_catalogue()).unwrap();
        // The line, and every other freight slot cleared (I0117).
        assert!(ok["directive"]["settings"].as_str().unwrap().starts_with("planned_freight_only=1,fr_from=13,fr_to=32,fr_cargo=1,fr2_from=-1,"));
        // The runner no longer lists track-planning: nothing may price the line.
        let (k, commit) = {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../schema/fixtures/battlebrain/catalogue.json")).unwrap()).unwrap();
            (v["kernels"].as_array().unwrap().clone(), v["commit"].as_str().unwrap().to_string())
        };
        let expired = bb::Catalogue::from_runner(&json!(k.iter().filter(|x| x["id"] != "track-planning").collect::<Vec<_>>()), &commit).unwrap();
        let e = attach_capabilities(&mut live(), &plan, &expired).unwrap_err();
        assert_eq!((e[0].code, e[0].path.as_str()), ("capability_unmatched", "rail-line-price"));
        assert!(e[0].message.starts_with("kernel_expired"));
        // The planner fell back to its built-in knapsack: the live gate refuses it before any match.
        let mut builtin = plan.clone();
        builtin["portfolio"]["kernel"] = Value::Null;
        assert!(run(&snap, &builtin, &fresh, 1, &p, Some(&gate)).unwrap_err().iter().any(|r| r.code == "missing_kernel"));
        // The planner names another kernel than the catalogue's match: the match refuses it.
        let mut other = plan.clone();
        other["portfolio"]["kernel"]["id"] = json!("greedy-knapsack");
        let out = run(&snap, &other, &fresh, 1, &p, Some(&gate)).unwrap();
        let e = attach_capabilities(&mut out.clone(), &other, &real_catalogue()).unwrap_err();
        assert_eq!(e[0].code, "capability_mismatch");
        assert!(e[0].message.contains("\"greedy-knapsack\""), "{}", e[0].message);
        // In shadow the same disagreement is recorded, not refused.
        let mut sh = shadow_on(&builtin).unwrap();
        attach_capabilities(&mut sh, &builtin, &real_catalogue()).unwrap();
        assert_eq!(sh["capabilities"]["matches"][0]["agrees"], false);
    }

    #[test]
    fn the_readme_match_table_is_the_real_catalogues() {
        let readme = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../research/battlebrain/README.md")).unwrap();
        let (begin, end) = ("<!-- battlebrain capabilities:begin -->\n", "<!-- battlebrain capabilities:end -->");
        let i = readme.find(begin).expect("README has the capabilities markers") + begin.len();
        let j = readme[i..].find(end).expect("an end marker") + i;
        let (table, ok) = match_table(&real_catalogue(), bb::ADAPTERS);
        assert!(ok, "every capability resolves on the real catalogue");
        assert_eq!(&readme[i..j], table, "regenerate with `battlebrain capabilities --catalogue tools/research/schema/fixtures/battlebrain/catalogue.json`");
    }

    #[test]
    fn nothing_here_executes_a_kernel() {
        // The matcher chooses; it never runs what it chose, so no synthetic kernel can be executed.
        // No process is spawned by the contract, the policy or this binary.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let needles = [concat!("Command", "::new"), concat!("process", "::Command"), concat!("process", "::Child")];
        for f in ["battlebrain_contract.rs", "battlebrain_policy.rs", "bin/battlebrain.rs"] {
            let text = std::fs::read_to_string(src.join(f)).unwrap();
            for n in needles {
                let code: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
                assert!(!code.contains(n), "{f} contains {n}");
            }
        }
    }

    /// Run the command line as `main` does, on `stdin`: its exit code, stdout and stderr.
    fn cli_run(args: &[&str], stdin: &str) -> (i32, String, String) {
        let (mut out, mut err) = (String::new(), String::new());
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let code = cli(&args, &mut stdin.as_bytes(), &mut out, &mut err);
        (code, out, err)
    }

    /// A fixture's path, as the command line takes it.
    fn path(rel: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../schema/fixtures/battlebrain").join(rel).display().to_string()
    }

    /// A scratch directory of this test process, `name` inside it, empty.
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("battlebrain-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// `v` written as `name` into `dir`, its path returned.
    fn put(dir: &Path, name: &str, v: &Value) -> String {
        let f = dir.join(name);
        std::fs::write(&f, v.to_string()).unwrap();
        f.display().to_string()
    }

    #[test]
    fn the_contract_subcommands_answer_and_refuse_with_their_exit_codes() {
        let d = scratch("contract");
        let missing = d.join("missing.json").display().to_string();
        assert_eq!(cli_run(&["schema", "record"], "").0, 0);
        assert!(cli_run(&["schema", "record"], "").1.contains("battlebrain-record"));
        assert_eq!(cli_run(&["schema", "nope"], "").0, 2);
        // validate: every kind of file, a refused one, an unreadable one and an invocation's catalogue.
        let (code, out, _) = cli_run(&["validate", &path("valid/decision.json"), &path("valid/decision-block.json"), &path("valid/invocation-knapsack.json")], "");
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("\"kind\":\"decision-block\"") && out.contains("\"kind\":\"invocation\""), "{out}");
        let mut bare = fx("valid/invocation-knapsack.json");
        bare.as_object_mut().unwrap().remove("catalogue");
        let mut bad_cat: Value = serde_json::from_str(&std::fs::read_to_string(path("valid/invocation-knapsack.json")).unwrap()).unwrap();
        bad_cat["catalogue"]["commit"] = json!("main");
        let (code, out, _) = cli_run(&["validate", &path("invalid/unknown-type.json"), &missing, &put(&d, "bare.json", &bare), &put(&d, "bad-cat.json", &bad_cat)], "");
        assert_eq!(code, 1);
        for want in ["invalid_field", "unreadable", "no_catalogue", "invalid_catalogue"] {
            assert!(out.contains(want), "{want}: {out}");
        }
        // id, transition and check.
        let (code, out, _) = cli_run(&["id", &path("valid/decision.json")], "");
        assert_eq!((code, out.trim()), (0, fx("valid/decision.json")["id"].as_str().unwrap()));
        assert_eq!(cli_run(&["id", &missing], "").0, 1);
        assert_eq!(cli_run(&["transition", "candidate", "proposed", "selected"], "").0, 0);
        assert!(cli_run(&["transition", "candidate", "selected", "proposed"], "").1.contains("illegal_transition"));
        let input = put(&d, "input.json", &json!({"weights": [3, 4, 5], "values": [30.0, 50.0, 60.0], "capacity": 8}));
        let good = put(&d, "good.json", &json!({"result": {"best": 90.0, "items": [0, 2]}}));
        let over = put(&d, "over.json", &json!({"result": {"best": 110.0, "items": [1, 2]}}));
        assert_eq!(cli_run(&["check", "knapsack-dp", &input, &good], "").0, 0);
        assert!(cli_run(&["check", "knapsack-dp", &input, &over], "").1.contains("invalid_output"));
        assert_eq!(cli_run(&["check", "knapsack-dp", &input, &missing], "").0, 1);
        // Anything else is a usage error.
        assert_eq!(cli_run(&["validate"], "").0, 2);
        assert!(cli_run(&[], "").2.starts_with("battlebrain <schema|"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_run_subcommands_write_their_records_or_refuse() {
        let d = scratch("runs");
        let missing = d.join("missing.json").display().to_string();
        // snapshot from a capture, refused on a changed schema, and its usage.
        let towns = json!({"game_tick": 100, "game_date": "1953-02-09", "data": []});
        std::fs::write(d.join("towns.json"), towns.to_string()).unwrap();
        let capture = |schema: &str| json!({"schema": schema, "window": {"tick_from": 100, "tick_to": 100, "date_from": "1953-02-09", "coherent": true},
            "resources": [{"uri": "openttd://towns", "ok": true, "file": "towns.json", "digest": bb::digest(&towns)}]});
        put(&d, "capture.json", &capture("mcp-plan-capture/v1"));
        let dir = d.display().to_string();
        let (code, out, _) = cli_run(&["snapshot", "--capture", &dir, "--instance", "t", "--company", "1", "--max-age-days", "7"], "");
        assert_eq!(code, 0, "{out}");
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["body"]["max_age_days"], 7);
        put(&d, "capture.json", &capture("mcp-plan-capture/v2"));
        assert!(cli_run(&["snapshot", "--capture", &dir, "--instance", "t", "--company", "1"], "").1.contains("unknown_schema"));
        assert_eq!(cli_run(&["snapshot", "--capture", &dir], "").0, 2);
        // shadow and live, with and without the catalogue.
        let (snap, fresh) = live_inputs();
        let (snap, fresh) = (put(&d, "snap.json", &snap), put(&d, "fresh.json", &fresh));
        let (plan, companies) = (path("shadow/plan.json"), path("shadow/companies.json"));
        let cat = path("catalogue.json");
        let shadow_args = ["shadow", "--snapshot", &snap, "--plan", &plan, "--companies", &companies, "--company", "1"];
        let (code, out, _) = cli_run(&shadow_args, "");
        assert_eq!((code, serde_json::from_str::<Value>(&out).unwrap()["mode"].clone()), (0, json!("shadow")));
        let mut with_cat = shadow_args.to_vec();
        with_cat.extend(["--catalogue", &cat, "--margin", "0.3", "--reserve", "1000", "--allow-unpriced"]);
        let out: Value = serde_json::from_str(&cli_run(&with_cat, "").1).unwrap();
        assert_eq!(out["comparison"]["params"], json!({"margin": 0.3, "reserve": 1000, "require_priced": false}));
        assert!(out["capabilities"]["matches"].is_array());
        assert!(cli_run(&["shadow", "--snapshot", &snap, "--plan", &plan, "--companies", &companies, "--company", "2"], "").1.contains("scope_mismatch"));
        assert_eq!(cli_run(&["shadow", "--snapshot", &missing, "--plan", &plan, "--companies", &companies, "--company", "1"], "").0, 1);
        assert_eq!(cli_run(&["shadow"], "").0, 2);
        let live_args = ["live", "--snapshot", &snap, "--plan", &plan, "--companies", &fresh, "--company", "1", "--catalogue", &cat, "--max-window-ticks", "0"];
        let (code, out, _) = cli_run(&live_args, "");
        assert_eq!(code, 0, "{out}");
        let live: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(live["directive"]["company_id"], 1);
        assert!(cli_run(&["live", "--snapshot", &snap, "--plan", &plan, "--companies", &companies, "--company", "1"], "").1.contains("stale_companies"));
        assert_eq!(cli_run(&["live", "--snapshot", &snap, "--plan", &missing, "--companies", &fresh, "--company", "1"], "").0, 1);
        assert_eq!(cli_run(&["live", "--snapshot", &snap], "").0, 2);
        assert_eq!(cli_run(&["live", "--snapshot", &snap, "--plan", &plan, "--companies", &fresh, "--company", "1", "--catalogue", &missing], "").0, 1,
                   "an unreadable catalogue refuses the run");
        // report and rollback.
        let result = put(&d, "result.json", &json!({"id": "r", "mode": "live", "comparison": {"baseline": null, "battlebrain": null}}));
        let (code, out, _) = cli_run(&["report", &result], "");
        assert_eq!(code, 0);
        assert!(out.contains("| planner v4 (baseline) | HOLD (no line) |") && !out.contains("**Finding:**"), "{out}");
        assert_eq!(cli_run(&["report", &missing], "").0, 1);
        assert_eq!(cli_run(&["rollback", "--company", "3"], "").1.trim(), r#"{"company_id":3,"settings":"planned_freight_only=0"}"#);
        assert_eq!(cli_run(&["rollback"], "").0, 2);
        // serve: a directive once, never its key twice; an unreadable directive; the usage.
        std::env::set_var("BATTLEBRAIN_STATE_DIR", d.join("state"));
        let directive = put(&d, "directive.json", &live["directive"]);
        let port = 38_000 + (std::process::id() % 1000) as u16;
        let client = std::thread::spawn(move || (0..100).find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            ureq::get(&format!("http://127.0.0.1:{port}/")).call().ok().and_then(|r| r.into_string().ok())
        }));
        let (code, out, _) = cli_run(&["serve", "--directive", &directive, "--port", &port.to_string()], "");
        assert_eq!(code, 0, "{out}");
        assert_eq!(serde_json::from_str::<Value>(&client.join().unwrap().unwrap()).unwrap(), live["directive"]);
        let (code, _, err) = cli_run(&["serve", "--directive", &directive, "--port", &port.to_string()], "");
        assert!(code == 1 && err.contains("served before"), "{err}");
        assert_eq!(cli_run(&["serve", "--directive", &missing, "--port", "1"], "").0, 1);
        assert_eq!(cli_run(&["serve", "--directive", &result, "--port", "1"], "").0, 2, "a directive names its company");
        std::env::remove_var("BATTLEBRAIN_STATE_DIR");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_catalogue_subcommands_digest_match_and_refuse() {
        let d = scratch("catalogue");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path("catalogue.json")).unwrap()).unwrap();
        let (kernels, commit) = (v["kernels"].to_string(), v["commit"].as_str().unwrap().to_string());
        let (code, out, _) = cli_run(&["catalogue", "--commit", &commit, "--kernel", "knapsack-dp", "--engine", "grid"], &kernels);
        let answer: Value = serde_json::from_str(&out).unwrap();
        assert_eq!((code, answer["known"].clone(), answer["engine"].clone()), (0, json!(true), json!("grid")), "{out}");
        assert_eq!(cli_run(&["catalogue", "--commit", &commit, "--kernel", "knapsack-dp", "--engine", "numeric"], &kernels).0, 1);
        assert!(cli_run(&["catalogue", "--commit", "main"], &kernels).2.starts_with("catalogue: invalid_catalogue"));
        assert_eq!(cli_run(&["catalogue", "--commit", &commit], "not json").0, 1);
        // capabilities: the table, the decisions, the records of a snapshot, and each refusal.
        let cat = path("catalogue.json");
        assert!(cli_run(&["capabilities", "--catalogue", &cat], "").1.starts_with("| capability | goal |"));
        let (code, out, _) = cli_run(&["capabilities", "--catalogue", &cat, "--json"], "");
        assert_eq!((code, serde_json::from_str::<Value>(&out).unwrap().as_array().unwrap().len()), (0, bb::CAPABILITIES.len()));
        let snap = path("valid/snapshot.json");
        let recs: Value = serde_json::from_str(&cli_run(&["capabilities", "--catalogue", &cat, "--snapshot", &snap], "").1).unwrap();
        assert!(recs.as_array().unwrap().iter().all(|r| bb::validate_record(r) == Ok("inference")));
        assert!(cli_run(&["capabilities", "--catalogue", &cat, "--snapshot", &path("valid/decision.json")], "").1.contains("not_a_snapshot"));
        let mut unnamed = fx("valid/snapshot.json");
        unnamed.as_object_mut().unwrap().remove("id");
        assert!(cli_run(&["capabilities", "--catalogue", &cat, "--snapshot", &put(&d, "unnamed.json", &unnamed)], "").1.contains("derived_from"),
                "a snapshot without its id cannot be derived from");
        let missing = d.join("missing.json").display().to_string();
        assert_eq!(cli_run(&["capabilities", "--catalogue", &cat, "--snapshot", &missing], "").0, 1);
        assert!(cli_run(&["capabilities", "--catalogue", &missing], "").1.contains("unreadable"));
        assert_eq!(cli_run(&["capabilities"], "").0, 2);
        let mut expired = v.clone();
        expired["kernels"].as_array_mut().unwrap().retain(|k| k["id"] != "track-planning");
        let (code, out, _) = cli_run(&["capabilities", "--catalogue", &put(&d, "expired.json", &expired)], "");
        assert_eq!(code, 1, "a refused capability fails the command");
        assert!(out.contains("rejected: kernel_absent") && out.contains("**refused** `kernel_expired`"), "{out}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_tie_leaves_every_compatible_kernel_unchosen_in_the_table() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path("catalogue.json")).unwrap()).unwrap();
        let mut kernels = v["kernels"].as_array().unwrap().clone();
        kernels.push(json!({"id": "knapsack-twin", "engine": "grid", "input": "the kernel's data form"}));
        let cat = bb::Catalogue::from_runner(&json!(kernels), v["commit"].as_str().unwrap()).unwrap();
        let knapsack = bb::ADAPTERS.iter().find(|a| a.kernel == "knapsack-dp").unwrap();
        let twin = bb::Adapter { kernel: "knapsack-twin", ..*knapsack };
        let (table, ok) = match_table(&cat, &[*knapsack, twin]);
        assert!(!ok);
        assert_eq!(table.matches("compatible, not chosen").count(), 2, "{table}");
        assert!(table.contains("**refused** `ambiguous_capability`"));
    }

    #[test]
    fn a_run_refuses_an_unknown_company_and_records_that_break_their_contract() {
        let p = policy::Params { margin: 0.2, reserve: 25_000, require_priced: true };
        let none = json!({"companies": [{"id": 7, "money": 1, "loan": 0, "max_loan": 0}]});
        assert_eq!(shadow(&fx("valid/snapshot.json"), &fx("shadow/plan.json"), &none, 1, &p).unwrap_err()[0].code, "unknown_company");
        let mut nameless = fx("shadow/plan.json");
        nameless["model"] = json!(":v4");
        let e = shadow(&fx("valid/snapshot.json"), &nameless, &fx("shadow/companies.json"), 1, &p).unwrap_err();
        assert!(e.iter().any(|r| r.path == "producer.version"), "the planner's candidate names no model version: {e:?}");
        // Live: a HOLD writes no directive; a window wider than the gate is named with its width.
        let (snap, fresh) = live_inputs();
        let hold = run(&snap, &fx("shadow/plan.json"), &fresh, 1, &policy::Params { margin: 50.0, ..p.clone() }, Some(&LiveGate { max_window_ticks: 0 })).unwrap();
        assert!(hold["directive"].is_null() && hold["mode"] == "live");
        let mut wide = snap.clone();
        wide["body"]["window"] = json!({"tick_from": 73000, "tick_to": 74000, "coherent": false});
        wide.as_object_mut().unwrap().remove("id");
        wide["id"] = json!(bb::record_id(&wide));
        let e = run(&wide, &fx("shadow/plan.json"), &fresh, 1, &p, Some(&LiveGate { max_window_ticks: 10 })).unwrap_err();
        assert!(e.iter().any(|r| r.message.contains("spans 1000 ticks, more than the 10")), "{e:?}");
    }

    #[test]
    fn the_ledger_and_the_peer_check_hold_without_a_network() {
        assert_eq!(served_ledger_in(Some("/s".into()), Some("/h".into())), PathBuf::from("/s/served-keys"));
        assert_eq!(served_ledger_in(None, Some("/h".into())), PathBuf::from("/h/.local/state/battlebrain/served-keys"));
        assert!(served_ledger().ends_with("served-keys"));
        assert!(loopback_only("127.0.0.1:1".parse().unwrap()).is_ok());
        assert_eq!(loopback_only("192.0.2.1:80".parse().unwrap()).unwrap_err(), "refused a non-loopback peer 192.0.2.1:80");
        assert!(claim_key(Path::new("/"), "k").is_err(), "a ledger path with no parent directory cannot be written");
        let report = report(&json!({"id": "r", "mode": "shadow", "comparison": {}, "findings": ["one"]}));
        assert!(report.contains("**Finding:** one") && report.contains("never submitted"));
    }

    #[test]
    fn game_dates_add_days_on_the_gregorian_calendar() {
        assert_eq!(bb::add_days("1961-01-01", 31).as_deref(), Some("1961-02-01"));
        assert_eq!(bb::add_days("1960-02-28", 1).as_deref(), Some("1960-02-29"));
        assert_eq!(bb::add_days("1961-12-31", 1).as_deref(), Some("1962-01-01"));
        assert_eq!(bb::add_days("x", 1), None);
    }
}
