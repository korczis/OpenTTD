//! RESEARCH-ONLY. Rust benchmark records in the panel: every research/results/bench-*.json of the
//! repository the panel serves (schema `rust-bench/v1`, written by the benchmark harness), checked
//! against that schema, oldest first, as `GET /api/bench`, and one record as `GET /api/bench?id=`.
//! A file that does not hold a valid record is not dropped silently: it is listed under `skipped`
//! with its reason. The route is a row of the panel's route table, so OpenAPI, Swagger UI, the hub
//! and Rhai derive it; `/bench` and `/bench/<group>/<name>` are its pages (static/bench.html).
//! Offline: nothing here touches the game or the network.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::panel::{Game, Input};
use crate::panel_results::repo;

/// The record schema this panel reads.
pub const SCHEMA: &str = "rust-bench/v1";

/// Where the records are: `MCP_BENCH_DIR` when it is set and not empty, else research/results of
/// the repository the panel serves. A browser check points it at fixtures; the gate never does.
pub fn bench_dir() -> PathBuf {
    dir_from(std::env::var_os("MCP_BENCH_DIR"))
}

/// `bench_dir()` for a given value of `MCP_BENCH_DIR`.
pub fn dir_from(var: Option<std::ffi::OsString>) -> PathBuf {
    match var.filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => repo().join("research/results"),
    }
}

/// A record id as the harness writes it (bench-<time>-<hex>), and nothing that walks out.
pub fn valid_id(id: &str) -> bool {
    id.starts_with("bench-") && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Seconds since 1970 of an RFC 3339 time (`2026-11-22T23:15:00+00:00`, `...Z`, fractions
/// allowed), so records written with different offsets still sort by when they were made.
pub fn epoch(s: &str) -> Option<i64> {
    // Every slice below is by byte offset: a non-ASCII byte anywhere is no RFC 3339 time, and is
    // refused before a slice could land inside a character (and panic every /api/bench request).
    if !s.is_ascii() {
        return None;
    }
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r).filter(|t| t.bytes().all(|c| c.is_ascii_digit())).and_then(|t| t.parse::<i64>().ok());
    let (y, mo, d, h, mi, se) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(f) = rest.strip_prefix('.') {
        let n = f.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        rest = &f[n..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        o if o.len() == 6 && matches!(&o[..1], "+" | "-") && &o[3..4] == ":" => {
            let (oh, om) = (o[1..3].parse::<i64>().ok()?, o[4..6].parse::<i64>().ok()?);
            (oh * 3600 + om * 60) * if o.starts_with('-') { -1 } else { 1 }
        }
        _ => return None,
    };
    // Days from the civil date (Howard Hinnant's algorithm), proleptic Gregorian.
    let (yy, m) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se - offset)
}

/// Whether a value holds a record of the contract: what is wrong with it, else Ok.
pub fn check(r: &Value) -> Result<(), String> {
    let s = |k: &str| r.get(k).and_then(Value::as_str);
    if s("schema") != Some(SCHEMA) {
        return Err(format!("schema: {} where {SCHEMA} is expected", r.get("schema").map_or("missing".into(), Value::to_string)));
    }
    if !s("id").is_some_and(valid_id) {
        return Err("id: bench-<time>-<hex> is expected".into());
    }
    if s("generated_at").and_then(epoch).is_none() {
        return Err("generated_at: an RFC 3339 time is expected".into());
    }
    if !s("rev").is_some_and(|v| !v.is_empty() && v.len() <= 64 && v.bytes().all(|c| c.is_ascii_hexdigit())) {
        return Err("rev: a git revision (hex) is expected".into());
    }
    if r.get("tree_dirty").is_some_and(|v| !v.is_boolean()) {
        return Err("tree_dirty: true or false is expected".into());
    }
    let benches = r.get("benches").and_then(Value::as_array).ok_or("benches: a list is expected")?;
    for (i, b) in benches.iter().enumerate() {
        let id = b.get("id").and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or(format!("benches[{i}].id: a name is expected"))?;
        for k in ["median", "mean"] {
            if !b.get(k).is_some_and(Value::is_number) {
                return Err(format!("benches[{i}] {id}: {k} is not a number"));
            }
        }
        if !b.get("unit").is_some_and(Value::is_string) {
            return Err(format!("benches[{i}] {id}: unit is missing"));
        }
    }
    match r.get("baseline") {
        None | Some(Value::Null) => {}
        Some(bl) if bl.is_object() => {
            if bl.get("regressions").is_some_and(|x| !x.is_null() && !x.is_array()) {
                return Err("baseline.regressions: a list is expected".into());
            }
        }
        Some(_) => return Err("baseline: an object or null is expected".into()),
    }
    Ok(())
}

/// Every bench-*.json of a directory: the valid records oldest first (by generated_at, then id),
/// and every other file with its reason ({file, error}).
pub fn load(dir: &Path) -> (Vec<Value>, Vec<Value>) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("bench-") && n.ends_with(".json")))
        .collect();
    files.sort();
    let (mut records, mut skipped) = (Vec::new(), Vec::new());
    for f in files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let parsed = std::fs::read_to_string(&f).map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| format!("not JSON: {e}")))
            .and_then(|v| check(&v).map(|()| v));
        match parsed {
            Ok(v) => records.push(v),
            Err(e) => skipped.push(json!({"file": name, "error": e})),
        }
    }
    let key = |v: &Value| (v["generated_at"].as_str().and_then(epoch).unwrap_or(0), v["id"].as_str().unwrap_or("").to_string());
    records.sort_by_key(key);
    (records, skipped)
}

/// The web page of a git remote on GitHub (`git@github.com:o/r.git`, `https://github.com/o/r.git`,
/// `ssh://git@github.com/o/r`): `https://github.com/o/r`, where `/commit/<rev>` is a commit. Another
/// host has no known commit page: None.
pub fn web_url(remote: &str) -> Option<String> {
    let r = remote.trim();
    let path = r.strip_prefix("git@github.com:")
        .or_else(|| r.strip_prefix("https://github.com/"))
        .or_else(|| r.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let ok = path.split('/').count() == 2 && path.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)));
    ok.then(|| format!("https://github.com/{path}"))
}

/// The commit pages of the repository the panel serves, from its `origin` remote (None without one).
pub fn repo_url() -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(repo()).args(["remote", "get-url", "origin"]).output().ok()?;
    out.status.success().then(|| web_url(&String::from_utf8_lossy(&out.stdout))).flatten()
}

/// The answer of GET /api/bench (every record) or GET /api/bench?id= (one), from a directory.
pub fn answer(dir: &Path, id: Option<&str>) -> (u16, Value) {
    let (records, skipped) = load(dir);
    if let Some(id) = id.filter(|i| !i.is_empty()) {
        if !valid_id(id) {
            return (400, json!({"error": "id: a record id, e.g. bench-20261122T231500-ab12cd34"}));
        }
        return match records.into_iter().find(|r| r["id"] == json!(id)) {
            Some(r) => (200, json!({"record": r})),
            None => (404, json!({"error": format!("no bench record {id}")})),
        };
    }
    let latest = records.last().map(|r| r["id"].clone()).unwrap_or(Value::Null);
    (200, json!({"schema": SCHEMA, "count": records.len(), "latest_id": latest, "records": records, "skipped": skipped,
                 "dir": dir.strip_prefix(repo()).unwrap_or(dir).display().to_string(), "repo_url": repo_url()}))
}

pub fn h_bench(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    Ok(answer(&bench_dir(), i.query.get("id")))
}

/// A duration in nanoseconds as a person reads it: 950 ns, 12.3 µs, 4.56 ms, 1.2 s. Another unit
/// is written as it is.
pub fn human(v: f64, unit: &str) -> String {
    if unit != "ns" {
        return format!("{v} {unit}");
    }
    let (x, u) = match v.abs() {
        a if a >= 1e9 => (v / 1e9, "s"),
        a if a >= 1e6 => (v / 1e6, "ms"),
        a if a >= 1e3 => (v / 1e3, "µs"),
        _ => (v, "ns"),
    };
    let digits = if x.abs() >= 100.0 { 0 } else if x.abs() >= 10.0 { 1 } else { 2 };
    format!("{x:.digits$} {u}")
}

/// The page title and description of /bench and /bench/<bench id>, and whether it exists, from
/// the records themselves (the newest one that measured the bench). `path` is percent-decoded.
pub fn page_meta(path: &str) -> (String, String, bool) {
    meta_in(&bench_dir(), path)
}

pub fn meta_in(dir: &Path, path: &str) -> (String, String, bool) {
    if path == "/bench" {
        return ("Benchmarks".into(), "Every Rust performance benchmark record (research/results/bench-*.json): the latest median of each bench, its history across records, and the regressions against the baseline".into(), true);
    }
    let id = path.strip_prefix("/bench/").unwrap_or("");
    let (records, _) = load(dir);
    let seen: Vec<(&Value, &Value)> = records.iter()
        .filter_map(|r| r["benches"].as_array()?.iter().find(|b| b["id"] == json!(id)).map(|b| (r, b))).collect();
    match seen.last() {
        Some((r, b)) if !id.is_empty() => (
            format!("{id}: median {}", human(b["median"].as_f64().unwrap_or(0.0), b["unit"].as_str().unwrap_or("ns"))),
            format!("{} records measured it; the latest is {} at rev {}", seen.len(), r["id"].as_str().unwrap_or(""), r["rev"].as_str().unwrap_or("").chars().take(10).collect::<String>()),
            true,
        ),
        _ => (format!("{id}: no such bench"), String::new(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixtures the panel's node tests and its browser check read (test/fixtures/bench).
    fn fixtures() -> PathBuf {
        crate::panel::here().join("test/fixtures/bench")
    }

    #[test]
    fn times_ids_and_units_are_checked() {
        assert_eq!(epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch("2026-11-22T23:15:00+00:00"), Some(1_795_389_300));
        assert_eq!(epoch("2026-11-23T00:15:00+01:00"), epoch("2026-11-22T23:15:00Z"), "an offset is applied");
        assert_eq!(epoch("2026-11-22T23:15:00.250-02:30"), Some(1_795_389_300 + 9000));
        for bad in ["", "2026-11-22", "2026-13-01T00:00:00Z", "2026-11-22T23:15:00", "2026-11-22T23:15:00.Z", "2026-11-22T23:15:00+0100",
                    "2026-11-22T23:15:00ä1:00", "2026-11-22T23:15:00+ä:0", "2026-11-22T23:15:00.5ä1:00", "2026-11-2ä23:15:00Z"] {
            assert_eq!(epoch(bad), None, "{bad}");
        }
        assert!(valid_id("bench-20261122T231500-ab12cd34"));
        assert!(!valid_id("../bench"));
        assert!(!valid_id("bench-x/../../etc"));
        assert_eq!(human(950.0, "ns"), "950 ns");
        assert_eq!(human(12_345.0, "ns"), "12.3 µs");
        assert_eq!(human(4_560_000.0, "ns"), "4.56 ms");
        assert_eq!(human(1.2e9, "ns"), "1.20 s");
        assert_eq!(human(3.0, "ops"), "3 ops");
        assert_eq!(web_url("git@github.com:korczis/OpenTTD.git\n").as_deref(), Some("https://github.com/korczis/OpenTTD"));
        assert_eq!(web_url("https://github.com/korczis/OpenTTD").as_deref(), Some("https://github.com/korczis/OpenTTD"));
        assert_eq!(web_url("ssh://git@github.com/o/r.git/").as_deref(), Some("https://github.com/o/r"));
        for other in ["git@gitlab.com:o/r.git", "https://github.com/o", "https://github.com/o/r/x", "https://github.com/o/r\"x", ""] {
            assert_eq!(web_url(other), None, "{other}");
        }
        assert_eq!(dir_from(None), repo().join("research/results"));
        assert_eq!(dir_from(Some("".into())), repo().join("research/results"));
        assert_eq!(dir_from(Some("/x".into())), PathBuf::from("/x"));
    }

    #[test]
    fn a_record_off_the_contract_says_why() {
        let good = json!({"schema": SCHEMA, "id": "bench-1-a", "generated_at": "2026-11-22T23:15:00Z", "rev": "abc123", "tree_dirty": false,
                          "benches": [{"id": "g/n", "median": 1.0, "mean": 1.0, "unit": "ns"}], "baseline": {"id": null, "regressions": []}});
        assert_eq!(check(&good), Ok(()));
        let with = |k: &str, v: Value| { let mut r = good.clone(); r[k] = v; check(&r).unwrap_err() };
        assert!(with("schema", json!("rust-bench/v0")).starts_with("schema"));
        assert!(with("id", json!("x")).starts_with("id"));
        assert!(with("generated_at", json!("yesterday")).starts_with("generated_at"));
        assert!(with("rev", json!("not hex")).starts_with("rev"));
        assert!(with("tree_dirty", json!("no")).starts_with("tree_dirty"));
        assert!(with("benches", json!({})).starts_with("benches"));
        assert!(with("benches", json!([{"median": 1}])).contains("id"));
        assert!(with("benches", json!([{"id": "g/n", "median": "1", "mean": 1, "unit": "ns"}])).contains("median"));
        assert!(with("benches", json!([{"id": "g/n", "median": 1, "mean": 1}])).contains("unit"));
        assert!(with("baseline", json!(3)).starts_with("baseline"));
        assert!(with("baseline", json!({"regressions": {}})).starts_with("baseline.regressions"));
        assert_eq!(check(&{ let mut r = good.clone(); r["baseline"] = Value::Null; r }), Ok(()));
    }

    #[test]
    fn the_fixture_records_are_listed_oldest_first_and_a_bad_one_is_reported() {
        let (code, v) = answer(&fixtures(), None);
        assert_eq!(code, 200);
        let ids: Vec<&str> = v["records"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["bench-20261120T101500-aa11bb22", "bench-20261121T101500-cc33dd44", "bench-20261122T231500-ab12cd34"]);
        assert_eq!(v["count"], json!(3));
        assert_eq!(v["latest_id"], json!("bench-20261122T231500-ab12cd34"));
        assert_eq!(v["repo_url"], json!(repo_url()), "the commit pages come from the origin remote");
        let skipped = v["skipped"].as_array().unwrap();
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        assert!(skipped.iter().any(|s| s["file"] == json!("bench-20261119T000000-0badc0de.json") && s["error"].as_str().unwrap().starts_with("schema")));
        assert!(skipped.iter().any(|s| s["file"] == json!("bench-20261119T000001-notjson0.json") && s["error"].as_str().unwrap().starts_with("not JSON")));

        let (code, one) = answer(&fixtures(), Some("bench-20261121T101500-cc33dd44"));
        assert_eq!(code, 200);
        assert_eq!(one["record"]["host"]["name"], json!("fixture-host"));
        assert_eq!(answer(&fixtures(), Some("bench-20991231T000000-00000000")).0, 404);
        assert_eq!(answer(&fixtures(), Some("../x")).0, 400);
        assert_eq!(answer(&fixtures(), Some("")).0, 200, "an empty id lists every record");

        // A record whose time carries a non-ASCII byte is skipped with its reason; the listing stands.
        let odd = std::env::temp_dir().join(format!("panel-bench-odd-{}", std::process::id()));
        std::fs::create_dir_all(&odd).unwrap();
        let mut r: Value = serde_json::from_str(&std::fs::read_to_string(fixtures().join("bench-20261121T101500-cc33dd44.json")).unwrap()).unwrap();
        r["generated_at"] = json!("2026-11-22T23:15:00ä1:00");
        std::fs::write(odd.join("bench-20261122T231500-0dd00000.json"), r.to_string()).unwrap();
        let (code, v) = answer(&odd, None);
        assert_eq!((code, v["count"].clone()), (200, json!(0)));
        assert!(v["skipped"][0]["error"].as_str().unwrap().starts_with("generated_at"), "{v}");
        std::fs::remove_dir_all(&odd).unwrap();

        let empty = std::env::temp_dir().join(format!("panel-bench-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let (_, none) = answer(&empty, None);
        assert_eq!((none["count"].clone(), none["latest_id"].clone()), (json!(0), Value::Null));
        assert_eq!(answer(&empty.join("missing"), None).1["count"], json!(0), "a missing directory holds no record");
        std::fs::remove_dir_all(&empty).unwrap();
    }

    #[test]
    fn the_pages_are_titled_from_the_records() {
        let (t, d, found) = meta_in(&fixtures(), "/bench");
        assert!(found && t == "Benchmarks" && d.contains("bench-*.json"));
        let (t, d, found) = meta_in(&fixtures(), "/bench/planner/plan_fixture");
        assert!(found, "{t}");
        assert_eq!(t, "planner/plan_fixture: median 120 ns");
        assert!(d.starts_with("3 records measured it; the latest is bench-20261122T231500-ab12cd34"), "{d}");
        assert!(!meta_in(&fixtures(), "/bench/no/such").2);
        assert!(!meta_in(&fixtures(), "/bench/").2);
        assert_eq!(page_meta("/bench").0, "Benchmarks");
    }
}
