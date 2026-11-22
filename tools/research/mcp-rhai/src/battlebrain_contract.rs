//! RESEARCH-ONLY: the typed contracts of the Battlebrain freight-line decision (I0095,
//! research/battlebrain/README.md).
//!
//! One source of truth. The record types, statuses, transitions, writers and the AlgoGlyph
//! invocation envelope are defined here once. The JSON Schemas under tools/research/schema/ are
//! generated from these definitions (`battlebrain schema <name>`), and a test refuses a committed
//! schema that differs from the generated one. Every validator fails closed: an unknown schema
//! version, an unknown type, a kernel absent from the discovered catalogue, a missing provenance
//! link, an output that breaks its domain check or an illegal transition is refused with a
//! structured reason (`Refusal`), never repaired or guessed.
//!
//! The blackboard is the Majordomus mesh journal (owner, 2026-11-20, research/agent-loop.md). A
//! record here is the body of one mesh record; its mesh kind is derived from its type
//! (`mesh_kind`), never stored separately. Nothing in this module touches a game.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// The version of a blackboard record body.
pub const RECORD_SCHEMA: &str = "battlebrain-record/v1";
/// The version of one AlgoGlyph kernel call's envelope.
pub const INVOCATION_SCHEMA: &str = "algoglyph-invocation/v1";
/// The version of the decision block a directive may carry.
/// v2 (I0117): the block names the digest of the exact settings it authorises (`settings_digest`)
/// and the game tick of the snapshot it was decided on (`snapshot_tick`): an executor refuses it in
/// a game whose clock is behind that tick (a reloaded earlier save or another world).
pub const DECISION_SCHEMA: &str = "battlebrain-decision/v2";
/// PrismaticAI's freight-line slots (info.nut declares fr_* then fr2_* to fr16_*; a test reads it).
pub const FREIGHT_SLOTS: usize = 16;

/// The setting prefix of freight-line slot `i` (1-based): "fr_" for 1, "fr<i>_" after it.
pub fn freight_slot_prefix(i: usize) -> String {
    if i <= 1 { "fr_".to_string() } else { format!("fr{i}_") }
}

/// The digest a decision block names for the settings string it authorises.
pub fn settings_digest(settings: &str) -> String {
    digest(&Value::String(settings.to_string()))
}

/// The settings that clear every freight-line slot after `first` (fr<i>_from=-1): a decision owns
/// all slots, so whatever it does not set it clears.
pub fn cleared_slots(first: usize) -> String {
    (first..=FREIGHT_SLOTS).map(|i| format!("{}from=-1", freight_slot_prefix(i))).collect::<Vec<_>>().join(",")
}

/// The policy id of a rollback decision.
pub const ROLLBACK_POLICY: (&str, &str) = ("rollback", "1");

/// The rollback of a live decision, as a live decision of its own (I0117 round 2): the bot plans
/// its own freight lines again (planned_freight_only=0) and every slot is cleared. It is decided on
/// `snapshot` (a snapshot record of the company's game), keyed `<prior key>:rollback` when the
/// rolled-back block is given (else `<instance>:<company>:<date>:rollback`), stale with the
/// snapshot, and applied through the executor's live path like any decision: one keyed pull, one
/// approval, the world checked first. Returns the directive and the records it rests on.
pub fn rollback_directive(snapshot: &Value, company: i64, prior: Option<&Value>, version: &str) -> Result<Value, Vec<Refusal>> {
    let refuse = |code: &'static str, path: &str, msg: String| vec![Refusal::new(code, path, msg)];
    match validate_record(snapshot) {
        Ok("snapshot") => {}
        Ok(other) => return Err(refuse("not_a_snapshot", "type", format!("--snapshot is a {other} record"))),
        Err(e) => return Err(e),
    }
    if snapshot["scope"]["company"].as_i64() != Some(company) {
        return Err(refuse("scope_mismatch", "scope.company", format!("the snapshot is scoped to company {}, not {company}", snapshot["scope"]["company"])));
    }
    if let Some(p) = prior {
        validate_decision_block(p)?;
        if p["mode"] != "live" {
            return Err(refuse("not_live", "mode", "only a live decision is rolled back; a shadow one never reached the game".into()));
        }
    }
    let (scope, at) = (snapshot["scope"].clone(), snapshot["at"].clone());
    let snap_id = snapshot["id"].as_str().map(str::to_string).unwrap_or_else(|| record_id(snapshot));
    let date = at["date"].as_str().unwrap_or_default();
    let max_age = snapshot.pointer("/body/max_age_days").and_then(Value::as_i64).unwrap_or(1);
    let valid_until = add_days(date, max_age).ok_or_else(|| refuse("invalid_input", "at.date", format!("the snapshot date {date:?} is not a date")))?;
    let key = match prior {
        Some(p) => format!("{}:rollback", p["idempotency_key"].as_str().unwrap_or_default()),
        None => format!("{}:{company}:{date}:rollback", scope["instance"].as_str().unwrap_or_default()),
    };
    let hold = record("candidate", "proposed", &scope, &at, (AUTHORITY, version), &[json!(snap_id)],
        json!({"action": {"kind": "hold"}, "assumptions": ["the bot plans its own freight lines again; no slot is handed to it"]}));
    let mut derived = vec![json!(snap_id), hold["id"].clone()];
    if let Some(p) = prior {
        derived.push(p["decision"].clone());
    }
    let decision = record("decision", "decided", &scope, &at, (AUTHORITY, version), &derived, json!({
        "policy": {"id": ROLLBACK_POLICY.0, "version": ROLLBACK_POLICY.1, "params": {"rolls_back": prior.map(|p| p["decision"].clone())}},
        "goal": "back to the verified baseline: planned_freight_only=0 and every freight-line slot cleared", "snapshot": snap_id,
        "selected": hold["id"], "rejected": [], "idempotency_key": key, "approval": "operator", "valid_until": valid_until}));
    let settings = format!("planned_freight_only=0,{}", cleared_slots(1));
    let block = json!({"schema": DECISION_SCHEMA, "decision": decision["id"], "snapshot_digest": snapshot["body"]["digest"],
        "snapshot_tick": at["tick"], "idempotency_key": key, "policy": format!("{}@{}", ROLLBACK_POLICY.0, ROLLBACK_POLICY.1),
        "valid_until": valid_until, "settings_digest": settings_digest(&settings), "mode": "live"});
    let mut errs = Vec::new();
    for r in [&hold, &decision] {
        if let Err(e) = validate_record(r) {
            errs.extend(e);
        }
    }
    if let Err(e) = validate_decision_block(&block) {
        errs.extend(e);
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(json!({"mode": "live", "directive": {"company_id": company, "settings": settings, "decision": block}, "records": [hold, decision]}))
}
/// The modes a decision block declares. An executor applies only `live`; `shadow` is refused
/// before any step (I0096), so a shadow decision cannot reach a game even if it is submitted.
pub const DECISION_MODES: &[&str] = &["shadow", "live"];
/// The one component allowed to write a decision (the single strategic authority).
pub const AUTHORITY: &str = "battlebrain";
/// The commands whose answer is the kernel catalogue and a kernel call; recorded, never guessed.
pub const CATALOGUE_SOURCE: &str = "npm run --silent flow -- kernels";

/// Record types, their mesh kind, their statuses and who may append them.
/// (type, mesh kind, statuses, initial status, writers; "*" = any producer but the authority)
pub const TYPES: &[(&str, &str, &[&str], &str, &[&str])] = &[
    ("snapshot", "observation", &["current", "superseded"], "current", &["*"]),
    ("inference", "hypothesis", &["valid", "invalidated"], "valid", &["*", AUTHORITY]),
    ("candidate", "plan", &["proposed", "selected", "rejected"], "proposed", &["*", AUTHORITY]),
    ("decision", "decision", &["decided", "submitted", "applied", "refused", "stale", "expired"], "decided", &[AUTHORITY]),
    ("outcome", "outcome", &["observed", "verified", "unverified"], "observed", &["*"]),
];

/// Legal status transitions (type, from, to). A transition is a new record that supersedes the
/// previous one; history is append-only. Terminal statuses have no outgoing edge.
pub const TRANSITIONS: &[(&str, &str, &str)] = &[
    ("snapshot", "current", "superseded"),
    ("inference", "valid", "invalidated"),
    ("candidate", "proposed", "selected"),
    ("candidate", "proposed", "rejected"),
    ("decision", "decided", "submitted"),
    ("decision", "decided", "expired"),
    ("decision", "submitted", "applied"),
    ("decision", "submitted", "refused"),
    ("decision", "submitted", "stale"),
    ("outcome", "observed", "verified"),
    ("outcome", "observed", "unverified"),
];

/// The actions a freight-line candidate or decision may name.
pub const ACTIONS: &[&str] = &["freight_line", "hold"];
/// The settings a freight_line action carries (the bot's own fr_* slot keys).
pub const FREIGHT_SETTINGS: &[&str] = &["fr_from", "fr_to", "fr_cargo"];
/// Invocation outcomes.
pub const INVOCATION_STATUSES: &[&str] = &["ok", "refused", "timeout", "invalid_output", "unknown_kernel"];
/// Units an amount may carry.
pub const UNITS: &[&str] = &["money", "money_per_year", "tiles", "game_days", "ms", "items", "ratio"];

/// Why a contract refused an input: a stable code, where, and what.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

impl Refusal {
    pub fn new(code: &'static str, path: &str, message: impl Into<String>) -> Refusal {
        Refusal { code, path: path.into(), message: message.into() }
    }
    pub fn to_json(&self) -> Value {
        json!({"code": self.code, "path": self.path, "message": self.message})
    }
}

/// The value with every object's keys sorted, serialised compactly: the input of every digest,
/// independent of serde_json's map order feature.
pub fn canonical(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for k in keys {
                    out.insert(k.clone(), sort(&m[k]));
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    let mut s = String::new();
    write_canonical(&sort(v), &mut s);
    s
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            out.push('{');
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*k).clone()).to_string());
                out.push(':');
                write_canonical(&m[*k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// `sha256:<hex>` of the canonical form.
pub fn digest(v: &Value) -> String {
    let mut h = Sha256::new();
    h.update(canonical(v).as_bytes());
    format!("sha256:{:x}", h.finalize())
}

/// A record's stable identity: the digest of everything but its id.
pub fn record_id(record: &Value) -> String {
    let mut r = record.clone();
    if let Some(m) = r.as_object_mut() {
        m.remove("id");
    }
    digest(&r).replacen("sha256:", "bb:", 1)
}

/// One record body with its id: the only way this crate builds a record, so every writer derives
/// the id the same way (the digest of everything else).
pub fn record(kind: &str, status: &str, scope: &Value, at: &Value, producer: (&str, &str), derived_from: &[Value], body: Value) -> Value {
    let mut r = json!({"schema": RECORD_SCHEMA, "type": kind, "status": status, "scope": scope, "at": at,
        "producer": {"id": producer.0, "version": producer.1}, "body": body});
    if kind != "snapshot" || !derived_from.is_empty() {
        r["derived_from"] = json!(derived_from);
    }
    r["id"] = json!(record_id(&r));
    r
}

/// A world-graph node id ("industry:41", as openttd://world/graph and the planner carry it), a
/// plain number, or a number in text, as the game's number (41). The one normalisation of a graph
/// id in this crate: the planner's candidates, the bot's fr_* settings and battlebrain read ids
/// through it.
pub fn graph_number(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.rsplit(':').next()?.trim().parse().ok(),
        _ => None,
    }
}

/// Days since 1970-01-01 of a civil date, and back (Howard Hinnant's algorithm): the game calendar
/// is the proleptic Gregorian one.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
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

pub fn add_days(date: &str, days: i64) -> Option<String> {
    let mut it = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    let (y, m, d) = civil_from_days(days_from_civil(y, m, d) + days);
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Whole days from game date `a` to game date `b` (negative when `b` is earlier).
pub fn days_between(a: &str, b: &str) -> Option<i64> {
    let parse = |d: &str| -> Option<i64> {
        let mut it = d.split('-').map(|p| p.parse::<i64>().ok());
        Some(days_from_civil(it.next()??, it.next()??, it.next()??))
    };
    Some(parse(b)? - parse(a)?)
}

/// The mesh kind a record type is written as.
pub fn mesh_kind(kind: &str) -> Option<&'static str> {
    TYPES.iter().find(|t| t.0 == kind).map(|t| t.1)
}

/// Whether `from` -> `to` is a legal transition for records of `kind`.
pub fn transition(kind: &str, from: &str, to: &str) -> Result<(), Refusal> {
    let Some(t) = TYPES.iter().find(|t| t.0 == kind) else {
        return Err(Refusal::new("unknown_type", "type", format!("no record type {kind:?}")));
    };
    for s in [from, to] {
        if !t.2.contains(&s) {
            return Err(Refusal::new("unknown_status", "status", format!("{kind} has no status {s:?}")));
        }
    }
    if TRANSITIONS.contains(&(kind, from, to)) {
        Ok(())
    } else {
        Err(Refusal::new("illegal_transition", "status", format!("{kind}: {from} -> {to} is not a legal transition")))
    }
}

fn is_digest(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.len() == 71 && s.starts_with("sha256:") && s[7..].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
}

fn is_record_ref(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.len() == 67 && s.starts_with("bb:") && s[3..].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
}

fn is_game_date(v: &Value) -> bool {
    let Some(s) = v.as_str() else { return false };
    let b = s.as_bytes();
    s.len() == 10 && b[4] == b'-' && b[7] == b'-' && s.bytes().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn nonempty_str(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.trim().is_empty())
}

struct Check<'a> {
    errs: Vec<Refusal>,
    root: &'a Value,
}

impl<'a> Check<'a> {
    fn get(&self, path: &str) -> Option<&'a Value> {
        let mut v = self.root;
        for p in path.split('.') {
            v = v.get(p)?;
        }
        Some(v)
    }
    fn need(&mut self, path: &str, ok: fn(&Value) -> bool, what: &str) -> Option<&'a Value> {
        match self.get(path) {
            None => {
                self.errs.push(Refusal::new("missing_field", path, format!("{path} is required ({what})")));
                None
            }
            Some(v) if !ok(v) => {
                self.errs.push(Refusal::new("invalid_field", path, format!("{path} must be {what}, got {v}")));
                None
            }
            Some(v) => Some(v),
        }
    }
    fn one_of(&mut self, path: &str, allowed: &[&str]) -> Option<&'a str> {
        let v = self.need(path, |v| v.is_string(), "a string")?;
        let s = v.as_str().unwrap_or_default();
        if allowed.contains(&s) {
            Some(s)
        } else {
            self.errs.push(Refusal::new("invalid_field", path, format!("{path} must be one of {allowed:?}, got {s:?}")));
            None
        }
    }
    fn amount(&mut self, path: &str) {
        if let Some(a) = self.need(path, |v| v.is_object(), "an amount {value, unit}") {
            if !a.get("value").is_some_and(|v| v.is_number()) {
                self.errs.push(Refusal::new("invalid_field", &format!("{path}.value"), "an amount's value must be a number"));
            }
            match a.get("unit").and_then(Value::as_str) {
                Some(u) if UNITS.contains(&u) => {}
                _ => self.errs.push(Refusal::new("unknown_unit", &format!("{path}.unit"), format!("an amount's unit must be one of {UNITS:?}"))),
            }
        }
    }
}

/// Validate one blackboard record body. On success, its type.
pub fn validate_record(r: &Value) -> Result<&'static str, Vec<Refusal>> {
    let mut c = Check { errs: vec![], root: r };
    match r.get("schema").and_then(Value::as_str) {
        Some(RECORD_SCHEMA) => {}
        Some(other) => return Err(vec![Refusal::new("unknown_schema", "schema", format!("{other:?} is not {RECORD_SCHEMA:?}"))]),
        None => return Err(vec![Refusal::new("missing_field", "schema", format!("schema is required ({RECORD_SCHEMA:?})"))]),
    }
    let type_names: Vec<&str> = TYPES.iter().map(|t| t.0).collect();
    let Some(kind) = c.one_of("type", &type_names) else { return Err(c.errs) };
    let t = TYPES.iter().find(|t| t.0 == kind).expect("type checked above");
    c.one_of("status", t.2);
    c.need("scope.instance", nonempty_str, "a non-empty instance name");
    c.need("scope.company", |v| v.as_i64().is_some_and(|n| (0..=14).contains(&n)), "a company id 0..14");
    c.need("at.tick", |v| v.as_u64().is_some(), "a game tick >= 0");
    c.need("at.date", is_game_date, "a game date YYYY-MM-DD");
    let producer = c.need("producer.id", nonempty_str, "the producing component").and_then(Value::as_str);
    c.need("producer.version", nonempty_str, "the producer's version");
    if let Some(p) = producer {
        let allowed = t.4.contains(&p) || (t.4.contains(&"*") && p != AUTHORITY);
        if !allowed {
            c.errs.push(Refusal::new("writer_not_allowed", "producer.id", format!("{p:?} may not append a {kind} (writers: {:?})", t.4)));
        }
    }
    if let Some(id) = r.get("id") {
        if !is_record_ref(id) {
            c.errs.push(Refusal::new("invalid_field", "id", "id must be bb:<sha256 hex>"));
        } else if id.as_str() != Some(record_id(r).as_str()) {
            c.errs.push(Refusal::new("id_mismatch", "id", format!("id is not the digest of the record ({})", record_id(r))));
        }
    }
    if let Some(s) = r.get("supersedes") {
        if !is_record_ref(s) {
            c.errs.push(Refusal::new("invalid_field", "supersedes", "supersedes must name a record bb:<sha256 hex>"));
        }
    }
    let derived = r.get("derived_from").and_then(Value::as_array);
    match (kind, derived) {
        ("snapshot", _) => {}
        (_, None) => c.errs.push(Refusal::new("missing_provenance", "derived_from", format!("a {kind} must name the records it is derived from"))),
        (_, Some(d)) if d.is_empty() => c.errs.push(Refusal::new("missing_provenance", "derived_from", format!("a {kind} must name at least one record"))),
        (_, Some(d)) => {
            if !d.iter().all(is_record_ref) {
                c.errs.push(Refusal::new("invalid_field", "derived_from", "every derived_from entry must be bb:<sha256 hex>"));
            }
        }
    }
    match kind {
        "snapshot" => {
            c.need("body.digest", is_digest, "sha256:<hex> of the observed fields");
            if let Some(srcs) = c.need("body.sources", |v| v.as_array().is_some_and(|a| !a.is_empty()), "a non-empty list of sources") {
                for (i, s) in srcs.as_array().into_iter().flatten().enumerate() {
                    if !s.get("uri").is_some_and(nonempty_str) || !s.get("digest").is_some_and(is_digest) {
                        c.errs.push(Refusal::new("invalid_field", &format!("body.sources.{i}"), "a source is {uri, digest: sha256:<hex>}"));
                    }
                }
            }
            c.need("body.max_age_days", |v| v.as_u64().is_some_and(|n| n > 0), "a positive number of game days");
            if let Some(w) = r.pointer("/body/window") {
                let (f, t) = (w["tick_from"].as_u64(), w["tick_to"].as_u64());
                let ok = matches!((f, t), (Some(f), Some(t)) if f <= t) && w["coherent"].as_bool() == Some(f.is_some() && f == t);
                if !ok {
                    c.errs.push(Refusal::new("invalid_field", "body.window", "a window is {tick_from <= tick_to, coherent: tick_from == tick_to}"));
                }
            }
        }
        "inference" => {
            c.need("body.claim", nonempty_str, "the claim");
            c.need("body.method.id", nonempty_str, "the method");
            c.need("body.method.version", nonempty_str, "the method's version");
            c.need("body.params", Value::is_object, "the parameters (an object, may be empty)");
            c.need("body.valid_until", is_game_date, "a game date");
            if let Some(conf) = r.pointer("/body/confidence") {
                if !conf.as_f64().is_some_and(|x| (0.0..=1.0).contains(&x)) {
                    c.errs.push(Refusal::new("invalid_field", "body.confidence", "confidence must be within 0..1"));
                }
            }
            // A capability match carries its capability-match/v1 decision as its params.
            if r.pointer("/body/method/id").and_then(Value::as_str) == Some(MATCH_METHOD) {
                if let Err(es) = validate_match(r.pointer("/body/params").unwrap_or(&Value::Null)) {
                    c.errs.extend(es.into_iter().map(|e| Refusal::new(e.code, &format!("body.params.{}", e.path), e.message)));
                }
            }
        }
        "candidate" => {
            let action = c.one_of("body.action.kind", ACTIONS);
            if action == Some("freight_line") {
                for k in FREIGHT_SETTINGS {
                    c.need(&format!("body.action.settings.{k}"), |v| v.as_i64().is_some_and(|n| n >= 0), "a non-negative id");
                }
                c.amount("body.estimate.cost");
                c.amount("body.estimate.profit");
                c.need("body.inputs_digest", is_digest, "sha256:<hex> of the exact input the producer estimated it from");
            }
            if let Some(inv) = r.pointer("/body/invocations") {
                if !inv.as_array().is_some_and(|a| a.iter().all(is_digest)) {
                    c.errs.push(Refusal::new("invalid_field", "body.invocations", "invocations are the digests of algoglyph-invocation/v1 envelopes"));
                }
            }
            c.need("body.assumptions", |v| v.as_array().is_some_and(|a| a.iter().all(|x| x.is_string())), "a list of assumptions (may be empty)");
        }
        "decision" => {
            c.need("body.policy.id", nonempty_str, "the policy");
            c.need("body.policy.version", nonempty_str, "the policy's version");
            c.need("body.policy.params", Value::is_object, "the policy's parameters");
            c.need("body.goal", nonempty_str, "the objective pursued");
            c.need("body.snapshot", is_record_ref, "the snapshot record decided on");
            c.need("body.selected", is_record_ref, "the selected candidate record");
            c.need("body.idempotency_key", nonempty_str, "an idempotency key");
            c.one_of("body.approval", &["operator"]);
            c.need("body.valid_until", is_game_date, "the game date after which the decision is stale");
            if let Some(rej) = c.need("body.rejected", Value::is_array, "the rejected alternatives (may be empty)") {
                for (i, x) in rej.as_array().into_iter().flatten().enumerate() {
                    if !x.get("candidate").is_some_and(is_record_ref) || !x.get("reason").is_some_and(nonempty_str) {
                        c.errs.push(Refusal::new("invalid_field", &format!("body.rejected.{i}"), "a rejected alternative is {candidate: bb:..., reason}"));
                    }
                }
            }
            if let (Some(s), Some(d)) = (r.pointer("/body/snapshot"), derived) {
                if !d.contains(s) {
                    c.errs.push(Refusal::new("missing_provenance", "derived_from", "a decision must be derived from the snapshot it names"));
                }
            }
        }
        "outcome" => {
            c.need("body.decision", is_record_ref, "the decision record it reports on");
            c.need("body.executor.id", nonempty_str, "the executor");
            c.one_of("body.result", &["applied", "refused", "stale", "failed"]);
            c.need("body.observed", Value::is_object, "the observed effect (an object)");
        }
        _ => unreachable!("types are closed"),
    }
    if c.errs.is_empty() {
        Ok(t.0)
    } else {
        Err(c.errs)
    }
}

/// One kernel as the AlgoGlyph runner lists it: exactly the fields its answer carries. `input` is
/// the runner's own description of the input (a shape such as `{from: {x, y}, ...}`, or prose);
/// graph kernels carry `goal`, `shape` and `params` instead. Nothing here is added by this fork.
#[derive(Clone, Debug, PartialEq)]
pub struct Kernel {
    pub id: String,
    pub engine: String,
    pub input: Option<String>,
    pub goal: Option<String>,
    pub params: Value,
}

/// The kernels the AlgoGlyph runner reports (`flow -- kernels`), with the digest of that answer
/// and the checkout it came from. Built from the runner's answer only: no kernel is known here
/// that the runner did not list.
#[derive(Clone, Debug, PartialEq)]
pub struct Catalogue {
    pub kernels: Vec<Kernel>,
    pub digest: String,
    pub commit: String,
}

impl Catalogue {
    /// Parse the runner's answer: a list of objects with at least `id` and `engine`.
    pub fn from_runner(answer: &Value, commit: &str) -> Result<Catalogue, Refusal> {
        let list = answer.as_array().ok_or_else(|| Refusal::new("invalid_catalogue", "", "the runner's kernel list is not an array"))?;
        let mut kernels = Vec::new();
        for (i, k) in list.iter().enumerate() {
            match (k.get("id").and_then(Value::as_str), k.get("engine").and_then(Value::as_str)) {
                (Some(id), Some(engine)) if !id.is_empty() => kernels.push(Kernel {
                    id: id.to_string(),
                    engine: engine.to_string(),
                    input: k.get("input").and_then(Value::as_str).map(str::to_string),
                    goal: k.get("goal").and_then(Value::as_str).map(str::to_string),
                    params: k.get("params").cloned().unwrap_or(Value::Null),
                }),
                _ => return Err(Refusal::new("invalid_catalogue", &i.to_string(), "a kernel entry needs a non-empty id and an engine")),
            }
        }
        if kernels.is_empty() {
            return Err(Refusal::new("invalid_catalogue", "", "the runner lists no kernel"));
        }
        if !(commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err(Refusal::new("invalid_catalogue", "commit", "the checkout commit must be 40 hex digits"));
        }
        Ok(Catalogue { kernels, digest: digest(answer), commit: commit.to_ascii_lowercase() })
    }
    pub fn kernel(&self, id: &str) -> Option<&Kernel> {
        self.kernels.iter().find(|k| k.id == id)
    }
    pub fn engine(&self, id: &str) -> Option<&str> {
        self.kernel(id).map(|k| k.engine.as_str())
    }
}

/// Validate one invocation envelope against the catalogue it claims to come from.
pub fn validate_invocation(inv: &Value, cat: &Catalogue) -> Result<(), Vec<Refusal>> {
    let mut c = Check { errs: vec![], root: inv };
    match inv.get("schema").and_then(Value::as_str) {
        Some(INVOCATION_SCHEMA) => {}
        Some(other) => return Err(vec![Refusal::new("unknown_schema", "schema", format!("{other:?} is not {INVOCATION_SCHEMA:?}"))]),
        None => return Err(vec![Refusal::new("missing_field", "schema", format!("schema is required ({INVOCATION_SCHEMA:?})"))]),
    }
    if let Some(id) = c.need("kernel.id", nonempty_str, "the kernel id").and_then(Value::as_str) {
        match cat.engine(id) {
            None => c.errs.push(Refusal::new("unknown_kernel", "kernel.id", format!("{id:?} is not in the discovered catalogue {}", cat.digest))),
            Some(e) if inv.pointer("/kernel/engine").and_then(Value::as_str) != Some(e) => {
                c.errs.push(Refusal::new("invalid_field", "kernel.engine", format!("the catalogue lists {id} with engine {e:?}")))
            }
            Some(_) => {}
        }
    }
    if let Some(d) = c.need("catalogue.digest", is_digest, "the digest of the runner's kernel list").and_then(Value::as_str) {
        if d != cat.digest {
            c.errs.push(Refusal::new("stale_catalogue", "catalogue.digest", format!("the envelope names another catalogue than the discovered one ({})", cat.digest)));
        }
    }
    if inv.pointer("/catalogue/commit").and_then(Value::as_str) != Some(cat.commit.as_str()) {
        c.errs.push(Refusal::new("stale_catalogue", "catalogue.commit", format!("the envelope names another checkout than {}", cat.commit)));
    }
    c.need("input.digest", is_digest, "sha256:<hex> of the exact input");
    if let Some(u) = c.need("input.units", Value::is_object, "the unit of every amount in the input") {
        for (k, v) in u.as_object().into_iter().flatten() {
            if !v.as_str().is_some_and(|s| UNITS.contains(&s)) {
                c.errs.push(Refusal::new("unknown_unit", &format!("input.units.{k}"), format!("unit must be one of {UNITS:?}")));
            }
        }
    }
    c.need("budget.timeout_ms", |v| v.as_u64().is_some_and(|n| n > 0), "a positive timeout");
    match c.one_of("status", INVOCATION_STATUSES) {
        Some("ok") => {
            c.need("output.digest", is_digest, "sha256:<hex> of the output");
        }
        Some(_) => {
            c.need("errors", |v| v.as_array().is_some_and(|a| !a.is_empty()), "the runner's errors");
        }
        None => {}
    }
    if c.errs.is_empty() {
        Ok(())
    } else {
        Err(c.errs)
    }
}

// ---------------------------------------------------------------------------------------------
// Capabilities (Phase 05, I0098): what Battlebrain needs computed, as typed requirements, and the
// kernels of the discovered catalogue that satisfy them. The requirement table is the one place a
// capability is declared; the adapter registry is the one explicit list of kernel ids in this
// crate's contract layer, at the protocol boundary where AlgoGlyph publishes no schema yet.
// ---------------------------------------------------------------------------------------------

/// The version of a capability-match decision (the `params` of a capability-match inference).
pub const MATCH_SCHEMA: &str = "capability-match/v1";
/// The method id of the inference record a capability match is written as.
pub const MATCH_METHOD: &str = "capability-match";
/// The kinds a requirement field may have. `amount` kinds carry a unit from `UNITS`; the others
/// carry none.
pub const FIELD_KINDS: &[&str] = &["amount", "amount[]", "number", "number[]", "index[]", "tile", "resource", "status", "list"];
/// Why a capability resolved to no kernel.
pub const MATCH_REFUSALS: &[&str] = &["unknown_capability", "no_adapter", "kernel_expired", "no_match", "ambiguous_capability"];
/// Why one candidate kernel was rejected for a capability.
pub const CANDIDATE_REJECTIONS: &[&str] = &["kernel_absent", "engine_mismatch", "unmapped_field", "unknown_field", "input_shape_drift"];
/// The ranking among compatible kernels, in order. It is deterministic and uses only the adapter
/// and the catalogue; a tie after every rule is refused, never broken by name.
pub const RANKING: &[&str] = &[
    "exact: a kernel whose adapter states it returns an optimal answer ranks above a heuristic one",
    "verified: a kernel whose catalogue input shape declares every key the adapter maps ranks above one whose shape is opaque",
    "a tie after both rules is refused as ambiguous_capability; a kernel id never breaks a tie",
];

/// One typed field of a requirement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    pub name: &'static str,
    pub kind: &'static str,
    pub unit: Option<&'static str>,
    pub doc: &'static str,
}

/// A capability Battlebrain needs computed: a goal with typed inputs and outputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capability {
    pub id: &'static str,
    pub goal: &'static str,
    /// Where Battlebrain consumes the answer.
    pub used_by: &'static str,
    pub inputs: &'static [Field],
    pub outputs: &'static [Field],
}

const fn field(name: &'static str, kind: &'static str, unit: Option<&'static str>, doc: &'static str) -> Field {
    Field { name, kind, unit, doc }
}

/// The capabilities, declared once.
pub const CAPABILITIES: &[Capability] = &[
    Capability {
        id: "portfolio-within-budget",
        goal: "the set of items with the highest total value whose total cost fits a budget",
        used_by: "the planner's portfolio (cargo.rs portfolio) every freight-line candidate rests on",
        inputs: &[
            field("costs", "amount[]", Some("money"), "each item's investment"),
            field("values", "number[]", None, "each item's value (the planner's score), one scale for all items"),
            field("budget", "amount", Some("money"), "what may be spent in total"),
        ],
        outputs: &[
            field("chosen", "index[]", None, "the chosen items' indices, each at most once"),
            field("total_value", "number", None, "the sum of the chosen items' values"),
        ],
    },
    Capability {
        id: "rail-line-price",
        goal: "what a rail line between two station tiles costs to build on this map, or that it has no route",
        used_by: "a freight-line candidate's track price (linecost.rs price; policy require_priced)",
        inputs: &[
            field("from", "tile", None, "the first station site's end tile and exit {x, y, dir}"),
            field("to", "tile", None, "the second station site's end tile and entry {x, y, dir}"),
            field("map", "resource", None, "openttd://map data"),
            field("prices", "resource", None, "openttd://economy/prices data: only with it is the price in game money"),
        ],
        outputs: &[
            field("status", "status", None, "found | unreachable | budget; only found prices a line"),
            field("price", "amount", Some("money"), "what the route costs to build, in game money"),
            field("pieces", "list", None, "the track pieces, bridges and tunnels to lay"),
        ],
    },
];

/// The capability with this id.
pub fn capability(id: &str) -> Option<&'static Capability> {
    CAPABILITIES.iter().find(|c| c.id == id)
}

/// An adapter: the declaration that one kernel of the catalogue satisfies one capability, how each
/// requirement field maps onto the kernel's input and output, and the domain check its output must
/// pass. It is the one explicit kernel list, at a protocol boundary: the runner's answers carry no
/// schema. Owner: `ADAPTER_OWNER`; retirement: `ADAPTER_RETIREMENT`.
#[derive(Clone, Copy)]
pub struct Adapter {
    pub capability: &'static str,
    pub kernel: &'static str,
    /// The engine the catalogue must list the kernel with.
    pub engine: &'static str,
    /// (requirement input, kernel input key, conversion).
    pub inputs: &'static [(&'static str, &'static str, &'static str)],
    /// (requirement output, path in the kernel's answer).
    pub outputs: &'static [(&'static str, &'static str)],
    /// Whether the kernel returns an optimal answer (the first ranking rule).
    pub exact: bool,
    /// Why `exact` holds, from the kernel's own source.
    pub exact_because: &'static str,
    pub check: fn(&Value, &Value) -> Result<(), Refusal>,
}

/// Who owns the adapter registry.
pub const ADAPTER_OWNER: &str = "battlebrain (tools/research/mcp-rhai/src/battlebrain_contract.rs, research/battlebrain/README.md)";
/// When the adapter registry retires.
pub const ADAPTER_RETIREMENT: &str = "when AlgoGlyph publishes, per kernel, a versioned input and output schema with units (research/battlebrain/algoglyph-contract-proposal.md): the matcher then reads the mapping from the catalogue, and an adapter shrinks to its domain check or goes";

/// The adapters, declared once. Nothing else in the contract layer names a kernel.
pub const ADAPTERS: &[Adapter] = &[
    Adapter {
        capability: "portfolio-within-budget",
        kernel: "knapsack-dp",
        engine: "grid",
        inputs: &[
            ("costs", "weights", "ceil(money / 1000): the kernel counts integer thousands (cargo.rs portfolio)"),
            ("values", "values", "as is"),
            ("budget", "capacity", "floor(money / 1000)"),
        ],
        outputs: &[("chosen", "result.items"), ("total_value", "result.best")],
        exact: true,
        exact_because: "dynamic programming over integer capacities: the optimum of the 0/1 knapsack",
        check: check_knapsack,
    },
    Adapter {
        capability: "rail-line-price",
        kernel: "track-planning",
        engine: "custom",
        inputs: &[
            ("from", "from", "the station tile itself, leaving by its exit (linecost.rs endpoints)"),
            ("to", "to", "the station tile itself, entered against its exit"),
            ("map", "map", "as read; ways, terrain and bridges ride along (hub_algoglyph.rs track_input)"),
            ("prices", "prices", "as read; without it result.money is null and result.cost is in tile units"),
        ],
        outputs: &[("status", "result.status"), ("price", "result.money"), ("pieces", "result.pieces")],
        exact: true,
        exact_because: "A* with an admissible heuristic, tested against Dijkstra (algoglyph src/plates/track-planning/plan.ts): the least-cost route of its cost model",
        check: check_track,
    },
];

fn check_knapsack(input: &Value, output: &Value) -> Result<(), Refusal> {
    let res = output.get("result").unwrap_or(output);
    let w = input.get("weights").and_then(Value::as_array).ok_or_else(|| Refusal::new("invalid_input", "weights", "knapsack input has no weights"))?;
    let vals = input.get("values").and_then(Value::as_array).ok_or_else(|| Refusal::new("invalid_input", "values", "knapsack input has no values"))?;
    let cap = input.get("capacity").and_then(Value::as_u64).ok_or_else(|| Refusal::new("invalid_input", "capacity", "knapsack input has no capacity"))?;
    let items = res.get("items").and_then(Value::as_array).ok_or_else(|| Refusal::new("invalid_output", "result.items", "the kernel answered no items"))?;
    let best = res.get("best").and_then(Value::as_f64).ok_or_else(|| Refusal::new("invalid_output", "result.best", "the kernel answered no best value"))?;
    let mut seen = std::collections::BTreeSet::new();
    let (mut weight, mut value) = (0u64, 0f64);
    for it in items {
        let i = it.as_u64().ok_or_else(|| Refusal::new("invalid_output", "result.items", "an item is not an index"))? as usize;
        if i >= w.len() || i >= vals.len() || !seen.insert(i) {
            return Err(Refusal::new("invalid_output", "result.items", format!("item {i} is out of range or repeated")));
        }
        weight += w[i].as_u64().unwrap_or(u64::MAX / 2);
        value += vals[i].as_f64().unwrap_or(f64::NAN);
    }
    if weight > cap {
        return Err(Refusal::new("invalid_output", "result.items", format!("chosen weight {weight} exceeds the capacity {cap}")));
    }
    if (value - best).abs() > 1e-6 * best.abs().max(1.0) {
        return Err(Refusal::new("invalid_output", "result.best", format!("best {best} is not the chosen items' value {value}")));
    }
    Ok(())
}

fn check_track(input: &Value, output: &Value) -> Result<(), Refusal> {
    let res = output.get("result").unwrap_or(output);
    // The kernel's statuses are found | unreachable | budget (algoglyph
    // src/plates/track-planning/plan.ts, Plan.status); only a found plan prices a line.
    let status = res.get("status").and_then(Value::as_str).unwrap_or_default();
    if status != "found" {
        return Err(Refusal::new("invalid_output", "result.status", format!("the plan's status is {status:?}, not \"found\"")));
    }
    if !res.get("cost").and_then(Value::as_f64).is_some_and(|c| c.is_finite() && c >= 0.0) {
        return Err(Refusal::new("invalid_output", "result.cost", "the plan has no finite non-negative cost"));
    }
    if !res.get("pieces").and_then(Value::as_array).is_some_and(|p| !p.is_empty()) {
        return Err(Refusal::new("invalid_output", "result.pieces", "the plan lays no piece"));
    }
    // With the game's prices in the input the answer's money is the line's price in game money
    // (algoglyph src/plates/track-planning/data.ts moneyOf); it must then be there.
    if input.get("prices").is_some_and(Value::is_object) && !res.get("money").and_then(Value::as_f64).is_some_and(|m| m.is_finite() && m >= 0.0) {
        return Err(Refusal::new("invalid_output", "result.money", "priced input, but the plan has no finite non-negative money"));
    }
    Ok(())
}

/// Domain checks of a kernel's output against its input, by the adapter registry (`ADAPTERS`): a
/// kernel without an adapter is accepted only as opaque evidence, never as a candidate's cost or
/// selection. A kernel with several adapters must pass every one of their checks.
pub fn domain_check(kernel: &str, input: &Value, output: &Value) -> Result<(), Refusal> {
    let mut any = false;
    for a in ADAPTERS.iter().filter(|a| a.kernel == kernel) {
        any = true;
        (a.check)(input, output)?;
    }
    if any {
        Ok(())
    } else {
        Err(Refusal::new("no_domain_check", "kernel.id", format!("{kernel:?} has no domain check; its output cannot price or select a candidate")))
    }
}

/// Whether the runner's input description declares `key` as a field: `key` after a separator and
/// before one of `:`, `?`, `,` or `}` (`{from: {x, y}, cash, loan?}` declares from, cash and loan). None when the description is opaque (prose without a `{...}` shape).
fn shape_declares(shape: Option<&str>, key: &str) -> Option<bool> {
    let s = shape.filter(|s| s.contains('{'))?;
    let b = s.as_bytes();
    let mut from = 0;
    while let Some(i) = s[from..].find(key).map(|i| i + from) {
        let before = i.checked_sub(1).map(|j| b[j]);
        let after = b.get(i + key.len()).copied();
        let sep = before.map_or(true, |c| !(c.is_ascii_alphanumeric() || c == b'_'));
        if sep && matches!(after, Some(b':' | b'?' | b',' | b'}')) {
            return Some(true);
        }
        from = i + key.len();
    }
    Some(false)
}

fn requirement_json(c: &Capability) -> Value {
    let f = |fs: &[Field]| fs.iter().map(|f| {
        let mut v = json!({"name": f.name, "kind": f.kind, "doc": f.doc});
        if let Some(u) = f.unit {
            v["unit"] = json!(u);
        }
        v
    }).collect::<Vec<_>>();
    json!({"id": c.id, "goal": c.goal, "used_by": c.used_by, "inputs": f(c.inputs), "outputs": f(c.outputs)})
}

/// The rejections of one adapter for its capability against the catalogue, and the evidence on it.
fn judge(c: &Capability, a: &Adapter, cat: &Catalogue) -> Value {
    let mut why = Vec::new();
    let k = cat.kernel(a.kernel);
    let mut shape = Value::Null;
    match k {
        None => why.push(Refusal::new("kernel_absent", "kernel.id", format!("{} is not in the catalogue {} at {}: the adapter has expired", a.kernel, cat.digest, cat.commit))),
        Some(k) => {
            if k.engine != a.engine {
                why.push(Refusal::new("engine_mismatch", "kernel.engine", format!("the adapter expects engine {:?}, the catalogue lists {:?}", a.engine, k.engine)));
            }
            let mut opaque = false;
            for (_, key, _) in a.inputs {
                let top = key.split('.').next().unwrap_or(key);
                match shape_declares(k.input.as_deref(), top) {
                    None => opaque = true,
                    Some(true) => {}
                    Some(false) => why.push(Refusal::new("input_shape_drift", &format!("input.{top}"), format!("the catalogue's input shape of {} no longer declares {top:?}", a.kernel))),
                }
            }
            shape = json!(if opaque { "opaque" } else { "verified" });
        }
    }
    for (side, fields, mapped) in [("inputs", c.inputs, a.inputs.iter().map(|m| m.0).collect::<Vec<_>>()), ("outputs", c.outputs, a.outputs.iter().map(|m| m.0).collect())] {
        for f in fields {
            let n = mapped.iter().filter(|m| **m == f.name).count();
            if n != 1 {
                why.push(Refusal::new("unmapped_field", &format!("{side}.{}", f.name), format!("the adapter maps {} {n} times, not once", f.name)));
            }
        }
        for m in &mapped {
            if !fields.iter().any(|f| f.name == *m) {
                why.push(Refusal::new("unknown_field", &format!("{side}.{m}"), format!("{} declares no field {m:?}", c.id)));
            }
        }
    }
    json!({"kernel": a.kernel, "engine": k.map(|k| k.engine.clone()), "compatible": why.is_empty(), "exact": a.exact,
        "exact_because": a.exact_because, "shape": shape, "units": "asserted by the adapter; the catalogue states none",
        "mapping": {"inputs": a.inputs.iter().map(|(r, k, conv)| json!({"field": r, "kernel": k, "conversion": conv})).collect::<Vec<_>>(),
                    "outputs": a.outputs.iter().map(|(r, k)| json!({"field": r, "kernel": k})).collect::<Vec<_>>()},
        "rejected": why.iter().map(Refusal::to_json).collect::<Vec<_>>()})
}

/// The outcome of matching one capability against one catalogue.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    /// The capability-match/v1 decision: requirement, catalogue, candidates, chosen, why.
    pub decision: Value,
    pub chosen: Option<String>,
    pub refusal: Option<Refusal>,
}

/// Select the kernel of the discovered catalogue that satisfies capability `cap`, using only what
/// the catalogue says (id, engine, input shape) and the adapters. Every catalogue kernel is
/// considered: those with an adapter for `cap` are judged, the rest are listed as unadapted.
/// Nothing is executed.
pub fn match_capability(cap: &str, cat: &Catalogue, adapters: &[Adapter]) -> Match {
    let catalogue = json!({"digest": cat.digest, "commit": cat.commit, "source": CATALOGUE_SOURCE, "kernels": cat.kernels.len()});
    let base = |requirement: Value, candidates: Vec<Value>, unadapted: Vec<&str>| json!({
        "schema": MATCH_SCHEMA, "requirement": requirement, "catalogue": catalogue,
        "adapters": {"owner": ADAPTER_OWNER, "retire_when": ADAPTER_RETIREMENT}, "ranking": RANKING,
        "candidates": candidates, "unadapted": unadapted, "chosen": null, "why": null, "refusal": null});
    let finish = |mut d: Value, chosen: Result<(String, String), Refusal>| -> Match {
        match chosen {
            Ok((k, why)) => {
                d["chosen"] = json!(k);
                d["why"] = json!(why);
                Match { decision: d, chosen: Some(k), refusal: None }
            }
            Err(r) => {
                d["refusal"] = r.to_json();
                Match { decision: d, chosen: None, refusal: Some(r) }
            }
        }
    };
    let Some(c) = capability(cap) else {
        let d = base(json!({"id": cap}), vec![], cat.kernels.iter().map(|k| k.id.as_str()).collect());
        return finish(d, Err(Refusal::new("unknown_capability", "requirement.id", format!("no capability {cap:?} is declared (CAPABILITIES)"))));
    };
    let mine: Vec<&Adapter> = adapters.iter().filter(|a| a.capability == cap).collect();
    let candidates: Vec<Value> = mine.iter().map(|a| judge(c, a, cat)).collect();
    let unadapted: Vec<&str> = cat.kernels.iter().map(|k| k.id.as_str()).filter(|id| !mine.iter().any(|a| a.kernel == *id)).collect();
    let d = base(requirement_json(c), candidates.clone(), unadapted);
    if mine.is_empty() {
        return finish(d, Err(Refusal::new("no_adapter", "adapters", format!("no adapter declares {cap}; the catalogue's {} kernels stay opaque to it", cat.kernels.len()))));
    }
    let ok: Vec<&Value> = candidates.iter().filter(|v| v["compatible"] == true).collect();
    if ok.is_empty() {
        let absent = candidates.iter().all(|v| v["rejected"].as_array().into_iter().flatten().any(|r| r["code"] == "kernel_absent"));
        return finish(d, Err(if absent {
            Refusal::new("kernel_expired", "candidates", format!("every kernel adapted for {cap} is absent from the catalogue at {}", cat.commit))
        } else {
            Refusal::new("no_match", "candidates", format!("no kernel adapted for {cap} is compatible with the catalogue at {}", cat.commit))
        }));
    }
    // The ranking: exact first, then a verified shape; what is left tied is refused.
    let rank = |v: &Value| (v["exact"] == true, v["shape"] == "verified");
    let top = ok.iter().map(|v| rank(v)).max().expect("ok is not empty");
    let best: Vec<&&Value> = ok.iter().filter(|v| rank(v) == top).collect();
    if best.len() > 1 {
        let ids: Vec<&str> = best.iter().filter_map(|v| v["kernel"].as_str()).collect();
        return finish(d, Err(Refusal::new("ambiguous_capability", "candidates", format!("{} kernels tie for {cap} after the ranking: {ids:?}", ids.len()))));
    }
    let k = best[0]["kernel"].as_str().unwrap_or_default().to_string();
    let why = if ok.len() == 1 {
        "the only compatible kernel".to_string()
    } else {
        format!("ranked first of {} compatible kernels (exact: {}, shape verified: {})", ok.len(), top.0, top.1)
    };
    finish(d, Ok((k, why)))
}

/// The inference record a capability match is written as (method capability-match, params the
/// decision), derived from the records the run rests on (its snapshot).
pub fn match_record(m: &Match, scope: &Value, at: &Value, derived_from: &[Value], valid_until: &str) -> Value {
    let d = &m.decision;
    let cap = d["requirement"]["id"].as_str().unwrap_or("?");
    let claim = match &m.chosen {
        Some(k) => format!("{k} satisfies {cap} in the AlgoGlyph catalogue {} at {}", d["catalogue"]["digest"].as_str().unwrap_or("?"), d["catalogue"]["commit"].as_str().unwrap_or("?")),
        None => format!("no kernel satisfies {cap} in the AlgoGlyph catalogue at {}: {}", d["catalogue"]["commit"].as_str().unwrap_or("?"), m.refusal.as_ref().map_or("", |r| r.code)),
    };
    record("inference", "valid", scope, at, (AUTHORITY, env!("CARGO_PKG_VERSION")), derived_from, json!({
        "claim": claim, "method": {"id": MATCH_METHOD, "version": "v1"}, "params": d, "valid_until": valid_until}))
}

/// Validate a capability-match/v1 decision (the `params` of a capability-match inference).
pub fn validate_match(d: &Value) -> Result<(), Vec<Refusal>> {
    let mut c = Check { errs: vec![], root: d };
    match d.get("schema").and_then(Value::as_str) {
        Some(MATCH_SCHEMA) => {}
        Some(other) => return Err(vec![Refusal::new("unknown_schema", "schema", format!("{other:?} is not {MATCH_SCHEMA:?}"))]),
        None => return Err(vec![Refusal::new("missing_field", "schema", format!("schema is required ({MATCH_SCHEMA:?})"))]),
    }
    c.need("requirement.id", nonempty_str, "the capability");
    c.need("catalogue.digest", is_digest, "the digest of the runner's kernel list");
    c.need("catalogue.commit", |v| v.as_str().is_some_and(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())), "the 40-hex checkout commit");
    c.need("unadapted", |v| v.as_array().is_some_and(|a| a.iter().all(nonempty_str)), "the catalogue kernels without an adapter");
    let cands = c.need("candidates", Value::is_array, "the judged candidates (may be empty)").and_then(Value::as_array).cloned().unwrap_or_default();
    for (i, x) in cands.iter().enumerate() {
        let codes_ok = x["rejected"].as_array().is_some_and(|a| a.iter().all(|r| r["code"].as_str().is_some_and(|s| CANDIDATE_REJECTIONS.contains(&s))));
        if !x.get("kernel").is_some_and(nonempty_str) || !x["compatible"].is_boolean() || !codes_ok
            || (x["compatible"] == true) != x["rejected"].as_array().is_some_and(Vec::is_empty) {
            c.errs.push(Refusal::new("invalid_field", &format!("candidates.{i}"), format!("a candidate is {{kernel, compatible, rejected: [{{code in {CANDIDATE_REJECTIONS:?}}}]}}, compatible exactly when nothing rejected it")));
        }
    }
    match (d.get("chosen").filter(|v| !v.is_null()), d.get("refusal").filter(|v| !v.is_null())) {
        (Some(k), None) => {
            if !cands.iter().any(|x| x["kernel"] == *k && x["compatible"] == true) {
                c.errs.push(Refusal::new("invalid_field", "chosen", format!("the chosen kernel {k} is not a compatible candidate")));
            }
            c.need("why", nonempty_str, "why it was chosen");
        }
        (None, Some(r)) => {
            if !r["code"].as_str().is_some_and(|s| MATCH_REFUSALS.contains(&s)) {
                c.errs.push(Refusal::new("invalid_field", "refusal.code", format!("a refusal code is one of {MATCH_REFUSALS:?}")));
            }
        }
        _ => c.errs.push(Refusal::new("invalid_field", "chosen", "exactly one of chosen and refusal is set")),
    }
    if c.errs.is_empty() {
        Ok(())
    } else {
        Err(c.errs)
    }
}

/// Validate the decision block a directive may carry (`decision`, battlebrain-decision/v1).
pub fn validate_decision_block(d: &Value) -> Result<(), Vec<Refusal>> {
    let mut c = Check { errs: vec![], root: d };
    match d.get("schema").and_then(Value::as_str) {
        Some(DECISION_SCHEMA) => {}
        Some(other) => return Err(vec![Refusal::new("unknown_schema", "schema", format!("{other:?} is not {DECISION_SCHEMA:?}"))]),
        None => return Err(vec![Refusal::new("missing_field", "schema", format!("schema is required ({DECISION_SCHEMA:?})"))]),
    }
    c.need("decision", is_record_ref, "the decision record bb:<sha256 hex>");
    c.need("snapshot_digest", is_digest, "the digest of the snapshot decided on");
    c.need("idempotency_key", nonempty_str, "the idempotency key");
    c.need("policy", nonempty_str, "policy id@version");
    c.need("valid_until", is_game_date, "the game date after which the directive is stale");
    c.need("settings_digest", is_digest, "sha256:<hex> of the exact settings string the decision authorises");
    c.need("snapshot_tick", |v| v.as_u64().is_some(), "the game tick of the snapshot decided on (>= 0)");
    c.one_of("mode", DECISION_MODES);
    if c.errs.is_empty() {
        Ok(())
    } else {
        Err(c.errs)
    }
}

/// The JSON Schema of a contract, generated from the definitions above.
pub fn schema(name: &str) -> Option<Value> {
    let rref = json!({"type": "string", "pattern": "^bb:[0-9a-f]{64}$"});
    let dig = json!({"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"});
    let date = json!({"type": "string", "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}$", "description": "a game date"});
    let amount = json!({"type": "object", "required": ["value", "unit"], "properties": {"value": {"type": "number"}, "unit": {"enum": UNITS}}});
    let nonempty = json!({"type": "string", "minLength": 1});
    match name {
        "record" => {
            let mut variants = Vec::new();
            for (kind, mesh, statuses, _, writers) in TYPES {
                let body = match *kind {
                    "snapshot" => json!({"type": "object", "required": ["digest", "sources", "max_age_days"], "properties": {
                        "digest": dig, "max_age_days": {"type": "integer", "minimum": 1},
                        "window": {"type": "object", "required": ["tick_from", "tick_to", "coherent"], "description": "the game ticks the sources were read between; coherent when every read saw one tick",
                            "properties": {"tick_from": {"type": "integer", "minimum": 0}, "tick_to": {"type": "integer", "minimum": 0}, "coherent": {"type": "boolean"}}},
                        "sources": {"type": "array", "minItems": 1, "items": {"type": "object", "required": ["uri", "digest"], "properties": {"uri": nonempty, "digest": dig}}}}}),
                    "inference" => json!({"type": "object", "required": ["claim", "method", "params", "valid_until"], "properties": {
                        "claim": nonempty, "params": {"type": "object"}, "valid_until": date, "confidence": {"type": "number", "minimum": 0, "maximum": 1},
                        "method": {"type": "object", "required": ["id", "version"], "properties": {"id": nonempty, "version": nonempty}}}}),
                    "candidate" => json!({"type": "object", "required": ["action", "assumptions"], "properties": {
                        "action": {"type": "object", "required": ["kind"], "properties": {"kind": {"enum": ACTIONS},
                            "settings": {"type": "object", "properties": FREIGHT_SETTINGS.iter().map(|k| (k.to_string(), json!({"type": "integer", "minimum": 0}))).collect::<Map<_, _>>()}}},
                        "estimate": {"type": "object", "properties": {"cost": amount, "profit": amount}},
                        "inputs_digest": dig,
                        "invocations": {"type": "array", "items": dig, "description": "the algoglyph-invocation/v1 envelopes the estimate rests on; empty when the producer's own model made it"},
                        "assumptions": {"type": "array", "items": {"type": "string"}}},
                        "description": "a freight_line action also requires action.settings.{fr_from,fr_to,fr_cargo}, estimate.cost, estimate.profit and inputs_digest"}),
                    "decision" => json!({"type": "object", "required": ["policy", "goal", "snapshot", "selected", "rejected", "idempotency_key", "approval", "valid_until"], "properties": {
                        "policy": {"type": "object", "required": ["id", "version", "params"], "properties": {"id": nonempty, "version": nonempty, "params": {"type": "object"}}},
                        "goal": nonempty, "snapshot": rref, "selected": rref, "idempotency_key": nonempty, "approval": {"const": "operator"}, "valid_until": date,
                        "rejected": {"type": "array", "items": {"type": "object", "required": ["candidate", "reason"], "properties": {"candidate": rref, "reason": nonempty}}}},
                        "description": "derived_from must contain the snapshot"}),
                    _ => json!({"type": "object", "required": ["decision", "executor", "result", "observed"], "properties": {
                        "decision": rref, "result": {"enum": ["applied", "refused", "stale", "failed"]}, "observed": {"type": "object"},
                        "executor": {"type": "object", "required": ["id"], "properties": {"id": nonempty}}}}),
                };
                let writers_doc = writers.iter().map(|w| if *w == "*" { format!("any producer but {AUTHORITY}") } else { w.to_string() }).collect::<Vec<_>>().join(", ");
                let mut req = vec!["schema", "type", "status", "scope", "at", "producer", "body"];
                if *kind != "snapshot" {
                    req.push("derived_from");
                }
                variants.push(json!({
                    "description": format!("{kind}: written to the mesh as kind {mesh}; writers: {writers_doc}"),
                    "required": req,
                    "properties": {"type": {"const": kind}, "status": {"enum": statuses}, "body": body,
                        "derived_from": if *kind == "snapshot" { json!({"type": "array", "items": rref}) } else { json!({"type": "array", "minItems": 1, "items": rref}) }}
                }));
            }
            Some(json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "https://github.com/korczis/OpenTTD/tools/research/schema/battlebrain-record.v1.schema.json",
                "title": RECORD_SCHEMA,
                "description": "RESEARCH-ONLY. GENERATED by `battlebrain schema record` from tools/research/mcp-rhai/src/battlebrain_contract.rs; do not edit. One blackboard record body of the Battlebrain freight-line decision (I0095). Transitions: see research/battlebrain/README.md.",
                "type": "object",
                "properties": {
                    "schema": {"const": RECORD_SCHEMA},
                    "id": rref,
                    "supersedes": rref,
                    "scope": {"type": "object", "required": ["instance", "company"], "properties": {"instance": nonempty, "company": {"type": "integer", "minimum": 0, "maximum": 14}}},
                    "at": {"type": "object", "required": ["tick", "date"], "properties": {"tick": {"type": "integer", "minimum": 0}, "date": date}},
                    "producer": {"type": "object", "required": ["id", "version"], "properties": {"id": nonempty, "version": nonempty}}
                },
                "oneOf": variants,
                "x-transitions": TRANSITIONS.iter().map(|(k, f, t)| json!([k, f, t])).collect::<Vec<_>>()
            }))
        }
        "invocation" => Some(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://github.com/korczis/OpenTTD/tools/research/schema/algoglyph-invocation.v1.schema.json",
            "title": INVOCATION_SCHEMA,
            "description": format!("RESEARCH-ONLY. GENERATED by `battlebrain schema invocation` from tools/research/mcp-rhai/src/battlebrain_contract.rs; do not edit. The caller-side envelope of one AlgoGlyph kernel call (I0095). AlgoGlyph is used from outside: the kernel must be in the catalogue the runner reported ({CATALOGUE_SOURCE}) at that checkout."),
            "type": "object",
            "required": ["schema", "kernel", "catalogue", "input", "budget", "status"],
            "properties": {
                "schema": {"const": INVOCATION_SCHEMA},
                "kernel": {"type": "object", "required": ["id", "engine"], "properties": {"id": nonempty, "engine": nonempty}},
                "catalogue": {"type": "object", "required": ["digest", "commit"], "properties": {"digest": dig, "commit": {"type": "string", "pattern": "^[0-9a-f]{40}$"}}},
                "input": {"type": "object", "required": ["digest", "units"], "properties": {"digest": dig, "units": {"type": "object", "additionalProperties": {"enum": UNITS}}}},
                "budget": {"type": "object", "required": ["timeout_ms"], "properties": {"timeout_ms": {"type": "integer", "minimum": 1}}},
                "status": {"enum": INVOCATION_STATUSES},
                "output": {"type": "object", "required": ["digest"], "properties": {"digest": dig}},
                "errors": {"type": "array", "minItems": 1, "items": {"type": "string"}},
                "duration_ms": {"type": "integer", "minimum": 0}
            },
            "if": {"properties": {"status": {"const": "ok"}}},
            "then": {"required": ["output"]},
            "else": {"required": ["errors"]}
        })),
        "decision" => Some(json!({
            "type": "object",
            "description": format!("GENERATED by `battlebrain schema decision` (I0095); do not edit by hand, regenerate. The Battlebrain decision a directive rests on: its blackboard record, the snapshot digest and game tick, the idempotency key, the policy, the game date after which the directive is stale, the digest of the settings it authorises, and its mode (an executor applies only live). Version-gated: a reader reads only a 'schema' it knows and refuses the directive for any other."),
            "required": ["schema", "decision", "snapshot_digest", "snapshot_tick", "idempotency_key", "policy", "valid_until", "settings_digest", "mode"],
            "additionalProperties": false,
            "properties": {
                "schema": {"const": DECISION_SCHEMA},
                "decision": rref,
                "snapshot_digest": dig,
                "snapshot_tick": {"type": "integer", "minimum": 0, "description": "the game tick of the snapshot decided on; an executor refuses the block in a game whose tick is behind it (another world, or an earlier save reloaded)"},
                "idempotency_key": nonempty,
                "policy": {"type": "string", "minLength": 1, "description": "policy id@version"},
                "valid_until": date,
                "settings_digest": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$", "description": "the digest (sha256 of the JSON string) of the directive's exact settings; the executor recomputes it"},
                "mode": {"enum": DECISION_MODES, "description": "an executor applies only live; shadow is refused before any step"}
            }
        })),
        "match" => {
            let fld = json!({"type": "object", "required": ["name", "kind", "doc"], "properties": {"name": nonempty, "kind": {"enum": FIELD_KINDS}, "unit": {"enum": UNITS}, "doc": nonempty},
                "description": "an amount kind carries a unit; no other kind does"});
            let refusal = |codes: &[&str]| json!({"type": "object", "required": ["code", "path", "message"], "properties": {"code": {"enum": codes}, "path": {"type": "string"}, "message": nonempty}});
            Some(json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "https://github.com/korczis/OpenTTD/tools/research/schema/capability-match.v1.schema.json",
                "title": MATCH_SCHEMA,
                "description": format!("RESEARCH-ONLY. GENERATED by `battlebrain schema match` from tools/research/mcp-rhai/src/battlebrain_contract.rs; do not edit. Which kernel of the discovered AlgoGlyph catalogue ({CATALOGUE_SOURCE}) satisfies one Battlebrain capability (I0098): the typed requirement, every kernel considered, the one chosen or the refusal. Written as the params of an inference record with method {MATCH_METHOD}."),
                "type": "object",
                "required": ["schema", "requirement", "catalogue", "adapters", "ranking", "candidates", "unadapted", "chosen", "why", "refusal"],
                "properties": {
                    "schema": {"const": MATCH_SCHEMA},
                    "requirement": {"type": "object", "required": ["id"], "properties": {"id": nonempty, "goal": nonempty, "used_by": nonempty,
                        "inputs": {"type": "array", "items": fld}, "outputs": {"type": "array", "items": fld}}},
                    "catalogue": {"type": "object", "required": ["digest", "commit", "source", "kernels"], "properties": {"digest": dig,
                        "commit": {"type": "string", "pattern": "^[0-9a-f]{40}$"}, "source": {"const": CATALOGUE_SOURCE}, "kernels": {"type": "integer", "minimum": 1}}},
                    "adapters": {"type": "object", "required": ["owner", "retire_when"], "properties": {"owner": nonempty, "retire_when": nonempty}},
                    "ranking": {"type": "array", "items": nonempty, "description": "the ranking rules applied among compatible kernels, in order"},
                    "candidates": {"type": "array", "description": "every kernel an adapter declares for this capability, judged against the catalogue", "items": {"type": "object",
                        "required": ["kernel", "engine", "compatible", "exact", "shape", "mapping", "rejected"], "properties": {
                            "kernel": nonempty, "engine": {"type": ["string", "null"], "description": "as the catalogue lists it; null when absent"},
                            "compatible": {"type": "boolean"}, "exact": {"type": "boolean"}, "exact_because": nonempty,
                            "shape": {"enum": ["verified", "opaque", null], "description": "verified: the catalogue's input shape declares every mapped key"},
                            "units": nonempty, "mapping": {"type": "object"}, "rejected": {"type": "array", "items": refusal(CANDIDATE_REJECTIONS)}}}},
                    "unadapted": {"type": "array", "items": nonempty, "description": "the catalogue's kernels no adapter declares for this capability: considered, never chosen"},
                    "chosen": {"type": ["string", "null"]},
                    "why": {"type": ["string", "null"]},
                    "refusal": {"oneOf": [{"type": "null"}, refusal(MATCH_REFUSALS)]}
                },
                "x-invariant": "exactly one of chosen and refusal is non-null; chosen is a compatible candidate"
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn fixtures(dir: &str) -> Vec<(String, Value)> {
        let d = repo().join("tools/research/schema/fixtures/battlebrain").join(dir);
        let mut out: Vec<(String, Value)> = std::fs::read_dir(&d)
            .unwrap_or_else(|e| panic!("{}: {e}", d.display()))
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap_or_else(|err| panic!("{name}: {err}"));
                (name, v)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        assert!(!out.is_empty(), "no fixtures in {}", d.display());
        out
    }

    fn catalogue() -> Catalogue {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(repo().join("tools/research/schema/fixtures/battlebrain/catalogue.json")).unwrap()).unwrap();
        Catalogue::from_runner(&v["kernels"], v["commit"].as_str().unwrap()).unwrap()
    }

    /// A fixture is {"case": ..., "value": ...}; an invalid one also names the refusal code it expects.
    fn check(name: &str, f: &Value) -> Result<(), Vec<Refusal>> {
        let v = &f["value"];
        match v.get("schema").and_then(Value::as_str) {
            Some(s) if s.starts_with("algoglyph-invocation/") => validate_invocation(v, &catalogue()),
            Some(s) if s.starts_with("battlebrain-decision/") => validate_decision_block(v),
            _ => validate_record(v).map(|_| ()),
        }
        .map_err(|e| {
            assert!(!e.is_empty(), "{name}: refused without a reason");
            e
        })
    }

    #[test]
    fn every_valid_fixture_is_accepted() {
        for (name, f) in fixtures("valid") {
            if let Err(e) = check(&name, &f) {
                panic!("{name} refused: {:?}", e);
            }
        }
    }

    #[test]
    fn every_invalid_fixture_is_refused_with_its_code() {
        for (name, f) in fixtures("invalid") {
            let want = f["expect"].as_str().unwrap_or_else(|| panic!("{name}: an invalid fixture names its expected code"));
            match check(&name, &f) {
                Ok(()) => panic!("{name} was accepted"),
                Err(e) => assert!(e.iter().any(|r| r.code == want), "{name}: expected {want}, got {:?}", e),
            }
        }
    }

    #[test]
    fn valid_record_fixtures_carry_their_own_digest_as_id() {
        for (name, f) in fixtures("valid") {
            let v = &f["value"];
            if v["schema"] == RECORD_SCHEMA {
                assert_eq!(v["id"].as_str(), Some(record_id(v).as_str()), "{name}");
            }
        }
    }

    #[test]
    fn transitions_are_closed_and_legal_ones_pass() {
        assert!(transition("candidate", "proposed", "selected").is_ok());
        assert!(transition("decision", "submitted", "stale").is_ok());
        assert_eq!(transition("decision", "applied", "submitted").unwrap_err().code, "illegal_transition");
        assert_eq!(transition("candidate", "rejected", "selected").unwrap_err().code, "illegal_transition");
        assert_eq!(transition("decision", "decided", "approved").unwrap_err().code, "unknown_status");
        assert_eq!(transition("plan", "a", "b").unwrap_err().code, "unknown_type");
        for (k, f, t) in TRANSITIONS {
            assert!(transition(k, f, t).is_ok(), "{k} {f}->{t}");
        }
        // Every type's initial status is one of its statuses, and terminal ones have no way out.
        for (k, _, statuses, init, _) in TYPES {
            assert!(statuses.contains(init), "{k}");
        }
    }

    #[test]
    fn only_the_authority_writes_a_decision_and_never_a_snapshot_or_outcome() {
        let base = |kind: &str, producer: &str| -> Value {
            let (_, f) = fixtures("valid").into_iter().find(|(_, f)| f["value"]["type"] == kind).unwrap();
            let mut v = f["value"].clone();
            v["producer"]["id"] = json!(producer);
            v.as_object_mut().unwrap().remove("id");
            v
        };
        let refused = |v: &Value| validate_record(v).unwrap_err().iter().any(|r| r.code == "writer_not_allowed");
        assert!(refused(&base("decision", "mcp-plan")));
        assert!(validate_record(&base("decision", AUTHORITY)).is_ok());
        assert!(refused(&base("snapshot", AUTHORITY)));
        assert!(refused(&base("outcome", AUTHORITY)));
        assert!(validate_record(&base("candidate", "mcp-plan")).is_ok());
    }

    #[test]
    fn mesh_kinds_are_the_closed_set_majordomus_admits() {
        for (k, _, _, _, _) in TYPES {
            let m = mesh_kind(k).unwrap();
            assert!(crate_mesh_kinds().iter().any(|x| x == m), "{k} -> {m}");
        }
        assert_eq!(mesh_kind("plan"), None);
    }

    // The closed kind set of tools/research/mcp-rhai/src/mesh_records.rs, read from its source so
    // this module does not keep a copy.
    fn crate_mesh_kinds() -> Vec<String> {
        let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/mesh_records.rs")).unwrap();
        let line = src.lines().find(|l| l.starts_with("pub const KINDS")).expect("mesh_records.rs declares KINDS");
        line.split('"').skip(1).step_by(2).map(str::to_string).collect()
    }

    #[test]
    fn a_kernel_outside_the_discovered_catalogue_is_unknown() {
        let cat = catalogue();
        assert!(cat.engine("knapsack-dp").is_some());
        assert!(cat.engine("knapsack-greedy-local-clone").is_none());
        assert!(Catalogue::from_runner(&json!([]), &"0".repeat(40)).is_err());
        assert!(Catalogue::from_runner(&json!([{"id": "x"}]), &"0".repeat(40)).is_err());
        assert!(Catalogue::from_runner(&json!([{"id": "x", "engine": "grid"}]), "main").is_err());
    }

    #[test]
    fn knapsack_output_is_checked_against_its_input() {
        let input = json!({"weights": [3, 4, 5], "values": [30.0, 50.0, 60.0], "capacity": 8});
        assert!(domain_check("knapsack-dp", &input, &json!({"result": {"best": 90.0, "items": [0, 2]}})).is_ok());
        // The 2026-11-22 audit's C7: an answer without items must not pass as a choice.
        assert_eq!(domain_check("knapsack-dp", &input, &json!({"result": {"best": 0.0}})).unwrap_err().code, "invalid_output");
        assert!(domain_check("knapsack-dp", &input, &json!({"result": {"best": 110.0, "items": [1, 2]}})).is_err(), "over capacity");
        assert!(domain_check("knapsack-dp", &input, &json!({"result": {"best": 60.0, "items": [0, 0]}})).is_err(), "repeated item");
        assert!(domain_check("knapsack-dp", &input, &json!({"result": {"best": 99.0, "items": [0, 2]}})).is_err(), "best != sum");
        assert_eq!(domain_check("transport-investment", &input, &json!({})).unwrap_err().code, "no_domain_check");
    }

    #[test]
    fn track_plan_output_must_be_a_priced_buildable_plan() {
        let ok = json!({"result": {"status": "found", "cost": 6487.0, "pieces": [{"tile": 1}]}});
        assert!(domain_check("track-planning", &json!({}), &ok).is_ok());
        for refused in ["unreachable", "budget", "ok"] {
            let v = json!({"result": {"status": refused, "cost": 0, "pieces": []}});
            assert!(domain_check("track-planning", &json!({}), &v).is_err(), "{refused}");
        }
        assert!(domain_check("track-planning", &json!({}), &json!({"result": {"status": "found", "cost": -1.0, "pieces": [1]}})).is_err());
        // With the game's prices in the input, the line's price is its money, and it must be there.
        let priced = json!({"prices": {"build_rail": 100}});
        assert_eq!(domain_check("track-planning", &priced, &ok).unwrap_err().path, "result.money");
        assert!(domain_check("track-planning", &priced, &json!({"result": {"status": "found", "cost": 6487.0, "money": 51000.0, "pieces": [{"tile": 1}]}})).is_ok());
    }

    fn real_kernels() -> (Value, String) {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(repo().join("tools/research/schema/fixtures/battlebrain/catalogue.json")).unwrap()).unwrap();
        (v["kernels"].clone(), v["commit"].as_str().unwrap().to_string())
    }

    /// The real catalogue with `edit` applied to its kernel list: a synthetic catalogue.
    fn synthetic(edit: impl FnOnce(&mut Vec<Value>)) -> Catalogue {
        let (k, commit) = real_kernels();
        let mut list = k.as_array().unwrap().clone();
        edit(&mut list);
        Catalogue::from_runner(&json!(list), &commit).unwrap()
    }

    fn code(m: &Match) -> &'static str {
        m.refusal.as_ref().map_or("chosen", |r| r.code)
    }

    #[test]
    fn capabilities_are_typed_and_every_adapter_maps_its_requirement_once() {
        let mut ids = std::collections::BTreeSet::new();
        for c in CAPABILITIES {
            assert!(ids.insert(c.id), "{} is declared twice", c.id);
            for f in c.inputs.iter().chain(c.outputs) {
                assert!(FIELD_KINDS.contains(&f.kind), "{}.{}: kind {}", c.id, f.name, f.kind);
                match (f.kind.starts_with("amount"), f.unit) {
                    (true, Some(u)) => assert!(UNITS.contains(&u), "{}.{}: unit {u}", c.id, f.name),
                    (false, None) => {}
                    other => panic!("{}.{}: an amount carries a unit and nothing else does ({other:?})", c.id, f.name),
                }
            }
        }
        for a in ADAPTERS {
            let c = capability(a.capability).unwrap_or_else(|| panic!("{} adapts the undeclared capability {}", a.kernel, a.capability));
            let j = judge(c, a, &catalogue());
            assert_eq!(j["rejected"], json!([]), "{} for {}", a.kernel, a.capability);
        }
    }

    #[test]
    fn on_the_real_catalogue_portfolio_is_knapsack_dp_and_rail_pricing_is_track_planning() {
        let cat = catalogue();
        assert_eq!(cat.kernels.len(), 53);
        for (cap, kernel, shape) in [("portfolio-within-budget", "knapsack-dp", "opaque"), ("rail-line-price", "track-planning", "verified")] {
            let m = match_capability(cap, &cat, ADAPTERS);
            assert_eq!(m.chosen.as_deref(), Some(kernel), "{cap}: {:?}", m.refusal);
            let d = &m.decision;
            assert_eq!(d["catalogue"]["digest"], json!(cat.digest));
            assert_eq!(d["catalogue"]["commit"], "1b8e009059c299b29d7a829d2d51d4e057304d4d");
            assert_eq!(d["candidates"][0]["shape"], shape, "{cap}");
            assert_eq!(d["why"], "the only compatible kernel");
            // Every catalogue kernel was considered: judged, or listed as unadapted.
            assert_eq!(d["candidates"].as_array().unwrap().len() + d["unadapted"].as_array().unwrap().len(), 53);
            assert!(validate_match(d).is_ok(), "{:?}", validate_match(d));
            let snap = format!("bb:{}", "a".repeat(64));
            let r = match_record(&m, &json!({"instance": "fixture", "company": 1}), &json!({"tick": 1, "date": "1961-01-01"}), &[json!(snap)], "1961-02-01");
            assert_eq!(validate_record(&r), Ok("inference"), "{r}");
        }
    }

    #[test]
    fn a_capability_without_a_declaration_or_an_adapter_is_refused() {
        let cat = catalogue();
        assert_eq!(code(&match_capability("schedule-under-uncertainty", &cat, ADAPTERS)), "unknown_capability");
        let m = match_capability("portfolio-within-budget", &cat, &[]);
        assert_eq!(code(&m), "no_adapter");
        assert_eq!(m.decision["unadapted"].as_array().unwrap().len(), 53, "every kernel stays opaque to it");
        assert!(validate_match(&m.decision).is_ok());
    }

    #[test]
    fn an_adapted_kernel_absent_from_the_catalogue_has_expired() {
        let cat = synthetic(|l| l.retain(|k| k["id"] != "knapsack-dp"));
        let m = match_capability("portfolio-within-budget", &cat, ADAPTERS);
        assert_eq!(code(&m), "kernel_expired");
        assert_eq!(m.decision["candidates"][0]["rejected"][0]["code"], "kernel_absent");
        // A synthetic adapter for a kernel the runner never listed is judged and rejected; the
        // real one is still chosen. Nothing is executed either way.
        let ghost = Adapter { kernel: "knapsack-quantum", ..ADAPTERS[0] };
        let m = match_capability("portfolio-within-budget", &catalogue(), &[ghost, ADAPTERS[0]]);
        assert_eq!(m.chosen.as_deref(), Some("knapsack-dp"));
        assert_eq!(m.decision["candidates"][0]["rejected"][0]["code"], "kernel_absent");
    }

    #[test]
    fn an_incompatible_kernel_is_refused_with_its_reason() {
        let cat = synthetic(|l| l.iter_mut().filter(|k| k["id"] == "knapsack-dp").for_each(|k| k["engine"] = json!("numeric")));
        let m = match_capability("portfolio-within-budget", &cat, ADAPTERS);
        assert_eq!(code(&m), "no_match");
        assert_eq!(m.decision["candidates"][0]["rejected"][0]["code"], "engine_mismatch");
        let cat = synthetic(|l| l.iter_mut().filter(|k| k["id"] == "track-planning").for_each(|k| k["input"] = json!("{map?: data, from: {x, y}, to: {x, y}}")));
        let m = match_capability("rail-line-price", &cat, ADAPTERS);
        assert_eq!(code(&m), "no_match");
        assert_eq!(m.decision["candidates"][0]["rejected"][0]["code"], "input_shape_drift");
        assert_eq!(m.decision["candidates"][0]["rejected"][0]["path"], "input.prices");
        // An adapter that leaves a requirement field unmapped, or maps one the requirement lacks.
        let short = Adapter { outputs: &[("chosen", "result.items")], ..ADAPTERS[0] };
        let m = match_capability("portfolio-within-budget", &catalogue(), &[short]);
        assert_eq!(code(&m), "no_match");
        assert_eq!(m.decision["candidates"][0]["rejected"][0]["code"], "unmapped_field");
        let extra = Adapter { outputs: &[("chosen", "result.items"), ("total_value", "result.best"), ("weight", "result.weight")], ..ADAPTERS[0] };
        assert_eq!(match_capability("portfolio-within-budget", &catalogue(), &[extra]).decision["candidates"][0]["rejected"][0]["code"], "unknown_field");
    }

    #[test]
    fn a_synthetic_kernel_without_an_adapter_is_considered_and_never_chosen() {
        let cat = synthetic(|l| l.push(json!({"id": "knapsack-quantum", "engine": "grid", "input": "{weights, values, capacity}", "title": "synthetic"})));
        let m = match_capability("portfolio-within-budget", &cat, ADAPTERS);
        assert_eq!(m.chosen.as_deref(), Some("knapsack-dp"));
        assert!(m.decision["unadapted"].as_array().unwrap().contains(&json!("knapsack-quantum")));
        assert_eq!(domain_check("knapsack-quantum", &json!({}), &json!({})).unwrap_err().code, "no_domain_check");
    }

    #[test]
    fn several_compatible_kernels_are_ranked_and_a_tie_is_refused() {
        let cat = synthetic(|l| {
            l.push(json!({"id": "knapsack-twin", "engine": "grid", "input": "the kernel's data form"}));
            l.push(json!({"id": "knapsack-shaped", "engine": "grid", "input": "{weights: number[], values: number[], capacity}"}));
        });
        let twin = Adapter { kernel: "knapsack-twin", ..ADAPTERS[0] };
        // Both exact, both opaque: a tie, refused rather than broken by name.
        let m = match_capability("portfolio-within-budget", &cat, &[ADAPTERS[0], twin]);
        assert_eq!(code(&m), "ambiguous_capability");
        assert!(validate_match(&m.decision).is_ok());
        // Exact beats heuristic, whatever the order.
        let greedy = Adapter { exact: false, ..twin };
        for set in [[ADAPTERS[0], greedy], [greedy, ADAPTERS[0]]] {
            let m = match_capability("portfolio-within-budget", &cat, &set);
            assert_eq!(m.chosen.as_deref(), Some("knapsack-dp"));
            assert!(m.decision["why"].as_str().unwrap().starts_with("ranked first of 2"));
        }
        // Among exact kernels, a verified shape beats an opaque one.
        let shaped = Adapter { kernel: "knapsack-shaped", ..ADAPTERS[0] };
        let m = match_capability("portfolio-within-budget", &cat, &[ADAPTERS[0], shaped]);
        assert_eq!(m.chosen.as_deref(), Some("knapsack-shaped"));
        assert_eq!(m.decision["candidates"][1]["shape"], "verified");
    }

    #[test]
    fn a_match_decision_that_contradicts_itself_is_refused() {
        let m = match_capability("portfolio-within-budget", &catalogue(), ADAPTERS);
        let mut d = m.decision.clone();
        d["chosen"] = json!("transport-investment");
        assert!(validate_match(&d).unwrap_err().iter().any(|r| r.path == "chosen"));
        let mut d = m.decision.clone();
        d["refusal"] = json!({"code": "no_match", "path": "", "message": "x"});
        assert!(validate_match(&d).is_err(), "chosen and refused at once");
        let mut d = m.decision.clone();
        d["schema"] = json!("capability-match/v2");
        assert_eq!(validate_match(&d).unwrap_err()[0].code, "unknown_schema");
    }

    #[test]
    fn shape_declarations_are_read_from_the_runners_own_text() {
        let track = catalogue().kernel("track-planning").unwrap().input.clone();
        for key in ["from", "to", "map", "prices"] {
            assert_eq!(shape_declares(track.as_deref(), key), Some(true), "{key}");
        }
        assert_eq!(shape_declares(track.as_deref(), "budget"), Some(false));
        assert_eq!(shape_declares(Some("{fromData: x}"), "from"), Some(false));
        assert_eq!(shape_declares(Some("{actions: [], cash, loan?}"), "cash"), Some(true));
        assert_eq!(shape_declares(Some("{a: 1} then from here"), "from"), Some(false));
        assert_eq!(shape_declares(Some("the kernel's data form (see its fromData)"), "from"), None);
        assert_eq!(shape_declares(None, "from"), None);
    }

    #[test]
    fn the_slot_count_is_what_prismatic_ai_declares() {
        let info = std::fs::read_to_string(repo().join("tools/research/prismatic-ai/PrismaticAI/info.nut")).unwrap();
        let line = info.lines().zip(info.lines().skip(1)).find(|(_, next)| next.contains("_from")).filter(|(l, _)| l.contains("for (local i = 2"));
        let want = format!("i <= {FREIGHT_SLOTS}");
        assert!(info.contains(&format!("for (local i = 2; {want}; i++)")), "info.nut no longer declares fr2..fr{FREIGHT_SLOTS} ({line:?})");
        assert_eq!(freight_slot_prefix(1), "fr_");
        assert_eq!(freight_slot_prefix(16), "fr16_");
    }

    #[test]
    fn game_dates_count_days_both_ways() {
        assert_eq!(days_between("1963-02-22", "1963-03-24"), Some(30));
        assert_eq!(days_between("1963-03-24", "1963-02-22"), Some(-30));
        assert_eq!(days_between("x", "1963-02-22"), None);
        assert_eq!(add_days("1963-02-22", 30).as_deref(), Some("1963-03-24"));
    }

    #[test]
    fn graph_ids_become_the_games_numbers() {
        assert_eq!(graph_number(&json!("industry:41")), Some(41));
        assert_eq!(graph_number(&json!(7)), Some(7));
        assert_eq!(graph_number(&json!("12")), Some(12));
        assert_eq!(graph_number(&json!("industry:")), None);
        assert_eq!(graph_number(&json!(null)), None);
    }

    #[test]
    fn digests_ignore_key_order() {
        assert_eq!(digest(&json!({"b": 1, "a": [1, {"d": 2, "c": 3}]})), digest(&json!({"a": [1, {"c": 3, "d": 2}], "b": 1})));
        assert_ne!(digest(&json!({"a": 1})), digest(&json!({"a": 2})));
    }

    #[test]
    fn committed_schemas_are_the_generated_ones() {
        for (name, file) in [("record", "battlebrain-record.v1.schema.json"), ("invocation", "algoglyph-invocation.v1.schema.json"), ("match", "capability-match.v1.schema.json")] {
            let path = repo().join("tools/research/schema").join(file);
            let committed: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap();
            assert_eq!(committed, schema(name).unwrap(), "{file} differs from `battlebrain schema {name}`: regenerate it");
        }
        let contract: Value = serde_json::from_str(&std::fs::read_to_string(repo().join("research/prismatic-bridge/decision-contract.schema.json")).unwrap()).unwrap();
        assert_eq!(contract["$defs"]["battlebrainDecision"], schema("decision").unwrap(), "decision-contract.schema.json $defs.battlebrainDecision differs from `battlebrain schema decision`");
        assert_eq!(contract["$defs"]["decisionDirective"]["properties"]["decision"]["$ref"], "#/$defs/battlebrainDecision");
    }
}
