//! The recorded experiments under `research/results/`, projected into what the atlas compares.
//!
//! Nothing here judges a bot. Each record keeps its own numbers; what the projection adds is
//! derived from fields the record holds, by the rules in [`COMPARABILITY`], and every row carries
//! the path of the JSON it came from and of the write-up beside it.

use serde_json::{json, Value};
use std::collections::BTreeSet;

use crate::repo::Repo;

/// How a row is classed, in the order the page and the README show the classes.
pub const COMPARABILITY: [(&str, &str); 3] = [
    ("valid", "a paired comparison: both companies in one game, from a held start, every month recorded"),
    ("limited", "comparable with a stated caveat: the start was not held, the two arms ran as two games, or the record was reconstructed"),
    ("invalid", "not a comparison: months missing, or the run fell back"),
];

fn int(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_i64)
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// `duel-20261119T124719-seed4242` -> `2026-11-19T12:47:19Z` (the run's start, from its name).
pub fn stamp(name: &str) -> Option<String> {
    let rest = name.split_once('-')?.1;
    let d = rest.get(0..15)?;
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !(digits(&d[0..8]) && &d[8..9] == "T" && digits(&d[9..15])) {
        return None;
    }
    Some(format!("{}-{}-{}T{}:{}:{}Z", &d[0..4], &d[4..6], &d[6..8], &d[9..11], &d[11..13], &d[13..15]))
}

fn company(c: &Value) -> Value {
    json!({
        "value": int(c, "value"), "money": int(c, "money"), "loan": int(c, "loan"),
        "trains": int(c, "trains"), "road": int(c, "road"), "planes": int(c, "planes"),
        "vehicles": int(c, "vehicles"), "name": text(c, "name"),
    })
}

/// The revision a record names for the code it ran, and whether that tree was dirty.
fn revision(d: &Value) -> (Option<String>, bool) {
    fn from(v: Option<&Value>) -> Option<&str> {
        v.and_then(|v| v.pointer("/data/version/revision")).and_then(Value::as_str)
    }
    let rev = from(d.get("verdict_event")).or_else(|| from(d.get("plan_events").and_then(|p| p.get(0))));
    match rev {
        Some(r) => {
            let dirty = r.ends_with("-dirty");
            (Some(r.trim_end_matches("-dirty").to_string()), dirty)
        }
        None => (None, false),
    }
}

/// B's live settings when the record read them (`rival`), as `key=value` pairs in key order.
fn rival_settings(d: &Value) -> Option<String> {
    let s = d.pointer("/rival/settings")?.as_object()?;
    Some(s.iter().map(|(k, v)| format!("{k}={}", v.as_i64().map(|n| n.to_string()).unwrap_or_else(|| v.to_string()))).collect::<Vec<_>>().join(","))
}

pub struct Rows {
    pub rows: Vec<Value>,
    pub revisions: BTreeSet<String>,
}

/// Every duel record (`duel-*.json`) and every A/B record (`ab-*.json`), oldest first.
pub fn comparisons(repo: &Repo) -> Result<Rows, String> {
    let mut rows = vec![];
    let mut revisions = BTreeSet::new();
    let mut paths: Vec<(String, &str)> = repo.records("research/results", "duel-", ".json").into_iter().map(|p| (p, "duel")).collect();
    paths.extend(repo.records("research/results", "ab-", ".json").into_iter().map(|p| (p, "ab")));
    for (path, kind) in paths {
        let raw = std::fs::read_to_string(repo.root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
        let d: Value = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
        let name = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".json").to_string();
        let (a, b, series) = if kind == "duel" {
            let (Some(a), Some(b)) = (d.pointer("/final/1"), d.pointer("/final/2")) else { continue };
            let s = d.get("series").and_then(Value::as_array).cloned().unwrap_or_default();
            let pick = |k: &str| -> Vec<Value> { s.iter().map(|m| m.pointer(&format!("/companies/{k}/value")).cloned().unwrap_or(Value::Null)).collect() };
            let months: Vec<Value> = s.iter().map(|m| m.get("month").cloned().unwrap_or(Value::Null)).collect();
            (a.clone(), b.clone(), json!({"month": months, "a": pick("1"), "b": pick("2")}))
        } else {
            let (Some(a), Some(b)) = (d.pointer("/final/A"), d.pointer("/final/B")) else { continue };
            let arm = |k: &str| -> Vec<Value> {
                d.pointer(&format!("/series/{k}")).and_then(Value::as_array).map(|s| s.iter().map(|m| m.get("value").cloned().unwrap_or(Value::Null)).collect()).unwrap_or_default()
            };
            let months: Vec<Value> = d.pointer("/series/A").and_then(Value::as_array)
                .map(|s| s.iter().map(|m| m.get("month").cloned().unwrap_or(Value::Null)).collect()).unwrap_or_default();
            (a.clone(), b.clone(), json!({"month": months, "a": arm("A"), "b": arm("B")}))
        };
        let months = int(&d, "months");
        let recorded = series["month"].as_array().map(Vec::len).unwrap_or(0) as i64;
        let missing = d.get("missing_months").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        let fallback = d.get("fallback").filter(|v| !v.is_null()).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
        let held = d.get("held_start").is_some_and(|v| !v.is_null());
        let reconstructed = ["rebuilt", "recovered"].iter().find_map(|k| d.get(*k).and_then(|v| v.get("why")).and_then(Value::as_str).map(str::to_string));
        let (rev, dirty) = revision(&d);
        if let Some(r) = &rev {
            revisions.insert(r.clone());
        }
        let (va, vb) = (int(&a, "value").unwrap_or(0), int(&b, "value").unwrap_or(0));
        let mut caveats: Vec<String> = vec![];
        let mut problems: Vec<String> = vec![];
        if months.is_some_and(|m| recorded < m) {
            problems.push(format!("{recorded} of {} months recorded", months.unwrap_or(0)));
        }
        if missing > 0 {
            problems.push(format!("{missing} month(s) missing"));
        }
        if let Some(f) = &fallback {
            problems.push(format!("fell back: {f}"));
        }
        if kind == "ab" {
            caveats.push("two separate games side by side, start not held".to_string());
        } else if !held {
            caveats.push("start not held: the companies may have started at different ticks".to_string());
        }
        if let Some(why) = &reconstructed {
            caveats.push(format!("record reconstructed: {why}"));
        }
        let class = if !problems.is_empty() { "invalid" } else if caveats.is_empty() { "valid" } else { "limited" };
        let mut confounders = vec![];
        match int(&b, "trains") {
            Some(0) => confounders.push("rival ran no train".to_string()),
            None => confounders.push("rival's fleet not recorded".to_string()),
            Some(_) => {}
        }
        if kind == "duel" && d.get("opponent").is_none() {
            confounders.push("rival AI not recorded".to_string());
        }
        if va <= 1 || vb <= 1 {
            // OpenTTD reports a company with nothing left (or not yet started) at value 1.
            confounders.push("a company ended with no assets (value 1)".to_string());
        }
        let rel = if vb > 1 { Some(((va - vb) as f64 / vb as f64 * 10000.0).round() / 10000.0) } else { None };
        rows.push(json!({
            "id": name,
            "kind": kind,
            "path": path,
            "page": path.trim_end_matches(".json").to_string() + ".md",
            "at": stamp(&name),
            "seed": int(&d, "seed"),
            "months": months,
            "recorded_months": recorded,
            "label": text(&d, "label").unwrap_or_default(),
            "experiment": text(&d, "experiment"),
            "opponent": text(&d, "opponent"),
            "bot_settings": text(&d, "bot_settings"),
            "rival_settings": rival_settings(&d),
            "held_start": held,
            "held_start_tick": int(&d, "held_start_tick"),
            "revision": rev,
            "revision_dirty": dirty,
            "a": company(&a),
            "b": company(&b),
            "winner": text(&d, "winner"),
            "a_ahead": va > vb,
            "delta": va - vb,
            "relative": rel,
            "class": class,
            "caveats": caveats,
            "problems": problems,
            "confounders": confounders,
            "series": series,
        }));
    }
    rows.sort_by(|x, y| (x["at"].as_str(), x["id"].as_str()).cmp(&(y["at"].as_str(), y["id"].as_str())));
    Ok(Rows { rows, revisions })
}

/// The LAN battle: every record's headline in time order, and the newest record in full.
pub fn battles(repo: &Repo) -> Result<Value, String> {
    let paths = repo.records("research/results", "battle-", ".json");
    let mut history = vec![];
    let mut newest: Option<(String, Value)> = None;
    for path in paths {
        let raw = std::fs::read_to_string(repo.root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
        let d: Value = serde_json::from_str(&raw).map_err(|e| format!("{path}: {e}"))?;
        if d.get("schema").and_then(Value::as_str) != Some("ai-battle-record/v1") {
            continue;
        }
        let board = d.get("leaderboard").and_then(Value::as_array).cloned().unwrap_or_default();
        let ai_of = |key: &str| -> Option<String> {
            d.get("contestants")?.as_array()?.iter().find(|c| c.get("key").and_then(Value::as_str) == Some(key)).and_then(|c| text(c, "ai"))
        };
        let ours: Vec<&Value> = board.iter().filter(|r| {
            let k = r.get("key").and_then(Value::as_str).unwrap_or("");
            ai_of(k).as_deref() == Some("PrismaticAI")
        }).collect();
        let best = ours.iter().min_by(|x, y| x["avg_rank"].as_f64().partial_cmp(&y["avg_rank"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
        let scored = d.get("matches").and_then(Value::as_array).map(|m| m.iter().filter(|x| x.get("status").and_then(Value::as_str) == Some("done") && x.get("replica_of").is_none_or(Value::is_null)).count()).unwrap_or(0);
        let det = d.get("determinism").and_then(Value::as_array).cloned().unwrap_or_default();
        let id = text(&d, "id").unwrap_or_default();
        history.push(json!({
            "id": id,
            "path": path,
            "page": path.trim_end_matches(".json").to_string() + ".md",
            "at": text(&d, "generated_at"),
            "edition": text(&d, "edition"),
            "rev": d.pointer("/provenance/rev").and_then(Value::as_str),
            "matches": scored,
            "field": board.len(),
            "winner": board.first().and_then(|r| text(r, "key")),
            "winner_points": board.first().and_then(|r| r.get("avg_points")).cloned(),
            "prismatic_best": best.and_then(|r| text(r, "key")),
            "prismatic_best_rank": best.and_then(|r| r.get("avg_rank")).cloned(),
            "prismatic_best_points": best.and_then(|r| r.get("avg_points")).cloned(),
            "prismatic_last": ours.iter().any(|r| r.get("avg_rank").and_then(Value::as_f64) == board.iter().filter_map(|x| x.get("avg_rank").and_then(Value::as_f64)).reduce(f64::max)),
            "replays": det.len(),
            "replays_identical": det.iter().filter(|r| r.get("identical").and_then(Value::as_bool) == Some(true)).count(),
        }));
        let at = text(&d, "generated_at").unwrap_or_default();
        if newest.as_ref().is_none_or(|(t, _)| at.as_str() >= t.as_str()) {
            let leaderboard: Vec<Value> = board.iter().map(|r| {
                let key = text(r, "key").unwrap_or_default();
                json!({
                    "key": key, "ai": ai_of(&key), "played": r.get("played"), "wins": r.get("wins"),
                    "points": r.get("points"), "avg_points": r.get("avg_points"), "avg_rank": r.get("avg_rank"),
                    "avg_value": r.get("avg_value"), "errors": r.get("errors"), "anomalies": r.get("anomalies"),
                })
            }).collect();
            let cfg = d.get("match_cfg").cloned().unwrap_or(Value::Null);
            let hosts: Vec<Value> = d.get("hosts").and_then(Value::as_array).map(|h| h.iter().map(|x| json!({"name": x.get("name"), "versions": x.get("versions")})).collect()).unwrap_or_default();
            newest = Some((at, json!({
                "id": id, "path": path, "page": path.trim_end_matches(".json").to_string() + ".md",
                "at": text(&d, "generated_at"), "edition": text(&d, "edition"), "scoring": text(&d, "scoring"),
                "rev": d.pointer("/provenance/rev"), "dirty": d.pointer("/provenance/dirty"),
                "baseset": d.pointer("/provenance/baseset"), "match_cfg": cfg, "hosts": hosts,
                "matches": scored, "leaderboard": leaderboard, "determinism": det,
            })));
        }
    }
    history.sort_by(|x, y| (x["at"].as_str(), x["id"].as_str()).cmp(&(y["at"].as_str(), y["id"].as_str())));
    Ok(json!({"records": history.len(), "history": history, "latest": newest.map(|(_, v)| v)}))
}

#[cfg(test)]
mod tests {
    use super::stamp;

    #[test]
    fn stamps_from_record_names() {
        assert_eq!(stamp("duel-20261119T124719-seed4242").as_deref(), Some("2026-11-19T12:47:19Z"));
        assert_eq!(stamp("duel-20261118T092354r-seed4242").as_deref(), Some("2026-11-18T09:23:54Z"));
        assert_eq!(stamp("ab-20261116T014214-seed22311").as_deref(), Some("2026-11-16T01:42:14Z"));
        assert_eq!(stamp("duel-x"), None);
    }
}
