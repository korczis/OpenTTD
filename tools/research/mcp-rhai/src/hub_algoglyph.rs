//! RESEARCH-ONLY / UNSTABLE: AlgoGlyph's kernels as hub methods.
//!
//! AlgoGlyph is the git submodule `tools/research/vendor/algoglyph`, pinned at one commit (I0227,
//! ADR 0039; `$MCP_ALGOGLYPH_DIR` overrides, see `dir`). It answers one kernel on caller data
//! through its flow CLI: `npm run --silent flow -- kernel <id> -` reads the input on stdin and
//! prints the answer as JSON; `flow -- kernels` lists every kernel with the input shape it reads.
//! Its Rust kernels (crate algoglyph-kernels, linked from the same submodule) are entries of that
//! catalogue with engine native-rust and answer in this process (`native`, src/algoglyph_native.rs).
//! Nothing here names a kernel except `track_plan`, which assembles the track-planning input from a
//! game: the `data` of its openttd://map, map/ways, map/terrain, economy/prices and bridges,
//! verbatim, read in that order (a resource the game does not publish is left out, and the kernel falls back to
//! its defaults). The planner checks slopes against heights, so the four are read back to back.
//!
//! Methods (added to the hub's table, so they are in rpc.discover, GraphQL and OpenAPI):
//!   algoglyph.kernels                          every kernel with the input it reads: the runner's, then the native ones
//!   algoglyph.kernel     {kernel, input}       one kernel on that input (a native one in-process)
//!   algoglyph.track_plan {instance, from, to, costs?, rules?}   a rail route in that game

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The longest a kernel may run before the hub gives up on it.
pub const KERNEL_TIMEOUT: Duration = Duration::from_secs(120);

/// The resources a track plan reads, as the keys of the kernel's input.
pub const TRACK_INPUTS: [(&str, &str); 5] = [("map", "map"), ("ways", "map/ways"), ("terrain", "map/terrain"),
    ("prices", "economy/prices"), ("bridges", "bridges")];

#[path = "algoglyph_native.rs"]
pub mod native;

/// The AlgoGlyph submodule this crate is built next to: tools/research/vendor/algoglyph.
pub fn submodule() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/algoglyph")
}

/// Whether the flow CLI can run in `dir`: a checkout with its package.json and its node_modules.
pub fn runnable(dir: &Path) -> bool {
    dir.join("package.json").is_file() && dir.join("node_modules").is_dir()
}

/// The commit a checkout is cleanly at: its HEAD when no tracked file differs from it, else None.
pub fn clean_head(dir: &Path) -> Option<String> {
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(dir).args(args).output().ok()
        .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    git(&["status", "--porcelain", "--untracked-files=no"]).filter(String::is_empty)?;
    git(&["rev-parse", "HEAD"])
}

/// Where AlgoGlyph is checked out: `$MCP_ALGOGLYPH_DIR` (an experiment against another version),
/// else the pinned checkout `default_dir` finds.
pub fn dir() -> PathBuf {
    std::env::var_os("MCP_ALGOGLYPH_DIR").map(PathBuf::from).unwrap_or_else(|| {
        default_dir(&submodule(), &PathBuf::from(std::env::var_os("HOME").unwrap_or_default()),
            native::pinned_commit().as_deref(), &clean_head)
    })
}

/// The checkout used when `$MCP_ALGOGLYPH_DIR` is not set: the first of the submodule,
/// `~/dev/algoglyph-wt/main` and `~/dev/algoglyph` (under `home`) that can run the flow CLI and is
/// cleanly at the pinned commit `pin` (`at` says which commit a checkout is at). The pin is the only
/// version the fork trusts, so a checkout elsewhere serves only at that commit (one whose
/// node_modules are installed spares installing them in every worktree); none: the submodule, whose
/// path the refusal names.
pub fn default_dir(submodule: &Path, home: &Path, pin: Option<&str>, at: &dyn Fn(&Path) -> Option<String>) -> PathBuf {
    [submodule.to_path_buf(), home.join("dev/algoglyph-wt/main"), home.join("dev/algoglyph")].into_iter()
        .find(|d| runnable(d) && pin.is_some() && at(d).as_deref() == pin)
        .unwrap_or_else(|| submodule.to_path_buf())
}

/// The discovered kernel catalogue: the runner's `flow -- kernels`, then every native kernel.
pub fn kernels() -> Result<Value, String> {
    flow(&["kernels"], None).map(native::merge)
}

/// One kernel on `input`: a native one in this process, any other through the flow CLI.
pub fn kernel(id: &str, input: &Value) -> Result<Value, String> {
    if native::routes(id) { Ok(native::run(id, input)) } else { flow(&["kernel", id, "-"], Some(input)) }
}

/// The method table entries, appended to the hub's own.
pub fn methods() -> Vec<Value> {
    vec![
        json!({"method": "algoglyph.kernels", "params": {},
               "result": "every AlgoGlyph kernel with the input shape it reads: flow -- kernels, then the native-rust kernels of the pinned crate"}),
        json!({"method": "algoglyph.kernel", "params": {"kernel": "required: a kernel id from algoglyph.kernels", "input": "object: that kernel's input"},
               "result": "the kernel's answer (flow -- kernel <id> -; a native-rust kernel in-process)"}),
        json!({"method": "algoglyph.track_plan",
               "params": {"instance": "required: a game instance", "from": "{x, y, dir: NE|SE|SW|NW}", "to": "{x, y, dir?}", "costs": "optional", "rules": "optional",
                          "railtype": "optional: a label (RAIL, ELRL, MONO, MGLV) or index of openttd://railtypes; plain rail when left out"},
               "result": "AlgoGlyph track-planning on the game's map, map/ways, map/terrain, economy/prices and bridges: status, cost, money, counts, pieces, runs"}),
    ]
}

/// Run the flow CLI with `args`, `stdin` on its standard input; its standard output parsed as JSON.
pub fn flow(args: &[&str], stdin: Option<&Value>) -> Result<Value, String> {
    let dir = dir();
    if !runnable(&dir) {
        return Err(format!("no installed AlgoGlyph at {} (tools/research/algoglyph-pin install, or set MCP_ALGOGLYPH_DIR)", dir.display()));
    }
    let mut child = Command::new("npm").args(["run", "--silent", "flow", "--"]).args(args)
        .current_dir(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("npm: {e}"))?;
    {
        let mut input = child.stdin.take().ok_or("no stdin")?;
        if let Some(v) = stdin { input.write_all(v.to_string().as_bytes()).map_err(|e| format!("stdin: {e}"))?; }
    }
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let mut errs = child.stderr.take().ok_or("no stderr")?;
    let reader = std::thread::spawn(move || { let mut s = String::new(); let _ = out.read_to_string(&mut s); s });
    let err_reader = std::thread::spawn(move || { let mut s = String::new(); let _ = errs.read_to_string(&mut s); s });
    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(s) => break s,
            None if started.elapsed() > KERNEL_TIMEOUT => {
                let _ = child.kill();
                return Err(format!("flow {} ran over {} s", args.join(" "), KERNEL_TIMEOUT.as_secs()));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let text = reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    parse_answer(&text).ok_or_else(|| {
        let why = stderr.lines().last().unwrap_or("").to_string();
        format!("flow {} exited {status}: {}", args.join(" "), if why.is_empty() { text.trim().chars().take(300).collect() } else { why })
    })
}

/// The JSON a flow run printed: the whole output, else its last line that parses.
pub fn parse_answer(text: &str) -> Option<Value> {
    serde_json::from_str(text.trim()).ok()
        .or_else(|| text.lines().rev().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(|v| v.is_object() || v.is_array())))
}

/// What a read of one resource answered, as `track_input` takes it: a panel's 404 (the build has
/// no such resource) is not published, None; any other HTTP error is an error; anything else is
/// the resource.
pub fn published(v: Value) -> Result<Option<Value>, String> {
    match v.get("http_error").and_then(Value::as_u64) {
        Some(404) => Ok(None),
        Some(code) => Err(format!("HTTP {code}: {}", v.get("body").map(Value::to_string).unwrap_or_default())),
        None => Ok(Some(v)),
    }
}

/// The track-planning input: `{map, ways, terrain, prices}` from `read` (resource -> its `data`,
/// None when the game does not publish it), plus the caller's from, to, costs and rules.
pub fn track_input(params: &Value, read: &dyn Fn(&str) -> Result<Option<Value>, String>) -> Result<Value, String> {
    let from = params.get("from").filter(|v| v.is_object()).ok_or("track_plan needs from {x, y, dir}")?;
    let to = params.get("to").filter(|v| v.is_object()).ok_or("track_plan needs to {x, y, dir?}")?;
    let mut input = json!({"from": from, "to": to});
    for (key, resource) in TRACK_INPUTS {
        match read(resource)? {
            Some(data) => { input[key] = data; }
            None if key == "map" => return Err("the instance publishes no openttd://map".into()),
            None => {}
        }
    }
    for key in ["costs", "rules"] {
        if let Some(v) = params.get(key).filter(|v| !v.is_null()) { input[key] = v.clone(); }
    }
    if let Some(want) = params.get("railtype").filter(|v| !v.is_null()) {
        let types = read("railtypes")?.ok_or("the instance publishes no openttd://railtypes")?;
        input["railtype"] = pick_railtype(&types, want)?;
    }
    Ok(input)
}

/// The entry of openttd://railtypes the caller asked for, by label ("RAIL", "ELRL") or index.
pub fn pick_railtype(types: &Value, want: &Value) -> Result<Value, String> {
    let list = types.get("types").and_then(Value::as_array).ok_or("openttd://railtypes has no types")?;
    list.iter().find(|t| match want {
        Value::String(label) => t.get("label").and_then(Value::as_str) == Some(label.as_str()),
        Value::Number(n) => t.get("railtype").and_then(Value::as_u64) == n.as_u64(),
        _ => false,
    }).cloned().ok_or_else(|| {
        let known: Vec<&str> = list.iter().filter_map(|t| t.get("label").and_then(Value::as_str)).collect();
        format!("no rail type {want}; the game has {}", known.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_is_the_output_or_its_last_json_line() {
        assert_eq!(parse_answer("{\"ok\":true}\n"), Some(json!({"ok": true})));
        assert_eq!(parse_answer("> algoglyph@1 flow\nnoise\n{\"status\":\"found\"}\n"), Some(json!({"status": "found"})));
        assert_eq!(parse_answer("nothing here"), None);
    }

    #[test]
    fn a_track_input_takes_the_games_resources_verbatim_and_skips_what_is_not_published() {
        let read = |r: &str| -> Result<Option<Value>, String> {
            Ok(match r { "map" => Some(json!({"width": 64})), "map/ways" => Some(json!({"roads": []})), _ => None })
        };
        let p = json!({"from": {"x": 1, "y": 2, "dir": "NE"}, "to": {"x": 9, "y": 2}, "rules": {"forbid90": true}, "costs": null});
        let input = track_input(&p, &read).unwrap();
        assert_eq!(input["map"], json!({"width": 64}));
        assert_eq!(input["ways"], json!({"roads": []}));
        assert!(input.get("terrain").is_none() && input.get("prices").is_none() && input.get("bridges").is_none() && input.get("costs").is_none());
        assert_eq!(input["rules"], json!({"forbid90": true}));
        assert!(track_input(&json!({"from": {"x": 1}}), &read).is_err());
        let nomap = |_: &str| -> Result<Option<Value>, String> { Ok(None) };
        assert!(track_input(&p, &nomap).unwrap_err().contains("openttd://map"));
    }

    #[test]
    fn a_railtype_is_picked_by_label_or_index_and_an_unknown_one_names_the_choices() {
        let types = json!({"types": [{"railtype": 0, "label": "RAIL", "cost_multiplier": 8}, {"railtype": 1, "label": "ELRL", "cost_multiplier": 12}]});
        assert_eq!(pick_railtype(&types, &json!("ELRL")).unwrap()["cost_multiplier"], 12);
        assert_eq!(pick_railtype(&types, &json!(0)).unwrap()["label"], "RAIL");
        assert!(pick_railtype(&types, &json!("MGLV")).unwrap_err().contains("RAIL, ELRL"));
        let read = |r: &str| -> Result<Option<Value>, String> {
            Ok(match r { "map" => Some(json!({})), "railtypes" => Some(types.clone()), _ => None })
        };
        let p = json!({"from": {"x": 1, "y": 1, "dir": "NE"}, "to": {"x": 5, "y": 1}, "railtype": "ELRL"});
        assert_eq!(track_input(&p, &read).unwrap()["railtype"]["label"], "ELRL");
        let plain = json!({"from": {"x": 1, "y": 1, "dir": "NE"}, "to": {"x": 5, "y": 1}});
        assert!(track_input(&plain, &read).unwrap().get("railtype").is_none());
    }

    #[test]
    fn a_panels_404_is_a_resource_not_published_and_any_other_error_fails() {
        assert_eq!(published(json!({"http_error": 404, "body": {"error": "no such resource"}})).unwrap(), None);
        assert!(published(json!({"http_error": 503, "body": "down"})).unwrap_err().contains("503"));
        assert_eq!(published(json!({"data": {"width": 8}})).unwrap(), Some(json!({"data": {"width": 8}})));
    }

    /// A checkout that can run the flow CLI: package.json and node_modules.
    fn installed(d: &Path) {
        std::fs::create_dir_all(d.join("node_modules")).unwrap();
        std::fs::write(d.join("package.json"), "{}").unwrap();
    }

    #[test]
    fn without_the_variable_the_submodule_serves_else_a_checkout_only_at_the_pin() {
        let home = std::env::temp_dir().join(format!("hub-algoglyph-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let sub = home.join("repo/tools/research/vendor/algoglyph");
        let (main, primary) = (home.join("dev/algoglyph-wt/main"), home.join("dev/algoglyph"));
        let pin = "1b8e009059c299b29d7a829d2d51d4e057304d4d";
        let at_pin = |_: &Path| Some(pin.to_string());
        // Nothing installed: the submodule, whose path the refusal names.
        assert_eq!(default_dir(&sub, &home, Some(pin), &at_pin), sub);
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("package.json"), "{}").unwrap();
        installed(&primary);
        assert_eq!(default_dir(&sub, &home, Some(pin), &at_pin), primary, "a submodule without node_modules does not run");
        installed(&main);
        assert_eq!(default_dir(&sub, &home, Some(pin), &at_pin), main);
        // A checkout at another commit, or a dirty one (no clean head), is passed over.
        let elsewhere = |d: &Path| if d == main { Some("0".repeat(40)) } else if d == primary { None } else { Some(pin.to_string()) };
        assert_eq!(default_dir(&sub, &home, Some(pin), &elsewhere), sub);
        installed(&sub);
        assert_eq!(default_dir(&sub, &home, Some(pin), &at_pin), sub, "the submodule first");
        assert_eq!(default_dir(&sub, &home, None, &at_pin), sub, "no pin record: nothing is trusted, the submodule is named");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_clean_head_is_the_commit_of_a_checkout_with_no_tracked_change() {
        let d = std::env::temp_dir().join(format!("hub-algoglyph-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let git = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(&d).args(args).output().unwrap().status.success(), "git {args:?}");
        assert_eq!(clean_head(&d), None, "not a checkout");
        git(&["init", "-q"]);
        std::fs::write(d.join("a"), "1").unwrap();
        git(&["add", "a"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "a"]);
        let head = clean_head(&d).expect("a clean checkout");
        assert_eq!(head.len(), 40);
        std::fs::write(d.join("untracked"), "x").unwrap();
        assert_eq!(clean_head(&d).as_deref(), Some(head.as_str()), "an untracked file changes no tracked one");
        std::fs::write(d.join("a"), "2").unwrap();
        assert_eq!(clean_head(&d), None, "a tracked change");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_submodule_is_the_vendored_checkout_next_to_this_crate() {
        assert!(submodule().ends_with("../vendor/algoglyph"));
        assert!(submodule().join("crates/algoglyph-kernels/Cargo.toml").is_file(), "the crate this binary links is there");
    }

    #[test]
    fn a_native_kernel_is_answered_without_the_runner_and_any_other_needs_it() {
        let id = native::entries(&json!([])).first().and_then(|k| k["id"].as_str()).map(str::to_string).expect("a native kernel");
        let v = kernel(&id, &json!({"keys": [2, 1]})).expect("in-process");
        assert_eq!(v["ok"], true);
        assert_eq!(v["kernel"]["engine"], native::ENGINE);
        // A runner kernel goes to the flow CLI: with no installed checkout that is a refusal naming it.
        if !runnable(&dir()) {
            assert!(kernel("knapsack-dp", &json!({})).unwrap_err().contains("algoglyph-pin install"));
            assert!(kernels().unwrap_err().contains("algoglyph-pin install"));
        }
    }

    #[test]
    fn every_method_says_what_it_reads() {
        for m in methods() {
            assert!(m["method"].as_str().unwrap().starts_with("algoglyph."));
            assert!(m["params"].is_object() && m["result"].is_string());
        }
    }
}
