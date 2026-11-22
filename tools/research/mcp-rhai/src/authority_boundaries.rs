//! RESEARCH-ONLY: the architecture boundaries around the one decision authority (ADR 0037, rule
//! project.decision-authority-boundaries), checked over the working tree's own files.
//!
//! These tests read source text, never a game. Each one names the boundary it proves (B1..B8 of
//! the rule) and fails with the file and line that crosses it. A boundary the tree already
//! crosses on purpose is an entry of a named list here, with its reason, and a list entry that no
//! longer matches anything fails too, so the lists can only shrink by being edited.
//!
//! Compiled into the `battlebrain` binary's tests only (`cargo test --bin battlebrain`), so that
//! `tools/gate.sh change` runs them with the contract they guard.

use crate::battlebrain_contract as bb;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const CRATE: &str = "tools/research/mcp-rhai/src";
const THIS: &str = "tools/research/mcp-rhai/src/authority_boundaries.rs";
const CONTRACT: &str = "tools/research/mcp-rhai/src/battlebrain_contract.rs";
const AUTHORITY_BIN: &str = "tools/research/mcp-rhai/src/bin/battlebrain.rs";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The tracked files and the new ones git does not ignore, so a file about to be added is
/// scanned before it is committed.
fn tree() -> Vec<String> {
    let out = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .current_dir(repo())
        .output()
        .expect("git ls-files");
    assert!(out.status.success(), "git ls-files failed");
    let mut v: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
    v.sort();
    v.dedup();
    v
}

/// Whether a path is code that runs (not a test, a fixture, a document or this file).
fn is_code(path: &str) -> bool {
    const EXT: &[&str] = &[".rs", ".rhai", ".py", ".js", ".mjs", ".ts", ".nut", ".sh", ".cpp", ".h", ".hpp"];
    let name = path.rsplit('/').next().unwrap_or(path);
    EXT.iter().any(|e| path.ends_with(e))
        && path != THIS
        && !path.contains("/test/")
        && !path.contains("/tests/")
        && !path.contains("/fixtures/")
        && !path.contains("/node_modules/")
        && !path.starts_with("src/3rdparty/")
        && !name.starts_with("test_")
        && !name.contains(".test.")
}

/// The offset of the brace that closes the one at `open`, skipping comments, strings, raw
/// strings and char literals (a lifetime is not a char literal).
fn matching_brace(s: &[u8], open: usize) -> Option<usize> {
    let (mut i, mut depth) = (open, 0i64);
    while i < s.len() {
        match s[i] {
            b'/' if s.get(i + 1) == Some(&b'/') => {
                while i < s.len() && s[i] != b'\n' {
                    i += 1;
                }
            }
            b'r' if matches!(s.get(i + 1), Some(b'"') | Some(b'#')) && (i == 0 || !s[i - 1].is_ascii_alphanumeric()) => {
                let mut j = i + 1;
                while s.get(j) == Some(&b'#') {
                    j += 1;
                }
                if s.get(j) == Some(&b'"') {
                    let hashes = j - i - 1;
                    let mut k = j + 1;
                    while k < s.len() && !(s[k] == b'"' && s[k + 1..].iter().take(hashes).filter(|c| **c == b'#').count() == hashes) {
                        k += 1;
                    }
                    i = k + hashes;
                }
            }
            b'"' => {
                i += 1;
                while i < s.len() && s[i] != b'"' {
                    if s[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'\'' if s.get(i + 2) == Some(&b'\'') => i += 2,
            b'\'' if s.get(i + 1) == Some(&b'\\') => {
                i += 2;
                while i < s.len() && s[i] != b'\'' {
                    i += 1;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Per line of a Rust file: whether it is test code (an item under `#[cfg(test)]`).
fn rust_test_lines(text: &str) -> Vec<bool> {
    let lines: Vec<&str> = text.lines().collect();
    let starts: Vec<usize> = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let mut test = vec![false; lines.len()];
    let mut n = 0;
    while n < lines.len() {
        if lines[n].trim() != "#[cfg(test)]" {
            n += 1;
            continue;
        }
        let mut item = n + 1;
        while item < lines.len() && (lines[item].trim().starts_with("#[") || lines[item].trim().is_empty()) {
            item += 1;
        }
        let end = match lines.get(item) {
            Some(l) if l.contains('{') => {
                let open = starts[item] + l.find('{').unwrap();
                let close = matching_brace(text.as_bytes(), open).unwrap_or(text.len());
                text[..close].matches('\n').count()
            }
            _ => item,
        };
        for t in test.iter_mut().take(end.min(lines.len() - 1) + 1).skip(n) {
            *t = true;
        }
        n = end + 1;
    }
    test
}

/// The lines of a code file that run: not a comment line, not test code. (line number, text)
fn code_lines(path: &str, text: &str) -> Vec<(usize, String)> {
    let test = if path.ends_with(".rs") { rust_test_lines(text) } else { vec![false; text.lines().count()] };
    let hash_comments = path.ends_with(".py") || path.ends_with(".sh");
    text.lines()
        .enumerate()
        .filter(|(i, l)| {
            let t = l.trim_start();
            !test[*i] && !t.starts_with("//") && !t.starts_with("/*") && !t.starts_with('*') && !(hash_comments && t.starts_with('#'))
        })
        .map(|(i, l)| (i + 1, l.to_string()))
        .collect()
}

/// Every code line of the tree that contains one of `needles`: (path, line, text).
fn hits(needles: &[&str]) -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    for path in tree().into_iter().filter(|p| is_code(p)) {
        let Ok(text) = std::fs::read_to_string(repo().join(&path)) else { continue };
        if !needles.iter().any(|n| text.contains(n)) {
            continue;
        }
        for (n, l) in code_lines(&path, &text) {
            if needles.iter().any(|x| l.contains(x)) {
                out.push((path.clone(), n, l.trim().to_string()));
            }
        }
    }
    out
}

fn show(v: &[(String, usize, String)]) -> String {
    // Line 0: a list literal, located by its file only (it may span lines).
    v.iter().map(|(p, n, l)| if *n == 0 { format!("\n  {p}: [{l}]") } else { format!("\n  {p}:{n}: {l}") }).collect()
}

/// A named list entry: the files allowed to cross a boundary, and why.
type Allowed = &'static [(&'static str, &'static str)];

/// Hits outside `allowed` fail; an allowed entry with no hit is stale and fails too.
fn closed(boundary: &str, needles: &[&str], allowed: Allowed) {
    let found = hits(needles);
    let outside: Vec<_> = found.iter().filter(|(p, _, _)| !allowed.iter().any(|(a, _)| p == a)).cloned().collect();
    assert!(outside.is_empty(), "{boundary}: crossed outside its named list (rule project.decision-authority-boundaries):{}", show(&outside));
    let used: BTreeSet<&str> = found.iter().map(|(p, _, _)| p.as_str()).collect();
    for (a, why) in allowed {
        assert!(used.contains(a), "{boundary}: {a} is listed ({why}) but no longer matches: remove it from the list and the rule");
    }
}

// ---------------------------------------------------------------------------------- the scanner

#[test]
fn the_scanner_sees_code_and_skips_tests_and_comments() {
    let src = "fn a() { let s = \"}\"; let c = '}'; }\n// DECISION_SCHEMA in a comment\n#[cfg(test)]\nmod tests {\n    fn b() { let x = r#\"}\"#; DECISION_SCHEMA; }\n}\nfn after() { DECISION_SCHEMA }\n";
    let lines = code_lines("x.rs", src);
    let with: Vec<usize> = lines.iter().filter(|(_, l)| l.contains("DECISION_SCHEMA")).map(|(n, _)| *n).collect();
    assert_eq!(with, vec![7], "only the line after the test module is code");
    assert!(is_code(AUTHORITY_BIN) && !is_code("tools/research/test_x.py") && !is_code("tools/research/mcp-panel/test/a.test.mjs"));
    assert!(!is_code("README.md") && !is_code(THIS));
}

// ------------------------------------------------------------------------- B1: one decision writer

/// B1. Exactly one component builds an executable decision: only `battlebrain` writes a decision
/// record or a battlebrain-decision/v1 block. The contract defines both and validates them;
/// everyone else may only validate (`validate_decision_block`), never build one.
#[test]
fn b1_only_battlebrain_builds_a_decision_record_or_block() {
    const ALLOWED: Allowed = &[
        (CONTRACT, "the contract: defines DECISION_SCHEMA and AUTHORITY, validates, generates the schema"),
        (AUTHORITY_BIN, "the authority: battlebrain run() writes the decision record and its block"),
    ];
    closed("B1", &["battlebrain-decision/", "DECISION_SCHEMA", "::AUTHORITY", "record(\"decision\""], ALLOWED);
    // Not vacuous: the authority does build both, through the contract.
    let bin = read(AUTHORITY_BIN);
    assert!(bin.contains("bb::record(\"decision\"") && bin.contains("bb::DECISION_SCHEMA"));
}

// ------------------------------------------------------------- B2: a kernel output is a candidate

/// B2. A kernel's output reaches a freight-line choice only as a candidate: the planner's records
/// are candidates, and only battlebrain's decision selects one (the shadow run's records prove
/// the shape: src/bin/battlebrain.rs `a_shadow_run_writes_valid_records_and_a_shadow_block_only`).
/// What a running bot is handed as a freight slot (`fr_*`) is written by the authority, declared
/// and read by the bot, and by two LEGACY paths that predate the cutover; a new one fails here.
#[test]
fn b2_freight_slots_are_written_only_by_the_authority_and_the_named_legacy_paths() {
    const ALLOWED: Allowed = &[
        (AUTHORITY_BIN, "the authority's live directive: planned_freight_only=1 plus one fr_* slot"),
        (CONTRACT, "FREIGHT_SETTINGS, the contract's names of the slot"),
        ("tools/research/mcp-rhai/src/panel_decision.rs", "the executor names a slot only to refuse it in a directive without a decision block (fr_needs_decision)"),
        ("tools/research/mcp-rhai/src/panel_bot.rs", "LEGACY, a gap until the cutover: settings_from_plan hands the planner's portfolio to the bot at start_ai"),
        ("tools/research/mcp-rhai/src/intent.rs", "LEGACY, a gap until the cutover: a plain-language answer names a slot, sent by the approval-gated set_setting or a later start_ai"),
        ("tools/research/prismatic-ai/PrismaticAI/info.nut", "the bot declares its own settings"),
        ("tools/research/prismatic-ai/PrismaticAI/main.nut", "the bot reads its own settings"),
        ("tools/research/prismatic-ai/PrismaticAI/rail.nut", "the bot reads its own settings"),
    ];
    // freight_slot_prefix is the contract's one spelling of a slot name: calling it names a slot.
    closed("B2", &["fr_from", "\"fr_\"", "\"fr\"", "format!(\"fr{", "freight_slot_prefix("], ALLOWED);
    // The contract's side of B2: anyone may propose a candidate, only the authority decides.
    let writers = |kind: &str| bb::TYPES.iter().find(|t| t.0 == kind).map(|t| t.4.to_vec()).unwrap();
    assert_eq!(writers("decision"), vec![bb::AUTHORITY]);
    assert!(writers("candidate").contains(&"*"));
}

// ------------------------------------------------------------ B3: the blackboard has no command path

/// B3. A blackboard write never changes the game. The mesh record client, the panel's mirror of
/// directives into it, the board announcer, the bus sinks and the bus clients reach Majordomus,
/// ZeroMQ and DuckDB only: none of them names a game tool, a panel route that acts, or rcon.
#[test]
fn b3_a_blackboard_write_has_no_command_path() {
    const BLACKBOARD: &[&str] = &[
        "tools/research/mcp-rhai/src/mesh_records.rs",
        "tools/research/mcp-rhai/src/panel_mesh.rs",
        "tools/research/mcp-rhai/src/board.rs",
        "tools/research/mcp-rhai/src/bus_sinks.rs",
        "tools/research/mcp-rhai/src/bin/mcp-bus-blackboard.rs",
        "tools/research/mcp-rhai/src/bin/mcp-bus-tail.rs",
    ];
    const ACTS: &[&str] = &[
        "openttd.cmd.", "openttd.ai.", "openttd.input.", "openttd.ui.", "openttd.view.", "/api/client/", "/api/decision",
        "/api/bot", "/api/cmd", "set_bot_setting", "start_bot", ".tool(", "rcon",
    ];
    let mut bad = Vec::new();
    for f in BLACKBOARD {
        for (n, l) in code_lines(f, &read(f)) {
            if ACTS.iter().any(|a| l.contains(a)) || (l.contains("tools/call") && !l.contains("\"majordomus_")) {
                bad.push((f.to_string(), n, l.trim().to_string()));
            }
        }
    }
    assert!(bad.is_empty(), "B3: a blackboard file names a path that acts on a game:{}", show(&bad));
    // Not vacuous: the client does write, and only to the mesh journal.
    assert!(read(BLACKBOARD[0]).contains("self.call(\"POST\", \"/api/v1/mesh/records\""));
}

// ------------------------------------------------------- B4: no hand-kept copy of a catalogue

/// B4. No source keeps its own list of the kernel catalogue or of the MCP tools. A list literal
/// naming two kernels of the discovered catalogue (the runner's own answer, the fixture
/// catalogue.json), or three `openttd.<group>.<tool>` names, is a hand copy. Calling one kernel
/// by name, and naming the resources a function reads, are uses, not copies; a list of the tools
/// one client needs, checked against the live tools/list before it acts, is a requirement and is
/// named below.
#[test]
fn b4_no_list_literal_copies_the_kernel_or_tool_catalogue() {
    const ALLOWED: Allowed = &[
        ("tools/research/mcp-human/src/play.rs", "require_tools: the tools the hand calls, checked against the client's live tools/list before it acts"),
    ];
    let cat: Value = serde_json::from_str(&read("tools/research/schema/fixtures/battlebrain/catalogue.json")).unwrap();
    let kernels: BTreeSet<String> = cat["kernels"].as_array().unwrap().iter().filter_map(|k| k["id"].as_str().map(str::to_string)).collect();
    assert!(kernels.len() > 2, "the discovered catalogue is not empty");
    let quoted = |s: &str| -> BTreeSet<String> { s.split('"').skip(1).step_by(2).map(str::to_string).collect() };
    let mut bad = Vec::new();
    for path in tree().into_iter().filter(|p| is_code(p) && (p.starts_with("tools/") || p.starts_with("src/"))) {
        let Ok(text) = std::fs::read_to_string(repo().join(&path)) else { continue };
        let code: String = code_lines(&path, &text).into_iter().map(|(_, l)| l + "\n").collect();
        let mut rest = code.as_str();
        while let Some(open) = rest.find('[') {
            let tail = &rest[open + 1..];
            let Some(close) = tail.find(|c| c == ']' || c == '[') else { break };
            let body = &tail[..close];
            let names = quoted(body);
            let k = names.iter().filter(|n| kernels.contains(*n)).count();
            let t = names.iter().filter(|n| n.starts_with("openttd.") && n.matches('.').count() >= 2 && !n.contains(' ')).count();
            if k >= 2 || t >= 3 {
                bad.push((path.clone(), 0, body.split_whitespace().collect::<Vec<_>>().join(" ")));
            }
            rest = &tail[close..];
        }
    }
    let outside: Vec<_> = bad.iter().filter(|(p, _, _)| !ALLOWED.iter().any(|(a, _)| p == a)).cloned().collect();
    assert!(outside.is_empty(), "B4: a hand-kept catalogue list (derive it from tools/list or `flow -- kernels`):{}", show(&outside));
    for (a, why) in ALLOWED {
        assert!(bad.iter().any(|(p, _, _)| p == a), "B4: {a} is listed ({why}) but no longer matches: remove it from the list and the rule");
    }
}

/// B4. The only per-kernel code allowed is the adapter registry of domain checks
/// (battlebrain_contract.rs `domain_check`), owned by battlebrain: the runner's outputs carry no
/// schema, so an adapter at that boundary is explicit. Adding one means naming it in the rule.
#[test]
fn b4_the_adapter_registry_is_the_named_domain_checks() {
    let cat: Value = serde_json::from_str(&read("tools/research/schema/fixtures/battlebrain/catalogue.json")).unwrap();
    let adapted: Vec<String> = cat["kernels"].as_array().unwrap().iter()
        .filter_map(|k| k["id"].as_str())
        .filter(|id| bb::domain_check(id, &json!({}), &json!({})).map_or_else(|r| r.code != "no_domain_check", |_| true))
        .map(str::to_string)
        .collect();
    assert_eq!(adapted, vec!["knapsack-dp", "track-planning"],
        "the domain-check registry changed: name the adapters in .ai/repo/rules/project/decision-authority-boundaries.v1.md (B4) and research/battlebrain/README.md");
}

// ------------------------------------------------------------------- B5: provenance and identity

/// B5. Every decision record carries its provenance, its policy's identity and the snapshot it
/// decided on; `validate_record` refuses one that lacks any of them.
#[test]
fn b5_a_decision_without_provenance_policy_or_snapshot_is_refused() {
    let f: Value = serde_json::from_str(&read("tools/research/schema/fixtures/battlebrain/valid/decision.json")).unwrap();
    let mut base = f["value"].clone();
    base.as_object_mut().unwrap().remove("id");
    assert!(bb::validate_record(&base).is_ok());
    for path in ["/derived_from", "/body/policy/id", "/body/policy/version", "/body/policy/params", "/body/snapshot",
                 "/producer/id", "/producer/version", "/body/idempotency_key", "/body/valid_until"] {
        let mut v = base.clone();
        let (parent, key) = path.rsplit_once('/').unwrap();
        v.pointer_mut(if parent.is_empty() { "" } else { parent }).and_then(Value::as_object_mut).unwrap().remove(key);
        assert!(bb::validate_record(&v).is_err(), "a decision without {path} was accepted");
    }
    let mut unrelated = base.clone();
    unrelated["derived_from"] = json!([format!("bb:{}", "9".repeat(64))]);
    assert!(bb::validate_record(&unrelated).unwrap_err().iter().any(|r| r.code == "missing_provenance"));
}

// --------------------------------------------------------------------- B6: revalidate before acting

/// B6. Stale dependencies are revalidated before execution: in the executor's pull, the decision
/// block (its key and stale date, `decision_gate`) and the observation it rests on (`freshness`)
/// are checked before the first step that changes the game. A live decision leaves the pull for
/// `apply_live` only after `decision_gate`, and there the world (`live_world`: not behind the
/// decision's snapshot tick, the company an AI company now) and the stale date (`live_expiry`) are
/// checked before `openttd.ai.set_settings` is called.
#[test]
fn b6_the_executor_checks_block_and_freshness_before_any_step() {
    let src = read("tools/research/mcp-rhai/src/panel_decision.rs");
    let body_of = |sig: &str| -> &str {
        let start = src.find(sig).unwrap_or_else(|| panic!("panel_decision.rs has no {sig}"));
        let body = &src[start..];
        &body[..body[1..].find("\npub fn ").or_else(|| body[1..].find("\nfn ")).map_or(body.len(), |e| e + 1)]
    };
    let body = body_of("pub fn pull_with(");
    let at = |n: &str| body.find(n).unwrap_or_else(|| panic!("pull_with no longer calls {n}"));
    let gate = at("decision_gate(&raw");
    let fresh = at("freshness(");
    let live = at("apply_live(g");
    let first_step = ["set_bot_setting(", "start_bot(", "\"openttd.ai.reload\""].iter().filter_map(|n| body.find(n)).min().expect("pull_with acts");
    assert!(gate < fresh && fresh < first_step, "B6: pull_with must check the decision block, then freshness, before any step");
    assert!(gate < live, "B6: a live decision reaches apply_live only after decision_gate");
    let body = body_of("fn apply_live(");
    let at = |n: &str| body.find(n).unwrap_or_else(|| panic!("apply_live no longer calls {n}"));
    let (world, stale, act) = (at("live_world("), at("live_expiry("), at("g.tool(\"openttd.ai.set_settings\""));
    assert!(world < stale && stale < act, "B6: apply_live must check the world, then the stale date, before set_settings");
    assert!(!body.contains("set_bot_setting(") && !body.contains("start_bot("), "B6: apply_live acts only through the one approval");
    // The authority's side: a live directive is refused on companies read before the snapshot.
    assert!(read(AUTHORITY_BIN).contains("\"stale_companies\""));
}

// ------------------------------------------------------------- B7: a fallback is labelled as one

/// B7. A fallback solver is never labelled as the kernel: cargo.rs names its built-in knapsack
/// by a label starting with "builtin" (its test `the_builtin_fallback_is_never_labelled_as_the_kernel`),
/// and battlebrain counts exactly that prefix as `fallback_solver`.
#[test]
fn b7_the_fallback_label_and_battlebrains_fallback_metric_agree() {
    let cargo = read(&format!("{CRATE}/cargo.rs"));
    let label = cargo.lines().find(|l| l.starts_with("const BUILTIN_SOLVER")).expect("cargo.rs BUILTIN_SOLVER");
    assert!(label.contains("= \"builtin "), "{label}");
    assert!(read(AUTHORITY_BIN).contains("solver.starts_with(\"builtin\")"));
}

// ------------------------------------------------------ B8: research code stays out of a release

/// B8. Research-only code is not in a default or release build: both research options default
/// OFF, and the MCP server option refuses to build without the instrumentation one.
/// `tools/gate.sh change` also proves the built default binary carries no research string.
#[test]
fn b8_research_options_default_off() {
    let opts = read("cmake/Options.cmake");
    for o in ["OPTION_RESEARCH_INSTRUMENTATION", "OPTION_PRISMATIC_MCP_SERVER"] {
        let decl = opts.lines().find(|l| l.trim_start().starts_with(&format!("option({o} "))).unwrap_or_else(|| panic!("{o} is not declared"));
        assert!(decl.trim_end().ends_with("OFF)"), "{o} must default OFF: {decl}");
    }
    assert!(opts.contains("OPTION_PRISMATIC_MCP_SERVER requires OPTION_RESEARCH_INSTRUMENTATION=ON"));
}
