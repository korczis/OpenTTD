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
/// v2 (I0117): the block names the digest of the exact settings it authorises (`settings_digest`).
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

/// The kernels the AlgoGlyph runner reports (`flow -- kernels`), with the digest of that answer
/// and the checkout it came from. Built from the runner's answer only: no kernel is known here
/// that the runner did not list.
#[derive(Clone, Debug, PartialEq)]
pub struct Catalogue {
    pub entries: Vec<(String, String)>,
    pub digest: String,
    pub commit: String,
}

impl Catalogue {
    /// Parse the runner's answer: a list of objects with at least `id` and `engine`.
    pub fn from_runner(answer: &Value, commit: &str) -> Result<Catalogue, Refusal> {
        let list = answer.as_array().ok_or_else(|| Refusal::new("invalid_catalogue", "", "the runner's kernel list is not an array"))?;
        let mut entries = Vec::new();
        for (i, k) in list.iter().enumerate() {
            match (k.get("id").and_then(Value::as_str), k.get("engine").and_then(Value::as_str)) {
                (Some(id), Some(engine)) if !id.is_empty() => entries.push((id.to_string(), engine.to_string())),
                _ => return Err(Refusal::new("invalid_catalogue", &i.to_string(), "a kernel entry needs a non-empty id and an engine")),
            }
        }
        if entries.is_empty() {
            return Err(Refusal::new("invalid_catalogue", "", "the runner lists no kernel"));
        }
        if !(commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err(Refusal::new("invalid_catalogue", "commit", "the checkout commit must be 40 hex digits"));
        }
        Ok(Catalogue { entries, digest: digest(answer), commit: commit.to_ascii_lowercase() })
    }
    pub fn engine(&self, id: &str) -> Option<&str> {
        self.entries.iter().find(|e| e.0 == id).map(|e| e.1.as_str())
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

/// Domain checks of a kernel's output against its input. These are adapters at a real protocol
/// boundary (the runner's outputs carry no schema): a kernel without a check here is accepted only
/// as opaque evidence, never as a candidate's cost or selection (`domain_check_known`).
pub fn domain_check(kernel: &str, input: &Value, output: &Value) -> Result<(), Refusal> {
    let res = output.get("result").unwrap_or(output);
    match kernel {
        "knapsack-dp" => {
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
        "track-planning" => {
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
            Ok(())
        }
        _ => Err(Refusal::new("no_domain_check", "kernel.id", format!("{kernel:?} has no domain check; its output cannot price or select a candidate"))),
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
            "description": format!("GENERATED by `battlebrain schema decision` (I0095); do not edit by hand, regenerate. The Battlebrain decision a directive rests on: its blackboard record, the snapshot digest, the idempotency key, the policy, the game date after which the directive is stale, and its mode (an executor applies only live). Version-gated: a reader reads only a 'schema' it knows and refuses the directive for any other."),
            "required": ["schema", "decision", "snapshot_digest", "idempotency_key", "policy", "valid_until", "settings_digest", "mode"],
            "additionalProperties": false,
            "properties": {
                "schema": {"const": DECISION_SCHEMA},
                "decision": rref,
                "snapshot_digest": dig,
                "idempotency_key": nonempty,
                "policy": {"type": "string", "minLength": 1, "description": "policy id@version"},
                "valid_until": date,
                "settings_digest": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$", "description": "the digest (sha256 of the JSON string) of the directive's exact settings; the executor recomputes it"},
                "mode": {"enum": DECISION_MODES, "description": "an executor applies only live; shadow is refused before any step"}
            }
        })),
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
        for (name, file) in [("record", "battlebrain-record.v1.schema.json"), ("invocation", "algoglyph-invocation.v1.schema.json")] {
            let path = repo().join("tools/research/schema").join(file);
            let committed: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap();
            assert_eq!(committed, schema(name).unwrap(), "{file} differs from `battlebrain schema {name}`: regenerate it");
        }
        let contract: Value = serde_json::from_str(&std::fs::read_to_string(repo().join("research/prismatic-bridge/decision-contract.schema.json")).unwrap()).unwrap();
        assert_eq!(contract["$defs"]["battlebrainDecision"], schema("decision").unwrap(), "decision-contract.schema.json $defs.battlebrainDecision differs from `battlebrain schema decision`");
        assert_eq!(contract["$defs"]["decisionDirective"]["properties"]["decision"]["$ref"], "#/$defs/battlebrainDecision");
    }
}
