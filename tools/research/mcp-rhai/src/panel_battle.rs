//! RESEARCH-ONLY: the LAN battle's contestants as the panel serves them (GET
//! /api/battle/contestants), read from tools/research/ai-battle/battle.json on every request.
//!
//! The live arena (tools/research/mcp-panel/rhai/arena.rhai, research/live-arena.md) plays the
//! battle's contestants and used to hold a hand copy of them, which drifted whenever the battle's
//! lineup changed. It reads this route instead (`api_battleContestants()`), so battle.json is the
//! one place the lineup is written. Read-only: the file stays battle.py's.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::panel::{Game, Input};

/// The battle's configuration, relative to the repository root.
pub const CONFIG: &str = "tools/research/ai-battle/battle.json";

/// The battle's configuration in the checkout the panel serves.
pub fn config_path() -> PathBuf {
    crate::panel_results::repo().join(CONFIG)
}

/// The contestants of a battle.json text, in the file's order, each as the file has it with
/// `settings` made explicit ("" for the AI's defaults, as start_ai takes it). Refused: a text that
/// is not JSON, no `contestants` list, an entry that is not an object, has no AI name or has
/// settings that are not a string.
pub fn contestants(text: &str) -> Result<Vec<Value>, String> {
    let cfg: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let list = cfg.get("contestants").and_then(Value::as_array).ok_or("no `contestants` list")?;
    list.iter().enumerate().map(|(i, c)| {
        let mut c = c.as_object().cloned().ok_or(format!("contestants[{i}] is not an object"))?;
        if !c.get("ai").and_then(Value::as_str).is_some_and(|a| !a.is_empty()) {
            return Err(format!("contestants[{i}] names no `ai`"));
        }
        match c.get("settings") {
            None | Some(Value::Null) => { c.insert("settings".into(), json!("")); }
            Some(Value::String(_)) => {}
            Some(_) => return Err(format!("contestants[{i}]: `settings` is not a string")),
        }
        Ok(Value::Object(c))
    }).collect()
}

/// The route's answer for the configuration at `path`: 200 with the contestants, 404 when the
/// file is missing, an error when it does not hold a lineup.
pub fn answer(path: &Path) -> Result<(u16, Value), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return Ok((404, json!({"error": format!("{}: {e}", path.display())}))),
    };
    let list = contestants(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((200, json!({"file": CONFIG, "count": list.len(), "contestants": list})))
}

pub fn h_contestants(_: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    answer(&config_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use rhai::module_resolvers::StaticModuleResolver;
    use rhai::{Dynamic, Engine, EvalAltResult, Module, Scope};

    use crate::panel::{parse_args, routes, Panel, Query, Request};

    fn battle() -> Vec<Value> {
        contestants(&std::fs::read_to_string(config_path()).unwrap()).unwrap()
    }

    #[test]
    fn the_contestants_are_battle_json_s_with_settings_made_explicit() {
        let text = r#"{"contestants": [{"ai": "A", "label": "A x", "group": "g"}, {"ai": "B", "settings": "k=1"},
                                       {"ai": "C", "settings": null}], "match": {}}"#;
        let list = contestants(text).unwrap();
        assert_eq!(list, vec![json!({"ai": "A", "label": "A x", "group": "g", "settings": ""}),
                              json!({"ai": "B", "settings": "k=1"}), json!({"ai": "C", "settings": ""})]);
        for (bad, why) in [("nope", "not JSON"), ("{}", "no `contestants` list"), (r#"{"contestants": [1]}"#, "is not an object"),
                           (r#"{"contestants": [{"ai": ""}]}"#, "names no `ai`"),
                           (r#"{"contestants": [{"ai": "A", "settings": 3}]}"#, "`settings` is not a string")] {
            let e = contestants(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn the_route_serves_the_checkouts_battle_json_and_says_when_it_cannot() {
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(config_path()).unwrap()).unwrap();
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", "1"].map(String::from)).unwrap();
        let p = Panel { game: Game::new(args), routes: routes() };
        let req = Request { method: "GET".into(), path: "/api/battle/contestants".into(), query: Query::default(), headers: vec![], body: vec![] };
        let (code, out) = p.dispatch(&req, Value::Null);
        assert_eq!(code, 200, "no MCP needed: {out}");
        assert_eq!(out["file"], CONFIG);
        let list = out["contestants"].as_array().unwrap();
        assert_eq!((out["count"].as_u64(), list.len()), (Some(list.len() as u64), raw["contestants"].as_array().unwrap().len()));
        for (served, declared) in list.iter().zip(raw["contestants"].as_array().unwrap()) {
            assert_eq!(served["ai"], declared["ai"]);
            assert_eq!(served["settings"], declared.get("settings").cloned().unwrap_or(json!("")));
        }
        let dir = std::env::temp_dir().join(format!("panel-battle-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (code, out) = answer(&dir.join("missing.json")).unwrap();
        assert_eq!(code, 404);
        assert!(out["error"].as_str().unwrap().contains("missing.json"), "{out}");
        std::fs::write(dir.join("bad.json"), "{\"contestants\": 5}").unwrap();
        assert!(answer(&dir.join("bad.json")).unwrap_err().contains("no `contestants` list"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What one round of arena.rhai did: the AIs it started as (name, settings), what it printed,
    /// and the error that ended it.
    struct Round {
        started: Vec<(String, String)>,
        printed: Vec<String>,
        end: String,
    }

    /// arena.rhai from this checkout, run for one round against stubs: `api_bots()` answers
    /// `running`, `api_battleContestants()` this route's answer (unregistered when `served` is
    /// false, as on a panel built before the route), `ttd` a module without a game, and `sleep`
    /// ends the run.
    fn one_round(running: Value, served: bool) -> Round {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel/rhai/arena.rhai")).unwrap();
        let started = Arc::new(Mutex::new(Vec::new()));
        let printed = Arc::new(Mutex::new(Vec::new()));
        let mut e = Engine::new();
        let s = started.clone();
        e.register_fn("started", move |name: &str, settings: &str| { s.lock().unwrap().push((name.to_string(), settings.to_string())); });
        let pr = printed.clone();
        e.on_print(move |t| pr.lock().unwrap().push(t.to_string()));
        e.register_fn("api_bots", move || rhai::serde::to_dynamic(json!({"running": running.clone()})));
        if served {
            e.register_fn("api_battleContestants", || -> Result<Dynamic, Box<EvalAltResult>> {
                rhai::serde::to_dynamic(answer(&config_path()).map_err(|e| e.to_string())?.1)
            });
        }
        e.register_fn("con_say", |_: &str| {});
        e.register_fn("loopback_get", |_: &str| -> Result<Dynamic, Box<EvalAltResult>> { Err("no battle here".into()) });
        e.register_fn("sleep", |_: i64| -> Result<(), Box<EvalAltResult>> { Err("one round".into()) });
        let stub = "fn wait_running(s) { #{} }\nfn start_ai(name, settings) { started(name, settings) }\n\
                    fn companies() { [] }\nfn state() { #{ date: \"1950-01-01\" } }\nfn news(t) { }\n";
        let m = Module::eval_ast_as_new(Scope::new(), &e.compile(stub).unwrap(), &e).unwrap();
        let mut r = StaticModuleResolver::new();
        r.insert("ttd", m);
        e.set_module_resolver(r);
        let end = e.run(&text).unwrap_err().to_string();
        let started = started.lock().unwrap().clone();
        let printed = printed.lock().unwrap().clone();
        Round { started, printed, end }
    }

    fn rows(ais: &[&str]) -> Value {
        Value::Array(ais.iter().enumerate().map(|(i, a)| json!({"id": i, "ai": a})).collect())
    }

    #[test]
    fn the_arena_starts_the_battles_contestants_first_then_its_local_entries() {
        let battle = battle();
        let ai = |c: &Value| c["ai"].as_str().unwrap().to_string();
        // An empty game: the battle's first contestant, as battle.json names and configures it.
        let r = one_round(json!([]), true);
        assert!(r.end.contains("one round"), "{}", r.end);
        assert_eq!(r.started, vec![(format!("\"{}\"", ai(&battle[0])), battle[0]["settings"].as_str().unwrap().to_string())]);
        assert!(r.printed.iter().any(|l| l.contains("local entries")), "{:?}", r.printed);
        // Every contestant of the battle runs: the next company is a local entry, which the
        // battle does not play.
        let names: Vec<String> = battle.iter().map(ai).collect();
        let r = one_round(rows(&names.iter().map(String::as_str).collect::<Vec<_>>()), true);
        assert_eq!(r.started.len(), 1, "{:?}", r.printed);
        let (name, settings) = &r.started[0];
        assert!(!battle.iter().any(|c| format!("\"{}\"", ai(c)) == *name && c["settings"] == json!(settings)), "{name} {settings}");
        // The whole lineup runs: nothing starts.
        let mut all = names.clone();
        for _ in 0..8 { all.push(name.trim_matches('"').to_string()); }
        assert!(one_round(rows(&all.iter().map(String::as_str).collect::<Vec<_>>()), true).started.is_empty());
    }

    #[test]
    fn an_arena_on_a_panel_without_the_route_starts_nothing_and_says_why() {
        let r = one_round(json!([]), false);
        assert!(r.started.is_empty());
        assert!(r.printed.iter().any(|l| l.starts_with("lineup: ") && l.contains("api_battleContestants") && l.contains("nothing starts")),
                "{:?}", r.printed);
        assert!(r.end.contains("one round"), "the round ran to its end: {}", r.end);
    }
}
