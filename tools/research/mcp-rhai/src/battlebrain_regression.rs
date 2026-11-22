//! RESEARCH-ONLY: the regression gate of Battlebrain Phase 06
//! (research/battlebrain/experiment-freight-margin.md, check C1).
//!
//! Every shadow result under research/results/battlebrain-*.json that names the current policy
//! (`freight-margin@<POLICY_VERSION>`) is run again here with `shadow` on the capture its provenance
//! names, and its `comparison` block must come back unchanged. A result of an older policy version
//! cannot be reproduced by this code and is skipped by name. If fewer than three results of the
//! current policy remain, the test fails: a policy change has to re-record its shadow results on
//! the captures first. The same results are also checked against hypothesis H1: no selected line is
//! unpriced or costs more than the budget the policy computed.
//!
//! Compiled into the battlebrain binary's tests only (`#[cfg(test)]` include in bin/battlebrain.rs).
//! Offline: it reads files and starts no game.

use super::*;
use std::path::{Path, PathBuf};

/// The repository root, from this crate's manifest directory (tools/research/mcp-rhai).
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn json_file(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: not JSON: {e}", path.display()));
    v.get("value").cloned().unwrap_or(v)
}

/// The policy label every result carries in `comparison.policy`.
fn current_policy() -> String {
    format!("{}@{}", policy::POLICY_ID, policy::POLICY_VERSION)
}

/// Every battlebrain result record under research/results, by file name, in name order.
fn results() -> Vec<(String, Value)> {
    let dir = repo().join("research/results");
    let mut out: Vec<(String, Value)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("battlebrain-") && n.ends_with(".json"))
        .map(|n| {
            let v = json_file(&dir.join(&n));
            (n, v)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The policy parameters a result was decided with, read from its own comparison block.
fn params_of(name: &str, c: &Value) -> policy::Params {
    let p = &c["params"];
    policy::Params {
        margin: p["margin"].as_f64().unwrap_or_else(|| panic!("{name}: comparison.params.margin")),
        reserve: p["reserve"].as_i64().unwrap_or_else(|| panic!("{name}: comparison.params.reserve")),
        require_priced: p["require_priced"].as_bool().unwrap_or_else(|| panic!("{name}: comparison.params.require_priced")),
    }
}

/// `shadow` on the capture a result's provenance names: its snapshot record for the company, the
/// planner's answer and the companies read, all from that capture's directory.
fn replay(name: &str, rec: &Value) -> Value {
    let pv = &rec["provenance"];
    let dir = repo().join(pv["capture"]["dir"].as_str().unwrap_or_else(|| panic!("{name}: provenance.capture.dir")));
    let company = pv["company"].as_i64().unwrap_or_else(|| panic!("{name}: provenance.company"));
    let snapshot = json_file(&dir.join(format!("snapshot-company-{company}.json")));
    let plan = json_file(&dir.join("plan.json"));
    let companies = json_file(&dir.join("companies.json"));
    let p = params_of(name, &rec["comparison"]);
    shadow(&snapshot, &plan, &companies, company, &p).unwrap_or_else(|e| panic!("{name}: the shadow run is refused: {e:?}"))
}

/// The results this code can reproduce (the current policy) and the names of those it cannot.
fn of_this_policy() -> (Vec<(String, Value)>, Vec<String>) {
    let policy = current_policy();
    let (mut ours, mut older) = (Vec::new(), Vec::new());
    for (n, r) in results() {
        if r["comparison"]["policy"].as_str() == Some(policy.as_str()) {
            ours.push((n, r));
        } else {
            older.push(n);
        }
    }
    (ours, older)
}

#[test]
fn every_recorded_shadow_result_of_this_policy_is_reproduced_on_its_capture() {
    let (ours, older) = of_this_policy();
    assert!(ours.len() >= 3, "only {} shadow result(s) of {} under research/results ({} of an older policy: {older:?}); \
        re-record them with `battlebrain shadow` on the captures under tools/research/schema/fixtures/battlebrain/captures/",
        ours.len(), current_policy(), older.len());
    for (name, rec) in &ours {
        let out = replay(name, rec);
        assert_eq!(out["comparison"], rec["comparison"],
            "{name}: `battlebrain shadow` on its capture no longer gives the recorded comparison");
    }
}

#[test]
fn no_recorded_decision_applies_an_unpriced_or_unaffordable_line() {
    // Hypothesis H1 on the recorded captures: what battlebrain selects is priced by track-planning
    // (`track: found`) and costs at most the budget it computed (money plus loan headroom minus the
    // reserve). A HOLD selects nothing and passes.
    let (ours, _) = of_this_policy();
    assert!(!ours.is_empty());
    for (name, rec) in &ours {
        let c = &rec["comparison"];
        let Some(line) = c["battlebrain"].as_object() else { continue };
        assert_eq!(line.get("track").and_then(Value::as_str), Some("found"), "{name}: the selected line is not priced");
        let cost = line.get("investment").and_then(Value::as_f64).unwrap_or(f64::INFINITY);
        let budget = c["budget"].as_f64().unwrap_or(f64::NEG_INFINITY);
        assert!(cost <= budget, "{name}: the selected line costs {cost}, more than the budget {budget}");
    }
}

#[test]
fn a_changed_comparison_is_caught() {
    // The gate itself can fail: a recorded comparison with one field changed is not what the
    // replay gives.
    let (ours, _) = of_this_policy();
    let (name, rec) = ours.first().expect("a result of this policy");
    let mut forged = rec["comparison"].clone();
    forged["divergence"] = json!("defect");
    assert_ne!(replay(name, rec)["comparison"], forged);
}
