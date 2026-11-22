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
