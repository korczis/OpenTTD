//! RESEARCH-ONLY: memory the Rhai scripts and services of one panel share (I0091,
//! research/rhai-ide.md).
//!
//! A key holds a JSON value with a revision, the writer that declared itself and the time. A write
//! may name the revision it expects (compare-and-swap; 0 = the key must not exist yet), so two
//! services updating one key never lose each other's write silently: the second one is refused
//! with the current revision and reads again. Every write and delete is published as a `rhai.shared`
//! event, so it reaches the ZeroMQ bus and the DuckDB store (ADR 0033) as an audit trail.
//!
//! Limits, stated where they apply:
//! - the memory lives in the panel process: a restarted panel starts empty (the store keeps the
//!   history of every write, not the current map);
//! - `writer` is what the caller declares; the panel's loopback API authenticates no script.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

/// The largest value a key holds, as JSON text.
pub const MAX_VALUE: usize = 64 * 1024;
/// The most keys one panel holds.
pub const MAX_KEYS: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub value: Value,
    pub rev: u64,
    pub writer: String,
    pub at: u64,
}

/// Why a write was refused.
#[derive(Debug, PartialEq)]
pub enum Refused {
    /// The key's revision is not the one the write expected (0: the key does not exist).
    Conflict { rev: u64 },
    /// The key, the value or the writer is not acceptable; the text says why.
    Invalid(String),
}

#[derive(Default)]
pub struct Shared {
    map: Mutex<BTreeMap<String, Entry>>,
}

/// A key is 1 to 128 characters of letters, digits and `_ . : / -`, so it reads well in a log and
/// groups by prefix (`rail.line.3`).
pub fn valid_key(key: &str) -> bool {
    (1..=128).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:/-".contains(&b))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn writer_of(w: &str) -> Result<String, Refused> {
    let w = if w.trim().is_empty() { "unnamed" } else { w.trim() };
    if w.len() > 64 || w.chars().any(char::is_control) { return Err(Refused::Invalid("writer: at most 64 printable characters".into())); }
    Ok(w.to_string())
}

impl Shared {
    pub fn get(&self, key: &str) -> Option<Entry> {
        self.map.lock().ok()?.get(key).cloned()
    }

    /// Every entry whose key starts with `prefix`, in key order.
    pub fn list(&self, prefix: &str) -> Vec<(String, Entry)> {
        self.map.lock().map(|m| m.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix)).map(|(k, e)| (k.clone(), e.clone())).collect())
            .unwrap_or_default()
    }

    /// Write `value` under `key`; with `expect`, only if the key's revision is that one (0: absent).
    /// Answers the new revision.
    pub fn set(&self, key: &str, value: Value, writer: &str, expect: Option<u64>) -> Result<u64, Refused> {
        if !valid_key(key) { return Err(Refused::Invalid(format!("key {key:?}: 1-128 characters of letters, digits and _ . : / -"))); }
        let size = value.to_string().len();
        if size > MAX_VALUE { return Err(Refused::Invalid(format!("value: {size} bytes as JSON, at most {MAX_VALUE}"))); }
        let writer = writer_of(writer)?;
        let mut m = self.map.lock().map_err(|_| Refused::Invalid("the shared memory's lock is poisoned".into()))?;
        let current = m.get(key).map(|e| e.rev).unwrap_or(0);
        if let Some(want) = expect {
            if want != current { return Err(Refused::Conflict { rev: current }); }
        }
        if current == 0 && m.len() >= MAX_KEYS { return Err(Refused::Invalid(format!("at most {MAX_KEYS} keys"))); }
        let rev = current + 1;
        m.insert(key.to_string(), Entry { value, rev, writer, at: now() });
        Ok(rev)
    }

    /// Delete `key`; with `expect`, only at that revision. Answers the revision it had.
    pub fn delete(&self, key: &str, writer: &str, expect: Option<u64>) -> Result<u64, Refused> {
        writer_of(writer)?;
        let mut m = self.map.lock().map_err(|_| Refused::Invalid("the shared memory's lock is poisoned".into()))?;
        let current = m.get(key).map(|e| e.rev).unwrap_or(0);
        if current == 0 || expect.is_some_and(|w| w != current) { return Err(Refused::Conflict { rev: current }); }
        m.remove(key);
        Ok(current)
    }
}

/// The one shared memory of this panel process.
pub fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(Shared::default)
}

/// One author per freight-line slot (src/fr_author.rs): the key format and the rule, pure.
#[path = "fr_author.rs"]
pub mod fr_author;

/// The shared check of this panel's writers: None when `writer` may propose `company`'s
/// freight-line slots (its planner key is unset, null or names it), else the refusal
/// `{key, company, holder, writer, why}` read from the memory `s`.
fn fr_refusal_in(s: &Shared, company: i64, writer: &str) -> Option<Value> {
    fr_author::refusal(planner_in(s, company).as_ref(), company, writer)
}

/// Every company whose planner key names another writer than `writer`, as refusals in key order.
/// Empty when no planner key is set, which is the case outside a Battlebrain cutover.
pub fn fr_refusals(writer: &str) -> Vec<Value> {
    fr_refusals_in(shared(), writer)
}

fn fr_refusals_in(s: &Shared, writer: &str) -> Vec<Value> {
    s.list(fr_author::PLANNER_KEY_PREFIX).iter()
        .filter_map(|(k, _)| fr_author::company_of_key(k))
        .filter_map(|c| fr_refusal_in(s, c, writer)).collect()
}

/// The value of `company`'s planner key in this panel's memory, None when it is unset: the
/// `holder` fr_author::change_refusal checks a writer-named change against (src/panel_bot.rs).
pub fn planner_of(company: i64) -> Option<Value> {
    planner_in(shared(), company)
}

fn planner_in(s: &Shared, company: i64) -> Option<Value> {
    s.get(&fr_author::planner_key(company)).map(|e| e.value)
}

pub fn entry_json(key: &str, e: &Entry) -> Value {
    json!({"key": key, "value": e.value, "rev": e.rev, "writer": e.writer, "at": e.at})
}

fn refused(r: Refused) -> (u16, Value) {
    match r {
        Refused::Conflict { rev } => (409, json!({"error": format!("conflict: the key is at revision {rev}"), "rev": rev})),
        Refused::Invalid(why) => (400, json!({"error": why})),
    }
}

fn expect_of(body: &Value) -> Result<Option<u64>, String> {
    match body.get("rev") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| "rev: a whole number (0 = the key must not exist)".to_string()),
    }
}

/// GET /api/rhai/shared: one key (?key=) or every key under ?prefix= (all without one).
pub fn h_shared(_: &Arc<crate::panel::Game>, i: &crate::panel::Input) -> Result<(u16, Value), String> {
    if let Some(key) = i.query.get("key") {
        return Ok(match shared().get(key) {
            Some(e) => (200, entry_json(key, &e)),
            None => (404, json!({"error": format!("no shared key {key:?}"), "rev": 0})),
        });
    }
    let prefix = i.query.get("prefix").unwrap_or("");
    let entries: Vec<Value> = shared().list(prefix).iter().map(|(k, e)| entry_json(k, e)).collect();
    Ok((200, json!({"entries": entries, "count": entries.len(), "limits": {"keys": MAX_KEYS, "value_bytes": MAX_VALUE}})))
}

/// POST /api/rhai/shared {key, value, writer?, rev?}: write, published as `rhai.shared`.
pub fn h_shared_set(g: &Arc<crate::panel::Game>, i: &crate::panel::Input) -> Result<(u16, Value), String> {
    let key = i.body.get("key").and_then(Value::as_str).unwrap_or("");
    let value = i.body.get("value").cloned().unwrap_or(Value::Null);
    let writer = i.body.get("writer").and_then(Value::as_str).unwrap_or("");
    let expect = match expect_of(&i.body) { Ok(e) => e, Err(why) => return Ok((400, json!({"error": why}))) };
    Ok(match shared().set(key, value.clone(), writer, expect) {
        Ok(rev) => {
            let e = shared().get(key).map(|e| entry_json(key, &e)).unwrap_or(Value::Null);
            g.bus.publish("rhai.shared", json!({"op": "set", "key": key, "rev": rev, "writer": e["writer"], "value": value}));
            (200, e)
        }
        Err(r) => refused(r),
    })
}

/// POST /api/rhai/shared/delete {key, writer?, rev?}: delete, published as `rhai.shared`.
pub fn h_shared_delete(g: &Arc<crate::panel::Game>, i: &crate::panel::Input) -> Result<(u16, Value), String> {
    let key = i.body.get("key").and_then(Value::as_str).unwrap_or("");
    let writer = i.body.get("writer").and_then(Value::as_str).unwrap_or("");
    let expect = match expect_of(&i.body) { Ok(e) => e, Err(why) => return Ok((400, json!({"error": why}))) };
    Ok(match shared().delete(key, writer, expect) {
        Ok(rev) => {
            g.bus.publish("rhai.shared", json!({"op": "delete", "key": key, "rev": rev, "writer": writer_of(writer).unwrap_or_default()}));
            (200, json!({"key": key, "deleted": true, "rev": rev}))
        }
        Err(r) => refused(r),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_bumps_the_revision_and_keeps_its_writer() {
        let s = Shared::default();
        assert_eq!(s.set("rail.line.1", json!({"trains": 2}), "rail_steward", None), Ok(1));
        assert_eq!(s.set("rail.line.1", json!({"trains": 3}), "rail_steward", None), Ok(2));
        let e = s.get("rail.line.1").unwrap();
        assert_eq!((e.rev, e.writer.as_str(), e.value["trains"].as_i64()), (2, "rail_steward", Some(3)));
        assert_eq!(s.set("x", json!(1), "  ", None), Ok(1));
        assert_eq!(s.get("x").unwrap().writer, "unnamed");
    }

    #[test]
    fn compare_and_swap_refuses_a_stale_or_duplicate_write() {
        let s = Shared::default();
        assert_eq!(s.set("lock", json!("a"), "one", Some(0)), Ok(1), "0: create only");
        assert_eq!(s.set("lock", json!("b"), "two", Some(0)), Err(Refused::Conflict { rev: 1 }), "a second claim is refused");
        assert_eq!(s.set("lock", json!("b"), "two", Some(1)), Ok(2));
        assert_eq!(s.set("lock", json!("c"), "one", Some(1)), Err(Refused::Conflict { rev: 2 }), "a stale revision is refused");
        assert_eq!(s.get("lock").unwrap().value, json!("b"), "the refused write changed nothing");
    }

    #[test]
    fn deletes_need_the_key_and_the_revision() {
        let s = Shared::default();
        assert_eq!(s.delete("none", "w", None), Err(Refused::Conflict { rev: 0 }));
        s.set("k", json!(1), "w", None).unwrap();
        assert_eq!(s.delete("k", "w", Some(7)), Err(Refused::Conflict { rev: 1 }));
        assert_eq!(s.delete("k", "w", Some(1)), Ok(1));
        assert!(s.get("k").is_none());
        assert_eq!(s.set("k", json!(2), "w", Some(0)), Ok(1), "a deleted key starts again at revision 1");
    }

    #[test]
    fn keys_values_and_writers_are_bounded() {
        let s = Shared::default();
        for bad in ["", "has space", "ünï", &"x".repeat(129)] {
            assert!(matches!(s.set(bad, json!(1), "w", None), Err(Refused::Invalid(_))), "{bad:?}");
        }
        assert!(valid_key("rail.line/3:a-b_c"));
        assert!(matches!(s.set("big", json!("x".repeat(MAX_VALUE)), "w", None), Err(Refused::Invalid(_))));
        assert!(matches!(s.set("k", json!(1), &"w".repeat(65), None), Err(Refused::Invalid(_))));
        assert!(matches!(s.set("k", json!(1), "a\nb", None), Err(Refused::Invalid(_))));
    }

    #[test]
    fn listing_groups_by_prefix_in_key_order() {
        let s = Shared::default();
        for k in ["rail.2", "bus.1", "rail.1", "railway"] { s.set(k, json!(k), "w", None).unwrap(); }
        assert_eq!(s.list("rail.").iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["rail.1", "rail.2"]);
        assert_eq!(s.list("").len(), 4);
        assert!(s.list("zzz").is_empty());
    }

    #[test]
    fn concurrent_compare_and_swap_increments_lose_no_update() {
        let s = std::sync::Arc::new(Shared::default());
        s.set("n", json!(0), "init", Some(0)).unwrap();
        let threads: Vec<_> = (0..8).map(|t| {
            let s = s.clone();
            std::thread::spawn(move || {
                for _ in 0..50 {
                    loop {
                        let e = s.get("n").unwrap();
                        let next = json!(e.value.as_i64().unwrap() + 1);
                        if s.set("n", next, &format!("t{t}"), Some(e.rev)).is_ok() { break; }
                    }
                }
            })
        }).collect();
        for t in threads { t.join().unwrap(); }
        let e = s.get("n").unwrap();
        assert_eq!((e.value.as_i64(), e.rev), (Some(400), 401));
    }

    #[test]
    fn the_key_limit_holds() {
        let s = Shared::default();
        for n in 0..MAX_KEYS { s.set(&format!("k{n}"), json!(n), "w", None).unwrap(); }
        assert!(matches!(s.set("one.more", json!(1), "w", None), Err(Refused::Invalid(_))));
        assert_eq!(s.set("k0", json!("again"), "w", None), Ok(2), "an existing key still updates");
    }

    #[test]
    fn the_planner_key_check_reads_this_panels_memory() {
        let s = Shared::default();
        assert!(fr_refusal_in(&s, 0, "replan").is_none(), "unset: every writer as before");
        assert!(fr_refusals_in(&s, "replan").is_empty());
        s.set("planner.0", json!("battlebrain"), "runbook", Some(0)).unwrap();
        s.set("planner.1", json!("replan"), "runbook", Some(0)).unwrap();
        s.set("planner.x", json!("battlebrain"), "runbook", Some(0)).unwrap();
        s.set("replan.0", json!({"date": "1950-01-01"}), "replan", None).unwrap();
        assert!(fr_refusal_in(&s, 0, "battlebrain").is_none(), "the holder itself");
        assert_eq!(fr_refusal_in(&s, 0, "replan").unwrap()["holder"], "battlebrain");
        let all: Vec<i64> = fr_refusals_in(&s, "panel_bot").iter().map(|r| r["company"].as_i64().unwrap()).collect();
        assert_eq!(all, [0, 1], "every planner key of a company, none of another prefix");
        assert_eq!(fr_refusals_in(&s, "replan").len(), 1);
        s.set("planner.0", Value::Null, "runbook", None).unwrap();
        assert!(fr_refusal_in(&s, 0, "replan").is_none(), "a null value hands the slots back");
        assert_eq!((planner_in(&s, 0), planner_in(&s, 1), planner_in(&s, 2)), (Some(Value::Null), Some(json!("replan")), None));
    }

    /// planner_of reads this process's own memory, the one POST /api/rhai/shared writes. No test
    /// writes a planner key there: the panel's tests share it, and planned() reads every such key.
    #[test]
    fn planner_of_reads_the_panels_own_memory() {
        assert_eq!(planner_of(97), None);
        assert!(shared().get(&fr_author::planner_key(97)).is_none(), "the same memory, the same key");
    }

    // ------------------------------------------------------------- the Rhai side of the same rule

    use std::cell::RefCell;
    use std::rc::Rc;

    const RHAI_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel/rhai");

    /// An engine that resolves `import` from mcp-panel/rhai as the panel's does, with `shared_get`
    /// over `store` and a running PrismaticAI company 0 whose `api_botSetting` calls are recorded
    /// as `name@writer` (the writer the call names for the panel's planner-key check, or nothing).
    fn guard_engine(store: Rc<Shared>, sent: Rc<RefCell<Vec<String>>>) -> rhai::Engine {
        let mut e = rhai::Engine::new();
        e.set_module_resolver(rhai::module_resolvers::FileModuleResolver::new_with_path(RHAI_DIR));
        e.register_fn("shared_get", move |key: &str| -> rhai::Dynamic {
            store.get(key).map(|en| rhai::serde::to_dynamic(en.value).unwrap()).unwrap_or(rhai::Dynamic::UNIT)
        });
        e.register_fn("env", |_: &str| String::new());
        e.register_fn("api_bots", || -> rhai::Dynamic {
            let names = ["fr_from", "fr_to", "fr_cargo", "fr2_from", "rail_a", "own_lines"];
            rhai::serde::to_dynamic(json!({"running": [{"id": 0, "name": "Prismatic", "ai": "PrismaticAI",
                "settings": names.iter().map(|n| json!({"name": n, "value": -1})).collect::<Vec<_>>()}]})).unwrap()
        });
        e.register_fn("api_botSetting", move |m: rhai::Map| -> rhai::Dynamic {
            sent.borrow_mut().push(format!("{}@{}", m["name"], m.get("writer").map(|w| w.to_string()).unwrap_or_default()));
            let n = sent.borrow().len() as i64;
            rhai::serde::to_dynamic(json!({"status": "pending_approval", "approval_id": n})).unwrap()
        });
        e
    }

    fn eval_with(store: &Rc<Shared>, script: &str) -> (Value, Vec<String>) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let out: rhai::Dynamic = guard_engine(store.clone(), sent.clone()).eval(script).unwrap_or_else(|e| panic!("{e}\n{script}"));
        (rhai::serde::from_dynamic(&out).unwrap(), sent.take())
    }

    /// fr_author.rhai and src/fr_author.rs are one rule: the same key and the same refusal in the
    /// same words, for an unset key, its own name, another name and a null value.
    #[test]
    fn the_rhai_guard_and_the_rust_guard_agree() {
        let store = Rc::new(Shared::default());
        for holder in [None, Some(json!("replan")), Some(json!("battlebrain")), Some(Value::Null)] {
            if let Some(h) = &holder { store.set("planner.4", h.clone(), "runbook", None).unwrap(); }
            let (r, _) = eval_with(&store, "import \"fr_author\" as fa; fa::refusal(4, \"replan\")");
            let want = fr_refusal_in(&store, 4, "replan").unwrap_or(Value::Null);
            assert_eq!(r, want, "holder {holder:?}");
        }
    }

    /// build_plan::apply_as is the write path of ask_relay, replan and build_plan itself: with the
    /// company's key unset every proposed setting is sent as before; with another writer's name
    /// nothing reaches api_botSetting (no fr_* slot either) and the first line says why; with its
    /// own name all.
    #[test]
    fn build_plan_sends_no_freight_slot_another_planner_holds() {
        let store = Rc::new(Shared::default());
        let script = r#"import "build_plan" as bp;
            let r = bp::apply_as(#{ build: #{ settings: #{ fr_from: 3, fr_to: 18, fr_cargo: 7, rail_a: 2 } } }, "ask_relay");
            #{ withheld: r.withheld, pending: r.pending.len(), held: r.held, first: r.lines[0] }"#;
        let (r, sent) = eval_with(&store, script);
        // Each call names its writer, so the panel's own check of planner.0 (POST /api/bots/setting)
        // lets the freight-line slots through.
        assert_eq!(sent, ["fr_cargo@ask_relay", "fr_from@ask_relay", "fr_to@ask_relay", "rail_a@ask_relay"], "unset: as before");
        assert_eq!((r["withheld"].clone(), r["held"].clone()), (json!([]), Value::Null));

        store.set("planner.0", json!("battlebrain"), "runbook", Some(0)).unwrap();
        let (r, sent) = eval_with(&store, script);
        assert!(sent.is_empty(), "nothing is sent, no fr_* slot either: {sent:?}");
        assert_eq!(r["withheld"], json!(["fr_cargo", "fr_from", "fr_to", "rail_a"]));
        assert_eq!(r["held"]["holder"], "battlebrain");
        assert!(r["first"].as_str().unwrap().contains("planned by battlebrain (shared key planner.0), not by ask_relay"), "{}", r["first"]);

        store.set("planner.0", json!("ask_relay"), "runbook", None).unwrap();
        assert_eq!(eval_with(&store, script).1.len(), 4, "its own name: as before");
        store.set("planner.0", json!("battlebrain"), "runbook", None).unwrap();
        let (_, sent) = eval_with(&store, r#"import "build_plan" as bp; bp::apply(#{ build: #{ settings: #{ fr2_from: 1 } } })"#);
        assert!(sent.is_empty(), "apply() is apply_as under build_plan's own name");
    }

    /// The text of a library script up to the line that starts its main loop, so its functions
    /// can be called without running it.
    fn head_of(script: &str, main_starts: &str) -> String {
        let text = std::fs::read_to_string(format!("{RHAI_DIR}/{script}.rhai")).unwrap();
        text.split(main_starts).next().unwrap().to_string()
    }

    /// prismatic_tune asks for nothing for a held company, so no freight-line slot whatever gene
    /// /api/tuning names, and notes each change; unset, every change stays.
    #[test]
    fn the_live_tuner_asks_for_no_freight_slot_another_planner_holds() {
        let store = Rc::new(Shared::default());
        let script = format!("{}\nlet p = #{{ changes: [#{{ name: \"fr_from\", to: 3, stage: \"ga\" }}, #{{ name: \"invest_ppv_div\", to: 5, stage: \"ga\" }}], notes: [] }};
            let q = own_changes(p, fa::refusal(0, WRITER)); #{{ names: q.changes.map(|x| x.name), notes: q.notes }}",
            head_of("prismatic_tune", "// ---- main"));
        let (r, _) = eval_with(&store, &script);
        assert_eq!((r["names"].clone(), r["notes"].clone()), (json!(["fr_from", "invest_ppv_div"]), json!([])));
        store.set("planner.0", json!("battlebrain"), "runbook", Some(0)).unwrap();
        let (r, _) = eval_with(&store, &script);
        assert_eq!(r["names"], json!([]));
        assert_eq!(r["notes"].as_array().unwrap().len(), 2);
        assert!(r["notes"][0].as_str().unwrap().contains("fr_from -> 3 (ga): not asked, company 0's freight-line slots are planned by battlebrain"), "{}", r["notes"]);
    }

    /// A whole round of the live tuner over host reads stubbed here: the GA's best variant proposes
    /// a freight-line slot and another setting, and each request names the tuner as its writer, so
    /// the panel's planner-key check lets the slot through while the key is unset; held, nothing.
    #[test]
    fn a_tuner_round_names_its_writer_on_every_request() {
        let round = |store: &Rc<Shared>| -> (Value, Vec<String>) {
            let sent = Rc::new(RefCell::new(Vec::new()));
            let mut e = guard_engine(store.clone(), sent.clone());
            let dynamic = |v: Value| rhai::serde::to_dynamic(v).unwrap();
            let setting = |name: &str| json!({"name": name, "value": 1, "min": 0, "max": 64, "step": 1, "ingame": true, "editable": true});
            let bots = json!({"running": [{"id": 0, "name": "Prismatic", "ai": "PrismaticAI", "settings": [setting("fr_from"), setting("rail_a")]}]});
            e.register_fn("api_bots", move || dynamic(bots.clone()));
            e.register_fn("status", move || dynamic(json!({"ai": "PrismaticAI", "pending": []})));
            e.register_fn("read", move |uri: &str| -> Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
                if uri != "openttd://companies" { return Err(format!("{uri}: not stubbed").into()); }
                Ok(dynamic(json!({"game_date": "1950-01-01", "data": {"companies": [
                    {"id": 0, "name": "Prismatic", "company_value": 1000, "money": 5000, "fleet": {"total": 1}}]}})))
            });
            e.register_fn("loopback_get", move |_: &str| dynamic(json!({"variants": [
                {"label": "v1", "settings": "fr_from=3,rail_a=2", "fitness": {"score": 1.0, "matches": 99}}]})));
            e.register_fn("shared_entry", |_: &str| rhai::Dynamic::UNIT);
            e.register_fn("con_say", |_: &str| rhai::Dynamic::UNIT);
            let script = format!("{}\ntune_round()", head_of("prismatic_tune", "// ---- main"));
            let out: rhai::Dynamic = e.eval(&script).unwrap_or_else(|err| panic!("{err}"));
            (rhai::serde::from_dynamic(&out).unwrap(), sent.take())
        };
        let store = Rc::new(Shared::default());
        let (every, sent) = round(&store);
        assert_eq!(sent, ["fr_from@prismatic_tune", "rail_a@prismatic_tune"]);
        assert_eq!(every, json!(600), "the round answers the seconds to the next one");
        store.set("planner.0", json!("battlebrain"), "runbook", Some(0)).unwrap();
        assert!(round(&store).1.is_empty(), "held by another planner: the tuner asks for nothing");
    }

    /// replan stands down on another writer's key and plans on its own name or an unset key.
    #[test]
    fn replan_stands_down_for_another_planner_only() {
        let store = Rc::new(Shared::default());
        let script = format!("{}\ntype_of(fa::refusal(2, WRITER))", head_of("replan", "let last = ();"));
        assert_eq!(eval_with(&store, &script).0, json!("()"));
        store.set("planner.2", json!("replan"), "runbook", Some(0)).unwrap();
        assert_eq!(eval_with(&store, &script).0, json!("()"));
        store.set("planner.2", json!("battlebrain"), "runbook", None).unwrap();
        assert_eq!(eval_with(&store, &script).0, json!("map"));
    }

    /// replan's plan record claims a path, because the mesh refuses a claim of none (400
    /// invalid_input "a claim names at least one path"), and every path it claims is in the
    /// repository.
    #[test]
    fn a_replan_record_claims_paths_that_exist() {
        let written = Rc::new(RefCell::new(Vec::<Value>::new()));
        let mut e = guard_engine(Rc::new(Shared::default()), Rc::new(RefCell::new(Vec::new())));
        let w = written.clone();
        e.register_fn("mesh_record_write", move |m: rhai::Map| -> rhai::Dynamic {
            w.borrow_mut().push(rhai::serde::from_dynamic(&rhai::Dynamic::from_map(m)).unwrap());
            rhai::Dynamic::UNIT
        });
        e.register_fn("shared_set", |_: &str, _: rhai::Dynamic| rhai::Dynamic::UNIT);
        e.register_fn("to_json", |_: rhai::Dynamic| String::from("{}"));
        let script = format!("{}\nrecord(2, #{{ date: \"1950-01-01\", model: #{{ adopt: true }} }});",
            head_of("replan", "let last = ();"));
        e.run(&script).unwrap_or_else(|err| panic!("{err}"));
        let written = written.take();
        assert_eq!(written.len(), 1, "one record per round");
        assert_eq!(written[0]["task"], "openttd:replan:2");
        let scope = written[0]["scope"].as_array().expect("scope is a list");
        assert!(!scope.is_empty(), "the mesh refuses a claim of no path");
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        for p in scope {
            let p = p.as_str().expect("a path");
            assert!(repo.join(p).is_file(), "{p} is not in the repository");
        }
    }

    /// A whole replan round, run as the service runs it once (REPLAN_ONCE=1) over host reads
    /// stubbed here: one plan for every running PrismaticAI company (one api_botPlanned call), each
    /// company proposed the settings of its own entry in bot_settings.companies that differ from its
    /// live ones, sent to that company under replan's name; a company whose key names another
    /// planner proposes nothing, one the plan has no entry for is said so, and nothing is approved.
    /// Answers the setting calls as `company:name@writer` and the printed lines.
    fn replan_round(store: &Rc<Shared>) -> (Vec<String>, Vec<String>, usize) {
        let setting = |name: &str, value: i64| json!({"name": name, "value": value});
        let live = json!([setting("fr_from", -1), setting("fr_to", -1), setting("fr_cargo", -1), setting("own_lines", 8)]);
        let trains = json!([{"owner": 0, "type": "train", "age": 400, "profit_last_year": 9000, "orders": [{"destination": 5}]},
                            {"owner": 2, "type": "train", "age": 400, "profit_last_year": 20000, "orders": [{"destination": 8}]},
                            {"owner": 6, "type": "train", "age": 400, "profit_last_year": 7000, "orders": [{"destination": 9}]}]);
        let r = replan_round_with(store, live, trains, "fr_from={p},fr_to=99,fr_cargo=1,own_lines=8", json!([]), |_, _| (0.8, 0.6, 0.9));
        (r.sent, r.printed, r.plans)
    }

    /// What one replan round did: the setting calls, the printed lines, the planner calls, and the
    /// blackboard writes (key, value).
    struct Round { sent: Vec<String>, printed: Vec<String>, plans: usize, records: Vec<(String, Value)> }

    /// replan_round over the companies' live settings `live`, the vehicles of openttd://vehicles,
    /// each plan entry's start_ai (`{p}` is the company's producer), the game's pending approvals
    /// and a bayes-update stub (earned, lost) -> (mean, lower, upper).
    fn replan_round_with(store: &Rc<Shared>, live: Value, vehicles: Value, start: &str, pending: Value,
                         bayes: fn(i64, i64) -> (f64, f64, f64)) -> Round {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let printed = Rc::new(RefCell::new(Vec::new()));
        let planned = Rc::new(RefCell::new(0usize));
        let records = Rc::new(RefCell::new(Vec::<(String, Value)>::new()));
        let mut e = guard_engine(store.clone(), Rc::new(RefCell::new(Vec::new())));
        let dynamic = |v: Value| rhai::serde::to_dynamic(v).unwrap();
        let bots = json!({"running": [{"id": 0, "name": "P0", "ai": "PrismaticAI", "settings": live},
                                      {"id": 2, "name": "Hog", "ai": "AAAHogEx", "settings": []},
                                      {"id": 6, "name": "P6", "ai": "PrismaticAI", "settings": live},
                                      {"id": 7, "name": "P7", "ai": "PrismaticAI", "settings": live}]});
        e.register_fn("api_bots", move || dynamic(bots.clone()));
        let line = |p: i64| json!({"mode": "freight_rail", "cargo": "Coal", "cargo_id": 1,
                                   "from": {"id": format!("industry:{p}"), "name": format!("Mine {p}")}, "to": {"id": "industry:99", "name": "Plant"}});
        let entry = |c: f64, p: i64| json!({"company": c, "start_ai": start.replace("{p}", &p.to_string()),
                                            "freight": [{"from": p, "to": 99, "cargo_id": 1, "monthly_profit": 5000.0}]});
        let plan = json!({"settings": "own_lines=8", "portfolio": {"investment_k": 100, "solver": "knapsack-dp", "chosen": [line(1), line(2)],
                                                                   "priced": {"lines": 2, "with_route": 2}},
                          "bot_settings": {"companies": [entry(0.0, 1), entry(6.0, 2)]}});
        let n = planned.clone();
        e.register_fn("api_botPlanned", move |_: rhai::Map| { *n.borrow_mut() += 1; dynamic(plan.clone()) });
        let s = sent.clone();
        e.register_fn("api_botSetting", move |m: rhai::Map| -> rhai::Dynamic {
            s.borrow_mut().push(format!("{}:{}@{}", m["company"], m["name"], m["writer"]));
            let id = s.borrow().len() as i64;
            dynamic(json!({"status": "pending_approval", "approval_id": id}))
        });
        e.register_fn("read", move |uri: &str| -> Result<rhai::Dynamic, Box<rhai::EvalAltResult>> {
            let v = match uri {
                "openttd://game/state" => json!({"game_date": "1951-03-01", "game_tick": 5, "data": {"paused": false}}),
                "openttd://companies" => json!({"data": {"companies": [{"id": 0, "name": "P0", "company_value": 900}, {"id": 2, "name": "Hog", "company_value": 2000},
                                                                       {"id": 6, "name": "P6", "company_value": 800}, {"id": 7, "name": "P7", "company_value": 700}]}}),
                "openttd://vehicles" => json!({"data": {"vehicles": vehicles.clone()}}),
                _ => return Err(format!("{uri}: not stubbed").into()),
            };
            Ok(dynamic(v))
        });
        for f in ["Income", "Expenses", "CompanyValue", "PerformanceRating", "CargoDelivered"] {
            e.register_fn(format!("openttd_api_ScriptCompany_GetQuarterly{f}"), move |_: rhai::Map| dynamic(json!({"ok": true, "result": 100})));
        }
        e.register_fn("api_dbNamedRun", move |m: rhai::Map| {
            dynamic(if m["name"].to_string() == "company_rank" { json!({"rows": [[1, 4, 900, "1951-03-01"]]}) }
                    else { json!({"rows": [["1951-02", 900, 10, 50, 5, 1]]}) })
        });
        e.register_fn("kernel", move |name: &str, m: rhai::Map| {
            let n = |k: &str| m.get(k).and_then(|v| v.as_int().ok()).unwrap_or(-1);
            let (mean, lower, upper) = bayes(n("successes"), n("failures"));
            dynamic(if name == "bayes-update" { json!({"mean": mean, "lower": lower, "upper": upper}) } else { json!({"p50": 10.0, "p80": 12.0, "p90": 14.0}) })
        });
        e.register_fn("status", move || dynamic(json!({"pending": pending.clone()})));
        e.register_fn("http_timeout", |s: i64| s);
        e.register_fn("con_say", |_: &str| ());
        let w = records.clone();
        e.register_fn("shared_set", move |k: &str, v: rhai::Dynamic| { w.borrow_mut().push((k.to_string(), rhai::serde::from_dynamic(&v).unwrap())); });
        e.register_fn("to_json", |_: rhai::Dynamic| String::from("{}"));
        e.register_fn("mesh_record_write", |_: rhai::Map| -> Result<rhai::Dynamic, Box<rhai::EvalAltResult>> { Err("no mesh session".into()) });
        e.register_fn("env", |k: &str| if k == "REPLAN_ONCE" { "1".to_string() } else { String::new() });
        let p = printed.clone();
        e.on_print(move |t| p.borrow_mut().push(t.to_string()));
        let script = std::fs::read_to_string(format!("{RHAI_DIR}/replan.rhai")).unwrap();
        e.run(&script).unwrap_or_else(|err| panic!("{err}"));
        let n = *planned.borrow();
        Round { sent: sent.take(), printed: printed.take(), plans: n, records: records.take() }
    }

    #[test]
    fn replan_proposes_each_company_of_its_ai_its_own_plan() {
        let store = Rc::new(Shared::default());
        let (sent, printed, plans) = replan_round(&store);
        assert_eq!(plans, 1, "one plan for every company");
        assert_eq!(sent, ["0:fr_cargo@replan", "0:fr_from@replan", "0:fr_to@replan", "0:own_lines@replan",
                          "6:fr_cargo@replan", "6:fr_from@replan", "6:fr_to@replan", "6:own_lines@replan"],
                   "each company its own entry, only what differs from its live settings, and the floor the plan would drop");
        let all = printed.join("\n");
        assert!(all.contains("fr_from = 1: pending approval #2") && all.contains("fr_from = 2: pending approval #6"), "{all}");
        assert!(all.contains("own_lines = 10: pending approval #4"), "{all}");
        assert!(all.contains("plan: 2 line(s), investment 100k, predicted 5000 a month (1 line(s) without a price count as 0)"), "{all}");
        assert!(all.contains("== company 8 (P7): the plan has no entry for it; nothing proposed"), "{all}");
        assert!(all.contains("mesh: no mesh session"), "{all}");
        assert!(!all.contains("(Hog): rank"), "another AI's company is not re-planned: {all}");

        store.set("planner.6", json!("battlebrain"), "runbook", Some(0)).unwrap();
        let (sent, printed, _) = replan_round(&store);
        assert_eq!(sent, ["0:fr_cargo@replan", "0:fr_from@replan", "0:fr_to@replan", "0:own_lines@replan"], "company 6 is held by another planner");
        assert!(printed.iter().any(|l| l.contains("holding: company 6's freight-line slots are planned by battlebrain")), "{printed:?}");
    }

    /// PrismaticAI's autopilot floors are kept once on the panel's side (bot_floors.rhai), and
    /// that copy is main.nut's floor block: each `max(b.x, CONST)` an at_least, each
    /// `min(b.x, CONST)` an at_most, each `b.x = n` an equals, with the AUTOPILOT_* values main.nut
    /// declares. The block is the first `if (b.autopiloted) {` or `if (b.grows) {` that assigns
    /// AUTOPILOT_OWN_LINES: master applies the floors to an unplanned bot only, and a branch that
    /// applies them planned or not (fix/prismatic-planned-and-early-rail) keeps the same five
    /// assignments. bot_floors' planned rule (one freight line or pair complete) is main.nut's, and
    /// can only err towards the floors: the bot counts a slot only when all three are set, and also
    /// checks it and waits for two reads that agree (FreightLines, #271). The autopilot default is
    /// info.nut's.
    #[test]
    fn the_floors_are_main_nuts_autopilot_block() {
        let bot = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prismatic-ai/PrismaticAI");
        let main = std::fs::read_to_string(bot.join("main.nut")).unwrap();
        let consts: std::collections::BTreeMap<String, i64> = main.lines()
            .filter_map(|l| l.trim().strip_prefix("const AUTOPILOT_"))
            .filter_map(|l| { let (k, v) = l.split_once('=')?; Some((format!("AUTOPILOT_{}", k.trim()), v.trim().trim_end_matches(';').trim().parse().ok()?)) })
            .collect();
        let block: Vec<&str> = ["if (b.autopiloted) {", "if (b.grows) {"].iter()
            .flat_map(|open| main.split(open).skip(1))
            .map(|after| after.lines().map(str::trim).take_while(|l| *l != "}").filter(|l| !l.is_empty() && !l.starts_with("/*") && !l.starts_with('*')).collect::<Vec<_>>())
            .find(|b| b.iter().any(|l| l.contains("AUTOPILOT_OWN_LINES")))
            .expect("main.nut has a floor block that assigns AUTOPILOT_OWN_LINES");
        let mut want = serde_json::Map::new();
        // A floor is `b.x = max(b.x, AUTOPILOT_*)`, `b.x = min(b.x, AUTOPILOT_*)` or `b.x = <n>`;
        // any other line of the block (a log line, a call) is not one.
        for l in &block {
            let Some((lhs, rhs)) = l.trim_end_matches(';').split_once(" = ") else { continue };
            let Some(name) = lhs.trim().strip_prefix("b.") else { continue };
            let floor = if let Some(args) = rhs.strip_prefix("max(").or_else(|| rhs.strip_prefix("min(")) {
                let Some(c) = args.trim_end_matches(')').split(", ").nth(1).filter(|c| c.starts_with("AUTOPILOT_")) else { continue };
                let kind = if rhs.starts_with("max(") { "at_least" } else { "at_most" };
                json!({kind: consts.get(c).unwrap_or_else(|| panic!("{c} is not an AUTOPILOT_ const: {consts:?}"))})
            } else if let Ok(n) = rhs.trim().parse::<i64>() {
                json!({"equals": n})
            } else {
                continue;
            };
            want.insert(name.to_string(), floor);
        }
        assert!(want.len() >= 4, "the block was read: {block:?}");
        let (got, _) = eval_with(&Rc::new(Shared::default()), r#"import "bot_floors" as bf; bf::floors()"#);
        assert_eq!(got["floors"], Value::Object(want), "bot_floors.rhai FLOORS drifted from main.nut's autopiloted block");
        let planned = main.lines().map(str::trim).find(|l| l.starts_with("b.planned <-")).expect("main.nut names when the bot is planned");
        for part in ["b.freight.len() > 0", "(b.air_a >= 0 && b.air_b >= 0)", "(b.rail_a >= 0 && b.rail_b >= 0)"] {
            assert!(planned.contains(part), "main.nut's planned rule changed ({planned}): bot_floors::planned follows it");
        }
        assert!(main.contains("if (line.from >= 0 && line.to >= 0 && line.cargo >= 0)"), "the bot counts a freight line only when all three are set");
        let info = std::fs::read_to_string(bot.join("info.nut")).unwrap();
        let decl = info.split(&format!("name = \"{}\"", got["name"].as_str().unwrap())).nth(1).expect("info.nut declares the autopilot setting");
        let decl = decl.split('}').next().unwrap();
        let value_of = |key: &str| decl.split(&format!("{key} = ")).nth(1)
            .and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next()?.parse::<i64>().ok());
        let default = value_of("default_value").or_else(|| value_of("custom_value")).expect("info.nut gives the autopilot a default");
        assert_eq!(got["default"], json!(default), "AUTOPILOT_DEFAULT is info.nut's default");
    }

    /// bot_floors::guard on its own: a touched line is filed whole, a plan that makes the bot
    /// planned carries each floor the bot's live settings would lose, a proposed floor key is held to
    /// its floor, the autopilot off keeps every value, and a bot planned already has its lost floors
    /// named.
    #[test]
    fn a_guarded_plan_files_a_line_whole_and_keeps_the_floors() {
        let live = |planned: bool| {
            let mut l: Vec<Value> = [("fr_from", -1), ("fr_to", -1), ("fr_cargo", -1), ("rail_a", -1), ("rail_b", -1), ("rail_after", 0),
                ("air_a", -1), ("air_b", -1), ("own_lines", 4), ("max_routes", 30), ("cash_reserve", 5000), ("intercity", 1),
                ("self_steward", 1), ("autopilot", 1)].iter().map(|(n, v)| json!({"name": n, "value": v})).collect();
            for (n, v) in [("fr2_from", 5), ("fr2_to", 6), ("fr2_cargo", 1)] {
                l.push(json!({"name": n, "value": if planned { v } else { -1 }}));
            }
            l
        };
        let guard = |live: Vec<Value>, diff: Value| -> Value {
            let script = format!("import \"bot_floors\" as bf; bf::guard({}, {})", to_rhai_literal(&json!(live)), to_rhai_literal(&diff));
            eval_with(&Rc::new(Shared::default()), &script).0
        };
        let (g, _) = eval_with(&Rc::new(Shared::default()), r#"import "bot_floors" as bf;
            bf::groups(["fr_from", "fr_to", "fr_cargo", "fr2_from", "fr2_to", "rail_a", "rail_b", "rail_after", "air_a", "air_b", "own_lines"])"#);
        assert_eq!(g, json!([{"prefix": "fr_", "members": ["fr_from", "fr_to", "fr_cargo"], "mode": "rail"},
                             {"prefix": "rail_", "members": ["rail_a", "rail_b"], "mode": "rail"},
                             {"prefix": "air_", "members": ["air_a", "air_b"], "mode": "air"}]), "fr2_ lacks its cargo: no group");

        let r = guard(live(false), json!({"fr_to": 7}));
        assert_eq!(r["settings"], json!({"fr_to": 7, "fr_from": -1, "fr_cargo": -1}), "one key of a line is filed with the rest of it");
        assert_eq!((r["grouped"].clone(), r["raised"].clone(), r["planned_after"].clone()), (json!(["fr_from", "fr_cargo"]), json!([]), json!(false)));
        assert_eq!(r["added_modes"], json!([]), "an incomplete line adds no mode");

        let r = guard(live(false), json!({"fr_from": 1, "fr_to": 2, "fr_cargo": 3, "own_lines": 4, "cash_reserve": 25000, "intercity": 0}));
        assert_eq!(r["planned_after"], json!(true));
        assert_eq!(r["settings"]["own_lines"], json!(10), "the plan makes the bot planned: own_lines stays at the floor");
        assert_eq!((r["settings"]["cash_reserve"].clone(), r["settings"]["intercity"].clone()), (json!(5000), json!(1)));
        assert!(r["settings"].get("max_routes").is_none() && r["settings"].get("self_steward").is_none(), "a floor the live value meets is not filed");
        assert_eq!(r["added_modes"], json!(["rail"]));
        assert_eq!(r["lost"], json!([]), "an unplanned bot has lost nothing yet");

        let r = guard(live(false), json!({"air_a": 3, "air_b": 4}));
        assert_eq!((r["added_modes"].clone(), r["settings"]["own_lines"].clone()), (json!(["air"]), json!(10)), "a pair plans the bot too");

        let r = guard(live(false), json!({"fr_from": 1, "fr_to": 2, "fr_cargo": 3, "autopilot": 0}));
        assert!(r["settings"].get("autopilot").is_none(), "a plan never turns the autopilot off: {r}");
        assert_eq!((r["raised"][0].clone(), r["settings"]["own_lines"].clone()), (json!({"name": "autopilot", "from": 0, "to": 1}), json!(10)));
        let mut off = live(false);
        for s in off.iter_mut() { if s["name"] == "autopilot" { s["value"] = json!(0); } }
        let r = guard(off, json!({"fr_from": 1, "fr_to": 2, "fr_cargo": 3}));
        assert!(r["settings"].get("own_lines").is_none() && r["raised"] == json!([]), "the owner turned the autopilot off: no floors: {r}");

        let r = guard(live(false), json!({"own_lines": 2}));
        assert_eq!((r["settings"].clone(), r["planned_after"].clone()), (json!({"own_lines": 10}), json!(false)), "a proposed floor key is held to it");

        let r = guard(live(true), json!({}));
        assert_eq!(r["lost"], json!([{"name": "own_lines", "from": 4, "to": 10}]), "planned already: own_lines=4 binds today");
        assert_eq!((r["planned_before"].clone(), r["settings"].clone()), (json!(true), json!({"own_lines": 10})));
    }

    /// A JSON value as a Rhai literal (maps as #{...}).
    fn to_rhai_literal(v: &Value) -> String {
        match v {
            Value::Object(m) => format!("#{{{}}}", m.iter().map(|(k, x)| format!("{k}: {}", to_rhai_literal(x))).collect::<Vec<_>>().join(", ")),
            Value::Array(a) => format!("[{}]", a.iter().map(to_rhai_literal).collect::<Vec<_>>().join(", ")),
            other => other.to_string(),
        }
    }

    fn live_of(values: &[(&str, i64)]) -> Value {
        Value::Array(values.iter().map(|(n, v)| json!({"name": n, "value": v})).collect())
    }

    /// The live settings of an unplanned PrismaticAI in game 10487 (1952-04): no line slot set,
    /// own_lines 4 from the run's start string, the autopilot on.
    const UNPLANNED_10487: &[(&str, i64)] = &[("fr_from", -1), ("fr_to", -1), ("fr_cargo", -1), ("own_lines", 4), ("max_routes", 30),
        ("cash_reserve", 5000), ("intercity", 1), ("self_steward", 1), ("autopilot", 1)];

    /// A bayes-update stub whose lower bound is (earned + 0.1) / (earned + lost + 1).
    fn bayes_share(earned: i64, lost: i64) -> (f64, f64, f64) {
        let lower = (earned as f64 + 0.1) / (earned + lost + 1) as f64;
        (lower.max(0.05), lower, (lower + 0.3).min(1.0))
    }

    fn train(owner: i64, stop: i64, profit: i64) -> Value {
        json!({"owner": owner, "type": "train", "age": 400, "profit_last_year": profit, "orders": [{"destination": stop}]})
    }

    fn bus(owner: i64, stop: i64, profit: i64) -> Value {
        json!({"owner": owner, "type": "road", "age": 400, "profit_last_year": profit, "orders": [{"destination": stop}]})
    }

    /// The trap of runs 12609 and 10487: the plan completes a freight slot of an unplanned bot whose
    /// live own_lines is 4 and asks own_lines 4 itself. Applied as filed, the bot turns planned and
    /// main.nut stops lifting own_lines to 10 (10487: `growth-on-plan own_lines=4` from 1952-04-15
    /// to 2114). The plan is not proposed that way: own_lines=10 is filed with the slot, in the same
    /// call, and the record says so.
    #[test]
    fn a_plan_that_would_drop_the_autopilot_floors_is_not_proposed() {
        let store = Rc::new(Shared::default());
        let r = replan_round_with(&store, live_of(UNPLANNED_10487), json!([train(0, 5, 9000), train(6, 9, 7000)]),
                                  "fr_from={p},fr_to=99,fr_cargo=1,own_lines=4", json!([]), |_, _| (0.8, 0.6, 0.9));
        let mine: Vec<&String> = r.sent.iter().filter(|s| s.starts_with("0:")).collect();
        assert_eq!(mine, ["0:fr_cargo@replan", "0:fr_from@replan", "0:fr_to@replan", "0:own_lines@replan"]);
        let all = r.printed.join("\n");
        assert!(all.contains("floor: own_lines 4 -> 10") && all.contains("own_lines = 10: pending approval #4"), "{all}");
        assert!(!all.contains("own_lines = 4"), "own_lines=4 is never filed: {all}");
        // Every setting the bot would hold once each filed row is approved meets the floors.
        let mut after: BTreeMap<String, i64> = UNPLANNED_10487.iter().map(|(n, v)| (n.to_string(), *v)).collect();
        for l in r.printed.iter().filter(|l| l.contains(": pending approval #")).take(4) {
            let (name, rest) = l.trim().split_once(" = ").unwrap();
            after.insert(name.to_string(), rest.split(':').next().unwrap().parse().unwrap());
        }
        assert!(after["fr_from"] >= 0 && after["fr_to"] >= 0 && after["fr_cargo"] >= 0, "the plan makes the bot planned: {after:?}");
        assert!(after["own_lines"] >= 10 && after["max_routes"] >= 30 && after["cash_reserve"] <= 5000 && after["intercity"] == 1, "{after:?}");
        let rec = &r.records.iter().find(|(k, _)| k == "replan.0").unwrap().1;
        assert_eq!(rec["floors"]["raised"], json!([{"name": "own_lines", "from": 4, "to": 10}]));
        assert_eq!((rec["floors"]["planned_after"].clone(), rec["held"].clone()), (json!(true), Value::Null), "proposed: nothing held");
        assert_eq!(rec["sent"], json!(["fr_cargo", "fr_from", "fr_to", "own_lines"]));
    }

    /// The record is kept per mode, and a plan is judged by the mode of the lines it adds: two
    /// earning train lines and three losing bus lines (game 10487, 1951-09-29: "3 of 5 lines
    /// earned" held a rail plan) no longer veto a rail plan. A company with no rail record holds,
    /// and says why in `held`; one held by another planner says that.
    #[test]
    fn a_rail_plan_is_judged_by_the_rail_record_and_a_hold_says_why() {
        let store = Rc::new(Shared::default());
        let vehicles = json!([train(0, 5, 35000), train(0, 6, 36000), bus(0, 7, -300), bus(0, 8, -200), bus(0, 9, -100), bus(6, 10, 500)]);
        let r = replan_round_with(&store, live_of(UNPLANNED_10487), vehicles.clone(), "fr_from={p},fr_to=99,fr_cargo=1", json!([]), bayes_share);
        let rec = |k: &str| r.records.iter().find(|(x, _)| x == k).unwrap().1.clone();
        let zero = rec("replan.0");
        assert_eq!(zero["record"]["by_mode"], json!({"rail": {"lines": 2, "earned": 2, "lost": 0, "profit": 71000, "earned_per_vehicle": 71000.0},
                                                     "road": {"lines": 3, "earned": 0, "lost": 3, "profit": -600, "earned_per_vehicle": 0.0}}));
        assert_eq!((zero["record"]["earned"].clone(), zero["record"]["lines"].clone()), (json!(2), json!(5)), "pooled: 2 of 5, below the switch");
        assert_eq!((zero["model"]["judged_by"].clone(), zero["model"]["adopt"].clone()), (json!("rail"), json!(true)));
        assert_eq!(zero["model"]["by_mode"]["rail"]["earned"], json!(2));
        assert!(r.sent.iter().any(|s| s == "0:fr_from@replan"), "the rail plan is proposed: {:?}", r.sent);
        assert!(r.printed.iter().any(|l| l.contains("judged by rail: 2 of 2")), "{:?}", r.printed);

        let six = rec("replan.6");
        assert_eq!(six["model"]["judged_by"], json!("rail"));
        let held = six["held"].as_str().expect("a hold is a reason");
        assert!(held.starts_with("the model holds the current plan (4 setting(s) would change): ") && held.contains("a new rail line"), "{held}");
        assert_eq!(six["held_by"], Value::Null);
        assert!(!r.sent.iter().any(|s| s.starts_with("6:")), "company 6 holds: {:?}", r.sent);
        assert!(r.printed.iter().any(|l| l.contains(&format!("holding: {held}"))), "the reason is logged too");

        store.set("planner.6", json!("battlebrain"), "runbook", Some(0)).unwrap();
        let r = replan_round_with(&store, live_of(UNPLANNED_10487), vehicles.clone(), "fr_from={p},fr_to=99,fr_cargo=1",
                                  json!([{"tool": "openttd.ai.set_setting", "args": {"company": 0.0}}]), bayes_share);
        let rec = |k: &str| r.records.iter().find(|(x, _)| x == k).unwrap().1.clone();
        assert!(rec("replan.6")["held"].as_str().unwrap().contains("planned by battlebrain"), "{}", rec("replan.6")["held"]);
        assert_eq!(rec("replan.6")["held_by"]["holder"], "battlebrain");
        assert_eq!(rec("replan.0")["held"], json!("1 earlier change(s) for company 1 still wait for the owner's approval"));
        assert!(r.sent.is_empty(), "{:?}", r.sent);
    }

    /// A plan of the company still pending (its rows filed under one plan id, fix/approval-plan-batch)
    /// holds the round: nothing is filed beside it, not even a lost floor, and `held` says why. A
    /// newer plan withdraws the older one only after the older one settled or expired, never every
    /// month (run 10487 re-filed the slots monthly, 1952-04 to 1953-10).
    #[test]
    fn a_pending_plan_holds_the_round() {
        let store = Rc::new(Shared::default());
        let mut planned: Vec<(&str, i64)> = UNPLANNED_10487.to_vec();
        planned.retain(|(n, _)| !n.starts_with("fr"));
        planned.extend([("fr_from", 50), ("fr_to", 99), ("fr_cargo", 1)]);
        let row = |name: &str| json!({"tool": "openttd.ai.set_setting", "args": {"company": 0, "name": name, "value": 1, "plan": "plan-c0-e7-t5"}});
        let pending = json!([row("fr_from"), row("fr_to"), row("fr_cargo"), row("own_lines")]);
        let r = replan_round_with(&store, live_of(&planned), json!([]), "fr_from={p},fr_to=99,fr_cargo=1", pending, |_, _| (0.8, 0.6, 0.9));
        assert!(!r.sent.iter().any(|s| s.starts_with("0:")), "company 0's pending plan holds it: {:?}", r.sent);
        let rec = &r.records.iter().find(|(k, _)| k == "replan.0").unwrap().1;
        assert_eq!(rec["held"], json!("4 earlier change(s) for company 1 still wait for the owner's approval"));
        assert_eq!(rec["sent"], json!([]));
        assert!(r.sent.iter().any(|s| s.starts_with("6:")), "another company's round goes on: {:?}", r.sent);
    }

    /// A bot planned already (10487 after 1952-04-15: a slot complete, own_lines 4) whose new plan
    /// the model holds is offered back the floor it lost, alone; a plan equal to the live settings
    /// of an unplanned bot files nothing.
    #[test]
    fn a_planned_bot_is_offered_its_lost_floors_while_the_plan_holds() {
        let store = Rc::new(Shared::default());
        let mut planned: Vec<(&str, i64)> = UNPLANNED_10487.to_vec();
        planned.retain(|(n, _)| !n.starts_with("fr"));
        planned.extend([("fr_from", 50), ("fr_to", 99), ("fr_cargo", 1)]);
        let r = replan_round_with(&store, live_of(&planned), json!([]), "fr_from={p},fr_to=99,fr_cargo=1", json!([]), |_, _| (0.2, 0.1, 0.3));
        assert_eq!(r.sent, ["0:own_lines@replan", "6:own_lines@replan"], "only the lost floor, while the plan holds");
        let rec = &r.records.iter().find(|(k, _)| k == "replan.0").unwrap().1;
        assert!(rec["held"].as_str().unwrap().starts_with("the model holds the current plan"), "{}", rec["held"]);
        assert_eq!((rec["sent"].clone(), rec["floors"]["lost"].clone()), (json!(["own_lines"]), json!([{"name": "own_lines", "from": 4, "to": 10}])));

        let r = replan_round_with(&store, live_of(UNPLANNED_10487), json!([]), "own_lines=4", json!([]), |_, _| (0.8, 0.6, 0.9));
        assert!(r.sent.is_empty(), "{:?}", r.sent);
        assert!(r.printed.iter().any(|l| l.contains("the plan equals the bot's live settings")), "{:?}", r.printed);
        assert_eq!(r.records.iter().find(|(k, _)| k == "replan.0").unwrap().1["held"], Value::Null);
    }

    /// The key's format lives in fr_author.rhai alone: no other script of the library spells it.
    #[test]
    fn no_other_script_spells_the_planner_key() {
        let mut stack = vec![std::path::PathBuf::from(RHAI_DIR)];
        while let Some(d) = stack.pop() {
            for f in std::fs::read_dir(&d).unwrap().flatten().map(|f| f.path()) {
                if f.is_dir() { stack.push(f); continue; }
                if f.extension().and_then(|x| x.to_str()) != Some("rhai") || f.file_name().unwrap() == "fr_author.rhai" { continue; }
                let code: String = std::fs::read_to_string(&f).unwrap().lines()
                    .map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
                assert!(!code.contains("planner."), "{} spells the planner key; import fr_author instead", f.display());
            }
        }
    }
}
