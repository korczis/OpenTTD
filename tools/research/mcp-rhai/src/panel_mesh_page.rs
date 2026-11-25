//! RESEARCH-ONLY. The Majordomus mesh in the panel: the machines, runtimes and sessions this
//! repository's Majordomus server cooperates with, read from its own HTTP API
//! (`GET /api/v1/mesh`, `/api/v1/mesh/peers`, `/api/v1/mesh/nodes`) and answered unchanged under one
//! panel route, so the page, OpenAPI, Swagger UI and Rhai read the same thing. Majordomus sends no
//! CORS headers, so the browser cannot read it directly; this is the panel's read-only proxy.
//!
//! Read-only: only GETs, never a claim, beat, enrol or session. The server is
//! `$MAJORDOMUS_URL`, else the server `majordomus serve ensure` names for this checkout: the one
//! resolution every tool here shares (src/board.rs `majordomus_url`), found once and kept.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::panel::{Game, Input};

/// The parts of the answer, each the Majordomus path it is read from.
pub const PARTS: [(&str, &str); 3] = [
    ("mesh", "/api/v1/mesh"),
    ("peers", "/api/v1/mesh/peers"),
    ("nodes", "/api/v1/mesh/nodes"),
];

/// The Majordomus server the panel reads, without a trailing slash: `$MAJORDOMUS_URL` when set,
/// else what `majordomus serve ensure` names for this checkout, asked once per process.
pub fn base_url() -> Result<String, String> {
    if let Ok(u) = std::env::var("MAJORDOMUS_URL") {
        let u = u.trim().trim_end_matches('/');
        if !u.is_empty() { return Ok(u.to_string()); }
    }
    static DERIVED: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    DERIVED.get_or_init(|| crate::rhai_host::board::majordomus_url(None, None).map(|u| u.trim_end_matches('/').to_string())).clone()
}

fn get(url: &str) -> Result<Value, String> {
    ureq::get(url).timeout(Duration::from_secs(3)).call()
        .map_err(|e| e.to_string())?
        .into_json::<Value>().map_err(|e| e.to_string())
}

/// The answer from the three parts' results: every part that answered under its name, every part
/// that did not under `errors`, `reachable` when at least one answered.
pub fn assemble(source: &str, results: Vec<(&str, Result<Value, String>)>, ms: u128) -> Value {
    let mut out = Map::new();
    let mut errors = Map::new();
    for (name, r) in results {
        match r {
            Ok(v) => { out.insert(name.to_string(), v); }
            Err(e) => { out.insert(name.to_string(), Value::Null); errors.insert(name.to_string(), json!(e)); }
        }
    }
    let reachable = errors.len() < PARTS.len();
    out.insert("source".into(), json!(source));
    out.insert("reachable".into(), json!(reachable));
    out.insert("errors".into(), Value::Object(errors));
    out.insert("elapsed_ms".into(), json!(ms));
    Value::Object(out)
}

/// GET /api/mesh: the mesh at a glance, its peers (machines, runtimes, sessions, claims) and every
/// discovered node, read in parallel. Answers 200 also when Majordomus is down (`reachable`
/// false, `errors` says why per part), like GET /api/bus.
pub fn h_mesh(_: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    let started = Instant::now();
    let base = match base_url() {
        Ok(b) => b,
        Err(e) => {
            let results = PARTS.iter().map(|(name, _)| (*name, Err(format!("no Majordomus server: {e}")))).collect();
            return Ok((200, assemble("(unresolved)", results, started.elapsed().as_millis())));
        }
    };
    let results = std::thread::scope(|s| {
        let handles: Vec<_> = PARTS.iter()
            .map(|(name, path)| {
                let url = format!("{base}{path}");
                (*name, s.spawn(move || get(&url)))
            })
            .collect();
        handles.into_iter()
            .map(|(name, h)| (name, h.join().unwrap_or_else(|_| Err("reader thread panicked".into()))))
            .collect::<Vec<_>>()
    });
    Ok((200, assemble(&base, results, started.elapsed().as_millis())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_majordomus_url_when_set() {
        let _e = crate::panel_mesh::test_env_lock();
        std::env::set_var("MAJORDOMUS_URL", "http://127.0.0.1:59999/");
        assert_eq!(base_url().unwrap(), "http://127.0.0.1:59999");
        std::env::remove_var("MAJORDOMUS_URL");
    }

    #[test]
    fn assemble_keeps_parts_and_errors_apart() {
        let v = assemble("http://x", vec![
            ("mesh", Ok(json!({"active": true}))),
            ("peers", Err("refused".into())),
            ("nodes", Ok(json!({"count": 0, "nodes": []}))),
        ], 7);
        assert_eq!(v["mesh"]["active"], json!(true));
        assert_eq!(v["peers"], Value::Null);
        assert_eq!(v["errors"]["peers"], json!("refused"));
        assert_eq!(v["reachable"], json!(true));
        assert_eq!(v["source"], json!("http://x"));
    }

    #[test]
    fn assemble_unreachable_when_every_part_fails() {
        let v = assemble("http://x", PARTS.iter().map(|(n, _)| (*n, Err("down".to_string()))).collect(), 1);
        assert_eq!(v["reachable"], json!(false));
        assert_eq!(v["errors"].as_object().unwrap().len(), PARTS.len());
    }
}
