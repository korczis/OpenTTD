//! RESEARCH-ONLY: the panel's Rhai scripts as a workspace (the Scripts page, static/scripts.html).
//!
//! Every `.rhai` file of the script library (`rhai_host::rhai_dir()`: $MCP_RHAI_PATH, else
//! tools/research/mcp-panel/rhai) is listed with what its own text says: the header comment as its
//! documentation, the libraries it imports, whether another script imports it (a library) and
//! whether it runs as a service now. Nothing here names a script.
//!
//! - read, check (the Rhai parser's error with its line and column, nothing run), save (only what
//!   parses; the file is written in place, so the repository's diff shows the change and committing
//!   stays the owner's);
//! - services: a script started in the background in this process (`spawn_script`), its printed
//!   lines kept (the newest SERVICE_LINES) and readable after a sequence number;
//! - hot reload: a watcher looks at the library's files every RELOAD_EVERY; a running service whose
//!   own file, or any library it imports, changed (by the page's Save or by an editor outside) is
//!   stopped and started again on the new text, and says so in its log. A library may live in a
//!   subdirectory (`import "policy/lib"`), and a template import (`` import `policy/${name}` ``)
//!   depends on every script under the directory its literal prefix names.
//! Stopping a service sets its abort flag, which ends it at its next operation (spawn_script).

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::panel::{spawn_script, Game, Input, ScriptLine};
use crate::rhai_host;

/// The lines a service keeps, newest last.
pub const SERVICE_LINES: usize = 2000;
/// How often the watcher looks for changed files.
pub const RELOAD_EVERY: Duration = Duration::from_millis(1000);

/// Whether `name` may name a script: letters, digits and underscores, so a name never leaves the
/// library's folder.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `name` names a script of the library or of a subdirectory of it (`policy/lib`): every
/// `/`-separated part a valid_name, so no part is `..` and the path stays inside the library.
pub fn valid_path(name: &str) -> bool {
    name.split('/').all(valid_name)
}

fn path_of(dir: &Path, name: &str) -> Result<PathBuf, String> {
    if !valid_path(name) { return Err(format!("{name:?} is not a script name (letters, digits, _, / between parts)")); }
    Ok(dir.join(format!("{name}.rhai")))
}

/// Every script under `dir`, its subdirectories included, by its name relative to `dir`
/// (`policy/lib` for dir/policy/lib.rhai), sorted. The tree decides what is there: no subdirectory
/// is named here. A symbolic link to a directory is not followed, so a loop cannot be walked.
pub fn scripts_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        let part = |s: Option<&std::ffi::OsStr>| s.and_then(|s| s.to_str()).filter(|s| valid_name(s)).map(str::to_string);
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            let Some(sub) = part(p.file_name()) else { continue };
            out.extend(scripts_under(&p).into_iter().map(|n| format!("{sub}/{n}")));
        } else if p.extension().and_then(|e| e.to_str()) == Some("rhai") {
            let Some(stem) = part(p.file_stem()) else { continue };
            out.push(stem);
        }
    }
    out.sort();
    out
}

/// The header comment of a script: its leading `//` lines, the comment marks taken off.
pub fn doc_of(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("//") {
            out.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else if t.is_empty() && out.is_empty() {
            continue;
        } else {
            break;
        }
    }
    out.join("\n").trim().to_string()
}

/// A line without its `//` comment (a `//` inside a string literal is kept).
fn code_of(line: &str) -> &str {
    let b = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// The libraries a script imports (`import "name"`), in the order it names them; an import in a
/// comment is not one.
pub fn imports_of(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = code_of(line);
        while let Some(i) = rest.find("import \"") {
            let after = &rest[i + 8..];
            if let Some(end) = after.find('"') {
                let name = &after[..end];
                if !out.iter().any(|n| n == name) { out.push(name.to_string()); }
                rest = &after[end..];
            } else {
                break;
            }
        }
    }
    out
}

/// The directories a script imports from by a template (`` import `policy/${name}` ``): the
/// literal prefix's directory (`policy`, or "" for the library itself), in the order named. Which
/// file the template names is known only when it runs, so each script under that directory is one
/// it may load.
pub fn template_dirs_of(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = code_of(line);
        while let Some(i) = rest.find("import `") {
            let after = &rest[i + 8..];
            let Some(end) = after.find('`') else { break };
            if let Some(var) = after[..end].find("${") {
                let dir = after[..var].rsplit_once('/').map_or("", |(d, _)| d);
                if (dir.is_empty() || valid_path(dir)) && !out.iter().any(|n| n == dir) { out.push(dir.to_string()); }
            }
            rest = &after[end + 1..];
        }
    }
    out
}

/// The Rhai parser's verdict on a text: null when it parses, else {message, line, column}.
pub fn check(text: &str) -> Value {
    let engine = rhai::Engine::new();
    match engine.compile(text) {
        Ok(_) => Value::Null,
        Err(e) => {
            let pos = e.position();
            json!({"message": e.to_string(), "line": pos.line(), "column": pos.position()})
        }
    }
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn secs(t: Option<SystemTime>) -> Value {
    t.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(Value::Null, |d| json!(d.as_secs_f64()))
}

/// Every script in `dir`: {name, file, doc, imports, imported_by, library, bytes, modified}, by name.
pub fn list(dir: &Path) -> Vec<Value> {
    let mut texts: BTreeMap<String, (String, Option<SystemTime>, u64)> = BTreeMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("rhai") { continue; }
        let Some(name) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
        if !valid_name(&name) { continue; }
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let bytes = text.len() as u64;
        texts.insert(name, (text, modified(&p), bytes));
    }
    let mut imported_by: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, (text, _, _)) in &texts {
        for lib in imports_of(text) { imported_by.entry(lib).or_default().insert(name.clone()); }
    }
    texts.iter().map(|(name, (text, m, bytes))| {
        let by: Vec<String> = imported_by.get(name).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        json!({"name": name, "file": format!("{name}.rhai"), "doc": doc_of(text), "imports": imports_of(text),
               "imported_by": by, "library": !by.is_empty(), "bytes": bytes, "modified": secs(*m)})
    }).collect()
}

// ------------------------------------------------------------------------------------- services

struct Service {
    abort: Arc<AtomicBool>,
    lines: Arc<Mutex<VecDeque<(u64, String)>>>,
    seq: Arc<Mutex<u64>>,
    running: Arc<AtomicBool>,
    started: SystemTime,
    reloads: u64,
    /// The modification times the service was started on: its own file and every library it imports.
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
}

fn services() -> &'static Mutex<BTreeMap<String, Service>> {
    static S: OnceLock<Mutex<BTreeMap<String, Service>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn push(lines: &Mutex<VecDeque<(u64, String)>>, seq: &Mutex<u64>, text: String) {
    let mut n = seq.lock().unwrap();
    *n += 1;
    let mut l = lines.lock().unwrap();
    l.push_back((*n, text));
    while l.len() > SERVICE_LINES { l.pop_front(); }
}

/// The files a service depends on: its own and, transitively, every library it imports, by name
/// (a subdirectory's too) or by a template (every script under the template's directory).
fn stamps(dir: &Path, name: &str) -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut seen = BTreeSet::new();
    let mut todo = vec![name.to_string()];
    let mut out = Vec::new();
    while let Some(n) = todo.pop() {
        if !seen.insert(n.clone()) { continue; }
        let Ok(p) = path_of(dir, &n) else { continue };
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        todo.extend(imports_of(&text));
        for sub in template_dirs_of(&text) {
            let prefix = if sub.is_empty() { String::new() } else { format!("{sub}/") };
            todo.extend(scripts_under(&dir.join(&sub)).into_iter().map(|s| format!("{prefix}{s}")));
        }
        out.push((p.clone(), modified(&p)));
    }
    out
}

/// Start `name` as a service on the panel at `base` (replacing a run of it that ended); its lines
/// carry `reloads` restarts already.
fn start_in(base: &str, dir: &Path, name: &str, reloads: u64, carry: Option<(Arc<Mutex<VecDeque<(u64, String)>>>, Arc<Mutex<u64>>)>,
            snapshot: Option<String>) -> Result<Service, String> {
    let p = path_of(dir, name)?;
    // A reload starts from the text it validated, never from a second read that may differ.
    let text = match snapshot { Some(t) => t, None => std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))? };
    if let Some(err) = check(&text).as_object() {
        return Err(format!("{name} does not parse: {}", err.get("message").and_then(Value::as_str).unwrap_or("?")));
    }
    let (lines, seq) = carry.unwrap_or_else(|| (Arc::new(Mutex::new(VecDeque::new())), Arc::new(Mutex::new(0))));
    let abort = Arc::new(AtomicBool::new(false));
    let running = Arc::new(AtomicBool::new(true));
    let (tx, rx) = mpsc::channel();
    let st = stamps(dir, name);
    push(&lines, &seq, format!("== {} {name} ({} file(s) watched)", if reloads == 0 { "started" } else { "reloaded" }, st.len()));
    spawn_script(base.to_string(), text, abort.clone(), tx);
    let (l2, s2, r2) = (lines.clone(), seq.clone(), running.clone());
    thread::spawn(move || {
        for msg in rx {
            match msg {
                ScriptLine::Out(t) => push(&l2, &s2, t),
                ScriptLine::Err(t) => push(&l2, &s2, format!("error: {t}")),
                ScriptLine::Exit(code) => { push(&l2, &s2, format!("== exit {code}")); break; }
            }
        }
        r2.store(false, Ordering::SeqCst);
    });
    Ok(Service { abort, lines, seq, running, started: SystemTime::now(), reloads, stamps: st })
}

/// Start a service (no-op answer when it already runs).
pub fn start(base: &str, dir: &Path, name: &str) -> Result<Value, String> {
    let mut all = services().lock().unwrap();
    if let Some(s) = all.get(name) {
        if s.running.load(Ordering::SeqCst) {
            // Asked to stop and not yet ended: starting now would run two copies.
            if s.abort.load(Ordering::SeqCst) { return Err(format!("{name} is still stopping; start it again once it has ended")); }
            return Ok(json!({"name": name, "running": true, "already": true}));
        }
    }
    let carry = all.get(name).map(|s| (s.lines.clone(), s.seq.clone()));
    let svc = start_in(base, dir, name, 0, carry, None)?;
    all.insert(name.to_string(), svc);
    Ok(json!({"name": name, "running": true}))
}

/// Stop a service; its lines stay readable.
pub fn stop(name: &str) -> Result<Value, String> {
    let all = services().lock().unwrap();
    let s = all.get(name).ok_or(format!("{name} is not a service here"))?;
    s.abort.store(true, Ordering::SeqCst);
    push(&s.lines, &s.seq, "== stop requested".into());
    Ok(json!({"name": name, "stopping": true}))
}

/// Every service: {name, running, started, reloads, last_seq, watched}.
pub fn status() -> Vec<Value> {
    services().lock().unwrap().iter().map(|(n, s)| json!({
        "name": n, "running": s.running.load(Ordering::SeqCst), "started": secs(Some(s.started)),
        "reloads": s.reloads, "last_seq": *s.seq.lock().unwrap(),
        "watched": s.stamps.iter().map(|(p, _)| p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()).collect::<Vec<_>>(),
    })).collect()
}

/// A service's lines after sequence number `since`: {name, running, lines: [{seq, text}], last_seq}.
pub fn log(name: &str, since: u64) -> Result<Value, String> {
    let all = services().lock().unwrap();
    let s = all.get(name).ok_or(format!("{name} has not run as a service here"))?;
    let lines: Vec<Value> = s.lines.lock().unwrap().iter().filter(|(n, _)| *n > since).map(|(n, t)| json!({"seq": n, "text": t})).collect();
    Ok(json!({"name": name, "running": s.running.load(Ordering::SeqCst), "lines": lines, "last_seq": *s.seq.lock().unwrap()}))
}

/// One look of the watcher: every running service whose watched files changed is restarted on the
/// new text (one that no longer parses keeps running on the old one and says why). Returns the
/// names reloaded.
pub fn reload_changed(base: &str, dir: &Path) -> Vec<String> {
    let mut all = services().lock().unwrap();
    let mut done = Vec::new();
    let names: Vec<String> = all.keys().cloned().collect();
    for name in names {
        let changed = {
            let s = &all[&name];
            // A service asked to stop is never brought back by an edit (it ends at its next operation).
            s.running.load(Ordering::SeqCst) && !s.abort.load(Ordering::SeqCst) && s.stamps.iter().any(|(p, t)| modified(p) != *t)
        };
        if !changed { continue; }
        let (lines, seq, reloads) = { let s = &all[&name]; (s.lines.clone(), s.seq.clone(), s.reloads) };
        // Every file it would load must be read and parse: its own and each library it imports. A
        // file that cannot be read (mid-save, removed) is a failure to wait out, never empty text.
        let mut own = None;
        let mut broken = None;
        for (p, _) in stamps(dir, &name) {
            let short = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            match std::fs::read_to_string(&p) {
                Err(e) => { broken = Some(format!("{short} cannot be read: {e}")); break; }
                Ok(text) => {
                    if let Some(e) = check(&text).as_object() {
                        broken = Some(format!("{short}: {}", e.get("message").and_then(Value::as_str).unwrap_or("?")));
                        break;
                    }
                    if p == path_of(dir, &name).unwrap_or_default() { own = Some(text); }
                }
            }
        }
        if let Some(why) = broken {
            push(&lines, &seq, format!("== not reloaded, the old run goes on: {why}"));
            if let Some(s) = all.get_mut(&name) { s.stamps = stamps(dir, &name); }
            continue;
        }
        all[&name].abort.store(true, Ordering::SeqCst);
        match start_in(base, dir, &name, reloads + 1, Some((lines.clone(), seq.clone())), own) {
            Ok(svc) => { all.insert(name.clone(), svc); done.push(name); }
            Err(e) => push(&lines, &seq, format!("== reload failed: {e}")),
        }
    }
    done
}

/// The watcher thread (started once by the panel): reload_changed every RELOAD_EVERY.
pub fn watch(g: Arc<Game>) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() { return; }
    thread::spawn(move || loop {
        thread::sleep(RELOAD_EVERY);
        for name in reload_changed(&g.panel_url(), &rhai_host::rhai_dir()) {
            g.event("rhai", &format!("service {name} reloaded: its file or a library it imports changed"));
        }
    });
}

// ------------------------------------------------------------------------------------- handlers

fn q<'a>(i: &'a Input, key: &str) -> Option<String> {
    i.body.get(key).and_then(Value::as_str).map(str::to_string).or_else(|| i.query.get(key).map(|s| s.to_string()))
}

pub fn h_scripts(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    watch(g.clone());
    let dir = rhai_host::rhai_dir();
    let running: BTreeMap<String, Value> = status().into_iter().map(|s| (s["name"].as_str().unwrap_or("").to_string(), s)).collect();
    let scripts: Vec<Value> = list(&dir).into_iter().map(|mut s| {
        let n = s["name"].as_str().unwrap_or("").to_string();
        s["service"] = running.get(&n).cloned().unwrap_or(Value::Null);
        s
    }).collect();
    // Shown relative to the repository (tools/research/mcp-panel/rhai), absolute only outside it.
    let repo = crate::panel_results::repo();
    let shown = dir.strip_prefix(repo.canonicalize().unwrap_or(repo.clone())).map(|p| p.display().to_string()).unwrap_or_else(|_| dir.display().to_string());
    Ok((200, json!({"dir": shown, "dir_abs": dir.display().to_string(), "scripts": scripts})))
}

pub fn h_script(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let name = q(i, "name").ok_or("?name= names the script")?;
    let p = path_of(&rhai_host::rhai_dir(), &name)?;
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{name}: {e}"))?;
    Ok((200, json!({"name": name, "text": text, "doc": doc_of(&text), "imports": imports_of(&text),
                    "modified": secs(modified(&p)), "error": check(&text)})))
}

pub fn h_script_check(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let text = q(i, "text").unwrap_or_default();
    Ok((200, json!({"error": check(&text), "imports": imports_of(&text)})))
}

pub fn h_script_save(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let name = q(i, "name").ok_or("name names the script")?;
    let text = q(i, "text").ok_or("text is the script")?;
    let err = check(&text);
    if !err.is_null() { return Ok((422, json!({"saved": false, "error": err}))); }
    let p = path_of(&rhai_host::rhai_dir(), &name)?;
    // Atomically: a watcher or a reader never sees half a file (write beside it, then rename).
    // One name per save: two saves at once (two tabs) must not share a temporary file.
    static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = p.with_extension(format!("rhai.{}.{}.tmp", std::process::id(), SAVES.fetch_add(1, Ordering::SeqCst)));
    std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, &p)).map_err(|e| { let _ = std::fs::remove_file(&tmp); format!("{}: {e}", p.display()) })?;
    g.event("rhai", &format!("script {name} saved ({} bytes)", text.len()));
    Ok((200, json!({"saved": true, "name": name, "modified": secs(modified(&p)), "error": Value::Null})))
}

pub fn h_services(_: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    Ok((200, json!({"services": status(), "reload_every_ms": RELOAD_EVERY.as_millis() as u64})))
}

pub fn h_service_start(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    watch(g.clone());
    let name = q(i, "name").ok_or("name names the script")?;
    let out = start(&g.panel_url(), &rhai_host::rhai_dir(), &name)?;
    g.event("rhai", &format!("service {name} started"));
    Ok((200, out))
}

pub fn h_service_stop(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let name = q(i, "name").ok_or("name names the script")?;
    let out = stop(&name)?;
    g.event("rhai", &format!("service {name} stopped"));
    Ok((200, out))
}

pub fn h_service_log(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let name = q(i, "name").ok_or("?name= names the service")?;
    let since = q(i, "since").and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok((200, log(&name, since)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MCP_RHAI_PATH is process-wide and a service resolves its imports against it: a test that
    /// sets it and runs a service holds this lock, so another one never moves its library.
    static LIBRARY: Mutex<()> = Mutex::new(());

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("panel-scripts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_name_never_leaves_the_library() {
        assert!(valid_name("rail_lib"));
        for bad in ["", "../x", "a/b", "a.rhai", "a b", &"x".repeat(65)] { assert!(!valid_name(bad), "{bad}"); }
        assert!(path_of(Path::new("/l"), "../etc").is_err());
        assert!(valid_path("policy/lib") && valid_path("rail_lib"));
        for bad in ["policy/", "/policy", "policy/../x", "policy//lib", "a.b/c"] { assert!(!valid_path(bad), "{bad}"); }
        assert_eq!(path_of(Path::new("/l"), "policy/lib").unwrap(), Path::new("/l/policy/lib.rhai"));
    }

    #[test]
    fn a_template_import_names_the_directory_of_its_prefix() {
        let t = "fn s(n) { import `policy/${n}` as st; st::f() }
import `${x}` as a; import `deep/er/${y}` as b;
                 import `plain` as c;
// import `commented/${z}` as d;
import `../up/${n}` as e;
import `policy/x_${n}` as f;
import `open";
        assert_eq!(template_dirs_of(t), vec!["policy", "", "deep/er"]);
    }

    #[test]
    fn the_tree_gives_every_script_under_the_library() {
        let d = tmp("tree");
        std::fs::create_dir_all(d.join("sub/deeper")).unwrap();
        std::fs::create_dir_all(d.join("bad.dir")).unwrap();
        for f in ["top.rhai", "sub/x.rhai", "sub/deeper/y.rhai", "bad.dir/z.rhai", "sub/bad-name.rhai", "notes.txt"] {
            std::fs::write(d.join(f), "1").unwrap();
        }
        assert_eq!(scripts_under(&d), vec!["sub/deeper/y", "sub/x", "top"]);
        assert!(scripts_under(&d.join("missing")).is_empty());
    }

    #[test]
    fn a_scripts_doc_and_imports_come_from_its_own_text() {
        let t = "// Rail steward: keeps lines paying.\n//\n// More.\nimport \"rail_lib\" as rl;\nimport \"ttd\" as ttd; // import \"no\"\n// import \"commented\"\nlet x = 1;\n";
        assert_eq!(doc_of(t), "Rail steward: keeps lines paying.\n\nMore.");
        assert_eq!(imports_of(t), vec!["rail_lib", "ttd"]);
    }

    #[test]
    fn check_names_the_line_and_column_of_a_parse_error() {
        assert!(check("let a = 1; a + 2").is_null());
        let e = check("let a = 1;\nlet b = ;\n");
        assert_eq!(e["line"], json!(2), "{e}");
        assert!(e["message"].as_str().unwrap().len() > 3);
    }

    /// Every script of the library parses: a script that does not is caught here, before a panel
    /// refuses to start it as a service.
    #[test]
    /// Its subdirectories included (policy/ and any other), walked from the tree.
    fn every_script_of_the_library_parses() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel/rhai"));
        let scripts = scripts_under(&dir);
        assert!(scripts.len() >= 10, "the library is read from {}", dir.display());
        assert!(scripts.iter().any(|n| n.contains('/')), "a subdirectory of {} is walked", dir.display());
        assert!(list(&dir).iter().all(|s| scripts.iter().any(|n| s["name"] == n.as_str())), "every listed script is parsed");
        for name in scripts {
            let text = std::fs::read_to_string(path_of(&dir, &name).unwrap()).unwrap();
            assert!(check(&text).is_null(), "{name}.rhai: {}", check(&text));
        }
    }

    #[test]
    fn a_library_is_a_script_another_one_imports() {
        let d = tmp("list");
        std::fs::write(d.join("lib_a.rhai"), "// A library.\nfn f() { 1 }\n").unwrap();
        std::fs::write(d.join("main_b.rhai"), "// Uses a.\nimport \"lib_a\" as a;\n").unwrap();
        std::fs::write(d.join("notes.txt"), "not a script").unwrap();
        let l = list(&d);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0]["name"], "lib_a");
        assert_eq!(l[0]["library"], true);
        assert_eq!(l[0]["imported_by"], json!(["main_b"]));
        assert_eq!(l[1]["library"], false);
        assert_eq!(l[1]["doc"], "Uses a.");
        let st = stamps(&d, "main_b");
        assert_eq!(st.len(), 2, "its own file and the library it imports are watched");
    }

    /// A service runs, its lines are readable after a sequence number, an edit of a library it
    /// imports reloads it, an edit that does not parse keeps the old one running, and stop ends it.
    #[test]
    fn a_service_reloads_when_its_library_changes_and_stops_on_request() {
        let d = tmp("svc");
        std::fs::write(d.join("svc_lib.rhai"), "fn word() { \"one\" }\n").unwrap();
        std::fs::write(d.join("svc_main.rhai"), "import \"svc_lib\" as l;\nprint(l::word());\nloop { sleep(20); }\n").unwrap();
        let _library = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("MCP_RHAI_PATH", &d);
        let base = "http://127.0.0.1:9";
        start(base, &d, "svc_main").unwrap();
        let wait = |needle: &str| {
            for _ in 0..200 {
                let l = log("svc_main", 0).unwrap();
                if l["lines"].as_array().unwrap().iter().any(|x| x["text"].as_str().unwrap_or("").contains(needle)) { return true; }
                thread::sleep(Duration::from_millis(25));
            }
            false
        };
        assert!(wait("one"), "{}", log("svc_main", 0).unwrap());
        thread::sleep(Duration::from_millis(1100)); // a new modification time on coarse file systems
        std::fs::write(d.join("svc_lib.rhai"), "fn word() { \"two\" }\n").unwrap();
        assert_eq!(reload_changed(base, &d), vec!["svc_main".to_string()]);
        assert!(wait("two") && wait("reloaded"));
        thread::sleep(Duration::from_millis(1100));
        std::fs::write(d.join("svc_lib.rhai"), "fn word( { \"three\" }\n").unwrap();
        assert!(reload_changed(base, &d).is_empty());
        assert!(wait("not reloaded"));
        assert_eq!(status().iter().find(|s| s["name"] == "svc_main").unwrap()["reloads"], json!(1));
        stop("svc_main").unwrap();
        // An edit while it stops must not bring it back (reload skips a service asked to stop).
        thread::sleep(Duration::from_millis(1100));
        std::fs::write(d.join("svc_lib.rhai"), "fn word() { \"four\" }\n").unwrap();
        assert!(reload_changed(base, &d).is_empty(), "a stopping service is not reloaded");
        for _ in 0..200 { if !log("svc_main", 0).unwrap()["running"].as_bool().unwrap() { break; } thread::sleep(Duration::from_millis(25)); }
        assert_eq!(log("svc_main", 0).unwrap()["running"], false);
        assert!(start(base, &d, "nope_missing").is_err());
    }

    /// A library in a subdirectory (`import "nest/lib"`) and a stage loaded by a template
    /// (`` import `nest/${n}` ``) are watched: an edit of either reloads the service, and the tree
    /// decides which files those are (a stage added later is watched from the next start).
    #[test]
    fn a_service_reloads_when_a_script_of_a_subdirectory_changes() {
        let d = tmp("nest");
        std::fs::create_dir_all(d.join("nest")).unwrap();
        std::fs::write(d.join("nest/lib.rhai"), "fn word() { \"one\" }\n").unwrap();
        std::fs::write(d.join("nest/stage_a.rhai"), "fn word() { \"alpha\" }\n").unwrap();
        std::fs::write(d.join("tree_main.rhai"),
            "import \"nest/lib\" as l;\nfn stage(n) { import `nest/${n}` as s; s::word() }\n\
             print(\"said \" + l::word() + \"-\" + stage(\"stage_a\"));\nloop { sleep(20); }\n").unwrap();
        let watched: Vec<PathBuf> = stamps(&d, "tree_main").into_iter().map(|(p, _)| p).collect();
        for f in ["tree_main.rhai", "nest/lib.rhai", "nest/stage_a.rhai"] { assert!(watched.contains(&d.join(f)), "{f}: {watched:?}"); }
        let _library = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("MCP_RHAI_PATH", &d);
        let base = "http://127.0.0.1:9";
        start(base, &d, "tree_main").unwrap();
        // Up to 200 looks, 25 ms apart; true at the first log holding `needle`.
        let wait = |needle: &str| (0..200).any(|_| {
            let l = log("tree_main", 0).unwrap();
            let hit = l["lines"].as_array().unwrap().iter().any(|x| x["text"].as_str().unwrap_or("").contains(needle));
            if !hit { thread::sleep(Duration::from_millis(25)); }
            hit
        });
        assert!(wait("said one-alpha"), "{}", log("tree_main", 0).unwrap());
        thread::sleep(Duration::from_millis(1100)); // a new modification time on coarse file systems
        std::fs::write(d.join("nest/stage_a.rhai"), "fn word() { \"beta\" }\n").unwrap();
        assert_eq!(reload_changed(base, &d), vec!["tree_main".to_string()], "a template's stage is watched");
        assert!(wait("said one-beta"), "{}", log("tree_main", 0).unwrap());
        thread::sleep(Duration::from_millis(1100));
        std::fs::write(d.join("nest/lib.rhai"), "fn word() { \"two\" }\n").unwrap();
        assert_eq!(reload_changed(base, &d), vec!["tree_main".to_string()], "a subdirectory's library is watched");
        assert!(wait("said two-beta"), "{}", log("tree_main", 0).unwrap());
        assert_eq!(status().iter().find(|s| s["name"] == "tree_main").unwrap()["reloads"], json!(2));
        stop("tree_main").unwrap();
        for _ in 0..200 { if !log("tree_main", 0).unwrap()["running"].as_bool().unwrap() { break; } thread::sleep(Duration::from_millis(25)); }
        assert_eq!(log("tree_main", 0).unwrap()["running"], false);
    }
}
