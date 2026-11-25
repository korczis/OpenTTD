//! RESEARCH-ONLY control panel for OpenTTD's embedded MCP server (ADR 0008, issue I0035): the Rust
//! port of tools/research/mcp-panel/server.py, served by `mcp-panel` (src/bin/mcp-panel.rs).
//!
//! Starts a headless dedicated server from an MCP-enabled build, owns its console (stdin), starts the
//! MCP server and, optionally, a company AI, and serves a local control window at
//! http://127.0.0.1:<panel-port>/ that drives the game through MCP. The bearer token lives in this
//! process only: it is redacted from the console log the window shows and never written to disk.
//! Everything binds to loopback.
//!
//! One declaration, every surface: the router dispatches from `routes()`, `/openapi.json` documents
//! the same table, and the routes derived from the live MCP catalogue (`derive_routes`) are added at
//! request time from the same tools/list and resources/list. Every POST body is checked against its
//! schema (`body_errors`) before a handler runs. One event bus (`Bus`) feeds REST (/api/events), SSE
//! (/api/events/stream) and Rhai; Rhai scripts (/api/rhai, /api/rhai/stream) run in this process,
//! in the engine mcp-rhai runs them in (src/main.rs `host`).
//!
//! JSON answers are written as Python's json.dumps writes them (", " and ": ", non-ASCII escaped):
//! launchers match `"mcp_up": true` in /api/status text.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::calibrate;
use crate::hub;
use crate::kernel_feed;
use crate::panel_bot;
use crate::panel_decision;
use crate::panel_events;
use crate::panel_results;
use crate::panel_bench;
use crate::panel_realworld;
use crate::panel_store;
use crate::panel_town;
use crate::rhai_host;

/// The panel's own directory (static pages, Rhai library, npm packages): `MCP_PANEL_DIR` when it is
/// set and not empty, else the checkout this binary was built from. Setting it serves one
/// checkout's pages while the installed PrismaticAI stays on another, so a UI fix deploys without
/// moving an AI a running experiment pinned (I0094).
pub fn here() -> PathBuf {
    dir_from(std::env::var_os("MCP_PANEL_DIR"))
}

/// `here()` for a given value of `MCP_PANEL_DIR`.
pub fn dir_from(var: Option<std::ffi::OsString>) -> PathBuf {
    match var.filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel")),
    }
}

fn statics() -> PathBuf {
    here().join("static")
}

/// A sibling binary of this crate (mcp-plan, mcp-intent, mcp-graph): next to this executable first
/// (any CARGO_TARGET_DIR), else where server.py looked for it.
pub fn sibling(name: &str) -> PathBuf {
    if let Some(p) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(name))) {
        if p.is_file() {
            return p;
        }
    }
    here().join("../mcp-rhai/target/release").join(name)
}

const BUILD_IT: &str = "build it: cargo build --release in tools/research/mcp-rhai";

// ---------------------------------------------------------------------- arguments

#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub bin: String,
    pub panel_port: u16,
    pub mcp_port: u16,
    pub game_port: u16,
    pub seed: i64,
    pub ai: String,
    pub auto_approve: bool,
    pub plan_bot: bool,
    /// Who may use the panel's web surface (src/web_access.rs).
    pub web: crate::web_access::Policy,
    /// A savegame to start from instead of a new world (`-g FILE`); its AI companies come with it.
    pub load: Option<String>,
    /// Where the owner secret lives (`--owner-secret-file`); unset, $PRISMATIC_PANEL_OWNER_SECRET_FILE,
    /// else ~/.config/prismatic/panel-owner-secret (see [`owner_secret_path`]).
    pub owner_file: Option<String>,
    /// A one-shot owner secret kept nowhere (`--ephemeral-owner-secret`): nobody can sign in over
    /// HTTP, so the owner approves at the game's console (a battle run, a test).
    pub ephemeral_owner: bool,
    /// The run token of this stack's own processes (`--run-token-file`, a 0600 file the launcher
    /// writes): its helpers send it in [`RUN_TOKEN_HEADER`]. Without it the panel makes one for its
    /// in-process scripts alone.
    pub run_token_file: Option<String>,
}

pub const USAGE: &str = "usage: mcp-panel --bin <openttd> [--panel-port 8800] [--mcp-port 8731] [--game-port 3979]\n\
                         \x20                [--seed 1] [--ai PrismaticAI] [--auto-approve] [--no-plan-bot] [--load FILE]\n\
                         \x20                [--bind 127.0.0.1] [--allow-origin <origin> ...] [--token <t>]   (src/web_access.rs)\n\
                         \x20                [--owner-secret-file FILE | --ephemeral-owner-secret]   (the owner's sign-in)\n\
                         \x20                [--run-token-file FILE]   (the token this stack's own helpers send, never an owner's)\n\
                         \x20      mcp-panel --write-event-kinds   (the knowledge note's table of event kinds, from src/panel_events.rs)\n\
                         \x20      mcp-panel --import-calibration FILE [NAME]   (a retired calibration file's readings, once, into the store)";

/// server.py's command line: `--name value` or `--name=value`.
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args { bin: String::new(), panel_port: 8800, mcp_port: 8731, game_port: 3979, seed: 1,
                       ai: "PrismaticAI".into(), auto_approve: false, plan_bot: true, load: None, web: Default::default(),
                       owner_file: None, ephemeral_owner: false, run_token_file: None };
    let mut bin = None;
    let mut i = 0;
    while i < argv.len() {
        let (name, inline) = match argv[i].split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (argv[i].clone(), None),
        };
        let value = |i: &mut usize| -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            *i += 1;
            argv.get(*i).cloned().ok_or_else(|| format!("argument {name}: expected one argument"))
        };
        let port = |v: String, n: &str| v.parse::<u16>().map_err(|_| format!("argument {n}: invalid int value: '{v}'"));
        match name.as_str() {
            "--bin" => bin = Some(value(&mut i)?),
            "--panel-port" => a.panel_port = port(value(&mut i)?, "--panel-port")?,
            "--mcp-port" => a.mcp_port = port(value(&mut i)?, "--mcp-port")?,
            "--game-port" => a.game_port = port(value(&mut i)?, "--game-port")?,
            "--seed" => {
                let v = value(&mut i)?;
                a.seed = v.parse().map_err(|_| format!("argument --seed: invalid int value: '{v}'"))?;
            }
            "--ai" => a.ai = value(&mut i)?,
            "--auto-approve" => a.auto_approve = true,
            "--no-plan-bot" => a.plan_bot = false,
            // The one web access policy's own flags (src/web_access.rs).
            web @ ("--bind" | "--allow-origin" | "--token") => { a.web.flag(web, &value(&mut i)?)?; }
            "--load" => a.load = Some(value(&mut i)?),
            "--owner-secret-file" => a.owner_file = Some(value(&mut i)?),
            "--ephemeral-owner-secret" => a.ephemeral_owner = true,
            "--run-token-file" => a.run_token_file = Some(value(&mut i)?),
            other => return Err(format!("unrecognized arguments: {other}")),
        }
        i += 1;
    }
    a.bin = bin.ok_or("the following arguments are required: --bin")?;
    Ok(a)
}

// ---------------------------------------------------------------------- JSON as Python writes it

struct PyFormatter;

impl serde_json::ser::Formatter for PyFormatter {
    fn begin_array_value<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> std::io::Result<()> {
        if first { Ok(()) } else { w.write_all(b", ") }
    }
    fn begin_object_key<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> std::io::Result<()> {
        if first { Ok(()) } else { w.write_all(b", ") }
    }
    fn begin_object_value<W: ?Sized + Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        w.write_all(b": ")
    }
    fn write_string_fragment<W: ?Sized + Write>(&mut self, w: &mut W, fragment: &str) -> std::io::Result<()> {
        for c in fragment.chars() {
            if c.is_ascii() {
                w.write_all(&[c as u8])?;
            } else {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    write!(w, "\\u{u:04x}")?;
                }
            }
        }
        Ok(())
    }
}

/// `json.dumps(v)`: ", " and ": " separators, non-ASCII as \uXXXX.
pub fn py_json(v: &Value) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, PyFormatter);
    serde::Serialize::serialize(v, &mut ser).expect("a Value always serialises");
    String::from_utf8(out).expect("the formatter writes ASCII")
}

/// A value as a Python f-string shows it: a string as it is, null as None, booleans as True/False.
pub fn py_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => if *b { "True".into() } else { "False".into() },
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => py_repr(other),
    }
}

/// `repr()` of a JSON value: strings quoted the way Python quotes them.
pub fn py_repr(v: &Value) -> String {
    match v {
        Value::String(s) => {
            let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
            let mut out = String::from(quote);
            for c in s.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    c if c == quote => { out.push('\\'); out.push(c); }
                    c => out.push(c),
                }
            }
            out.push(quote);
            out
        }
        Value::Array(a) => format!("[{}]", a.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(m) => format!("{{{}}}", m.iter().map(|(k, v)| format!("{}: {}", py_repr(&json!(k)), py_repr(v)))
            .collect::<Vec<_>>().join(", ")),
        other => py_str(Some(other)),
    }
}

/// Python truthiness of a JSON value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map_or(true, |f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
    }
}

/// `body or {}`.
fn or_empty(v: &Value) -> Value {
    if truthy(Some(v)) { v.clone() } else { json!({}) }
}

/// `int(v)` for a JSON value: numbers truncated, booleans as 0/1, numeric strings parsed.
pub fn py_int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)).ok_or_else(|| format!("int() of {n}")),
        Value::String(s) => s.trim().parse::<i64>().map_err(|_| format!("invalid literal for int() with base 10: {}", py_repr(v))),
        other => Err(format!("int() argument must be a string or a number, not {}", py_repr(other))),
    }
}

/// Seconds as Python's `float()` reads a JSON number (default when absent).
fn py_float(v: Option<&Value>, default: f64) -> Result<f64, String> {
    match v {
        None => Ok(default),
        Some(Value::Number(n)) => Ok(n.as_f64().unwrap_or(default)),
        Some(Value::Bool(b)) => Ok(if *b { 1.0 } else { 0.0 }),
        Some(Value::String(s)) => s.trim().parse().map_err(|_| format!("could not convert string to float: {}", py_repr(&json!(s)))),
        Some(other) => Err(format!("float() argument must be a string or a real number, not {}", py_repr(other))),
    }
}

/// `text[:n]`, by characters.
fn head(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

// ---------------------------------------------------------------------- time

fn strftime_local(fmt: &str) -> String {
    let c = std::ffi::CString::new(fmt).unwrap_or_default();
    let mut buf = [0u8; 64];
    // SAFETY: localtime_r and strftime write into the local, correctly sized buffers.
    let n = unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        libc::strftime(buf.as_mut_ptr().cast(), buf.len(), c.as_ptr(), &tm)
    };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

// ---------------------------------------------------------------------- the event bus

/// The event bus every surface reads: REST (/api/events), SSE (/api/events/stream), Rhai. Each event
/// is `{seq, time, kind, data}`; the newest 5000 are kept.
pub struct Bus {
    inner: Mutex<(u64, VecDeque<Value>)>,
    cv: Condvar,
}

pub const BUS_KEEP: usize = 5000;

/// Does an event kind pass a filter? A filter entry matches its kind and every kind below it
/// (`game` takes `game.month`); no filter takes everything.
pub fn kind_matches(kind: &str, kinds: &[String]) -> bool {
    kinds.is_empty() || kinds.iter().any(|k| kind == k || (kind.starts_with(k.as_str()) && kind[k.len()..].starts_with('.')))
}

/// `?kinds=a,b` as a list, empty entries dropped.
pub fn kinds_of(text: &str) -> Vec<String> {
    text.split(',').filter(|k| !k.is_empty()).map(str::to_string).collect()
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Bus {
        Bus { inner: Mutex::new((0, VecDeque::new())), cv: Condvar::new() }
    }

    pub fn publish(&self, kind: &str, data: Value) -> u64 {
        let mut g = self.inner.lock().unwrap();
        g.0 += 1;
        let seq = g.0;
        g.1.push_back(json!({"seq": seq, "time": strftime_local("%Y-%m-%dT%H:%M:%S%z"), "kind": kind, "data": data}));
        while g.1.len() > BUS_KEEP {
            g.1.pop_front();
        }
        self.cv.notify_all();
        seq
    }

    pub fn seq(&self) -> u64 {
        self.inner.lock().unwrap().0
    }

    /// The first `limit` events after `since` that pass `kinds`.
    pub fn since(&self, since: u64, kinds: &[String], limit: usize) -> Vec<Value> {
        let g = self.inner.lock().unwrap();
        g.1.iter().filter(|e| e["seq"].as_u64().unwrap_or(0) > since && kind_matches(e["kind"].as_str().unwrap_or(""), kinds))
            .take(limit).cloned().collect()
    }

    /// Wait until an event after `since` exists or `timeout` passes; the newest sequence number.
    pub fn wait_past(&self, since: u64, timeout: Duration) -> u64 {
        let g = self.inner.lock().unwrap();
        let (g, _) = self.cv.wait_timeout_while(g, timeout, |g| g.0 <= since).unwrap();
        g.0
    }

    /// One step of a stream: the events after `since` that pass `kinds`, among the next 500 of all
    /// kinds, and the sequence number the stream continues from.
    pub fn stream_step(&self, since: u64, kinds: &[String]) -> (Vec<Value>, u64) {
        let all = self.since(since, &[], 500);
        let next = all.last().and_then(|e| e["seq"].as_u64()).unwrap_or(since).max(since);
        let batch = all.into_iter().filter(|e| kind_matches(e["kind"].as_str().unwrap_or(""), kinds)).collect();
        (batch, next)
    }
}

// ---------------------------------------------------------------------- console lines

/// The console's token line, whatever its wording: "Bearer token (<anything>): <32+ hex digits>".
/// Returns (start, end, token) of the matched text.
pub fn find_token(line: &str) -> Option<(usize, usize, String)> {
    let mut from = 0;
    while let Some(p) = line[from..].find("Bearer token (") {
        let start = from + p;
        let after = start + "Bearer token (".len();
        if let Some(close) = line[after..].find(')') {
            let rest = &line[after + close..];
            if let Some(hex) = rest.strip_prefix("): ") {
                let n = hex.chars().take_while(char::is_ascii_hexdigit).count();
                if n >= 32 {
                    let end = after + close + 3 + n;
                    return Some((start, end, hex[..n].to_string()));
                }
            }
        }
        from = after;
    }
    None
}

/// The token file a console line names: "Token written to <path> (mode 0600)", which
/// `prismatic_mcp start` and `rotate` print in place of the token itself.
pub fn find_token_file(line: &str) -> Option<&str> {
    let after = line.find("Token written to ")? + "Token written to ".len();
    let path = line[after..].trim_end().strip_suffix(" (mode 0600)")?;
    (!path.is_empty()).then_some(path)
}

/// A line as the window shows it: the token line reworded, and the token itself nowhere. The
/// token is the one in the file the line names, or (an older game) the one the line carries.
pub fn redact(line: &str, token: Option<&str>) -> (String, Option<String>) {
    let mut out = line.to_string();
    let mut found = find_token_file(line).and_then(|path| token_file(path).ok()).map(|(_, t)| t);
    if let Some((s, e, t)) = find_token(line) {
        out = format!("{}Bearer token (held by the panel): <REDACTED>{}", &line[..s], &line[e..]);
        found = Some(t);
    }
    if let Some(t) = found.as_deref().or(token) {
        out = out.replace(t, "<REDACTED>");
    }
    (out, found)
}

/// The ~ console's plan verb: "[prismatic-plan] <n> <text>".
pub fn plan_request(line: &str) -> Option<(i64, String)> {
    let mut from = 0;
    while let Some(p) = line[from..].find("[prismatic-plan] ") {
        let at = from + p + "[prismatic-plan] ".len();
        let rest = &line[at..];
        let n = rest.chars().take_while(char::is_ascii_digit).count();
        if n > 0 {
            if let Some(text) = rest[n..].strip_prefix(' ') {
                if !text.is_empty() {
                    if let Ok(id) = rest[..n].parse() {
                        return Some((id, text.to_string()));
                    }
                }
            }
        }
        from = at;
    }
    None
}

/// An AI log line: "[script:<n>] [<company>] [<S|E|P|W|I>] <text>".
pub fn ai_log(line: &str) -> Option<(i64, String, String)> {
    let mut from = 0;
    while let Some(p) = line[from..].find("[script:") {
        let at = from + p + "[script:".len();
        from = at;
        let rest = &line[at..];
        let n = rest.chars().take_while(char::is_ascii_digit).count();
        let Some(rest) = rest[n..].strip_prefix("] [").filter(|_| n > 0) else { continue };
        let c = rest.chars().take_while(char::is_ascii_digit).count();
        let Some(after) = rest[c..].strip_prefix("] [").filter(|_| c > 0) else { continue };
        let mut chars = after.chars();
        let Some(level) = chars.next().filter(|l| "SEPWI".contains(*l)) else { continue };
        let Some(text) = chars.as_str().strip_prefix("] ") else { continue };
        let level = match level {
            'S' => "squirrel_error", 'E' => "error", 'P' => "squirrel_info", 'W' => "warning", _ => "info",
        };
        return Some((rest[..c].parse().ok()?, level.to_string(), text.to_string()));
    }
    None
}

/// The value in the console's answer to `setting difficulty.max_loan` ("... max_loan' is '300000'").
pub fn max_loan_of(line: &str) -> Option<i64> {
    let mut from = 0;
    while let Some(p) = line[from..].find("max_loan") {
        let mut rest = &line[from + p + "max_loan".len()..];
        from += p + 1;
        rest = rest.strip_prefix('\'').unwrap_or(rest);
        let Some(r) = rest.strip_prefix(" is") else { continue };
        let r = r.strip_prefix(':').unwrap_or(r);
        let Some(r) = r.strip_prefix(' ') else { continue };
        let r = r.strip_prefix('\'').unwrap_or(r);
        let n = r.chars().take_while(char::is_ascii_digit).count();
        if n > 0 {
            return r[..n].parse().ok();
        }
    }
    None
}

/// The console's `companies` line: "#:1(Blue) Company Name: '...'  Year Founded: 1950  Money: 5694
/// Loan: 100000 ..." as (company number, money, loan).
pub fn company_row(line: &str) -> Option<(i64, i64, i64)> {
    let rest = &line[line.find("#:")? + 2..];
    let n = rest.chars().take_while(char::is_ascii_digit).count();
    let id: i64 = rest[..n].parse().ok()?;
    let rest = rest[n..].strip_prefix('(')?;
    let rest = &rest[rest.find(')')? + 1..];
    let rest = rest.trim_start().strip_prefix("Company Name: '")?;
    let rest = &rest[rest.find('\'')? + 1..];
    let rest = rest.trim_start().strip_prefix("Year Founded: ")?;
    let y = rest.chars().take_while(char::is_ascii_digit).count();
    let rest = rest[y..].trim_start().strip_prefix("Money: ")?;
    let m = rest.chars().enumerate().take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && *c == '-')).count();
    let money: i64 = rest[..m].parse().ok()?;
    let rest = rest[m..].trim_start().strip_prefix("Loan: ")?;
    let l = rest.chars().take_while(char::is_ascii_digit).count();
    Some((id, money, rest[..l].parse().ok()?))
}

/// A console line split into tokens as the game splits it (`IConsoleCmdExec`, src/console.cpp):
/// spaces separate, `"` toggles a quoted run and is dropped, `\"` is a literal quote. So a check
/// reads `"prismatic_mcp" appr""ove 1` as the game does: `prismatic_mcp`, `approve`, `1`.
pub fn console_tokens(cmd: &str) -> Vec<String> {
    let (mut tokens, mut buf, mut found, mut quoted) = (Vec::new(), String::new(), false, false);
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' if !quoted => {
                if found { tokens.push(std::mem::take(&mut buf)); found = false; }
            }
            '"' => { quoted = !quoted; found = true; }
            '\\' if chars.peek() == Some(&'"') => { chars.next(); buf.push('"'); found = true; }
            _ => { buf.push(c); found = true; }
        }
    }
    if found { tokens.push(buf); }
    tokens
}

/// A console word as the game resolves it: commands ignore underscores (`RemoveUnderscores`) and
/// a chat-style leading `/` names the same command; case and dashes are dropped as well, so that
/// every spelling of a word counts.
fn console_word(token: &str) -> String {
    token.to_ascii_lowercase().chars().filter(|c| *c != '_' && *c != '-').collect()
}

/// Why the panel will not send `cmd` to the game's console for a caller, or None. Approval is the
/// owner's act, so the panel refuses any line that approves, in any spelling the game accepts
/// (a token `approve` or `auto_approve` anywhere), and the commands that run lines it cannot read
/// first: `alias` (an alias composes a command from its arguments), `exec` and `schedule` (they
/// run a script file). The standing rules (`rule`, `rules`), the core's `--via` marker and a bulk
/// `deny all` are the owner routes' alone (POST /api/rules, /api/deny/all). Withdrawing one request
/// (`prismatic_mcp deny <id>`) stays open.
pub fn console_refusal(cmd: &str) -> Option<String> {
    if cmd.chars().any(char::is_control) {
        return Some("one console line without control characters".into());
    }
    let tokens = console_tokens(cmd);
    let first = tokens.first().map(|t| console_word(t.strip_prefix('/').unwrap_or(t))).unwrap_or_default();
    if ["alias", "exec", "schedule"].contains(&first.as_str()) {
        return Some(format!("`{first}` runs console lines the panel cannot check first; type it at the game's own console"));
    }
    if tokens.iter().any(|t| matches!(console_word(t).as_str(), "approve" | "autoapprove")) {
        return Some("approval is the owner's act: the panel's Approve with the owner secret (POST /api/approve), \
                     or `prismatic_mcp approve <id>` at the game's own console, never a console line sent through the panel".into());
    }
    // The standing rules (allow, deny, remove, the list), the core's `--via` marker and a bulk
    // denial are the owner routes' alone (POST /api/rules, /api/rules/remove, /api/deny/all).
    let words: Vec<String> = tokens.iter().map(|t| console_word(t)).collect();
    if words.iter().any(|w| matches!(w.as_str(), "approver" | "approvers")) {
        return Some("who may approve is the owner's to say: the panel's POST /api/approvers with the owner secret, \
                     or `prismatic_mcp approvers ...` at the game's own console, never a console line sent through the panel".into());
    }
    if words.iter().any(|w| matches!(w.as_str(), "rule" | "rules" | "via" | "nonce")) {
        return Some("standing approval rules are the owner's: the panel's Rules with the owner secret (POST /api/rules, \
                     /api/rules/remove), or `prismatic_mcp rule ...` at the game's own console, never a console line sent through the panel".into());
    }
    if words.windows(2).any(|w| w[0] == "deny" && w[1] == "all") {
        return Some("a bulk denial is the owner's: the panel's Deny all with the owner secret (POST /api/deny/all or /api/deny/many), \
                     or `prismatic_mcp deny all` at the game's own console".into());
    }
    None
}

/// A console line that approves (or could approve unseen: `alias`, `exec`, `schedule`); the panel
/// refuses it (see [`console_refusal`]).
pub fn is_approval_line(cmd: &str) -> bool {
    console_refusal(cmd).is_some()
}

/// Who may send a console line through the panel (the owner's decision of 2026-11-23: an
/// allowlist). `Read` and `Chat` lines are open to anyone; every other line changes the game and
/// needs the owner secret or the stack's run token ([`may_change`]). Lines [`console_refusal`]
/// refuses stay refused whoever sends them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Authority {
    Read,
    Chat,
    Owner,
}

/// The console commands that only read: they list or show, and change nothing.
const READ_COMMANDS: &[&str] = &["companies", "clients", "getdate", "help", "info", "listai", "listailibs", "listgame",
                                 "listgamelibs", "listcmds", "listaliases", "listsettings", "listsaves", "listscenarios",
                                 "listheightmaps", "listdirs", "listfiles", "pwd", "ls", "dir", "version", "list", "resetengines_check"];

/// The console commands that only talk: chat to the players, or echo to the console.
const CHAT_COMMANDS: &[&str] = &["say", "sayclient", "saycompany", "echo", "echoc"];

/// How much authority `cmd` needs, in any spelling the game accepts ([`console_word`]): a read
/// (`companies`, `setting <name>` without a value, `prismatic_mcp status` or `approvals`), a chat
/// line (`say`, `echo`, withdrawing one request with `prismatic_mcp deny <id>`), or the owner's
/// authority for everything else.
pub fn console_authority(cmd: &str) -> Authority {
    let tokens = console_tokens(cmd);
    let words: Vec<String> = tokens.iter().map(|t| console_word(t.strip_prefix('/').unwrap_or(t))).collect();
    let first = words.first().map(String::as_str).unwrap_or("");
    match first {
        w if READ_COMMANDS.contains(&w) => Authority::Read,
        w if CHAT_COMMANDS.contains(&w) => Authority::Chat,
        "setting" if words.len() <= 2 => Authority::Read,
        "prismaticmcp" => match words.get(1).map(String::as_str) {
            Some("status" | "approvals") if words.len() == 2 => Authority::Read,
            Some("deny") if words.len() == 3 && words[2] != "all" => Authority::Chat,
            _ => Authority::Owner,
        },
        _ => Authority::Owner,
    }
}

/// Whether `who` may change the game through the panel: the owner (secret, by cookie or header) or
/// the stack's own processes (the run token). Anyone else reads and chats only.
pub fn may_change(who: &Who) -> bool {
    matches!(who.role, "owner" | "run")
}

/// The refusal of a change the caller may not make: 401 with where to sign in.
fn needs_credential(what: &str) -> (u16, Value) {
    (401, json!({"error": format!("{what} changes the game: it needs the owner secret (sign in on the panel's page, or send {OWNER_HEADER}) \
                                   or the stack's run token ({RUN_TOKEN_HEADER}); reads and chat lines are open to anyone"),
                 "refused": "owner_secret", "needs": "owner_or_token", "sign_in": "/api/owner/login"}))
}

// ---------------------------------------------------------------------- the owner's approval

/// Tools whose approval belongs to the person at the game's console alone: a Battlebrain live
/// decision applies through `openttd.ai.set_settings` (one approval, every setting, an expiry). The
/// panel never approves one, not through POST /api/approve, not by auto-approve (`--auto-approve`
/// included), not from Rhai; it answers with the console command to type instead.
pub const OWNER_ONLY_TOOLS: &[&str] = &["openttd.ai.set_settings"];

pub fn owner_only(tool: &str) -> bool {
    OWNER_ONLY_TOOLS.contains(&tool)
}

/// The structured refusal of an owner-only approval: what to type, and where.
pub fn owner_only_refusal(id: i64, tool: &str) -> Value {
    json!({"error": format!("approval #{id} ({tool}) is the owner's alone: type `prismatic_mcp approve {id}` at the game's console; \
                             the panel never approves it, auto-approve included"),
           "refused": "owner_only", "approval_id": id, "tool": tool, "console": format!("prismatic_mcp approve {id}")})
}

/// The header that carries the owner secret, for a client that is not the page (curl).
pub const OWNER_HEADER: &str = "X-Panel-Owner";

/// The cookie that carries the owner secret for the page: one per panel port, since cookies ignore ports.
pub fn owner_cookie_name(port: u16) -> String {
    format!("mcp_owner_{port}")
}

/// A fresh owner secret: 32 bytes of /dev/urandom as hex; None (no secret, so the owner's routes
/// refuse every caller) when the system's randomness cannot be read. Fails closed, never weak.
pub fn new_owner_secret() -> Option<String> {
    crate::web_access::random_hex(32)
}

/// Whether a request carries the owner secret, in the header or in the page's cookie, compared in
/// constant time. A panel without a secret has no owner over HTTP.
pub fn presents_owner(secret: Option<&str>, port: u16, headers: &[(String, String)]) -> bool {
    let Some(secret) = secret else { return false };
    let header = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string());
    let cookie = owner_cookie_name(port);
    let from_cookie = header("Cookie").and_then(|c| c.split(';').find_map(|kv| kv.trim().strip_prefix(cookie.as_str())?.strip_prefix('=').map(str::to_string)));
    [header(OWNER_HEADER), from_cookie].into_iter().flatten().any(|g| crate::web_access::same(secret, &g))
}

/// The Set-Cookie header that keeps the owner secret for the page, when `given` (?owner=) is it.
pub fn owner_cookie(secret: Option<&str>, port: u16, given: Option<&str>) -> Option<String> {
    let (s, g) = (secret?, given?);
    crate::web_access::same(s, g).then(|| owner_set_cookie(port, s))
}

/// How long the page keeps the owner signed in: 12 hours (the owner's decision of 2026-11-23), so a
/// cookie taken from the browser serves a day at most.
pub const OWNER_COOKIE_MAX_AGE: u64 = 12 * 3600;

/// How long a one-time sign-in link (`?once=`, [`h_owner_once`]) serves, and that it serves once.
pub const OWNER_ONCE_TTL: Duration = Duration::from_secs(60);

/// A one-time sign-in nonce, made for a caller that holds the owner secret (`POST /api/owner/once`).
pub fn owner_once(nonces: &mut HashMap<String, Instant>, now: Instant) -> Option<String> {
    nonces.retain(|_, until| *until > now);
    let n = crate::web_access::random_hex(16)?;
    nonces.insert(n.clone(), now + OWNER_ONCE_TTL);
    Some(n)
}

/// Spend `given` (`?once=`): true once for a nonce made within [`OWNER_ONCE_TTL`], false after, for
/// a second use, and for one never made.
pub fn spend_once(nonces: &mut HashMap<String, Instant>, given: &str, now: Instant) -> bool {
    matches!(nonces.remove(given), Some(until) if until > now)
}

/// The Set-Cookie header of a signed-in owner: HttpOnly (no script reads it), SameSite=Strict (no
/// other site sends it), kept across browser restarts.
pub fn owner_set_cookie(port: u16, secret: &str) -> String {
    format!("Set-Cookie: {}={secret}; HttpOnly; SameSite=Strict; Path=/; Max-Age={OWNER_COOKIE_MAX_AGE}\r\n", owner_cookie_name(port))
}

/// The Set-Cookie header that signs the owner out of this browser.
pub fn owner_clear_cookie(port: u16) -> String {
    format!("Set-Cookie: {}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0\r\n", owner_cookie_name(port))
}

/// The environment variable naming the owner secret's file (the same name mcp-e2e reads).
pub const OWNER_FILE_ENV: &str = "PRISMATIC_PANEL_OWNER_SECRET_FILE";

/// The file the owner secret lives in: `--owner-secret-file`, else $PRISMATIC_PANEL_OWNER_SECRET_FILE,
/// else $HOME/.config/prismatic/panel-owner-secret. Err without any of them.
pub fn owner_secret_path(flag: Option<&str>, env: Option<&str>, home: Option<&str>) -> Result<PathBuf, String> {
    let nonempty = |v: Option<&str>| v.filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    nonempty(flag).or_else(|| nonempty(env))
        .or_else(|| nonempty(home).map(|h| h.join(".config/prismatic/panel-owner-secret")))
        .ok_or_else(|| format!("no owner secret file: give --owner-secret-file, set {OWNER_FILE_ENV} or HOME"))
}

/// The owner secret kept in `path`, made once (32 bytes of /dev/urandom as hex, the file 0600 in a
/// 0700 directory it creates) and read on every later start, so the owner's sign-in survives a
/// restart. A file others may read, or one that holds no secret, is refused: never weak, never shared.
pub fn load_owner_secret(path: &Path) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    let shown = path.display();
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty() && !d.exists()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if !path.exists() {
        make_owner_secret_file(path)?;
    }
    let mode = std::fs::metadata(path).map_err(|e| format!("{shown}: {e}"))?.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(format!("{shown} is readable by others (mode {mode:o}); the owner secret must be 0600: chmod 600 {shown}"));
    }
    let secret = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?.trim().to_string();
    if secret.len() < 32 || !secret.bytes().all(|c| c.is_ascii_graphic()) {
        return Err(format!("{shown} holds no owner secret (32 or more printable characters); delete it to make a new one"));
    }
    Ok(secret)
}

/// A new owner secret written to `path`, created 0600 and only if it is not there yet: a panel that
/// made it first keeps its secret, and this one then reads that.
pub fn make_owner_secret_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let secret = new_owner_secret().ok_or("/dev/urandom is unreadable: no owner secret made")?;
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path) {
        Ok(mut f) => f.write_all(format!("{secret}\n").as_bytes()).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The owner secret of a panel and what its start says about it (never the secret itself): from
/// its file (see [`owner_secret_path`]), or one-shot with `--ephemeral-owner-secret`. Err when the
/// file cannot serve, which stops the panel before it starts.
pub struct Owner {
    pub secret: Option<String>,
    /// What the panel's start prints: where the secret lives, never the secret.
    pub note: String,
    /// The secret's file, which the sign-in box names; None for a one-shot secret.
    pub file: Option<String>,
}

pub fn owner_of(args: &Args, env: Option<&str>, home: Option<&str>) -> Result<Owner, String> {
    let port = args.panel_port;
    if args.ephemeral_owner {
        return Ok(Owner { secret: new_owner_secret(), file: None, note: format!(
            "mcp-panel: one-shot owner secret, kept nowhere (--ephemeral-owner-secret): approve at the game's console (prismatic_mcp approve <id>); http://127.0.0.1:{port}/") });
    }
    let path = owner_secret_path(args.owner_file.as_deref(), env, home)?;
    let secret = load_owner_secret(&path)?;
    Ok(Owner { secret: Some(secret), file: Some(path.display().to_string()), note: format!(
        "mcp-panel: owner secret in {} (sign in once at http://127.0.0.1:{port}/ with it, or send {OWNER_HEADER}: <it>)", path.display()) })
}

/// Failed owner sign-ins allowed in [`LOGIN_WINDOW`] before every further one is refused (429).
pub const LOGIN_TRIES: usize = 5;
pub const LOGIN_WINDOW: Duration = Duration::from_secs(60);
/// How long a wrong secret waits for its answer: guessing is slow even within the window.
#[cfg(not(test))]
pub const LOGIN_DELAY: Duration = Duration::from_secs(1);
#[cfg(test)]
pub const LOGIN_DELAY: Duration = Duration::from_millis(0);

/// The owner's sign-in (POST /api/owner/login): Ok when `given` is the secret (compared in constant
/// time), always: guesses from any local caller never lock the owner out. Any other answer is 401,
/// with nothing about the secret's length or prefix, or 429 while [`LOGIN_TRIES`] wrong ones fall
/// within [`LOGIN_WINDOW`] (a 256-bit secret is not guessed; the limit and [`LOGIN_DELAY`] keep a
/// loop from flooding the event feed). `failures` keeps their times.
pub fn owner_login(secret: Option<&str>, given: &str, failures: &mut VecDeque<Instant>, now: Instant) -> Result<(), (u16, Value)> {
    if secret.is_some_and(|s| crate::web_access::same(s, given.trim())) {
        return Ok(());
    }
    while failures.front().is_some_and(|t| now.duration_since(*t) >= LOGIN_WINDOW) {
        failures.pop_front();
    }
    if failures.len() >= LOGIN_TRIES {
        return Err((429, json!({"error": format!("{LOGIN_TRIES} wrong owner secrets within {} s: wait, then sign in again", LOGIN_WINDOW.as_secs()),
                                "refused": "owner_login_rate"})));
    }
    failures.push_back(now);
    Err((401, json!({"error": "that is not the owner secret", "refused": "owner_login"})))
}

/// The POST routes only the owner secret opens, whoever else asks (the run token included): approving
/// one, the bulk decisions over a set of ids or a tool glob, the standing rules (allow and deny alike:
/// a deny rule withdraws requests nobody has seen yet), who may approve in the game, the one-time
/// sign-in link. A route that decides for the owner is added here, and nowhere else; none of them is
/// reachable through /api/console ([`console_refusal`]).
pub const OWNER_ROUTES: &[&str] = &["/api/approve", "/api/approve/many", "/api/approve/all", "/api/deny/many", "/api/deny/all",
                                    "/api/rules", "/api/rules/remove", "/api/approvers", "/api/owner/once"];

/// The owner's routes: [`OWNER_ROUTES`] and switching auto-approve on (POST /api/auto-approve
/// `{on: true}`; switching it off is open to anyone). They need the owner secret; any other request
/// passes here (a route that changes the game asks [`may_change`] itself). Err is the refusal to answer.
pub fn owner_gate(secret: Option<&str>, port: u16, method: &str, path: &str, body: &Value, headers: &[(String, String)]) -> Result<(), (u16, Value)> {
    let owners = method == "POST" && (OWNER_ROUTES.contains(&path) || (path == "/api/auto-approve" && truthy(body.get("on"))));
    if !owners || presents_owner(secret, port, headers) {
        return Ok(());
    }
    Err((401, json!({"error": format!(
        "{path} is the owner's: sign in on the panel's page with the owner secret (the file mcp-panel named when it started), \
         open http://127.0.0.1:{port}/?owner=<secret> once, or send {OWNER_HEADER}: <secret>; or approve at the game's console with prismatic_mcp approve <id>"),
        "refused": "owner_secret"})))
}

// ---------------------------------------------------------------------- bulk decisions and standing rules

// The game server's approval core keeps the standing rules and applies them (deny wins over allow,
// to waiting and new approvals alike; feature/approval-rules). The panel only asks, through the
// same channel as Approve: one line on the dedicated server's stdin. The core cannot tell stdin
// lines apart, so every line the panel sends that approves in bulk or adds a rule ends with
// `--via panel`: the core records it as the panel's and never lets it decide an owner-only tool.

/// The marker every bulk approval and every rule line of the panel ends with.
pub const VIA_PANEL: &str = "--via panel";
/// The longest matcher the panel sends: the game's console line holds 1024 bytes.
pub const MATCHER_MAX: usize = 800;
/// The most ids one bulk decision takes.
pub const BULK_MAX: usize = 500;

/// A tool glob as the core reads it: an MCP tool name or an action type, with `*` and `?`. Only
/// the characters such names hold, so a glob can never add a word to the console line.
pub fn rule_glob(tool: &str) -> Result<String, String> {
    let t = tool.trim();
    if t.is_empty() || t.len() > 128 {
        return Err("a tool glob of 1 to 128 characters (openttd.cmd.*, openttd.ai.set_setting, command.BuildRoad)".into());
    }
    if let Some(c) = t.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '*' | '?'))) {
        return Err(format!("a tool glob holds letters, digits, . _ - * and ? only, not {c:?}"));
    }
    Ok(t.to_string())
}

/// A JSON matcher as ONE console token: compact JSON (no newline survives serialisation), every
/// `;` written as `;` (the same JSON, and never a separator), every `"` as `\"` (the game's
/// tokenizer turns it back), the whole in quotes so its spaces stay inside. The game reads it back
/// as exactly the compact JSON ([`console_tokens`] mirrors its tokenizer). Only an object.
pub fn matcher_token(m: &Value) -> Result<String, String> {
    if !m.is_object() {
        return Err("a matcher is a JSON object: plain keys match a subset of the arguments, $.a.b / $.list[*] keys are JSON paths".into());
    }
    let text = serde_json::to_string(m).map_err(|e| e.to_string())?.replace(';', "\\u003b");
    if text.chars().any(char::is_control) || text.len() > MATCHER_MAX {
        return Err(format!("a matcher of at most {MATCHER_MAX} bytes of JSON on one line"));
    }
    Ok(format!("\"{}\"", text.replace('"', "\\\"")))
}

/// The glob and matcher words of a bulk line: none (every approval), the glob, or both (a matcher
/// alone means every tool, `*`). An empty object is no matcher.
fn scope_words(tool: Option<&str>, m: Option<&Value>) -> Result<String, String> {
    let tool = tool.map(str::trim).filter(|t| !t.is_empty());
    let m = m.filter(|m| !m.is_null() && m.as_object().is_none_or(|o| !o.is_empty()));
    Ok(match (tool, m) {
        (None, None) => String::new(),
        (t, None) => format!(" {}", rule_glob(t.unwrap_or("*"))?),
        (t, Some(m)) => format!(" {} {}", rule_glob(t.unwrap_or("*"))?, matcher_token(m)?),
    })
}

/// `prismatic_mcp approve all [<glob> [<matcher>]] --via panel`.
pub fn approve_all_line(tool: Option<&str>, m: Option<&Value>) -> Result<String, String> {
    Ok(format!("prismatic_mcp approve all{} {VIA_PANEL}", scope_words(tool, m)?))
}

/// `prismatic_mcp deny all [<glob> [<matcher>]] --via panel`.
pub fn deny_all_line(tool: Option<&str>, m: Option<&Value>) -> Result<String, String> {
    Ok(format!("prismatic_mcp deny all{} {VIA_PANEL}", scope_words(tool, m)?))
}

/// The channel a panel decision is recorded under: `auto` for the auto-approve switch, `panel` for
/// every other (a person's Approve or Deny, one or many).
pub fn via_of(by: &Value) -> &'static str {
    if by["channel"] == json!("auto-approve") { "auto" } else { "panel" }
}

/// `prismatic_mcp approve|deny <id> [<id> ...] [--via <channel>] [--nonce <n>]`. An older core (no
/// rules) takes neither flag, so both are optional.
pub fn decide_line(approve: bool, ids: &[i64], via: Option<&str>, nonce: Option<&str>) -> String {
    let ids: Vec<String> = ids.iter().map(i64::to_string).collect();
    let mut line = format!("prismatic_mcp {} {}", if approve { "approve" } else { "deny" }, ids.join(" "));
    if let Some(v) = via { line.push_str(&format!(" --via {v}")); }
    if let Some(n) = nonce { line.push_str(&format!(" --nonce {n}")); }
    line
}

/// What starts the approval core's one result line for a command sent with `--nonce`:
/// `[approval-core] {"nonce", "ok", "decided", "refused", "skipped", "rule_id", "error"}`.
pub const CORE_RESULT: &str = "[approval-core] ";

/// The core's result line for `nonce` among console lines, parsed; None while it has not come.
pub fn core_result(lines: &[String], nonce: &str) -> Option<Value> {
    lines.iter().find_map(|l| {
        let at = l.find(CORE_RESULT)?;
        let v: Value = serde_json::from_str(l[at + CORE_RESULT.len()..].trim()).ok()?;
        let n = &v["nonce"];
        (n.as_str().map(str::to_string).unwrap_or_else(|| n.to_string()) == nonce).then_some(v)
    })
}

/// A fresh nonce for one core line: digits only, unique in this process and across restarts.
pub fn next_nonce() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    format!("{}", base.wrapping_mul(1000).wrapping_add(n % 1000))
}

/// The id of one entry of a core result's `decided`, `refused` or `skipped`: an id, or an object
/// with `id` (and `error` or `reason`).
fn entry_of(v: &Value) -> Option<(i64, String)> {
    let id = v.as_i64().or_else(|| v["id"].as_i64())?;
    let why = v.get("error").or(v.get("reason")).and_then(Value::as_str).unwrap_or_default();
    Some((id, why.to_string()))
}

/// The per-id results of a multi-id core line: decided 200; rejected 422 (approved and carried out,
/// but the game refused the command, with its reason); refused and skipped 409 (the core would not
/// decide it: not pending, or not the panel's to decide); 504 for an id the answer does not name.
/// An entry is an id or `{id, error}` (the core names no reason for a bare id).
pub fn per_id(ids: &[i64], result: &Value, approve: bool) -> Vec<(i64, u16, Value)> {
    let list = |k: &str| -> Vec<(i64, String)> { result[k].as_array().into_iter().flatten().filter_map(entry_of).collect() };
    let (decided, refused, skipped, rejected) = (list("decided"), list("refused"), list("skipped"), list("rejected"));
    ids.iter().map(|&id| {
        let why = |w: &str| if w.is_empty() { "no reason given".to_string() } else { w.to_string() };
        if let Some((_, w)) = rejected.iter().find(|(d, _)| *d == id) {
            return (id, 422, json!({"status": "rejected", "error": format!("#{id} was approved, but the game rejected it: {}", why(w)), "approval_id": id}));
        }
        if decided.iter().any(|(d, _)| *d == id) {
            return (id, 200, json!({"status": if approve { "executed" } else { "denied" }, "approval_id": id}));
        }
        if let Some((_, w)) = refused.iter().find(|(d, _)| *d == id) {
            return (id, 409, json!({"error": format!("the approval core refused #{id}: {}", why(w)), "approval_id": id}));
        }
        if let Some((_, w)) = skipped.iter().find(|(d, _)| *d == id) {
            return (id, 409, json!({"error": format!("the approval core skipped #{id}: {}", why(w)), "approval_id": id}));
        }
        (id, 504, json!({"error": format!("the approval core's answer does not name #{id}"), "approval_id": id, "pending_decision": true}))
    }).collect()
}

/// `prismatic_mcp approvers [add <client id> | remove <key prefix>]`: Err for a body that names
/// neither a client id nor a key prefix of 8 to 128 key characters.
pub fn approvers_line(body: &Value) -> Result<String, String> {
    if let Some(add) = body.get("add") {
        let id = add.as_u64().filter(|id| *id > 0).ok_or("add: a client id (`clients` lists them)")?;
        return Ok(format!("prismatic_mcp approvers add {id}"));
    }
    if let Some(remove) = body.get("remove") {
        let key = remove.as_str().unwrap_or_default();
        if !(8..=128).contains(&key.len()) || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-')) {
            return Err("remove: 8 to 128 characters of one approver's key".into());
        }
        return Ok(format!("prismatic_mcp approvers remove {key}"));
    }
    Ok("prismatic_mcp approvers".into())
}

/// The approval core's glob (prismatic_mcp_rules.h `Glob`): `*` any run, `?` one character, a
/// backslash makes the next character itself.
pub fn core_glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' { star = Some(pi); pi += 1; mark = ti; continue; }
        if pi < p.len() && p[pi] == '?' { pi += 1; ti += 1; continue; }
        if pi < p.len() {
            let lit = if p[pi] == '\\' && pi + 1 < p.len() { pi + 1 } else { pi };
            if p[lit] == t[ti] { pi = lit + 1; ti += 1; continue; }
        }
        let Some(st) = star else { return false };
        pi = st + 1;
        mark += 1;
        ti = mark;
    }
    while pi < p.len() && p[pi] == '*' { pi += 1; }
    pi == p.len()
}

/// Whether a glob names more than one tool (an unescaped `*` or `?`).
pub fn has_wildcard(pattern: &str) -> bool {
    let mut it = pattern.chars();
    while let Some(c) = it.next() {
        if c == '\\' { it.next(); continue; }
        if c == '*' || c == '?' { return true; }
    }
    false
}

/// The values a JSON path (`$`, `.name`, `['name']`, `[n]`, `[*]`, `.*`, `..name`) selects in
/// `root`; None when it does not parse (prismatic_mcp_rules.h `ParsePath`, `Select`).
fn path_select<'a>(root: &'a Value, path: &str) -> Option<Vec<&'a Value>> {
    fn descend<'a>(n: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
        match n {
            Value::Object(o) => for (k, v) in o { if k == key { out.push(v); } descend(v, key, out); },
            Value::Array(a) => for v in a { descend(v, key, out); },
            _ => {}
        }
    }
    let b = path.strip_prefix('$')?;
    let mut at = vec![root];
    let mut i = 0;
    let end = |from: usize| b[from..].find(['.', '[']).map(|e| from + e).unwrap_or(b.len());
    while i < b.len() {
        let mut next = Vec::new();
        if b[i..].starts_with("..") {
            let e = end(i + 2);
            if e == i + 2 { return None; }
            for n in &at { descend(n, &b[i + 2..e], &mut next); }
            i = e;
        } else if b[i..].starts_with('.') {
            let e = end(i + 1);
            if e == i + 1 { return None; }
            let name = &b[i + 1..e];
            for n in &at {
                if name == "*" { match n { Value::Array(a) => next.extend(a.iter()), Value::Object(o) => next.extend(o.values()), _ => {} } }
                else if let Some(v) = n.get(name) { next.push(v); }
            }
            i = e;
        } else if b[i..].starts_with('[') {
            let close = i + b[i..].find(']')?;
            let inner = &b[i + 1..close];
            let quoted = inner.len() >= 2 && (inner.starts_with('\'') || inner.starts_with('"')) && inner.ends_with(&inner[..1]);
            for n in &at {
                if inner == "*" { match n { Value::Array(a) => next.extend(a.iter()), Value::Object(o) => next.extend(o.values()), _ => {} } }
                else if quoted { if let Some(v) = n.get(&inner[1..inner.len() - 1]) { next.push(v); } }
                else {
                    if inner.is_empty() || inner.len() > 9 || !inner.bytes().all(|c| c.is_ascii_digit()) { return None; }
                    if let Some(v) = n.get(inner.parse::<usize>().ok()?) { next.push(v); }
                }
            }
            i = close + 1;
        } else {
            return None;
        }
        at = next;
    }
    Some(at)
}

/// Whether `value` matches the JSON matcher `pattern`, as the approval core decides it
/// (prismatic_mcp_rules.h `Match`): an object's plain keys match a subset, `$...` keys are paths
/// that must select at least one value, every one matching; `$lt` `$lte` `$gt` `$gte` `$in` `$ne`
/// compare the value itself; `"*"` is any value present, another string a glob over a string or the
/// JSON text of anything else; an array matches element by element; other scalars by JSON equality
/// (or a string holding their JSON text).
pub fn core_match(pattern: &Value, value: &Value) -> bool {
    match pattern {
        Value::Object(o) => o.iter().all(|(k, p)| match k.as_str() {
            "$in" => p.as_array().is_some_and(|a| a.iter().any(|e| core_match(e, value))),
            "$ne" => !core_match(p, value),
            op @ ("$lt" | "$lte" | "$gt" | "$gte") => match (value.as_f64(), p.as_f64()) {
                (Some(a), Some(b)) => match op { "$lt" => a < b, "$lte" => a <= b, "$gt" => a > b, _ => a >= b },
                _ => false,
            },
            k if k.starts_with('$') => path_select(value, k).is_some_and(|found| !found.is_empty() && found.iter().all(|f| core_match(p, f))),
            k => value.get(k).is_some_and(|v| core_match(p, v)),
        }),
        Value::String(p) if p == "*" => true,
        Value::String(p) => core_glob(p, &value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string())),
        Value::Array(a) => value.as_array().is_some_and(|v| v.len() == a.len() && a.iter().zip(v).all(|(p, x)| core_match(p, x))),
        _ => match value.as_str() { Some(s) => pattern.to_string() == s, None => pattern == value },
    }
}

/// Whether a matcher value constrains nothing: a non-empty string of `*` only, or an object that
/// is itself broad (an operator object whose only operator is `$ne`, or a nested matcher).
fn non_constraining(v: &Value) -> bool {
    match v {
        Value::String(t) => !t.is_empty() && t.chars().all(|c| c == '*'),
        Value::Object(_) => broad_matcher(Some(v)),
        _ => false,
    }
}

/// Whether a matcher constrains no value (the core's `IsBroadMatcher`, the same structural test):
/// null, `{}`, or an object whose every entry is non-constraining. Any key, plain or a path
/// (`$`, `$.a`, `$['a b']`, `$.l[*]`, `$.*`, `$..name`), is non-constraining when its value is a
/// non-empty string of `*` only, an operator object whose only operator is `$ne`, or a nested
/// object that is itself broad; a `$ne` entry never narrows. Everything else narrows: a literal,
/// `""`, a glob with any other character or `?`, `$in`, `$lt`/`$lte`/`$gt`/`$gte`, an array. The
/// presence of a key alone never narrows.
pub fn broad_matcher(m: Option<&Value>) -> bool {
    match m {
        None | Some(Value::Null) => true,
        Some(Value::Object(o)) => o.iter().all(|(k, v)| match k.as_str() {
            "$ne" => true,
            "$in" | "$lt" | "$lte" | "$gt" | "$gte" => false,
            _ => non_constraining(v),
        }),
        Some(_) => false,
    }
}

/// `prismatic_mcp rule allow|deny <glob> [<matcher>] [--persist] --via panel`. An allow rule that
/// names an owner-only tool outright is refused here: the core never lets the panel's rule decide
/// one (ADR 0038), so it could only ever mislead.
pub fn rule_line(effect: &str, tool: &str, m: Option<&Value>, persist: bool) -> Result<String, String> {
    if effect != "allow" && effect != "deny" {
        return Err(format!("a rule's effect is allow or deny, not {effect:?}"));
    }
    let glob = rule_glob(tool)?;
    let names_owner_only = OWNER_ONLY_TOOLS.iter().any(|o| glob == *o || glob == o.trim_start_matches("openttd."));
    if effect == "allow" && names_owner_only {
        return Err(format!("{glob} is the owner's alone: it is decided at the game's console, and no allow rule of the panel decides it"));
    }
    // The core's BroadAllowRefusal, mirrored: an allow rule over several tools that takes any
    // arguments approves whatever they ask; only the game's console makes one, with --force.
    if effect == "allow" && has_wildcard(&glob) && broad_matcher(m) {
        return Err(format!("an allow rule for every tool of '{glob}' with any arguments approves whatever they ask: give a matcher that narrows \
                            the arguments, name one tool, or make it at the game's console with --force"));
    }
    let words = scope_words(Some(&glob), m)?;
    Ok(format!("prismatic_mcp rule {effect}{words}{} {VIA_PANEL}", if persist { " --persist" } else { "" }))
}

/// `prismatic_mcp rule remove <id>`.
pub fn rule_remove_line(id: i64) -> String {
    format!("prismatic_mcp rule remove {id}")
}

/// What the approvals list reads (GET /api/approvals), from the server's queue (openttd://approvals
/// data): every approval with `owner_only` and `decidable` (pending, not expired) added, the
/// standing rules, and whether the core keeps rules at all (`rules_supported`: the queue carries
/// `rules`, as the approval-rules core answers it).
pub fn approvals_view(data: &Value) -> Value {
    let approvals: Vec<Value> = data["approvals"].as_array().into_iter().flatten().map(|a| {
        let mut a = a.clone();
        a["owner_only"] = json!(a["tool"].as_str().is_some_and(owner_only));
        a["decidable"] = json!(a["state"] == json!("pending") && a["expired"] != json!(true));
        a
    }).collect();
    json!({"approvals": approvals, "rules": data.get("rules").cloned().unwrap_or(json!([])),
           "rules_supported": data.get("rules").is_some_and(Value::is_array),
           "capacity": data.get("capacity").cloned().unwrap_or(Value::Null), "owner_only_tools": OWNER_ONLY_TOOLS})
}

/// The answer of a bulk decision over ids: one result per id (its HTTP status and the core's
/// answer), and how many went through.
pub fn bulk_answer(results: Vec<(i64, u16, Value)>) -> Value {
    let done = results.iter().filter(|(_, c, _)| *c == 200).count();
    json!({"done": done, "failed": results.len() - done,
           "results": results.into_iter().map(|(id, code, out)| json!({"id": id, "code": code, "answer": out})).collect::<Vec<_>>()})
}

/// The ids of a bulk body: 1 to [`BULK_MAX`] integers, each once, in the order given.
pub fn bulk_ids(body: &Value) -> Result<Vec<i64>, String> {
    let ids = body["ids"].as_array().ok_or("ids: a list of approval ids")?;
    let mut out: Vec<i64> = Vec::new();
    for v in ids {
        let id = v.as_i64().ok_or_else(|| format!("ids: {v} is not an approval id"))?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    if out.is_empty() || out.len() > BULK_MAX {
        return Err(format!("ids: 1 to {BULK_MAX} approval ids"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------- who asked (governance audit)

/// The environment variable the game server reads its approval deadline from (`<days>:<verdict>`,
/// research/mechanisms/approval-gate.md); the panel only reports it.
pub const DEADLINE_ENV: &str = "PRISMATIC_MCP_APPROVAL_DEADLINE";

/// The header the stack's own processes send the panel's run token in (`--run-token-file`): it names
/// the caller as the stack's own (credential `token`, role `run`) and never opens an owner's route.
pub const RUN_TOKEN_HEADER: &str = crate::rhai_host::RUN_TOKEN_HEADER;

/// The panel's run token by its URL, for the scripts it runs in process ([`spawn_script`]): a test
/// process runs several panels, each on a port of its own.
fn run_tokens() -> &'static Mutex<HashMap<String, String>> {
    static T: std::sync::OnceLock<Mutex<HashMap<String, String>>> = std::sync::OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The run token of the panel at `base`, if this process runs it.
pub fn run_token_for(base: &str) -> Option<String> {
    run_tokens().lock().unwrap().get(base.trim_end_matches('/')).cloned()
}

/// The header the panel itself puts on a request with the address it came from (`Panel::handle`);
/// a client's own header of that name is dropped first, so it cannot claim another address.
pub const PEER_HEADER: &str = "X-Panel-Peer";

/// Who sent a request, as the governance audit records it ([`Game::governance`]): which credential
/// it carried, never the credential itself. `owner-cookie` is the page's signed-in cookie, `token`
/// the owner secret in the [`OWNER_HEADER`] header (a script, curl), `none` anything else, a wrong
/// secret included.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Who {
    pub credential: &'static str,
    /// Whose credential it is: `owner` (the owner secret, by cookie or header), `run` (the stack's
    /// run token) or `none`.
    pub role: &'static str,
    pub origin: Option<String>,
    pub user_agent: Option<String>,
    pub peer: Option<String>,
}

impl Who {
    /// No request at all: a flag or the environment at start.
    pub fn none() -> Who {
        Who { credential: "none", role: "none", ..Who::default() }
    }

    /// Who sent `headers`, judged against the owner `secret` of the panel on `port`.
    pub fn of(secret: Option<&str>, port: u16, headers: &[(String, String)]) -> Who {
        let header = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.trim().to_string());
        let cookie = owner_cookie_name(port);
        let from_cookie = header("Cookie").and_then(|c| c.split(';').find_map(|kv| kv.trim().strip_prefix(cookie.as_str())?.strip_prefix('=').map(str::to_string)));
        let matches = |given: Option<String>| secret.zip(given).is_some_and(|(s, g)| crate::web_access::same(s, &g));
        let credential = if matches(from_cookie) { "owner-cookie" } else if matches(header(OWNER_HEADER)) { "token" } else { "none" };
        let role = if credential == "none" { "none" } else { "owner" };
        Who { credential, role, origin: header("Origin"), user_agent: header("User-Agent"), peer: header(PEER_HEADER) }
    }

    /// [`Who::of`], and the stack's run `token` in [`RUN_TOKEN_HEADER`] when no owner secret came.
    pub fn of_run(secret: Option<&str>, token: Option<&str>, port: u16, headers: &[(String, String)]) -> Who {
        let w = Who::of(secret, port, headers);
        let given = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(RUN_TOKEN_HEADER)).map(|(_, v)| v.trim().to_string());
        if w.role == "none" && token.zip(given).is_some_and(|(t, g)| crate::web_access::same(t, &g)) {
            return Who { credential: "token", role: "run", ..w };
        }
        w
    }
}

/// The governance change a console line makes, if it makes one: `prismatic_mcp approval_deadline
/// <days> [verdict]` (what an unanswered approval becomes), as (setting, to), in any spelling the game
/// accepts. Who may approve changes only through `POST /api/approvers` ([`approvers_change`]): the
/// console refuses those lines.
pub fn console_governance(cmd: &str) -> Option<(&'static str, Value)> {
    let tokens = console_tokens(cmd);
    let words: Vec<String> = tokens.iter().map(|t| console_word(t.strip_prefix('/').unwrap_or(t))).collect();
    (words.first().map(String::as_str) == Some("prismaticmcp") && words.get(1).map(String::as_str) == Some("approvaldeadline") && tokens.len() > 2)
        .then(|| ("approval_deadline", json!(tokens[2..].join(" "))))
}

/// The change of who may approve a `POST /api/approvers` body asks for (`{add}` or `{remove}`), as
/// the governance event's `to`; None for a listing (`{}`).
pub fn approvers_change(body: &Value) -> Option<Value> {
    ["add", "remove"].into_iter().find_map(|op| body.get(op).filter(|v| !v.is_null()).map(|v| json!({"op": op, "args": v})))
}

// ---------------------------------------------------------------------- subprocesses

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// The children of `run` while they run, by pid. Each leads a process group of its own, so a stop
/// reaches what it started too (mcp-plan --track runs AlgoGlyph's node). Before this, a panel
/// stopped while the bot's planner ran left mcp-plan running, re-parented to PID 1.
pub struct Children {
    pids: Mutex<std::collections::BTreeSet<u32>>,
    stopping: AtomicBool,
}

/// Unregisters a child of `run` once it is reaped, or when `run` returns early.
struct Registered<'a>(&'a Children, u32);

impl Drop for Registered<'_> {
    fn drop(&mut self) {
        self.0.pids.lock().unwrap().remove(&self.1);
    }
}

impl Children {
    pub const fn new() -> Self {
        Children { pids: Mutex::new(std::collections::BTreeSet::new()), stopping: AtomicBool::new(false) }
    }

    pub fn running(&self) -> usize {
        self.pids.lock().unwrap().len()
    }

    /// Spawn `cmd` in a process group of its own and register it; refused once `stop` has begun, so
    /// nothing starts after the last child was stopped.
    fn spawn(&self, cmd: &mut Command) -> Result<(Child, Registered<'_>), String> {
        use std::os::unix::process::CommandExt;
        let mut pids = self.pids.lock().unwrap();
        if self.stopping.load(Ordering::SeqCst) {
            return Err(format!("{cmd:?}: the panel is stopping"));
        }
        let child = cmd.process_group(0).spawn().map_err(|e| format!("{cmd:?}: {e}"))?;
        let pid = child.id();
        pids.insert(pid);
        Ok((child, Registered(self, pid)))
    }

    /// Stop every child and its process group: SIGTERM, then SIGKILL after `grace`; each is reaped by
    /// the `run` that waits on it. The number of children that were running.
    pub fn stop(&self, grace: Duration) -> usize {
        let signal = |sig: libc::c_int| for &pid in self.pids.lock().unwrap().iter() {
            // SAFETY: kill(2) on a process group this process created and has not yet reaped.
            unsafe { libc::kill(-(pid as libc::pid_t), sig); }
        };
        let n = {
            let pids = self.pids.lock().unwrap();
            self.stopping.store(true, Ordering::SeqCst);
            pids.len()
        };
        let wait = |d: Duration| {
            let deadline = Instant::now() + d;
            while self.running() > 0 && Instant::now() < deadline { thread::sleep(Duration::from_millis(20)); }
        };
        signal(libc::SIGTERM);
        wait(grace);
        signal(libc::SIGKILL);
        wait(Duration::from_secs(1));
        n
    }
}

pub static CHILDREN: Children = Children::new();

/// Stop every subprocess the panel runs (`CHILDREN`): the shutdown path, before the game stops.
pub fn stop_children(grace: Duration) -> usize {
    CHILDREN.stop(grace)
}

/// Run a command to its end within `timeout`, its output captured (stdout to `stdout_file` when
/// given), `input` on its stdin.
pub fn run(cmd: &mut Command, input: Option<Vec<u8>>, timeout: Duration, stdout_file: Option<std::fs::File>) -> Result<Run, String> {
    run_in(&CHILDREN, cmd, input, timeout, stdout_file)
}

fn run_in(children: &Children, cmd: &mut Command, input: Option<Vec<u8>>, timeout: Duration, stdout_file: Option<std::fs::File>) -> Result<Run, String> {
    let shown = format!("{cmd:?}");
    cmd.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() }).stderr(Stdio::piped());
    match stdout_file {
        Some(f) => cmd.stdout(f),
        None => cmd.stdout(Stdio::piped()),
    };
    let (mut child, registered) = children.spawn(cmd)?;
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        thread::spawn(move || { let _ = stdin.write_all(&data); });
    }
    let reader = |r: Option<Box<dyn Read + Send>>| thread::spawn(move || {
        let mut s = Vec::new();
        if let Some(mut r) = r { let _ = r.read_to_end(&mut s); }
        String::from_utf8_lossy(&s).into_owned()
    });
    let out = reader(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err = reader(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? { break s; }
        if Instant::now() >= deadline {
            // SAFETY: kill(2) on the process group of a child not yet reaped; what it started goes too.
            unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL); }
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Command {shown} timed out after {} seconds", timeout.as_secs_f64()));
        }
        thread::sleep(Duration::from_millis(20));
    };
    // Reaped: unregistered now, not after the readers, which a grandchild can hold open.
    drop(registered);
    Ok(Run { code: status.code().unwrap_or(-1), stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
}

// ---------------------------------------------------------------------- MCP over HTTP

/// One JSON-RPC call on an MCP endpoint with a bearer token; the whole JSON-RPC answer. An HTTP
/// error status is an error, as urllib raises it.
pub fn mcp_post(endpoint: &str, token: &str, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let r = ureq::post(endpoint).timeout(timeout)
        .set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream")
        .set("Authorization", &format!("Bearer {token}"))
        .send_string(&body.to_string());
    match r {
        Ok(resp) => resp.into_json::<Value>().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(code, resp)) => Err(format!("HTTP Error {code}: {}", resp.status_text())),
        Err(e) => Err(e.to_string()),
    }
}

/// Whether a failed MCP call is tried once more. The game's MCP server answers one request at a time,
/// and the main page fires many reads at once: now and then one connection is refused or dropped and
/// the page saw a 502 (/api/catalogue, /api/bots/choices, 2026-11-17). A read is tried again after any
/// transport error; any other method only when the connection never opened, so a command the server
/// may have received is never sent twice.
pub fn rpc_retryable(method: &str, kind: ureq::ErrorKind, attempt: u32) -> bool {
    let read = matches!(method, "initialize" | "tools/list" | "resources/list" | "resources/read" | "prompts/list" | "ping");
    attempt == 0 && (read || kind == ureq::ErrorKind::ConnectionFailed)
}

/// A token file `{agent_token, endpoint}` as `prismatic_mcp start <port> <file>` writes it.
fn token_file(path: &str) -> Result<(String, String), String> {
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
        .map_err(|e| format!("{path}: {e}"))?;
    let endpoint = cfg["endpoint"].as_str().ok_or_else(|| format!("{path}: no endpoint"))?.to_string();
    let token = cfg["agent_token"].as_str().ok_or_else(|| format!("{path}: no agent_token"))?.to_string();
    Ok((endpoint, token))
}

/// The JSON a tool or resource answer carries in its first text content, else None.
fn content_json(r: &Value, pointer: &str) -> Option<Value> {
    serde_json::from_str(r.pointer(pointer)?.as_str()?).ok()
}

/// The HTTP status of an MCP answer the panel passes on: 200 for content, 404 when the server does
/// not know the resource or tool (JSON-RPC -32602, e.g. a resource this build does not have), 502 for
/// any other JSON-RPC error. A tool's own refusal inside its content stays 200 with its body.
pub fn rpc_status(out: &Value) -> u16 {
    let Some(err) = out.get("error").filter(|e| e.is_object()) else { return 200 };
    if err.get("jsonrpc").is_none() {
        return 200;
    }
    match err.get("error").filter(|e| e.is_object()) {
        Some(rpc) if rpc.get("code").and_then(Value::as_i64) == Some(-32602) => 404,
        Some(_) => 502,
        None => 200,
    }
}

// ---------------------------------------------------------------------- the game

struct Console {
    stdin: Option<ChildStdin>,
    last: Option<Instant>,
}

#[derive(Default)]
struct Log {
    seq: u64,
    lines: VecDeque<(u64, String)>,
}

/// One dedicated server, its console and its MCP endpoint.
pub struct Game {
    pub args: Args,
    tmp: PathBuf,
    child: Mutex<Option<Child>>,
    running: AtomicBool,
    console_w: Mutex<Console>,
    token: Mutex<Option<String>>,
    log: Mutex<Log>,
    events: Mutex<VecDeque<Value>>,
    pub bus: Bus,
    /// The approvals already announced as an event: the queue itself is the game server's
    /// (openttd://approvals, ADR 0038), and auto-approve takes only these, the panel's own requests.
    announced: Mutex<HashSet<i64>>,
    /// Tests stand in for the game server's queue (openttd://approvals) with this.
    #[cfg(test)]
    pub test_approvals: Mutex<Vec<Value>>,
    /// Tests stand in for the approval-rules core's `rules` in openttd://approvals with this (None:
    /// a core without rules).
    #[cfg(test)]
    pub test_rules: Mutex<Option<Vec<Value>>>,
    /// Tests read the console lines the panel sends here instead of a game's stdin (None: closed).
    #[cfg(test)]
    pub test_console: Mutex<Option<Vec<String>>>,
    /// The owner secret (see [`owner_gate`]): read from its 0600 file at start ([`owner_of`]), never
    /// printed, never sent to the bus, the store, the event feed or any answer.
    owner_secret: Option<String>,
    /// What [`Game::start`] says about the owner secret (where it lives), or why it cannot serve.
    owner_note: Result<String, String>,
    /// The owner secret's file, named to whoever is refused an owner's route.
    owner_file: Option<String>,
    /// The times of the recent failed sign-ins ([`owner_login`]).
    owner_failures: Mutex<VecDeque<Instant>>,
    /// The one-time sign-in nonces not spent yet, each with when it stops serving ([`owner_once`]).
    owner_nonces: Mutex<HashMap<String, Instant>>,
    /// The stack's run token ([`RUN_TOKEN_HEADER`]): from `--run-token-file`, else made for the
    /// panel's in-process scripts; Err when the file cannot serve (the panel stops at start).
    run_token: Result<String, String>,
    pub auto_approve: AtomicBool,
    pub mcp_up: AtomicBool,
    pub ai_started: AtomicBool,
    rpc_id: AtomicU64,
    server_tools: Mutex<(Vec<Value>, Option<Instant>)>,
    client_tools: Mutex<(Vec<Value>, Option<Instant>)>,
    owner_q: Mutex<Option<mpsc::Sender<(String, String)>>>,
    plans_seen: Mutex<HashSet<i64>>,
    // Answered questions already published as ask.answered, by (id, question): the game window
    // numbers its questions from 1 again when the GUI client restarts, so an id alone matched a
    // different, earlier question and the new answer was never published.
    asks_reported: Mutex<HashSet<String>>,
    view_token: Mutex<Option<String>>,
    /// The overview's history: one sample per game day, newest last (see `sample_of`).
    history: Mutex<VecDeque<Value>>,
}

pub const NO_CLIENT: &str = "no GUI client attached (POST /api/view/attach {token_file})";

impl Game {
    pub fn new(args: Args) -> Arc<Game> {
        let tmp = std::env::temp_dir().join(format!("openttd-mcp-panel.{}.{}", std::process::id(), args.panel_port));
        // A test panel never touches the person's own secret file: without --owner-secret-file it is one-shot.
        #[cfg(test)]
        let (env, home) = (None::<String>, None::<String>);
        #[cfg(not(test))]
        let (env, home) = (std::env::var(OWNER_FILE_ENV).ok(), std::env::var("HOME").ok());
        let owner = if cfg!(test) && args.owner_file.is_none() {
            owner_of(&Args { ephemeral_owner: true, ..args.clone() }, None, None)
        } else {
            owner_of(&args, env.as_deref(), home.as_deref())
        };
        let (owner_secret, owner_note, owner_file) = match owner {
            Ok(o) => (o.secret, Ok(o.note), o.file),
            Err(e) => (None, Err(e), None),
        };
        #[cfg(test)]
        let deadline = None::<String>;
        #[cfg(not(test))]
        let deadline = std::env::var(DEADLINE_ENV).ok();
        let run_token = match &args.run_token_file {
            Some(f) => crate::rhai_host::read_secret_file(f),
            None => crate::web_access::random_hex(32).ok_or_else(|| "no run token: /dev/urandom cannot be read".to_string()),
        };
        if let Ok(t) = &run_token {
            run_tokens().lock().unwrap().insert(format!("http://127.0.0.1:{}", args.panel_port), t.clone());
        }
        let g = Arc::new(Game {
            run_token,
            auto_approve: AtomicBool::new(args.auto_approve),
            args, tmp,
            child: Mutex::new(None), running: AtomicBool::new(false),
            console_w: Mutex::new(Console { stdin: None, last: None }),
            token: Mutex::new(None), log: Mutex::new(Log::default()), events: Mutex::new(VecDeque::new()),
            bus: Bus::new(), announced: Mutex::new(HashSet::new()),
            #[cfg(test)]
            test_approvals: Mutex::new(vec![]),
            #[cfg(test)]
            test_rules: Mutex::new(None),
            #[cfg(test)]
            test_console: Mutex::new(None),
            owner_secret, owner_note, owner_file, owner_failures: Mutex::new(VecDeque::new()), owner_nonces: Mutex::new(HashMap::new()),
            mcp_up: AtomicBool::new(false), ai_started: AtomicBool::new(false), rpc_id: AtomicU64::new(0),
            server_tools: Mutex::new((vec![], None)), client_tools: Mutex::new((vec![], None)),
            owner_q: Mutex::new(None), plans_seen: Mutex::new(HashSet::new()), history: Mutex::new(VecDeque::new()),
            asks_reported: Mutex::new(HashSet::new()), view_token: Mutex::new(None),
        });
        g.start_governance(deadline.as_deref());
        g
    }

    // ------------------------------------------------------------------ process
    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        // Where the owner secret lives, never the secret: stdout ends up in logs anyone may read.
        // A secret file that cannot serve stops the panel here, before a game starts.
        println!("{}", self.owner_note.clone()?);
        self.run_token.as_ref().map_err(String::clone)?;
        std::fs::create_dir_all(&self.tmp).map_err(|e| format!("{}: {e}", self.tmp.display()))?;
        let cfg = self.tmp.join("openttd.cfg");
        std::fs::write(&cfg, format!(
            "[network]\nserver_name = OpenTTD MCP research server\nserver_game_type = local\n\
             server_port = {}\nmax_companies = 15\n[difficulty]\nmax_no_competitors = 1\n\
             [script]\nscript_max_opcode_till_suspend = {}\n", self.args.game_port, SCRIPT_OPCODES))
            .map_err(|e| format!("{}: {e}", cfg.display()))?;
        // A savegame (--load) is the world to start from; otherwise a new one from the seed.
        let world: Vec<String> = match &self.args.load {
            Some(file) => vec!["-g".into(), file.clone()],
            None => vec!["-g".into(), "-G".into(), self.args.seed.to_string()],
        };
        let mut argv: Vec<String> = vec![self.args.bin.clone(), "-D".into(), format!("127.0.0.1:{}", self.args.game_port), "-x".into()];
        argv.extend(world);
        argv.extend(["-c".into(), cfg.display().to_string(), "-d".into(), "net=1,script=4".into()]);
        self.event("server", &format!("starting {}", argv.join(" ")));
        // stdout and stderr in one pipe, in the order the game writes them (exec keeps the pid).
        let mut child = Command::new("/bin/sh").arg("-c").arg("exec \"$0\" \"$@\" 2>&1").args(&argv)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| format!("{}: {e}", self.args.bin))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        self.console_w.lock().unwrap().stdin = child.stdin.take();
        *self.child.lock().unwrap() = Some(child);
        self.running.store(true, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        *self.owner_q.lock().unwrap() = Some(tx);
        let g = self.clone();
        thread::spawn(move || g.read_log(stdout));
        let g = self.clone();
        thread::spawn(move || g.bring_up());
        let g = self.clone();
        thread::spawn(move || g.auto_loop());
        let g = self.clone();
        thread::spawn(move || g.watch());
        let g = self.clone();
        thread::spawn(move || g.watch_news());
        let g = self.clone();
        thread::spawn(move || g.owner_console_loop(rx));
        Ok(())
    }

    pub fn server_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn read_log(self: Arc<Self>, stdout: std::process::ChildStdout) {
        let mut r = BufReader::new(stdout);
        let mut raw = Vec::new();
        loop {
            raw.clear();
            match r.read_until(b'\n', &mut raw) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let text = String::from_utf8_lossy(&raw);
            let held = self.token.lock().unwrap().clone();
            let (line, found) = redact(text.trim_end(), held.as_deref());
            if let Some(t) = found {
                *self.token.lock().unwrap() = Some(t);
            }
            {
                let mut log = self.log.lock().unwrap();
                log.seq += 1;
                let seq = log.seq;
                log.lines.push_back((seq, line.clone()));
                while log.lines.len() > 5000 { log.lines.pop_front(); }
            }
            if let Some((n, text)) = plan_request(&line) {
                if self.plans_seen.lock().unwrap().insert(n) {
                    let g = self.clone();
                    thread::spawn(move || g.answer_plan(n, &text));
                }
            }
            match ai_log(&line) {
                Some((company, level, text)) => { self.bus.publish("ai.log", json!({"company": company, "level": level, "text": text})); }
                None => { self.bus.publish("console.line", json!({"line": line})); }
            }
        }
        let code = self.child.lock().unwrap().as_mut().and_then(|c| c.wait().ok()).and_then(|s| s.code());
        self.running.store(false, Ordering::SeqCst);
        self.event("server", &format!("server process exited with {}", code.map_or("None".into(), |c| c.to_string())));
    }

    /// One console line. One at a time, 0.25 s apart: the dedicated server tests stdin with select()
    /// and reads it with std::getline (src/video/dedicated_v.cpp), so lines that arrive together sit
    /// in the stream buffer, invisible to select(), until the next write (seen: a plan answer's lines
    /// appeared minutes late). A gap longer than one game-loop pass gives each line its own read.
    pub fn console(&self, command: &str) -> Result<(), String> {
        // One line, always: a newline inside a value (an AI's settings, a content query) would
        // otherwise reach the game as a second command nobody checked, an approval included.
        if command.chars().any(char::is_control) {
            return Err(format!("refused console line with a control character: {}", head(&command.escape_debug().to_string(), 80)));
        }
        #[cfg(test)]
        if let Some(lines) = self.test_console.lock().unwrap().as_mut() {
            lines.push(command.to_string());
            self.test_core(command);
            return Ok(());
        }
        let mut c = self.console_w.lock().unwrap();
        if let Some(last) = c.last {
            let wait = Duration::from_millis(250).saturating_sub(last.elapsed());
            if !wait.is_zero() { thread::sleep(wait); }
        }
        let stdin = c.stdin.as_mut().ok_or("the server's console is closed")?;
        stdin.write_all(format!("{command}\n").as_bytes()).and_then(|_| stdin.flush()).map_err(|e| e.to_string())?;
        c.last = Some(Instant::now());
        Ok(())
    }

    /// The approval-rules core as tests stand in for it (with `test_rules` set): `rule allow|deny
    /// <glob> ...` adds a rule, `rule remove <id>` removes one, `approve|deny all` and `approve|deny
    /// <id> ...` decide (a pending one of `test_approvals`; others skipped), each answered with the
    /// core's result line for its `--nonce`. A glob starting `bad` is refused (`ok: false`);
    /// `approvers add 404` is answered with the game's refusal.
    #[cfg(test)]
    fn test_core(&self, command: &str) {
        let t = console_tokens(command);
        let say = |line: &str| {
            let mut log = self.log.lock().unwrap();
            log.seq += 1;
            let seq = log.seq;
            log.lines.push_back((seq, line.to_string()));
        };
        if t.get(1).is_some_and(|w| w == "approvers") {
            return say(if t.get(3).is_some_and(|w| w == "404") { "No connected client #404 with a key ('clients' lists them)." } else { "Client #2 (owner) is an approver now." });
        }
        let mut rules = self.test_rules.lock().unwrap();
        let Some(rules) = rules.as_mut() else { return };
        let nonce = t.iter().position(|w| w == "--nonce").and_then(|i| t.get(i + 1)).cloned().unwrap_or_default();
        let answer = |extra: Value| {
            let mut v = json!({"nonce": nonce, "ok": true});
            for (k, x) in extra.as_object().into_iter().flatten() { v[k] = x.clone(); }
            say(&format!("{CORE_RESULT}{v}"));
        };
        if t.get(3).is_some_and(|g| g.starts_with("bad")) {
            return answer(json!({"ok": false, "error": format!("{} is not a tool glob", t[3])}));
        }
        match (t.get(1).map(String::as_str), t.get(2).map(String::as_str)) {
            (Some("rule"), Some(effect @ ("allow" | "deny"))) => {
                let id = rules.iter().filter_map(|r| r["id"].as_i64()).max().unwrap_or(0) + 1;
                let m = t.get(4).filter(|w| !w.starts_with("--")).and_then(|w| serde_json::from_str::<Value>(w).ok());
                rules.push(json!({"id": id, "effect": effect, "tool": t[3], "match": m, "created_via": "panel", "hits": 0,
                                  "persisted": t.iter().any(|w| w == "--persist")}));
                answer(json!({"rule_id": id, "decided": []}));
            }
            (Some("rule"), Some("remove")) => {
                let id = t.get(3).and_then(|w| w.parse::<i64>().ok());
                rules.retain(|r| r["id"].as_i64() != id);
                answer(json!({}));
            }
            (Some("approve" | "deny"), Some("all")) => answer(json!({"decided": []})),
            (Some(verb @ ("approve" | "deny")), Some(_)) => {
                let ids: Vec<i64> = t[2..].iter().map_while(|w| w.parse::<i64>().ok()).collect();
                let mut queue = self.test_approvals.lock().unwrap();
                let (mut decided, mut skipped) = (vec![], vec![]);
                for id in ids {
                    match queue.iter_mut().find(|a| a["id"] == json!(id) && a["state"] == json!("pending")) {
                        Some(a) => { a["state"] = json!(if verb == "approve" { "consumed" } else { "denied" }); decided.push(id); }
                        None => skipped.push(json!(id)),
                    }
                }
                // As the real core answers: a bare id it would not decide goes under `refused`.
                if !nonce.is_empty() { answer(json!({"decided": decided, "refused": skipped, "skipped": []})); }
            }
            _ => {}
        }
    }

    pub fn log_seq(&self) -> u64 {
        self.log.lock().unwrap().seq
    }

    pub fn log_after(&self, mark: u64) -> Vec<String> {
        self.log.lock().unwrap().lines.iter().filter(|(n, _)| *n > mark).map(|(_, l)| l.clone()).collect()
    }

    fn log_has(&self, needle: &str) -> bool {
        self.log.lock().unwrap().lines.iter().any(|(_, l)| l.contains(needle))
    }

    /// Print a line in the owner's ~ console (the GUI client), coloured by level, through
    /// openttd.view.console.print; queued so lines keep their order and a missing client costs nothing.
    pub fn to_owner_console(&self, text: &str, level: &str) {
        if self.view_token.lock().unwrap().is_none() {
            return;
        }
        if let Some(tx) = self.owner_q.lock().unwrap().as_ref() {
            let _ = tx.send((text.to_string(), level.to_string()));
        }
    }

    fn owner_console_loop(self: Arc<Self>, rx: mpsc::Receiver<(String, String)>) {
        for (text, level) in rx {
            // No client, or an older one without the tool: nothing to do.
            let _ = self.view_call("openttd.view.console.print", json!({"text": text, "level": level}));
        }
    }

    /// Answer a sentence typed into the ~ console (`plan <text>`) through the intent resolver, into
    /// the same console with echo: the reading, the first candidates, and for a build its settings.
    fn answer_plan(&self, n: i64, text: &str) {
        let say = |msg: &str| {
            let line = format!("[prismatic-plan] {n} {}", head(&msg.replace('"', "'"), 240));
            let _ = self.console(&format!("echo \"{line}\""));
            self.to_owner_console(&line, "plan");
        };
        let exe = sibling("mcp-intent");
        if !exe.exists() {
            return say("the intent resolver is not built (cargo build --release in tools/research/mcp-rhai)");
        }
        let mut cmd = Command::new(&exe);
        cmd.arg("--panel").arg(self.panel_url());
        let mut params = Map::new();
        if let Some(ml) = self.max_loan() {
            params.insert("budget".into(), json!(ml));
            cmd.arg("--params").arg(Value::Object(params.clone()).to_string());
        }
        cmd.arg(text);
        let Ok(r) = run(&mut cmd, None, Duration::from_secs(180), None) else { return };
        if r.code != 0 {
            return say(&format!("not understood: {}", head(r.stderr.trim(), 200)));
        }
        let Ok(out) = serde_json::from_str::<Value>(&r.stdout) else { return };
        let (i, a) = (&out["intent"], &out["answer"]);
        let names: Vec<String> = i["subjects"].as_array().into_iter().flatten().map(|x| py_str(x.get("name"))).collect();
        let ordering = if truthy(i.get("ordering")) { py_str(i.get("ordering")) } else { String::new() };
        let subjects = if names.is_empty() { "anything".to_string() } else { names.join(", ") };
        say(&format!("{} {ordering} {subjects} ({}): {}", py_str(i.get("action")), py_str(i.get("language")), py_str(a.get("how"))));
        let f0 = |v: &Value| format!("{:.0}", v.as_f64().unwrap_or(0.0));
        for (k, c) in a["candidates"].as_array().into_iter().flatten().take(3).enumerate() {
            let k = k + 1;
            if c.get("from").is_some() {
                say(&format!("{k}. {} ({}/month) -> {}, {}, {} tiles", py_str(c["from"].get("name")), f0(&c["from"]["production"]),
                             py_str(c["to"].get("name")), py_str(c.get("cargo")), f0(&c["distance"])));
            } else if c.get("production").is_some() {
                say(&format!("{k}. {} at ({},{}), {}/month", py_str(c.get("name")), py_str(c.get("x")), py_str(c.get("y")), f0(&c["production"])));
            } else {
                let pick = |a: &str, b: &str| -> Value {
                    [a, b].iter().map(|k| c.get(*k).cloned().unwrap_or(Value::Null)).find(|v| truthy(Some(v))).unwrap_or(json!({}))
                };
                let (f, t) = (pick("a", "producer"), pick("b", "acceptor"));
                let cargo = if truthy(c.get("cargo")) { py_str(c.get("cargo")) } else { "passengers".into() };
                say(&format!("{k}. {cargo} by {}: {} -> {}, score {}", py_str(c.get("mode")), py_str(f.get("name")), py_str(t.get("name")),
                             py_str(c.get("score"))));
            }
        }
        if truthy(a.get("build")) {
            say(&format!("build: settings {}; {}", py_json(&a["build"]["settings"]), py_str(a["build"].get("execution"))));
        }
        let count = a["candidates"].as_array().map_or(0, Vec::len);
        self.event("intent", &format!("~ plan #{n} {}: {count} candidate(s)", py_repr(&json!(text))));
        self.plan("plan.intent", intent_event(text, &format!("console ~ plan #{n}"), &params, &out));
    }

    fn bring_up(self: Arc<Self>) {
        // The console reads stdin only once the game loop runs; repeat until it answers.
        for _ in 0..90 {
            if !self.server_running() {
                return;
            }
            let _ = self.console(&format!("prismatic_mcp start {}", self.args.mcp_port));
            thread::sleep(Duration::from_secs(2));
            if self.token.lock().unwrap().is_some() || self.log_has("already running") {
                break;
            }
        }
        if self.token.lock().unwrap().is_none() {
            self.event("mcp", "the MCP server did not start; see the console log");
            return;
        }
        self.mcp_up.store(true, Ordering::SeqCst);
        self.event("mcp", &format!("MCP server up at {} (token held by the panel)", self.endpoint()));
        // The script budget is set in openttd.cfg before the world is made (SCRIPT_OPCODES): the setting
        // is NewgameOnly, so the console's `setting` here was refused and every game so far ran at the
        // default 10000. Read it back so the log says what the bots really get.
        let mark = self.log_seq();
        if self.console("setting script.script_max_opcode_till_suspend").is_ok() {
            thread::sleep(Duration::from_millis(600));
            if let Some(line) = self.log_after(mark).into_iter().find(|l| l.contains("script_max_opcode_till_suspend")) {
                self.event("server", &format!("script budget: {}", line.trim()));
            }
        }
        // A start every run repeats (PRISMATIC_START_PAUSED=N, set by a duel): the game stays paused
        // while the planner plans and the bots start, and runs once N AI companies exist. Planning
        // takes minutes of wall clock, so without it the planned bot and its rival started at game
        // ticks that depended on the machine's load, and two runs of one bot ended 10% apart, the same
        // rival 52% (knowledge duels-run-side-by-side-are-noisy). The server console's start_ai founds
        // a company while paused; its script runs from the unpause on.
        let hold = std::env::var("PRISMATIC_START_PAUSED").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|n| *n > 0);
        if let Some(n) = hold {
            let _ = self.console("pause");
            // `planner:` lines of kind ai are what a duel result keeps (mcp-duel), so the result says how it started.
            self.event("ai", &format!("planner: game paused for a repeatable start; it runs once {n} AI company(ies) exist"));
            let g = Arc::clone(&self);
            thread::spawn(move || {
                let ai_companies = |g: &Game| g.resource("openttd://companies").ok()
                    .and_then(|c| c.pointer("/data/companies").and_then(Value::as_array).map(|a| a.iter().filter(|x| truthy(x.get("is_ai"))).count()))
                    .unwrap_or(0);
                let started = Instant::now();
                while ai_companies(&g) < n && started.elapsed() < Duration::from_secs(1800) {
                    thread::sleep(Duration::from_millis(500));
                }
                let tick = g.resource("openttd://game/state").ok().and_then(|s| s.get("game_tick").cloned()).unwrap_or(Value::Null);
                let _ = g.console("unpause");
                g.event("ai", &format!("planner: game unpaused at tick {tick} with {} AI company(ies) after {} s", ai_companies(&g), started.elapsed().as_secs()));
            });
        }
        if let Some(file) = &self.args.load {
            // A loaded game brings its AI companies, each restarted by the game from its saved data.
            self.event("ai", &format!("loaded {file}: its AI companies play on; no new bot is started"));
        } else if !self.args.ai.is_empty() {
            // The bot starts with what the planner's portfolio chose (freight rail, passenger rail, air),
            // so its routes come from the one profitability model; without a plan it plays as before.
            let settings = panel_bot::planned_air(&self);
            let (_, flat) = panel_bot::settings_json(&settings);
            let _ = self.console(format!("start_ai {} {flat}", self.args.ai).trim());
            self.ai_started.store(true, Ordering::SeqCst);
            self.event("ai", &format!("start_ai {} {flat} sent: the company plays on its own", self.args.ai).replace("  ", " "));
        }
    }

    /// The most a new company can borrow (difficulty.max_loan), read from the console; None if unknown.
    pub fn max_loan(&self) -> Option<i64> {
        let mark = self.log_seq();
        self.console("setting difficulty.max_loan").ok()?;
        thread::sleep(Duration::from_millis(600));
        self.log_after(mark).iter().find_map(|l| max_loan_of(l))
    }

    pub fn stop(&self) {
        if self.server_running() {
            let _ = self.console("quit");
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let done = self.child.lock().unwrap().as_mut().map_or(true, |c| matches!(c.try_wait(), Ok(Some(_))));
                if done || !self.server_running() { break; }
                if Instant::now() >= deadline {
                    if let Some(c) = self.child.lock().unwrap().as_mut() { let _ = c.kill(); }
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
        let _ = std::fs::remove_dir_all(&self.tmp);
    }

    // ------------------------------------------------------------------ MCP
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.args.mcp_port)
    }

    pub fn panel_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.args.panel_port)
    }

    /// Raw MCP JSON-RPC with the panel's token: the whole answer; an HTTP error status as
    /// `{http_error, body}`.
    pub fn rpc(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let token = self.token.lock().unwrap().clone().ok_or("MCP server not up yet")?;
        let id = self.rpc_id.fetch_add(1, Ordering::SeqCst) + 1;
        let mut body = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if let Some(p) = params {
            body["params"] = p;
        }
        let mut attempt = 0;
        loop {
            let r = ureq::post(&self.endpoint()).timeout(Duration::from_secs(5))
                .set("Content-Type", "application/json").set("Accept", "application/json, text/event-stream")
                .set("Authorization", &format!("Bearer {token}"))
                .send_string(&body.to_string());
            match r {
                Ok(resp) => {
                    let text = resp.into_string().map_err(|e| e.to_string())?;
                    return serde_json::from_str(if text.is_empty() { "{}" } else { &text }).map_err(|e| e.to_string());
                }
                Err(ureq::Error::Status(code, resp)) => return Ok(json!({"http_error": code, "body": resp.into_string().unwrap_or_default()})),
                Err(e) if rpc_retryable(method, e.kind(), attempt) => {
                    attempt += 1;
                    thread::sleep(Duration::from_millis(150));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    pub fn resource(&self, uri: &str) -> Result<Value, String> {
        let r = self.rpc("resources/read", Some(json!({"uri": uri})))?;
        Ok(content_json(&r, "/result/contents/0/text").unwrap_or_else(|| json!({"error": r})))
    }

    pub fn tool(&self, name: &str, arguments: Value) -> Result<Value, String> {
        let r = self.rpc("tools/call", Some(json!({"name": name, "arguments": arguments})))?;
        let Some(out) = content_json(&r, "/result/content/0/text") else { return Ok(json!({"error": r})) };
        // A request that waits for the operator is announced once; the queue it waits in is the game
        // server's (openttd://approvals), which the panel reads and never copies (ADR 0038). A request
        // the server refuses (its queue is full) is an event too, never only a server log line.
        match out.get("status").and_then(Value::as_str) {
            Some("pending_approval") => {
                if let Some(aid) = out.get("approval_id").filter(|v| !v.is_null()).and_then(|v| py_int(v).ok()) {
                    if self.announced.lock().unwrap().insert(aid) {
                        self.event("mcp", &format!("{name}({}) -> pending approval #{aid}", py_json(&arguments)));
                    }
                }
            }
            Some("refused") => self.event("mcp", &format!("{name}({}) -> refused: {}", py_json(&arguments), py_str(out.get("error")))),
            _ => {}
        }
        Ok(out)
    }

    pub fn tool_names(&self) -> HashSet<String> {
        self.rpc("tools/list", None).ok().and_then(|r| r.pointer("/result/tools").and_then(Value::as_array).cloned())
            .unwrap_or_default().iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect()
    }

    // ------------------------------------------------------------------ the owner's view (GUI client)
    pub fn view_token(&self) -> Option<String> {
        self.view_token.lock().unwrap().clone()
    }

    /// A tool call on the GUI client's own MCP server, from its token file (reread each call, so a
    /// restarted client is picked up).
    pub fn view_call(&self, tool: &str, arguments: Value) -> Result<Value, String> {
        let file = self.view_token().ok_or(NO_CLIENT)?;
        let (endpoint, token) = token_file(&file)?;
        let out = mcp_post(&endpoint, &token, "tools/call", json!({"name": tool, "arguments": or_empty(&arguments)}),
                           Duration::from_secs(10))?;
        Ok(content_json(&out, "/result/content/0/text").unwrap_or(out))
    }

    /// The game server's tools/list, cached for a few seconds.
    pub fn server_tools(&self) -> Vec<Value> {
        let mut c = self.server_tools.lock().unwrap();
        if c.1.map_or(true, |t| t.elapsed() > Duration::from_secs(5)) && self.mcp_up.load(Ordering::SeqCst) {
            if let Ok(r) = self.rpc("tools/list", None) {
                c.0 = r.pointer("/result/tools").and_then(Value::as_array).cloned().unwrap_or_default();
                c.1 = Some(Instant::now());
            }
        }
        c.0.clone()
    }

    /// The attached GUI client's tools/list, cached for a few seconds; [] without a client.
    pub fn client_tools(&self) -> Vec<Value> {
        let Some(file) = self.view_token() else { return vec![] };
        let mut c = self.client_tools.lock().unwrap();
        if c.1.map_or(true, |t| t.elapsed() > Duration::from_secs(5)) {
            // The client restarts: keep the last list.
            if let Ok(r) = token_file(&file).and_then(|(e, t)| mcp_post(&e, &t, "tools/list", json!({}), Duration::from_secs(3))) {
                c.0 = r.pointer("/result/tools").and_then(Value::as_array).cloned().unwrap_or_default();
                c.1 = Some(Instant::now());
            }
        }
        c.0.clone()
    }

    // ------------------------------------------------------------------ approvals
    pub fn set_pause(&self, pause: bool) -> Result<Value, String> {
        let out = self.tool("openttd.game.set_pause", json!({"pause": pause}))?;
        if out.get("status").and_then(Value::as_str) != Some("pending_approval") {
            self.event("mcp", &format!("set_pause({}) -> {} {}", if pause { "True" } else { "False" }, py_str(out.get("status")),
                                       out.get("command_result").map_or(String::new(), |v| py_str(Some(v)))));
        }
        Ok(out)
    }

    /// The game server's approval queue (openttd://approvals, ADR 0038): every approval it holds,
    /// pending first; [] while the server does not answer.
    /// Whether the game server's approval core keeps standing rules (its queue answers `rules`): it
    /// then takes `--via` and `--nonce` on its lines, and an older core does not.
    pub fn keeps_rules(&self) -> bool {
        self.queue_data()["rules"].is_array()
    }

    /// The server's whole queue resource (openttd://approvals data): its approvals and, from the
    /// approval-rules core, its standing rules. Null when the MCP is down or the read fails.
    pub fn queue_data(&self) -> Value {
        #[cfg(test)]
        if !self.test_approvals.lock().unwrap().is_empty() || self.test_rules.lock().unwrap().is_some() {
            let mut d = json!({"approvals": self.test_approvals.lock().unwrap().clone(), "capacity": 64});
            if let Some(r) = self.test_rules.lock().unwrap().clone() { d["rules"] = json!(r); }
            return d;
        }
        if !self.mcp_up.load(Ordering::SeqCst) { return Value::Null; }
        self.resource("openttd://approvals").ok().and_then(|r| r.get("data").cloned()).unwrap_or(Value::Null)
    }

    pub fn approvals(&self) -> Vec<Value> {
        #[cfg(test)]
        if !self.test_approvals.lock().unwrap().is_empty() { return self.test_approvals.lock().unwrap().clone(); }
        if !self.mcp_up.load(Ordering::SeqCst) { return vec![]; }
        self.resource("openttd://approvals").ok()
            .and_then(|r| r.pointer("/data/approvals").and_then(Value::as_array).cloned()).unwrap_or_default()
    }

    /// Approve `id` through the game's console, the operator's channel. The game server carries it
    /// out the moment it is approved (ADR 0038): nothing is resubmitted; the panel reads back the
    /// decision the server recorded for it. `by` is the channel it came through (`api`: POST
    /// /api/approve, with the approver it declares, if any; `auto-approve`: the panel's switch, no
    /// person), recorded so every directive step can say how its approval was given
    /// (research/agent-loop.md, Authorize). POST /api/approve checks the owner secret before it gets
    /// here ([`owner_gate`]); the declared name is still not checked. An owner-only tool
    /// ([`OWNER_ONLY_TOOLS`]) is never approved here, whoever asks: the answer names the console
    /// command to type, and the request stays pending.
    pub fn approve(&self, id: i64, by: Value) -> Result<Value, String> {
        if let Some(refusal) = self.owner_only_pending(id) {
            self.event("operator", &format!("refused to approve #{id} ({}): the owner approves it at the game's console",
                                             py_str(refusal.get("tool"))));
            return Ok(refusal);
        }
        let Some(a) = self.approvals().into_iter().find(|a| a["id"] == json!(id)) else {
            return Ok(json!({"error": format!("the game server holds no approval #{id}")}));
        };
        if a["state"] != json!("pending") || a["expired"] == json!(true) {
            let st = if a["expired"] == json!(true) { "expired".to_string() } else { py_str(a.get("state")) };
            return Ok(json!({"error": format!("approval #{id} is {st}, not pending"), "approval": a}));
        }
        panel_decision::record_approval(id, by.clone());
        let via = self.keeps_rules().then(|| via_of(&by));
        self.console(&decide_line(true, &[id], via, None))?;
        self.event("operator", &format!("approved #{id} in the OpenTTD console ({})", panel_decision::approval_label(&by)));
        // The decision the server recorded when it carried the approval out.
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(150));
            let decisions = self.resource("openttd://decisions").ok()
                .and_then(|r| r.pointer("/data/decisions").and_then(Value::as_array).cloned()).unwrap_or_default();
            if let Some(d) = decisions.into_iter().find(|d| d["approval_id"] == json!(id)) {
                let mut out = d.clone();
                let rejected = d["command_result"] == json!("rejected");
                out["status"] = json!(if rejected { "rejected" } else { "executed" });
                self.event("mcp", &format!("#{id} {} -> {} {} verdict={}", py_str(a.get("tool")), py_str(out.get("status")),
                    py_str(d.get("command_result")), py_str(d.get("verdict"))));
                return Ok(out);
            }
        }
        Ok(json!({"error": format!("approved #{id} in the console, but the game server recorded no decision for it within 3 s"), "approval_id": id, "pending_decision": true}))
    }

    /// Deny `id` through the game's console, as the owner would type it: the game server's one
    /// approval core withdraws it (ADR 0038) and enacts nothing, so no owner secret is needed. The
    /// panel reads the server's queue back for the approval's new state; it keeps no state of its
    /// own. 404 when the server holds no such approval, 409 when it is not pending, 504 when the
    /// server did not mark it denied within 3 s.
    pub fn deny(&self, id: i64) -> Result<Value, String> {
        let Some(a) = self.approvals().into_iter().find(|a| a["id"] == json!(id)) else {
            return Ok(json!({"error": format!("the game server holds no approval #{id}")}));
        };
        if a["state"] != json!("pending") || a["expired"] == json!(true) {
            let st = if a["expired"] == json!(true) { "expired".to_string() } else { py_str(a.get("state")) };
            return Ok(json!({"error": format!("approval #{id} is {st}, not pending"), "approval": a}));
        }
        self.console(&decide_line(false, &[id], self.keeps_rules().then_some("panel"), None))?;
        self.event("operator", &format!("denied #{id} in the OpenTTD console"));
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(150));
            if let Some(now) = self.approvals().into_iter().find(|a| a["id"] == json!(id)) {
                if now["state"] == json!("denied") {
                    return Ok(json!({"status": "denied", "approval_id": id, "approval": now}));
                }
            }
        }
        Ok(json!({"error": format!("denied #{id} in the console, but the game server did not mark it denied within 3 s"), "approval_id": id, "pending_decision": true}))
    }

    /// The approvals that wait for the operator, as /api/status and the scripts read them
    /// (`status().pending`): {id, tool, args, created_tick, expires_tick}, from the server's queue.
    pub fn pending_json(&self) -> Vec<Value> {
        self.approvals().into_iter().filter(|a| a["state"] == json!("pending") && a["expired"] != json!(true))
            .map(|a| json!({"id": a["id"], "tool": a["tool"], "args": a["args"], "created_tick": a["created_tick"], "expires_tick": a["expires_tick"]}))
            .collect()
    }

    /// The refusal for pending approval `id` when its tool is owner-only, else None.
    fn owner_only_pending(&self, id: i64) -> Option<Value> {
        let tool = self.pending_json().into_iter().find(|p| p["id"] == json!(id))
            .and_then(|p| p["tool"].as_str().map(str::to_string))?;
        owner_only(&tool).then(|| owner_only_refusal(id, &tool))
    }

    /// One pass of auto-approve: every pending approval except the owner-only ones, which wait for
    /// the console however auto-approve was switched on. Returns the ids it approved (or tried to).
    fn auto_approve_once(&self) -> Vec<i64> {
        if !self.auto_approve.load(Ordering::SeqCst) { return vec![]; }
        // Only the panel's own requests (announced), never another MCP client's, and never an
        // owner-only one.
        let announced = self.announced.lock().unwrap().clone();
        let ids: Vec<i64> = self.pending_json().iter()
            .filter(|p| !p["tool"].as_str().is_some_and(owner_only))
            .filter_map(|p| p["id"].as_i64()).filter(|id| announced.contains(id)).collect();
        for &id in &ids {
            let _ = self.approve(id, json!({"channel": "auto-approve"}));
        }
        ids
    }

    fn auto_loop(self: Arc<Self>) {
        loop {
            thread::sleep(Duration::from_secs(1));
            self.auto_approve_once();
        }
    }

    pub fn event(&self, kind: &str, text: &str) {
        {
            let mut ev = self.events.lock().unwrap();
            ev.push_front(json!({"t": strftime_local("%H:%M:%S"), "kind": kind, "text": text}));
            ev.truncate(200);
        }
        self.bus.publish(&format!("panel.{kind}"), json!({"text": text}));
    }

    /// The token a script sent by `who` holds: the run token when `who` may change the game, else
    /// none (its console lines then read and chat only).
    pub fn script_token(&self, who: &Who) -> Option<String> {
        may_change(who).then(|| self.run_token.clone().ok()).flatten()
    }

    /// Publish one change of who may decide what (`governance.changed`, read by the digest): the
    /// setting, its old and new value, how it changed, and who changed it ([`Who`]: the credential's
    /// kind, never the credential), at the latest known game clock.
    pub fn governance(&self, setting: &str, from: Value, to: Value, via: &str, who: &Who) -> u64 {
        let clock = self.latest_sample().unwrap_or(Value::Null);
        self.bus.publish("governance.changed", json!({
            "setting": setting, "from": from, "to": to, "via": via, "credential": who.credential,
            "role": who.role, "origin": who.origin, "user_agent": who.user_agent, "peer": who.peer,
            "game_date": clock.get("date").cloned().unwrap_or(Value::Null),
            "game_tick": clock.get("t").cloned().unwrap_or(Value::Null)}))
    }

    /// The governance the panel starts with, published once: auto-approve on by `--auto-approve`, and
    /// the approval deadline the game server reads from [`DEADLINE_ENV`] (`deadline`).
    pub fn start_governance(&self, deadline: Option<&str>) {
        if self.args.auto_approve {
            self.governance("auto_approve", json!(false), json!(true), "--auto-approve at start", &Who::none());
        }
        if let Some(d) = deadline.map(str::trim).filter(|d| !d.is_empty()) {
            self.governance("approval_deadline", Value::Null, json!(d), &format!("{DEADLINE_ENV} at start"), &Who::none());
        }
    }

    /// Publish one planning decision as data (ADR 0034 point 3, I0081): a plan.* kind of the declared
    /// table (src/panel_events.rs) with {decision, version, inputs, choice}, beside the decision's
    /// panel.* sentence. A kind outside the table is refused, and the refusal is a panel.server line.
    pub fn plan(&self, kind: &str, data: Value) -> Option<u64> {
        if !kind.starts_with("plan.") || !panel_events::declared(kind) {
            self.event("server", &format!("refused to publish {kind}: not a plan.* kind of src/panel_events.rs KINDS"));
            return None;
        }
        let mirrored = data.clone();
        let seq = self.bus.publish(kind, data);
        // RESEARCH-ONLY, opt-in: the directive's decision and outcome as typed mesh records (src/panel_mesh.rs).
        for line in crate::panel_mesh::take_lines() { self.event("mesh", &line); }
        crate::panel_mesh::mirror(kind, seq, &mirrored, &format!("panel-{}", self.args.panel_port));
        Some(seq)
    }

    pub fn events_json(&self) -> Vec<Value> {
        for line in crate::panel_mesh::take_lines() { self.event("mesh", &line); }
        self.events.lock().unwrap().iter().cloned().collect()
    }

    /// The game window's questions and answers. Every answer not yet reported is published as
    /// ask.answered here, so a reader of GET /api/ask and the watcher see the same events.
    pub fn ask_answers(&self) -> Result<Value, String> {
        let out = self.view_call("openttd.view.ask.answers", json!({}))?;
        let fresh: Vec<Value> = {
            let mut seen = self.asks_reported.lock().unwrap();
            out["asks"].as_array().into_iter().flatten()
                .filter(|a| a.get("answer").map_or(false, |v| !v.is_null()))
                .filter(|a| seen.insert(format!("{}\u{1f}{}", a["id"], a.get("question").unwrap_or(&Value::Null))))
                .cloned().collect()
        };
        for a in fresh {
            self.bus.publish("ask.answered", a);
        }
        Ok(out)
    }

    /// The news the owner's window shows, as events news.shown: what the game reports to the player
    /// (new companies, first arrivals, subsidies, ...) and what any MCP caller posted to them.
    fn watch_news(self: Arc<Self>) {
        let mut seen: Option<HashSet<String>> = None;
        loop {
            thread::sleep(Duration::from_secs(2));
            if self.view_token().is_none() {
                continue;
            }
            // The client restarts: try again next round.
            let Ok(out) = self.view_call("openttd.view.news.list", json!({"limit": 30})) else { continue };
            let news = out["news"].as_array().cloned().unwrap_or_default();
            let _ = self.ask_answers();
            let key = |n: &Value| py_json(&json!([n["date"], n["type"], n["text"]]));
            match seen.as_mut() {
                // What was there before the panel attached is history, not news.
                None => seen = Some(news.iter().map(key).collect()),
                Some(s) => {
                    for n in news.iter().rev() {
                        if s.insert(key(n)) {
                            self.bus.publish("news.shown", n.clone());
                        }
                    }
                }
            }
        }
    }

    /// Derive game events from what MCP reports, by difference between two reads.
    fn watch(self: Arc<Self>) {
        let mut prev: Option<Snapshot> = None;
        loop {
            thread::sleep(Duration::from_secs(1));
            if !self.mcp_up.load(Ordering::SeqCst) {
                continue;
            }
            let (Ok(st), Ok(cs), Ok(ds)) = (self.resource("openttd://game/state"), self.resource("openttd://companies"),
                                            self.resource("openttd://decisions")) else { continue };
            let Some(cur) = Snapshot::of(&st, &cs, &ds) else { continue };
            for (kind, data) in derive_game_events(prev.as_ref(), &cur) {
                // A decision record may settle a directive that waited (research/agent-loop.md step 2).
                let settles = kind.starts_with("decision.").then(|| data.clone());
                self.bus.publish(kind, data);
                for outcome in panel_decision::on_decision(settles.as_ref(), Instant::now()) {
                    self.plan("plan.directive_outcome", outcome);
                }
            }
            for outcome in panel_decision::on_decision(None, Instant::now()) {
                self.plan("plan.directive_outcome", outcome);
            }
            if let Some(sample) = sample_of(&st, &cs) { self.record_sample(sample); }
            prev = Some(cur);
        }
    }
}

/// Opcodes a script may run per tick before it is suspended: the game's maximum for
/// script.script_max_opcode_till_suspend (default 10000). The bot plans rail paths in Squirrel; at the
/// default a 240-tile search ate months of game time, and with eight lines handed over the bot built
/// one line every two to three months while its money waited (research/results duel-20261117T100202).
/// Scripts run on the server only, so this changes nothing clients compute.
pub const SCRIPT_OPCODES: u32 = 250000;

impl Game {
    /// Keeps one sample per game day (a later read of the same day replaces it), at most
    /// `HISTORY_MAX`; a tick lower than the last one is a new world, and the history starts again.
    /// A new game day is also published as game.sample (the same object /api/overview/history
    /// answers), once per day: the bus tap fills the store's `samples` from it and ZeroMQ subscribers
    /// see the daily history like every other event (ADR 0033, I0080).
    fn record_sample(&self, sample: Value) {
        let new_day = {
            let mut h = self.history.lock().unwrap();
            let new_day = h.back().map_or(true, |last| last["date"] != sample["date"]);
            push_sample(&mut h, sample.clone());
            new_day
        };
        if new_day {
            self.bus.publish("game.sample", sample);
        }
    }

    /// The newest overview sample (the latest known game clock), None before the first.
    pub fn latest_sample(&self) -> Option<Value> {
        self.history.lock().unwrap().back().cloned()
    }

    /// The samples with a tick above `since`.
    pub fn history_since(&self, since: f64) -> Vec<Value> {
        self.history.lock().unwrap().iter().filter(|s| s["t"].as_f64().unwrap_or(0.0) > since).cloned().collect()
    }
}

pub const HISTORY_MAX: usize = 8000;

pub fn push_sample(h: &mut VecDeque<Value>, sample: Value) {
    let tick = sample["t"].as_f64().unwrap_or(0.0);
    if let Some(last) = h.back() {
        if last["t"].as_f64().unwrap_or(0.0) > tick { h.clear(); }
        else if last["date"] == sample["date"] { h.pop_back(); }
    }
    h.push_back(sample);
    while h.len() > HISTORY_MAX { h.pop_front(); }
}

/// The fleet as a player counts it: openttd://game/state's `fleet.total` (primary vehicles), and
/// only from a server older than that field the pool count `num_vehicles`, which also counts every
/// wagon, articulated part and effect (smoke, sparks) and so swings by dozens a day.
pub fn fleet_total(data: &Value) -> Value {
    data.pointer("/fleet/total").cloned().unwrap_or_else(|| data.get("num_vehicles").cloned().unwrap_or(Value::Null))
}

/// One overview sample from openttd://game/state and openttd://companies: tick, date, counts, the
/// fleet by type, and per company money, value, loan, net (money − loan) and fleet; None without
/// state data.
pub fn sample_of(st: &Value, cs: &Value) -> Option<Value> {
    let d = st.get("data")?;
    let companies: Vec<Value> = cs.pointer("/data/companies").and_then(Value::as_array).cloned().unwrap_or_default()
        .into_iter().map(|c| {
            let n = |k: &str| c.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            json!({"id": c.get("id"), "name": c.get("name"), "is_ai": c.get("is_ai"), "money": n("money"),
                   "value": n("company_value"), "loan": n("loan"), "net": n("money") - n("loan"),
                   "fleet": c.pointer("/fleet/total").cloned().unwrap_or(Value::Null)})
        }).collect();
    Some(json!({"t": st.get("game_tick"), "date": st.get("game_date"), "vehicles": fleet_total(d),
                "fleet": d.get("fleet").cloned().unwrap_or(Value::Null),
                "towns": d.get("num_towns"), "paused": d.get("paused"), "companies": companies}))
}

fn h_history(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let since = i.query.get("since").and_then(|v| v.parse::<f64>().ok()).unwrap_or(-1.0);
    let samples = g.history_since(since);
    // The newest tick kept: a client whose cursor is past it saw an older world, and starts again.
    let last = g.history.lock().unwrap().back().map(|s| s["t"].clone()).unwrap_or(Value::Null);
    Ok((200, json!({"since": since, "last": last, "count": samples.len(), "samples": samples})))
}

/// What the watcher compares between two reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    date: Value,
    paused: Value,
    vehicles: Value,
    towns: Value,
    tick: Value,
    companies: BTreeMap<String, (Value, Value)>,
    decisions: BTreeMap<String, Value>,
}

/// A JSON object key as json.dumps writes a dict key: a string as it is, anything else as JSON.
fn id_key(v: &Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())
}

impl Snapshot {
    /// From openttd://game/state, openttd://companies and openttd://decisions; None without state data.
    pub fn of(st: &Value, cs: &Value, ds: &Value) -> Option<Snapshot> {
        let data = st.get("data")?;
        let list = |v: &Value, k: &str| v.get("data").and_then(|d| d.get(k)).and_then(Value::as_array).cloned().unwrap_or_default();
        let mut companies = BTreeMap::new();
        for c in list(cs, "companies") {
            if let Some(id) = c.get("id") { companies.insert(id_key(id), (id.clone(), c.clone())); }
        }
        let mut decisions = BTreeMap::new();
        for d in list(ds, "decisions") {
            if let Some(id) = d.get("decision_id") { decisions.insert(id_key(id), d.clone()); }
        }
        let g = |v: &Value, k: &str| v.get(k).cloned().unwrap_or(Value::Null);
        Some(Snapshot { date: g(st, "game_date"), paused: g(data, "paused"), vehicles: fleet_total(data),
                        towns: g(data, "num_towns"), tick: g(st, "game_tick"), companies, decisions })
    }
}

/// The events between two reads of the game (the first read announces what it observes).
pub fn derive_game_events(prev: Option<&Snapshot>, cur: &Snapshot) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    let Some(prev) = prev else {
        out.push(("game.observed", json!({"date": cur.date, "tick": cur.tick, "vehicles": cur.vehicles,
                                          "companies": cur.companies.len(), "towns": cur.towns})));
        for (_, c) in cur.companies.values() {
            out.push(("company.observed", c.clone()));
        }
        return out;
    };
    if let (Some(a), Some(b)) = (cur.tick.as_f64(), prev.tick.as_f64()) {
        if a < b {
            out.push(("game.new_world", json!({"date": cur.date, "towns": cur.towns})));
        }
    }
    let month = |d: &Value| head(d.as_str().unwrap_or(""), 7);
    if month(&cur.date) != month(&prev.date) {
        let companies: Map<String, Value> = cur.companies.iter()
            .map(|(k, (_, c))| (k.clone(), json!({"money": c.get("money"), "value": c.get("company_value")}))).collect();
        out.push(("game.month", json!({"date": cur.date, "vehicles": cur.vehicles, "companies": companies})));
    }
    if cur.paused != prev.paused {
        out.push((if truthy(Some(&cur.paused)) { "game.paused" } else { "game.resumed" }, json!({"date": cur.date, "tick": cur.tick})));
    }
    if cur.vehicles != prev.vehicles {
        out.push(("vehicles.count", json!({"from": prev.vehicles, "to": cur.vehicles, "date": cur.date})));
    }
    for (k, (_, c)) in &cur.companies {
        if !prev.companies.contains_key(k) { out.push(("company.founded", c.clone())); }
    }
    for (k, (_, c)) in &prev.companies {
        if !cur.companies.contains_key(k) { out.push(("company.removed", c.clone())); }
    }
    for (k, (id, b)) in &cur.companies {
        if let Some((_, a)) = prev.companies.get(k) {
            if a.get("name") != b.get("name") && truthy(b.get("name")) {
                out.push(("company.renamed", json!({"id": id, "from": a.get("name"), "to": b.get("name")})));
            }
        }
    }
    for (k, d) in &cur.decisions {
        match prev.decisions.get(k) {
            None => out.push(("decision.recorded", d.clone())),
            Some(p) if p != d => out.push(("decision.updated", d.clone())),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------- discovery (cluster)
// Every panel and every GUI client it attaches registers here; any panel lists the whole cluster and
// checks each entry with a live probe. One JSON file per instance (mcp-instance/v1), written
// atomically, 0600, removed when its panel stops; an entry whose owning process is gone is dropped.

pub fn registry() -> PathBuf {
    match std::env::var_os("PRISMATIC_MCP_REGISTRY").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state/prismatic-mcp/instances"),
    }
}

/// The record an instance writes for itself, schema mcp-instance/v1.
pub fn registry_record(name: &str, entry: &Value) -> Value {
    let mut m = Map::new();
    m.insert("schema".into(), json!("mcp-instance/v1"));
    m.insert("name".into(), json!(name));
    m.insert("owner_pid".into(), json!(std::process::id()));
    m.insert("registered".into(), json!(unix_now()));
    if let Some(sup) = supervisor() { m.insert("supervisor".into(), sup); }
    if let Some(e) = entry.as_object() { m.extend(e.clone()); }
    Value::Object(m)
}

/// Who supervises this process: `PRISMATIC_MCP_SUPERVISOR`, a JSON object (mcp-e2e sets it to the
/// Majordomus task it was started under), or None when it is unset or not an object.
pub fn supervisor() -> Option<Value> {
    std::env::var("PRISMATIC_MCP_SUPERVISOR").ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok()).filter(Value::is_object)
}

pub fn registry_write(name: &str, entry: &Value) {
    use std::os::unix::fs::PermissionsExt;
    let dir = registry();
    let _ = std::fs::create_dir_all(&dir);
    let tmp = dir.join(format!(".{name}.tmp"));
    if std::fs::write(&tmp, py_json(&registry_record(name, entry))).is_ok() {
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        let _ = std::fs::rename(&tmp, dir.join(format!("{name}.json")));
    }
}

pub fn registry_remove(name: &str) {
    let _ = std::fs::remove_file(registry().join(format!("{name}.json")));
}

fn pid_alive(pid: i64) -> bool {
    // SAFETY: kill with signal 0 only checks that the process exists.
    pid > 0 && pid <= i64::from(i32::MAX) && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
}

/// The panel that holds a server's token: a server a panel started announces no token file (the
/// panel keeps the token in memory), and the panel's record names that server's endpoint as `mcp`.
pub fn owning_panel(records: &[Value], server: &Value) -> Option<String> {
    let endpoint = server.get("endpoint")?.as_str()?;
    records.iter()
        .find(|r| r.get("kind").and_then(Value::as_str) == Some("panel") && r.get("mcp").and_then(Value::as_str) == Some(endpoint))
        .and_then(|r| r.get("url")?.as_str().map(|u| u.trim_end_matches('/').to_string()))
}

/// Every instance in the registry, each checked with a live probe; records of dead owners removed.
/// A server without a token file is reached through the panel that owns it (`via`).
pub fn discover() -> Vec<Value> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(registry()).map(|d| {
        d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json")).collect()
    }).unwrap_or_default();
    files.sort();
    let mut live = Vec::new();
    for f in files {
        let Ok(e) = std::fs::read_to_string(&f).map_err(|_| ()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|_| ())) else { continue };
        if !e.is_object() { continue; }
        if !e.get("owner_pid").and_then(|p| py_int(p).ok()).map_or(false, pid_alive) {
            let _ = std::fs::remove_file(&f); // its panel is gone
            continue;
        }
        live.push(e);
    }
    let all = live.clone();
    let mut out = Vec::new();
    for mut e in live {
        e["alive"] = json!(false);
        let via = if e.get("token_file").is_none() && e.get("kind").and_then(Value::as_str) != Some("panel") {
            owning_panel(&all, &e)
        } else { None };
        let probe = || -> Result<Value, String> {
            if e.get("kind").and_then(Value::as_str) == Some("panel") {
                let url = e["url"].as_str().ok_or("no url in the record")?;
                let st: Value = ureq::get(&format!("{url}/api/status")).timeout(Duration::from_secs(2)).call()
                    .map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
                Ok(json!({"alive": true, "mcp_up": st.get("mcp_up"), "ai": st.get("ai"), "game_port": st.get("game_port")}))
            } else if let Some(url) = &via {
                let st: Value = ureq::get(&format!("{url}/api/status")).timeout(Duration::from_secs(2)).call()
                    .map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
                Ok(json!({"alive": st.get("mcp_up") == Some(&json!(true)), "via": url}))
            } else {
                let file = e["token_file"].as_str().ok_or("no token_file in the record")?;
                let (endpoint, token) = token_file(file)?;
                let alive = mcp_post(&endpoint, &token, "ping", json!({}), Duration::from_secs(2))?.get("result").is_some();
                Ok(json!({"endpoint": endpoint, "alive": alive}))
            }
        };
        match probe() {
            Ok(Value::Object(m)) => { for (k, v) in m { e[k.as_str()] = v; } }
            Ok(_) => {}
            Err(err) => e["error"] = json!(err), // a probe failure is the answer
        }
        out.push(e);
    }
    out
}

/// One MCP JSON-RPC call on a named instance: a panel through its /api/rpc, a server a panel owns
/// through that panel, a client (or a server with its own token file) directly.
pub fn instance_call(name: &str, method: &str, params: &Value) -> Result<(u16, Value), String> {
    let Some(e) = discover().into_iter().find(|x| x["name"] == json!(name)) else {
        return Ok((404, json!({"error": format!("no instance {name}")})));
    };
    let params = or_empty(params);
    let panel = match e.get("kind").and_then(Value::as_str) {
        Some("panel") => Some(e["url"].as_str().ok_or("no url in the record")?.to_string()),
        _ => e.get("via").and_then(Value::as_str).map(str::to_string),
    };
    if let Some(url) = panel {
        let v = ureq::post(&format!("{url}/api/rpc")).timeout(Duration::from_secs(10))
            .set("Content-Type", "application/json")
            .send_string(&json!({"method": method, "params": params}).to_string())
            .map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
        return Ok((200, v));
    }
    let (endpoint, token) = token_file(e["token_file"].as_str().ok_or("no token_file in the record")?)?;
    Ok((200, mcp_post(&endpoint, &token, method, params, Duration::from_secs(10))?))
}

/// WindowClass numbers to names, derived from src/window_type.h (aliases kept as extra names), so the
/// windows openttd.ui.windows reports by number get the names the source gives them.
pub fn window_classes(header: &str) -> Result<Value, String> {
    let body = header.split_once("enum class WindowClass").ok_or("no enum class WindowClass")?.1;
    let body = body.split_once("\n};").map_or(body, |(b, _)| b);
    let mut text = String::new();
    let mut rest = body;
    // Comments out: /* ... */ first, then // to the end of the line.
    while let Some(p) = rest.find("/*") {
        text.push_str(&rest[..p]);
        rest = rest[p + 2..].split_once("*/").map_or("", |(_, r)| r);
    }
    text.push_str(rest);
    let text: String = text.lines().map(|l| l.split_once("//").map_or(l, |(a, _)| a)).collect::<Vec<_>>().join("\n");
    let ident_end = |s: &str| -> Option<String> {
        // The last identifier in s that starts with a capital letter, if it ends s.
        let t = s.trim_end();
        let start = t.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map_or(0, |p| p + 1);
        let id = &t[start..];
        (id.starts_with(|c: char| c.is_ascii_uppercase()) && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_string())
    };
    let mut names: BTreeMap<String, i64> = BTreeMap::new();
    let mut by_value: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    let mut value = -1i64;
    let chunks: Vec<&str> = text.split(',').collect();
    for chunk in &chunks[..chunks.len().saturating_sub(1)] {
        let (lhs, expr) = match chunk.split_once('=') { Some((l, r)) => (l, Some(r.trim())), None => (*chunk, None) };
        let Some(name) = ident_end(lhs) else { continue };
        let v = match expr {
            Some(e) if e.starts_with("WindowClass::") => *names.get(&e["WindowClass::".len()..]).ok_or_else(|| format!("unknown {e}"))?,
            Some(e) => match e.strip_prefix("0x").or_else(|| e.strip_prefix("0X")) {
                Some(h) => i64::from_str_radix(h, 16).map_err(|_| format!("bad value {e}"))?,
                None => e.parse().map_err(|_| format!("bad value {e}"))?,
            },
            None => value + 1,
        };
        value = v;
        names.insert(name.clone(), v);
        by_value.entry(v).or_default().push(name);
    }
    let mut out = Map::new();
    for (v, ns) in by_value {
        let name = if ns.len() > 1 { ns.last() } else { ns.first() }.cloned().unwrap_or_default();
        out.insert(v.to_string(), json!({"name": name, "aliases": ns}));
    }
    Ok(Value::Object(out))
}

// ---------------------------------------------------------------------- the declared routes

/// What a handler gets: the query (GET), the body (POST, `{}` when empty) and who sent it.
pub struct Input {
    pub query: Query,
    pub body: Value,
    pub who: Who,
}

pub type Handler = fn(&Arc<Game>, &Input) -> Result<(u16, Value), String>;

pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    pub op_id: &'static str,
    pub summary: &'static str,
    pub needs_mcp: bool,
    pub schema: Option<Value>,
    pub handler: Handler,
}

fn obj() -> Value {
    json!({"type": "object"})
}

fn s(props: Value, required: &[&str]) -> Option<Value> {
    Some(json!({"type": "object", "properties": props, "required": required}))
}

fn ok(v: Value) -> Result<(u16, Value), String> {
    Ok((200, v))
}

/// The query string as urllib's parse_qs reads it: `+` is a space, percent-escapes decoded, blank
/// values dropped, the first of repeated names wins.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query(pub Vec<(String, String)>);

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 2 < b.len())
            .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        match (b[i], hex) {
            (_, Some(v)) => { out.push(v); i += 2; }
            (b'+', _) => out.push(b' '),
            (c, _) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Query {
    pub fn parse(qs: &str) -> Query {
        Query(qs.split('&').filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            let v = percent_decode(v);
            (!v.is_empty()).then(|| (percent_decode(k), v))
        }).collect())
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn str_or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.get(name).unwrap_or(default)
    }

    /// `int(q.get(name, [default])[0])`.
    pub fn int(&self, name: &str, default: i64) -> Result<i64, String> {
        match self.get(name) {
            None => Ok(default),
            Some(v) => v.trim().parse().map_err(|_| format!("invalid literal for int() with base 10: {}", py_repr(&json!(v)))),
        }
    }
}

/// When the content list was last fetched from the content server (`content update`).
static CONTENT_UPDATED: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// One `content state` line, "10884238, AI, Not selected, AdmiralAI, 4C4D4441, <md5>", as JSON; the
/// name may hold commas, so it is everything between the state and the two trailing ids.
fn content_line(line: &str) -> Option<Value> {
    let text = line.rsplit("] ").next().unwrap_or(line).trim();
    let f: Vec<&str> = text.split(", ").collect();
    if f.len() < 6 { return None; }
    let id: u64 = f[0].trim().parse().ok()?;
    Some(json!({"id": id, "type": f[1], "state": f[2], "name": f[3..f.len() - 2].join(", ")}))
}

/// GET /api/content?q=<text>: what the content server (BaNaNaS) offers whose name matches, from the
/// game's own `content state` (the list is fetched with `content update` at most every 10 minutes).
fn h_content(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let q = i.query.get("q").unwrap_or("").to_string();
    if q.trim().is_empty() || q.contains('\n') || q.contains('"') {
        return Ok((400, json!({"error": "?q= names what to look for, one line, e.g. AdmiralAI or AI"})));
    }
    let stale = CONTENT_UPDATED.lock().unwrap().map_or(true, |t| t.elapsed() > Duration::from_secs(600));
    if stale {
        g.console("content update")?;
        thread::sleep(Duration::from_secs(6));
        *CONTENT_UPDATED.lock().unwrap() = Some(std::time::Instant::now());
    }
    let mark = g.log_seq();
    g.console(&format!("content state {}", q.trim()))?;
    thread::sleep(Duration::from_secs(2));
    let items: Vec<Value> = g.log_after(mark).iter().filter_map(|l| content_line(l)).collect();
    ok(json!({"query": q.trim(), "items": items, "count": items.len()}))
}

/// POST /api/content/download {ids}: the owner's download of those content items (and the libraries
/// they need, which the game selects itself): `content select <id>` for each, then `content
/// download`. Answers the console's output and the AIs the game offers afterwards.
fn h_content_download(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let ids: Vec<u64> = i.body.get("ids").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
    if ids.is_empty() {
        return Ok((400, json!({"error": "ids: the content ids to download, from GET /api/content"})));
    }
    let mark = g.log_seq();
    for id in &ids { g.console(&format!("content select {id}"))?; }
    g.console("content download")?;
    g.event("operator", &format!("content download of {} item(s): {:?}", ids.len(), ids));
    thread::sleep(Duration::from_secs_f64(py_float(i.body.get("wait"), 20.0)?.clamp(1.0, 300.0)));
    let output = g.log_after(mark);
    // What the game offers now: the AI list rescans its folders on start_ai, so the names come from
    // the downloaded tars (content_download/ai) as the console reports them.
    ok(json!({"ids": ids, "output": output}))
}

fn h_console(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let cmd = py_str(i.body.get("command").or(Some(&json!("")))).trim().to_string();
    if cmd.is_empty() || cmd.contains('\n') {
        return Ok((400, json!({"error": "one non-empty line"})));
    }
    if let Some(why) = console_refusal(&cmd) {
        // Approval is the owner's act: the Approve button with the owner secret, or the game's own
        // console, never a console line any caller (an agent included) could send, in any spelling
        // the game accepts. Withdrawing (deny) stays open.
        g.event("operator", &format!("refused console line: {cmd}"));
        return Ok((403, json!({"error": why, "refused": "console_line"})));
    }
    if console_authority(&cmd) == Authority::Owner && !may_change(&i.who) {
        g.event("operator", &format!("refused console line without a credential: {cmd}"));
        return Ok(needs_credential(&format!("the console line `{}`", head(&cmd, 60))));
    }
    let mark = g.log_seq();
    g.console(&cmd)?;
    g.event("console", &cmd);
    if let Some((setting, to)) = console_governance(&cmd) {
        g.governance(setting, Value::Null, to, "POST /api/console", &i.who);
    }
    thread::sleep(Duration::from_secs_f64(py_float(i.body.get("wait"), 0.6)?.clamp(0.0, 600.0)));
    ok(json!({"sent": cmd, "output": g.log_after(mark)}))
}

fn h_rhai(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let script = py_str(i.body.get("script").or(Some(&json!(""))));
    let timeout = py_float(i.body.get("timeout"), 120.0)?.max(0.0);
    let (tx, rx) = mpsc::channel();
    let abort = Arc::new(AtomicBool::new(false));
    let runner = spawn_script_as(g.panel_url(), g.script_token(&i.who), script.clone(), abort.clone(), tx);
    let deadline = Instant::now() + Duration::from_secs_f64(timeout.min(86_400.0));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(ScriptLine::Out(l)) => out.push(l),
            Ok(ScriptLine::Err(l)) => err.push(l),
            Ok(ScriptLine::Exit(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                abort.store(true, Ordering::SeqCst);
                return Err(format!("the Rhai script ran longer than {timeout} seconds and was stopped"));
            }
        }
    }
    let code = runner.join().unwrap_or(1);
    g.event("rhai", &format!("{} -> exit {code}", head(&script, 80)));
    for l in &out { g.to_owner_console(&format!("[rhai] {l}"), "rhai"); }
    for l in &err { g.to_owner_console(&format!("[rhai] {l}"), "error"); }
    let mut text = out.join("\n");
    if !out.is_empty() { text.push('\n'); }
    text.push_str(&err.join("\n"));
    ok(json!({"exit": code, "output": text.trim_end()}))
}

fn h_auto(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let on = truthy(i.body.get("on"));
    let was = g.auto_approve.swap(on, Ordering::SeqCst);
    g.event("operator", &format!("auto-approve {}", if on { "ON" } else { "OFF" }));
    g.governance("auto_approve", json!(was), json!(on), "POST /api/auto-approve", &i.who);
    ok(json!({"auto_approve": on}))
}

/// The owner signs in (POST /api/owner/login {secret}): the right secret sets the page's HttpOnly
/// cookie, the same one ?owner= sets, and answers 204; a wrong one waits [`LOGIN_DELAY`] and answers
/// 401, and a run of them 429 ([`owner_login`]). The secret reaches no event, log or answer.
fn h_owner_login(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let given = py_str(i.body.get("secret"));
    let verdict = owner_login(g.owner_secret.as_deref(), &given, &mut g.owner_failures.lock().unwrap(), Instant::now());
    match verdict {
        Ok(()) => {
            let secret = g.owner_secret.as_deref().unwrap_or_default();
            crate::web_access::set_headers(&format!("{}{}", crate::web_access::headers(), owner_set_cookie(g.args.panel_port, secret)));
            g.event("operator", "the owner signed in");
            g.governance("owner_session", json!(false), json!(true), "POST /api/owner/login", &i.who);
            Ok((204, Value::Null))
        }
        Err((code, why)) => {
            g.event("operator", if code == 429 { "owner sign-in refused: too many wrong secrets" } else { "owner sign-in refused: wrong secret" });
            if code == 401 { thread::sleep(LOGIN_DELAY); }
            Ok((code, why))
        }
    }
}

/// A one-time sign-in link for the owner's browser (POST /api/owner/once, the owner secret's alone):
/// `?once=<nonce>` sets the page's cookie once within a minute and redirects to the page without the
/// query, so a launcher opens the page signed in and the secret is in no URL, history or process list.
fn h_owner_once(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    let n = owner_once(&mut g.owner_nonces.lock().unwrap(), Instant::now()).ok_or("no randomness: /dev/urandom cannot be read")?;
    ok(json!({"once": n, "url": format!("{}/?once={n}", g.panel_url()), "expires_s": OWNER_ONCE_TTL.as_secs()}))
}

/// The owner signs this browser out (POST /api/owner/logout): its cookie is cleared; open to anyone,
/// since it only ever takes the owner's power away from the browser that asks.
fn h_owner_logout(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    crate::web_access::set_headers(&format!("{}{}", crate::web_access::headers(), owner_clear_cookie(g.args.panel_port)));
    g.governance("owner_session", json!(true), json!(false), "POST /api/owner/logout", &i.who);
    Ok((204, Value::Null))
}

/// An AI's settings as `start_ai` takes them (`a=1,b=2`) as the map `openttd.ai.start` takes:
/// Err for a part that is no `name=integer`.
pub fn settings_map(text: &str) -> Result<Value, String> {
    let mut m = serde_json::Map::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').ok_or_else(|| format!("setting `{part}` is not name=value"))?;
        let n: i64 = v.trim().parse().map_err(|_| format!("setting `{part}`: the value is not an integer"))?;
        m.insert(k.trim().to_string(), json!(n));
    }
    Ok(Value::Object(m))
}

fn h_start_ai(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let name = if truthy(i.body.get("ai")) { py_str(i.body.get("ai")) } else { g.args.ai.clone() };
    let settings = py_str(i.body.get("settings").or(Some(&json!("")))).trim().to_string();
    if format!("{name} {settings}").chars().any(char::is_control) {
        return Err("refused a start_ai with a control character: one console line, never two".into());
    }
    // Without a credential a new company is the owner's to approve (the countdown decides when they
    // do not): the game's approval-gated openttd.ai.start, never the console.
    if !may_change(&i.who) {
        if !g.tool_names().contains("openttd.ai.start") {
            return Ok(needs_credential("starting an AI"));
        }
        let out = g.tool("openttd.ai.start", json!({"name": name, "settings": settings_map(&settings)?}))?;
        g.event("ai", &format!("start_ai {name} sent for approval (no credential)"));
        return Ok((rpc_status(&out), out));
    }
    let cmd = format!("start_ai {name} {settings}").trim().to_string();
    g.console(&cmd)?;
    g.ai_started.store(true, Ordering::SeqCst);
    g.event("ai", &cmd);
    ok(json!({"ok": true}))
}

fn h_view_attach(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    // Which window is the owner's view, and so which client the panel drives: the stack's or the owner's to say.
    if !may_change(&i.who) {
        return Ok(needs_credential("attaching a GUI client"));
    }
    let f = PathBuf::from(py_str(i.body.get("token_file").or(Some(&json!("")))));
    if !f.is_file() {
        return Ok((400, json!({"error": format!("no token file at {}", f.display())})));
    }
    *g.view_token.lock().unwrap() = Some(f.display().to_string());
    registry_write(&format!("client-{}", g.args.panel_port), &json!({
        "kind": "client", "role": "GUI client (owner view)", "token_file": f.display().to_string(), "panel": g.panel_url()}));
    g.event("view", &format!("GUI client attached ({})", f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    ok(json!({"attached": true, "state": g.view_call("openttd.view.state", json!({}))?}))
}

fn h_action_plan(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let Some(d) = std::env::var("MCP_ALGOGLYPH_DIR").ok().filter(|d| !d.is_empty()) else {
        return Ok((503, json!({"error": "no AlgoGlyph runner: set MCP_ALGOGLYPH_DIR (mcp-e2e finds one)"})));
    };
    let mut inp = i.body.as_object().cloned().unwrap_or_default();
    // Where cash, loan and the loan limit came from, for plan.action.
    let mut books = books_from_caller(&inp);
    // Cash and loan from the company's newest day in the store (I0086, ADR 0034 point 1), whichever
    // the body left out; the console below fills what the store could not, and the event names which.
    let stored = books_from_store(&mut inp, &mut books, &format!("panel-{}", g.args.panel_port), |q| {
        let (code, v) = panel_store::run_named(q);
        if code == 200 { Ok(v) } else { Err(py_str(v.get("error"))) }
    });
    if !inp.contains_key("cash") || !inp.contains_key("loan") {
        // Money and loan from the console's `companies` line (openttd://companies has no loan on
        // every build): the requested company, else the first AI company.
        let mark = g.log_seq();
        g.console("companies")?;
        thread::sleep(Duration::from_millis(800));
        let rows: Vec<(i64, i64, i64)> = g.log_after(mark).iter().filter_map(|l| company_row(l)).collect();
        let want = inp.get("company").and_then(Value::as_i64);
        let row = rows.iter().find(|r| want.map_or(false, |w| r.0 - 1 == w)).or(rows.first());
        if let Some(r) = row {
            let line = json!({"console": "companies", "company": r.0 - 1});
            for (k, v) in [("cash", r.1), ("loan", r.2)] {
                if !inp.contains_key(k) { inp.insert(k.into(), json!(v)); books[k] = line.clone(); }
            }
        }
    }
    if !inp.contains_key("max_loan") {
        let ml = g.max_loan();
        books["max_loan"] = if ml.is_some() { json!("console: setting difficulty.max_loan") } else { json!("default 300000 (the console gave none)") };
        inp.insert("max_loan".into(), json!(ml.unwrap_or(300000)));
    }
    inp.entry("loan_step").or_insert(json!(10000));
    inp.remove("company");
    let inp = Value::Object(inp);
    // The answer goes to a file, not a pipe: the flow CLI ends with process.exit(), which drops what a
    // pipe has not taken yet (an answer above 64 KiB arrived cut off, 2026-11-16).
    let path = g.tmp.join(format!("actionplan.{}.{:?}.json", std::process::id(), thread::current().id()));
    let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let r = run(Command::new("npm").args(["run", "--silent", "flow", "--", "kernel", "action-plan", "-"]).current_dir(&d),
                Some(inp.to_string().into_bytes()), Duration::from_secs(120), Some(file));
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    let r = r?;
    let Ok(Value::Object(mut out)) = serde_json::from_str::<Value>(&text) else {
        let msg = if r.stderr.is_empty() { text.trim().to_string() } else { r.stderr.trim().to_string() };
        return Ok((502, json!({"error": head(&msg, 400)})));
    };
    out.insert("input".into(), inp.clone());
    out.insert("runner".into(), json!(d));
    let ok_ = truthy(out.get("ok"));
    let n = inp["actions"].as_array().map_or(0, Vec::len);
    g.event("plan", &format!("action plan: {n} action(s) -> {}", if ok_ {
        format!("{} step(s)", out.get("result").and_then(|r| r.get("steps")).and_then(Value::as_array).map_or(0, Vec::len))
    } else {
        format!("refused: {}", py_str(out.get("errors")))
    }));
    g.plan("plan.action", action_event(&i.body, &inp, &books, &out, &stored));
    Ok((if ok_ { 200 } else { 422 }, Value::Object(out)))
}

/// The days of samples an action plan's books are read with: the newest one feeds the kernel, the
/// window travels in plan.action as the trailing books.
pub const ACTION_BOOK_DAYS: usize = 30;

/// Cash and loan of an action plan, whichever `inp` lacks, from the newest stored day of the asked
/// company (else the lowest company id) of this instance's latest run, read with `read` (the
/// panel's own store: `kernel_feed::from_store`). `books` records where each came from. What
/// plan.action names as its stored input: the named query and its rows, or why the store gave
/// nothing, so the console fills the rest (ADR 0034 point 2).
pub fn books_from_store(inp: &mut Map<String, Value>, books: &mut Value, instance: &str,
                        read: impl FnOnce(&crate::store_sql::Named) -> Result<Value, String>) -> panel_events::Stored {
    if inp.contains_key("cash") && inp.contains_key("loan") {
        return panel_events::Stored::Fallback("the caller gave cash and loan".into());
    }
    let company = inp.get("company").and_then(Value::as_i64);
    let (fed_input, fed, w) = kernel_feed::from_store("action-plan", &Value::Object(inp.clone()), company, instance,
                                                      ACTION_BOOK_DAYS, read);
    if let Ok(used) = &fed {
        let day = json!({"store": "samples", "company": used["company"], "game_date": used["game_date"],
                         "game_tick": used["game_tick"]});
        for k in used["took"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            inp.insert(k.into(), fed_input[k].clone());
            books[k] = day.clone();
        }
        books["trailing"] = used["trailing"].clone();
    }
    kernel_feed::stored(&fed, &w)
}

/// Where cash, loan and the loan limit of an action plan come from before the console is read:
/// `caller` for what the body gave, `missing` for the rest until something fills it.
pub fn books_from_caller(body: &Map<String, Value>) -> Value {
    Value::Object(["cash", "loan", "max_loan"].iter()
        .map(|k| (k.to_string(), json!(if body.contains_key(*k) { "caller" } else { "missing" }))).collect())
}

/// The data of plan.action: the kernel's input, where its money came from, and its answer.
pub fn action_event(body: &Value, input: &Value, books: &Value, out: &Map<String, Value>, stored: &panel_events::Stored) -> Value {
    let steps = out.get("result").and_then(|r| r.get("steps")).cloned();
    panel_events::plan("algoglyph.action_plan", json!({"loan_step": input.get("loan_step"), "max_loan": input.get("max_loan")}),
                       json!({"kernel": "action-plan", "runner": out.get("runner"), "books": books, "request": body}),
                       stored,
                       json!({"ok": truthy(out.get("ok")), "steps": steps, "errors": out.get("errors"), "kernel_input": input}))
}

fn h_intent(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let text = py_str(i.body.get("text"));
    let exe = sibling("mcp-intent");
    if !exe.exists() {
        return Ok((503, json!({"error": BUILD_IT})));
    }
    // The planner's portfolio needs a budget to solve: the caller's, else what a company may borrow.
    let mut params = i.body.get("params").and_then(Value::as_object).cloned().unwrap_or_default();
    if !params.contains_key("budget") {
        if let Some(ml) = g.max_loan().filter(|m| *m != 0) { params.insert("budget".into(), json!(ml)); }
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("--panel").arg(g.panel_url());
    if !params.is_empty() { cmd.arg("--params").arg(Value::Object(params.clone()).to_string()); }
    cmd.arg(&text);
    let r = run(&mut cmd, None, Duration::from_secs(180), None)?;
    if r.code != 0 {
        return Ok((if r.code == 2 { 400 } else { 502 }, json!({"error": r.stderr.trim()})));
    }
    let out: Value = serde_json::from_str(&r.stdout).map_err(|e| e.to_string())?;
    let names: Vec<Value> = out["intent"]["subjects"].as_array().into_iter().flatten().map(|x| x["name"].clone()).collect();
    g.event("intent", &format!("{} -> {} {} {}: {} candidate(s)", py_repr(&json!(text)), py_str(out["intent"].get("action")),
                              py_str(out["intent"].get("ordering")), py_repr(&Value::Array(names)),
                              out["answer"]["candidates"].as_array().map_or(0, Vec::len)));
    g.plan("plan.intent", intent_event(&text, "POST /api/intent", &params, &out));
    ok(out)
}

/// How many candidates of an intent answer the event carries; the answer itself has them all.
pub const INTENT_CANDIDATES: usize = 5;

/// The data of plan.intent: the sentence, its reading and the first candidates.
pub fn intent_event(text: &str, via: &str, params: &Map<String, Value>, out: &Value) -> Value {
    let a = &out["answer"];
    let cands = a["candidates"].as_array().cloned().unwrap_or_default();
    panel_events::plan("intent.answer", json!({"request": params, "candidates_kept": INTENT_CANDIDATES}),
                       json!({"text": text, "via": via, "resolver": "mcp-intent", "reads": out.get("reads"),
                              "vocabulary_size": out.get("vocabulary_size")}),
                       &panel_events::live_only("no issue plans history for it yet"),
                       json!({"intent": out.get("intent"), "how": a.get("how"), "candidates": cands.len(),
                              "first": cands.into_iter().take(INTENT_CANDIDATES).collect::<Vec<_>>(), "build": a.get("build")}))
}

/// Every calibration reading the stores hold (I0082): `?scope=all` (default) or `seed` (this game's
/// seed and map), the query that read them with its rows, and the factors derived from them, or the
/// fallback `uncalibrated` with its reason (store off, no reading).
fn h_calibration(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let scope = match i.query.str_or("scope", "all") {
        "all" => calibrate::Scope::All,
        "seed" => calibrate::Scope::Seed,
        other => return Ok((400, json!({"error": format!("scope: all or seed, not {other}")}))),
    };
    let game = calibrate::game_of(g.args.seed, &g.resource("openttd://game/state").unwrap_or(Value::Null));
    let cal = panel_bot::calibration(&scope, &game, panel_store::every_store);
    let mut out = cal.record;
    out["params"] = json!({"calibration": cal.factors.unwrap_or_else(|| json!({}))});
    out["stored"] = cal.stored.to_json();
    Ok((200, out))
}

/// Measure the running game's lines against the plan and publish the reading as plan.calibration,
/// which puts it in this instance's store; the factors answered are those of every stored reading
/// with this one.
fn h_calibration_record(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    let exe = sibling("mcp-plan");
    if !exe.exists() {
        return Ok((503, json!({"error": BUILD_IT})));
    }
    let mut cmd = Command::new(&exe);
    cmd.arg("--panel").arg(g.panel_url()).arg("--limit").arg(calibrate::CANDIDATES.to_string()).arg("--calibrate");
    let r = run(&mut cmd, None, Duration::from_secs(180), None)?;
    if r.code != 0 {
        return Ok((502, json!({"error": r.stderr.trim()})));
    }
    let out: Value = serde_json::from_str(&r.stdout).map_err(|e| e.to_string())?;
    let game = calibrate::game_of(g.args.seed, &g.resource("openttd://game/state").unwrap_or(Value::Null));
    let before = panel_bot::calibration(&calibrate::Scope::All, &game, panel_store::every_store);
    let e = calibration_event(&out, &game, &before);
    g.event("ai", &format!("calibration recorded in the store ({}): {} reading(s), factors {}", py_str(Some(&panel_store::store_status())),
                           e["choice"]["readings"], e["choice"]["factors"]));
    let answer = json!({"reading": out["calibration"], "recorded": {"store": panel_store::store_status(), "readings": e["choice"]["readings"],
                                                                    "params": {"calibration": e["choice"]["factors"]}, "stored": e["inputs"]["stored"]}});
    g.plan("plan.calibration", e);
    Ok((200, answer))
}

/// The data of plan.calibration: this game's reading of real against predicted profit (`out`, the
/// answer of mcp-plan --calibrate) for `game` (its seed and map, which the `seed` scope compares),
/// and the factors of every stored reading (`before`, read with the store's query) with this one.
pub fn calibration_event(out: &Value, game: &Value, before: &panel_bot::Calibration) -> Value {
    let reading = out["calibration"].clone();
    let mut all = before.readings.clone();
    let lines = calibrate::measured(&reading["lines"]);
    // A reading that measured no line is stored but is no reading when read back (readings_of).
    if !lines.is_empty() { all.push(json!({"lines": lines})); }
    let f = calibrate::factors(&all);
    let n = all.len();
    let mut params = calibrate::rule_params();
    params["effective"] = out.get("params").cloned().unwrap_or(Value::Null);
    params["read_limit"] = json!(calibrate::QUERY_READINGS);
    panel_events::plan("calibration.record", params,
                       json!({"planner": "mcp-plan --calibrate", "reads": out.get("reads"), "game": game, "before": before.record}),
                       &before.stored, json!({"reading": reading, "readings": n, "factors": f["params"]["calibration"], "modes": f["modes"]}))
}

fn h_plan(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let exe = sibling("mcp-plan");
    if !exe.exists() {
        return Ok((503, json!({"error": BUILD_IT})));
    }
    let limit = i.query.int("limit", 20)?;
    let params = i.query.str_or("params", "");
    let track = i.query.int("track", 0)?;
    let yards = i.query.int("yards", 0)?;
    let calibrate = i.query.int("calibrate", 0)? != 0;
    let mut cmd = Command::new(&exe);
    // Calibration matches running lines to candidates, so it needs the long list, not the top few.
    let limit = if calibrate { limit.max(calibrate::CANDIDATES as i64) } else { limit };
    cmd.arg("--panel").arg(g.panel_url()).arg("--limit").arg(limit.clamp(1, 500).to_string());
    if calibrate { cmd.arg("--calibrate"); }
    if !params.is_empty() { cmd.arg("--params").arg(params); }
    if track > 0 {
        if std::env::var("MCP_ALGOGLYPH_DIR").map_or(true, |d| d.is_empty()) {
            return Ok((503, json!({"error": "track=N needs AlgoGlyph track-planning: MCP_ALGOGLYPH_DIR is not set"})));
        }
        cmd.arg("--track").arg(track.min(10).to_string());
        if yards > 0 { cmd.arg("--yards").arg(yards.min(6).to_string()); }
    }
    let r = run(&mut cmd, None, Duration::from_secs(60 + 60 * track.clamp(0, 10) as u64 + 60 * yards.clamp(0, 6) as u64), None)?;
    if r.code != 0 {
        return Ok((if r.code == 2 { 400 } else { 502 }, json!({"error": r.stderr.trim()})));
    }
    ok(serde_json::from_str(&r.stdout).map_err(|e| e.to_string())?)
}

/// What to build, decided by AlgoGlyph's transport-investment kernel over the planner's own
/// candidates (one kernel run per cargo, then the portfolio knapsack): mcp-plan --invest.
fn h_investment(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let exe = sibling("mcp-plan");
    if !exe.exists() {
        return Ok((503, json!({"error": BUILD_IT})));
    }
    if std::env::var("MCP_ALGOGLYPH_DIR").map_or(true, |d| d.is_empty()) {
        return Ok((503, json!({"error": "MCP_ALGOGLYPH_DIR is not set (mcp-e2e sets it when it finds an AlgoGlyph checkout)"})));
    }
    let params = i.query.str_or("params", "");
    let p: Value = if params.is_empty() { json!({}) } else {
        match serde_json::from_str(params) { Ok(v) => v, Err(e) => return Ok((400, json!({"error": format!("params is not JSON: {e}")}))) }
    };
    let Value::Object(mut p) = p else { return Ok((400, json!({"error": "params must be a JSON object"}))) };
    if !p.contains_key("budget") {
        if let Some(ml) = g.max_loan().filter(|m| *m != 0) { p.insert("budget".into(), json!(ml)); }
    }
    let p_req = Value::Object(p);
    let r = run(Command::new(&exe).arg("--panel").arg(g.panel_url()).arg("--invest").arg("--params").arg(p_req.to_string()),
                None, Duration::from_secs(240), None)?;
    if r.code != 0 {
        return Ok((if r.code == 2 { 400 } else { 502 }, json!({"error": r.stderr.trim()})));
    }
    let out: Value = serde_json::from_str(&r.stdout).map_err(|e| e.to_string())?;
    let inv = out.get("investment").cloned().unwrap_or(json!({}));
    let chosen = inv.pointer("/portfolio/chosen").and_then(Value::as_array).cloned().unwrap_or_default();
    g.event("plan", &format!("investment: {} chosen of {} by {}: {}", chosen.len(), py_str(inv.get("candidates")),
                             py_str(inv.pointer("/portfolio/solver")),
                             chosen.iter().map(|c| format!("{} {} roi {:.2}", py_str(c.get("cargo")), py_str(c.get("mode")),
                                                           c.get("roi").and_then(Value::as_f64).unwrap_or(0.0)))
                                 .collect::<Vec<_>>().join(", ")));
    g.plan("plan.investment", investment_event(&p_req, &out));
    ok(json!({"investment": inv, "planner_portfolio": out.get("portfolio"), "params": out.get("params")}))
}

/// The data of plan.investment: what AlgoGlyph's transport-investment chose over the planner's
/// candidates, beside the planner's own portfolio.
pub fn investment_event(request: &Value, out: &Value) -> Value {
    let inv = &out["investment"];
    panel_events::plan("algoglyph.transport_investment", json!({"request": request, "effective": out.get("params")}),
                       json!({"planner": "mcp-plan --invest", "reads": out.get("reads"), "candidates": inv.get("candidates")}),
                       &panel_events::live_only("I0086"),
                       json!({"solver": inv.pointer("/portfolio/solver"), "chosen": inv.pointer("/portfolio/chosen"),
                              "planner_chosen": out.pointer("/portfolio/chosen").and_then(Value::as_array).map(Vec::len),
                              "error": inv.get("error")}))
}

fn h_graph(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    let exe = sibling("mcp-graph");
    if !exe.exists() {
        return Ok((503, json!({"error": BUILD_IT})));
    }
    let r = run(Command::new(&exe).arg("--panel").arg(g.panel_url()), None, Duration::from_secs(60), None)?;
    if r.code != 0 {
        return Ok((502, json!({"error": r.stderr.trim()})));
    }
    ok(serde_json::from_str(&r.stdout).map_err(|e| e.to_string())?)
}

fn h_status(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(json!({
        "server_running": g.server_running(), "mcp_up": g.mcp_up.load(Ordering::SeqCst), "endpoint": g.endpoint(),
        "ai": g.args.ai, "ai_started": g.ai_started.load(Ordering::SeqCst), "auto_approve": g.auto_approve.load(Ordering::SeqCst),
        "pending": g.pending_json(), "game_port": g.args.game_port, "events": g.events_json(), "event_seq": g.bus.seq(),
        "bus": panel_store::endpoint(), "store": panel_store::store_status(), "store_access": panel_store::store_access()}))
}

fn h_events(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let since = i.query.int("since", 0)?.max(0) as u64;
    let kinds = kinds_of(i.query.str_or("kinds", ""));
    let limit = i.query.int("limit", 500)?.max(0) as usize;
    ok(json!({"events": g.bus.since(since, &kinds, limit), "last_seq": g.bus.seq()}))
}

fn h_resource(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    ok(g.resource(i.query.str_or("uri", ""))?)
}

fn h_log(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    let log = g.log.lock().unwrap();
    let skip = log.lines.len().saturating_sub(150);
    ok(json!({"lines": log.lines.iter().skip(skip).map(|(_, l)| l.clone()).collect::<Vec<_>>()}))
}

fn h_overview(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(json!({"state": g.resource("openttd://game/state")?, "companies": g.resource("openttd://companies")?,
              "decisions": g.resource("openttd://decisions")?}))
}

fn h_catalogue(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(json!({"resources": g.rpc("resources/list", None)?, "tools": g.rpc("tools/list", None)?,
              "initialize": g.rpc("initialize", Some(json!({"protocolVersion": "2025-11-25", "capabilities": {},
                                                             "clientInfo": {"name": "openttd-mcp-panel", "version": "1"}})))?}))
}

fn h_rpc(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let method = i.body["method"].as_str().unwrap_or("");
    // A raw tools/call names no writer: it sets no freight-line slot (panel_bot::raw_call_refusal).
    if method == "tools/call" {
        let p = &i.body["params"];
        if let Some(r) = panel_bot::raw_call_refusal(g, &py_str(p.get("name")), &p["arguments"]) {
            return Ok(r);
        }
    }
    ok(g.rpc(method, i.body.get("params").cloned())?)
}

fn h_pause(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    ok(g.set_pause(i.body.get("pause").map_or(true, |v| truthy(Some(v))))?)
}

/// The HTTP status of an approval's answer: an approval that did not take is an error the owner
/// sees (the page's one API client shows a non-2xx answer), never a 200 with an error inside. The
/// status of the decision decides first, since an executed decision carries "error": "".
pub fn approve_status(out: &Value) -> u16 {
    let nonempty_error = out.get("error").is_some_and(|e| !e.as_str().is_some_and(str::is_empty));
    match out.get("status").and_then(Value::as_str) {
        _ if out.get("refused").is_some() => 403,
        Some("executed") | Some("denied") => 200,
        Some("rejected") => 422,
        _ if out.get("pending_decision").is_some() => 504,
        _ if out.get("approval").is_some() => 409,
        _ if nonempty_error => 404,
        _ => 502,
    }
}

fn h_approve(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let declared = i.body.get("by").and_then(Value::as_str).map(|b| b.chars().take(64).collect::<String>());
    let id = py_int(&i.body["id"])?;
    // The owner secret was checked (owner_gate); the name in `by` is declared, not checked.
    let out = g.approve(id, json!({"channel": "api", "declared": declared, "owner_secret": true}))?;
    Ok((approve_status(&out), out))
}

fn h_deny(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let out = g.deny(py_int(&i.body["id"])?)?;
    Ok((approve_status(&out), out))
}

/// How often a rule route reads the core back for its effect, 150 ms apart (3 s; 0.3 s in tests).
#[cfg(not(test))]
const READBACK: u32 = 20;
#[cfg(test)]
const READBACK: u32 = 2;

/// The approvals list: the server's whole queue (pending, decided, expired), each approval marked
/// `owner_only` and `decidable`, the core's standing rules and whether it keeps any (see
/// [`approvals_view`]).
fn h_approvals(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(approvals_view(&g.queue_data()))
}

/// The picked ids decided together. With the approval-core of feature/approval-rules: the ids the
/// panel may not decide are answered here as POST /api/approve and /api/deny would (403 owner-only,
/// 404, 409), the rest go in ONE line `prismatic_mcp approve|deny <id> ... --via panel --nonce <n>`
/// and the core's result line names each. With an older core: one decision per id, as before.
fn decide_many(g: &Arc<Game>, i: &Input, approve: bool) -> Result<(u16, Value), String> {
    let ids = match bulk_ids(&i.body) {
        Ok(ids) => ids,
        Err(e) => return Ok((400, json!({"error": e}))),
    };
    let one = |id: i64| {
        let out = if approve { g.approve(id, json!({"channel": "api", "bulk": true, "owner_secret": true})) } else { g.deny(id) };
        match out {
            Ok(out) => (id, approve_status(&out), out),
            Err(e) => (id, 502, json!({"error": e})),
        }
    };
    let results: Vec<(i64, u16, Value)> = if !g.keeps_rules() {
        ids.into_iter().map(one).collect()
    } else {
        let queue = g.approvals();
        let mut out = Vec::new();
        let mut send = Vec::new();
        for id in ids {
            let held = queue.iter().find(|a| a["id"] == json!(id));
            match held {
                None => out.push((id, 404, json!({"error": format!("the game server holds no approval #{id}")}))),
                Some(a) if a["state"] != json!("pending") || a["expired"] == json!(true) => {
                    let st = if a["expired"] == json!(true) { "expired".to_string() } else { py_str(a.get("state")) };
                    out.push((id, 409, json!({"error": format!("approval #{id} is {st}, not pending"), "approval": a})));
                }
                Some(a) if approve && a["tool"].as_str().is_some_and(owner_only) => out.push((id, 403, owner_only_refusal(id, &py_str(a.get("tool"))))),
                Some(_) => send.push(id),
            }
        }
        if !send.is_empty() {
            if approve {
                for &id in &send { panel_decision::record_approval(id, json!({"channel": "api", "bulk": true, "owner_secret": true})); }
            }
            let (code, answer) = core_line(g, &decide_line(approve, &send, Some("panel"), None))?;
            match code {
                200 => out.extend(per_id(&send, &answer["result"], approve)),
                _ => out.extend(send.iter().map(|&id| (id, code, answer.clone()))),
            }
        }
        out
    };
    let answer = bulk_answer(results);
    g.event("operator", &format!("{} {} of {} selected approval(s)", if approve { "approved" } else { "denied" }, answer["done"],
                                 answer["done"].as_u64().unwrap_or(0) + answer["failed"].as_u64().unwrap_or(0)));
    ok(answer)
}

fn h_approve_many(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    decide_many(g, i, true)
}

fn h_deny_many(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    decide_many(g, i, false)
}

/// The refusal while the game server's core keeps no rules (it answers no `rules` in its queue).
fn no_rules_core() -> (u16, Value) {
    (501, json!({"error": "this game server's approval core has no bulk lines and no standing rules (it answers no rules in openttd://approvals); \
                           decide one at a time, or run a game built with the approval-rules core", "refused": "no_rules_core"}))
}

/// Send one line to the approval core with a fresh `--nonce` and wait for its one result line
/// (`[approval-core] {"nonce", "ok", ...}`, [`core_result`]): 200 with it as `result` when ok, 422
/// with its `error` when not, 504 when none came within [`READBACK`] reads.
fn core_line(g: &Arc<Game>, line: &str) -> Result<(u16, Value), String> {
    let nonce = next_nonce();
    let line = format!("{line} --nonce {nonce}");
    let mark = g.log_seq();
    g.console(&line)?;
    g.event("operator", &format!("sent to the approval core: {line}"));
    for _ in 0..READBACK {
        thread::sleep(Duration::from_millis(150));
        if let Some(result) = core_result(&g.log_after(mark), &nonce) {
            if result["ok"] == json!(true) {
                return Ok((200, json!({"sent": line, "result": result})));
            }
            let why = result["error"].as_str().filter(|e| !e.is_empty()).unwrap_or("the approval core refused the line").to_string();
            return Ok((422, json!({"error": why, "sent": line, "result": result})));
        }
    }
    Ok((504, json!({"error": format!("sent `{line}`, but the approval core gave no result within {} ms", READBACK * 150),
                    "sent": line, "output": g.log_after(mark), "pending_decision": true})))
}

/// `approve all` or `deny all` over a tool glob and matcher, as the core decides them.
fn decide_all(g: &Arc<Game>, i: &Input, approve: bool) -> Result<(u16, Value), String> {
    if !g.keeps_rules() {
        return Ok(no_rules_core());
    }
    let (tool, m) = (i.body.get("tool").and_then(Value::as_str), i.body.get("match"));
    let line = match if approve { approve_all_line(tool, m) } else { deny_all_line(tool, m) } {
        Ok(l) => l,
        Err(e) => return Ok((400, json!({"error": e}))),
    };
    let (code, mut out) = core_line(g, &line)?;
    if code == 200 {
        out["status"] = json!(if approve { "approved" } else { "denied" });
        out["decided"] = out["result"]["decided"].clone();
    }
    Ok((code, out))
}

fn h_approve_all(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    decide_all(g, i, true)
}

fn h_deny_all(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    decide_all(g, i, false)
}

/// A standing rule: `always allow` or `always deny` a tool glob, optionally only for arguments
/// the JSON matcher takes, kept across restarts with `persist`. 200 with the new rule as the core
/// lists it.
fn h_rule_add(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    if !g.keeps_rules() {
        return Ok(no_rules_core());
    }
    let line = match rule_line(&py_str(i.body.get("effect")), &py_str(i.body.get("tool")), i.body.get("match"), truthy(i.body.get("persist"))) {
        Ok(l) => l,
        Err(e) => return Ok((400, json!({"error": e}))),
    };
    let (code, mut out) = core_line(g, &line)?;
    if code == 200 {
        let rules = g.queue_data()["rules"].clone();
        let id = out["result"]["rule_id"].clone();
        out["rule"] = rules.as_array().into_iter().flatten().find(|r| r["id"] == id).cloned().unwrap_or(json!({"id": id}));
        out["rules"] = rules;
    }
    Ok((code, out))
}

/// Remove standing rule `id`: 404 for a rule the core does not hold, 200 once it says it is gone.
fn h_rule_remove(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    if !g.keeps_rules() {
        return Ok(no_rules_core());
    }
    let id = py_int(&i.body["id"])?;
    let held = g.queue_data()["rules"].as_array().into_iter().flatten().any(|r| r["id"].as_i64() == Some(id));
    if !held {
        return Ok((404, json!({"error": format!("the approval core holds no rule #{id}")})));
    }
    let (code, mut out) = core_line(g, &rule_remove_line(id))?;
    if code == 200 {
        out["removed"] = json!(id);
        out["rules"] = g.queue_data()["rules"].clone();
    }
    Ok((code, out))
}

/// Who may approve from inside the game (the in-game approvers, ADR 0038): `{add: <client id>}`,
/// `{remove: <key prefix>}`, or `{}` to list them. The owner's alone; `/api/console` refuses every
/// `approvers` line. 422 when the game printed a refusal, else 200 with its output.
fn h_approvers(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let line = match approvers_line(&i.body) {
        Ok(l) => l,
        Err(e) => return Ok((400, json!({"error": e}))),
    };
    let mark = g.log_seq();
    g.console(&line)?;
    g.event("operator", &format!("sent: {line}"));
    thread::sleep(Duration::from_millis(if cfg!(test) { 0 } else { 600 }));
    let output = g.log_after(mark);
    let refused = output.iter().find(|l| ["No connected", "Could not", "No single"].iter().any(|p| l.trim_start().starts_with(p))).cloned();
    if refused.is_none() {
        if let Some(to) = approvers_change(&i.body) {
            g.governance("approvers", Value::Null, to, "POST /api/approvers", &i.who);
        }
    }
    Ok(match refused {
        Some(why) => (422, json!({"error": why, "sent": line, "output": output})),
        None => (200, json!({"sent": line, "output": output})),
    })
}

fn h_bots(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(panel_bot::bots(g)?)
}

fn h_bot_choices(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(panel_bot::bot_choices(g)?)
}

fn h_bot_start(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    ok(panel_bot::start_bot(g, &py_str(i.body.get("ai")), i.body.get("settings").cloned().unwrap_or(json!({})))?)
}

fn h_bot_setting(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    panel_bot::bot_setting(g, &i.body)
}

fn h_bot_planned(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let ai = py_str(i.body.get("ai"));
    let track = i.body.get("track").and_then(Value::as_u64).unwrap_or(0).min(16) as usize;
    let Some(p) = panel_bot::planned(g, track) else {
        return Ok((503, json!({"error": format!("no planner: {BUILD_IT} (mcp-plan)")})));
    };
    // A failed planner is a failed plan: answered 502 with both events' data, and no bot starts
    // unplanned (the bot start of a stack does start one; this route was asked for a planned bot).
    if let Some(e) = p.portfolio.get("error").filter(|e| !e.is_null()) {
        return Ok((502, json!({"error": py_str(Some(e)), "ai": ai, "settings": "", "started": null,
                               "portfolio": p.portfolio, "bot_settings": p.bot_settings})));
    }
    let (settings, flat) = panel_bot::settings_json(&p.settings);
    let started = if truthy(i.body.get("start")) {
        Some(panel_bot::start_bot(g, &ai, settings)?)
    } else {
        None
    };
    ok(json!({"ai": ai, "settings": flat, "started": started, "portfolio": p.portfolio, "bot_settings": p.bot_settings}))
}

/// GET /api/bus: the tap's answer (src/panel_store.rs) with the declared kinds added. The kinds are
/// a table of this binary, so they are answered (200) also when the tap is down; `endpoint` is then
/// null and `error` says why.
fn h_bus(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let (code, mut out) = panel_store::h_bus(g, i)?;
    if code == 503 {
        out["endpoint"] = Value::Null;
        out["schema"] = json!(crate::bus_sinks::SCHEMA);
    }
    out["kinds"] = panel_events::table(panel_events::PANEL_BUS);
    Ok((if code == 503 { 200 } else { code }, out))
}

fn h_bus_publish(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let kind = py_str(i.body.get("kind"));
    let data = i.body.get("data").cloned().unwrap_or(Value::Null);
    if let Some(why) = panel_events::refuse_external(&kind, &data) {
        return Ok((400, json!({"error": why})));
    }
    let seq = g.plan(&kind, data);
    ok(json!({"published": kind, "seq": seq}))
}

/// GET /api/evidence?q=&k=: passages of this repository that answer `q`, with their provenance and
/// freshness, or `insufficient evidence`.
fn h_evidence(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let q = i.query.get("q").unwrap_or("").trim().to_string();
    if q.is_empty() { return Ok((400, json!({"error": "?q= is the question"}))); }
    let k = i.query.get("k").and_then(|k| k.parse::<usize>().ok()).unwrap_or(5).clamp(1, 20);
    let root = crate::panel_results::repo();
    let index = crate::evidence::index_of(&root)?;
    let now = |p: &str| std::fs::read_to_string(root.join(p)).ok();
    Ok((200, index.answer(&q, k, &now)))
}

fn h_bridge_companies(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(panel_decision::observe(g)?)
}

/// GET /api/bridge/observation: the latest answer of GET /api/bridge/companies as an observation.
fn h_bridge_observation(_: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    let (count, latest) = panel_decision::last();
    ok(json!({"observations": count, "latest": latest.map(|o| o.json()), "max_age_days": panel_decision::max_age_days()}))
}

fn h_decision_pull(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    let url = match i.body.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) {
        Some(u) => u.to_string(),
        None => std::env::var("PRISMATIC_DECISION_URL").unwrap_or_default(),
    };
    if url.is_empty() {
        return Ok((501, json!({"error": "no decision URL: pass url, or set PRISMATIC_DECISION_URL (prismatic_api: http://127.0.0.1:4000/api/openttd/decision)"})));
    }
    let key = i.body.get("key").and_then(Value::as_str);
    panel_decision::keyed(key, Instant::now(), || panel_decision::pull_with(g, &url, key))
}

fn h_instances(_: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(json!({"registry": registry().display().to_string(), "instances": discover()}))
}

fn h_instances_call(_: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    instance_call(&py_str(i.body.get("name")), &py_str(i.body.get("method")), i.body.get("params").unwrap_or(&Value::Null))
}

fn h_news(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    ok(g.view_call("openttd.view.news.list", json!({"limit": i.query.int("limit", 20)?}))?)
}

fn h_news_show(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    ok(g.view_call("openttd.view.news.show", i.body.clone())?)
}

fn h_ui_classes(_: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    let src = here().join("../../../src/window_type.h");
    window_classes(&std::fs::read_to_string(&src).map_err(|e| format!("{}: {e}", src.display()))?).map(|v| (200, v))
}

fn h_ask(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    ok(g.view_call("openttd.view.ask", i.body.clone())?)
}

fn h_ask_answers(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(g.ask_answers()?)
}

fn h_news_close(g: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> {
    ok(g.view_call("openttd.view.news.close", json!({}))?)
}

fn h_view_tool(g: &Arc<Game>, i: &Input) -> Result<(u16, Value), String> {
    ok(g.view_call(&py_str(i.body.get("name")), i.body.get("arguments").cloned().unwrap_or(json!({})))?)
}

/// The panel's own HTTP surface, declared once: the router dispatches from this table and
/// /openapi.json documents it. Routes derived from the live MCP catalogue are added at request time
/// (derive_routes), from the same tools/list and resources/list every other surface reads.
pub fn routes() -> Vec<Route> {
    let r = |method, path, op_id, summary, needs_mcp, schema, handler: Handler| Route { method, path, op_id, summary, needs_mcp, schema, handler };
    let st = || json!({"type": "string"});
    vec![
        r("GET", "/api/status", "status", "Panel, server and MCP status; pending approvals; recent events", false, None, h_status),
        r("GET", "/api/log", "log", "The server console log (token redacted)", false, None, h_log),
        r("GET", "/api/events", "events", "Events after ?since=<seq>, optionally ?kinds=game,company,... ; SSE: /api/events/stream", false, None, h_events),
        r("GET", "/api/overview", "overview", "Game state, companies and decisions in one read", true, None, h_overview),
        r("GET", "/api/results", "results", "Every duel result (research/results, written by mcp-duel), newest first: seed, months, label, opponent, winner and both companies' final value", false, None, panel_results::h_results),
        r("GET", "/api/result", "result", "One duel result by name (?name=duel-...): summary, month-by-month series, the plan, both bots' logs and the written reading", false, None, panel_results::h_result),
        r("POST", "/api/duel", "duelStart", "Start a duel in the background (tools/research/mcp-duel): the planned bot against an unplanned one or an opponent AI, on its own ports; one at a time", false,
          s(json!({"months": {"type": "integer", "minimum": 1, "maximum": 240}, "seed": {"type": "integer"}, "label": {"type": "string"}, "opponent": {"type": "string"}, "bot_settings": {"type": "string", "description": "extra settings for the planned bot, e.g. own_lines=8"}, "calibrated": {"type": "boolean", "description": "start the planned bot's plan from every calibration reading the stores hold (PRISMATIC_PLAN_CALIBRATION=all, GET /api/calibration)"}}), &[]), panel_results::h_duel_start),
        r("GET", "/api/calibration", "calibration", "Every calibration reading the stores hold (plan.calibration events of this panel's store and every other one, I0082): ?scope=all (default) or seed (this game's seed and map), the query with its rows, the readings used and params.calibration derived from them (per mode, the median over every line), or the fallback uncalibrated with its reason", false,
          None, h_calibration),
        r("POST", "/api/calibration", "calibrationRecord", "Measure this game's running lines against the plan (mcp-plan --calibrate) and publish the reading as plan.calibration, which puts it in this instance's store; the planned bot start reads every stored reading when PRISMATIC_PLAN_CALIBRATION is all or seed (a duel with calibrated: true)", true,
          s(json!({}), &[]), h_calibration_record),
        r("GET", "/api/scoreboard", "scoreboard", "The README duel scoreboard as tools/research/readme-scoreboard derives it from research/results: best run, wins of all duels, the seed-31337 series, and whether README.md is current or stale", false, None, panel_results::h_scoreboard),
        r("POST", "/api/scoreboard", "scoreboardWrite", "Regenerate the scoreboard region of README.md from research/results (tools/research/readme-scoreboard); answers the new scoreboard. Commit and push stay the owner's", false,
          s(json!({}), &[]), panel_results::h_scoreboard_write),
        r("GET", "/api/bench", "bench", "Every Rust benchmark record (research/results/bench-*.json, schema rust-bench/v1, written by the benchmark harness), oldest first: rev, host, toolchain, each bench's median, mean, stddev, min, max and samples, and the regressions against its baseline; a file off the schema is listed under `skipped` with its reason. ?id=bench-... answers that one record", false, None, panel_bench::h_bench),
        r("GET", "/api/realworld", "realWorld", "Every real-world place snapshot (tools/research/real-world/fixtures/*/snapshot.json, research/real-world-scenario.md): the query, the candidate it resolved to, the map's size and scale, each source with its licence and completeness, and what the offline compiler makes of it (entities and economy edges by evidence class: OBSERVED, DERIVED, INFERRED, HYPOTHETICAL)", false, None, panel_realworld::h_list),
        r("GET", "/api/realworld/scenario", "realWorldScenario", "One place compiled in process by the one compiler (?name=brno): the scenario (heightmap, towns, industries, landmark signs, economy edges, each with its evidence class, rationale, confidence and rule), the snapshot's candidates, sources, licences and features, the heightmap as a PNG data URI, and both validations. Offline and deterministic", false, None, panel_realworld::h_scenario),
        r("POST", "/api/realworld/compile", "realWorldCompile", "Write one place's compiled artifacts (heightmap.png, towns.json, the RealWorldScenario GameScript, openttd.cfg, preview.svg, scenario.json, SHA256SUMS) to build-real-world/<name>/ and say whether they match the hashes recorded with its fixture. Offline; the headless load is tools/research/real-world/rw-scenario load-check", false,
          s(json!({"name": st()}), &["name"]), panel_realworld::h_compile),
        r("GET", "/api/town/model", "townModel", "The town-authority model derived from the engine sources (tools/research/town-model/town_model.json, generated by derive.py; research/mechanisms/town-authority.md): rating constants, the bands an AI sees, every effect on a company's rating (trigger, delta, limit), every gate (what the authority refuses below which threshold, removal thresholds by tolerance), the town actions with their cost factors and effects, links (statue, exclusivity, noise, station service, growth), the settings they read and the relations between them, each rule with its source file:line. Offline", false, None, panel_town::h_model),
        r("GET", "/api/town/live", "townLive", "Every live town (openttd://towns) with one company's rating, band and the advice of town_lib.rhai advice(town, tolerance), run in process: what to do, why and the rule's source line. ?company=N (default: the first company with a rating). The tolerance is read from the console (difficulty.town_council_tolerance), else lenient. Ratings, station counts, statue, exclusivity and growth fields are used when the game publishes them. Without a game: an empty list and the reason, never an error", false, None, panel_town::h_live),
        r("GET", "/api/duel", "duelStatus", "The duel started from the panel: running or finished, the last month line, the winner line and the result's name", false, None, panel_results::h_duel_status),
        r("GET", "/api/bus", "bus", "The event bus on ZeroMQ (ADR 0033): the PUB endpoint (tcp://127.0.0.1:<port>), the envelope schema prismatic-bus/v1, every topic seen with its count, how many events went out, and `kinds`: every event kind the bus carries, the one declared table (src/panel_events.rs, I0081), answered also when the tap is down (endpoint null, `error` says why); subscribe with any ZeroMQ SUB client (tools/research/mcp-rhai mcp-bus-tail)", false, None, h_bus),
        r("POST", "/api/bus/publish", "busPublish", "Put one event of a kind the table declares as external (`by: external` in GET /api/bus kinds; plan.kernel from mcp-kernel-feed --publish) on this panel's bus, so it reaches ZeroMQ and the store like the panel's own; the data must carry {decision, version, inputs, choice}. Any other kind is refused", false,
          s(json!({"kind": st(), "data": obj()}), &["kind", "data"]), h_bus_publish),
        r("GET", "/api/db/tables", "dbTables", "The DuckDB store of this instance (ADR 0033): its file and every table (runs, events, samples) with its columns", false, None, panel_store::h_db_tables),
        r("GET", "/api/db/query", "dbQuery", "One read-only statement on the store (?sql=SELECT ...; SELECT, WITH, DESCRIBE, SUMMARIZE or SHOW, one statement, anything else 400; without sql the newest 20 events): {columns, rows, count, capped}, each row a list in column order, at most 5000 rows", false, None, panel_store::h_db_query),
        r("POST", "/api/db/query", "dbQueryPost", "The same read-only query with the statement in the body ({sql, params?}), for statements too long for an address; `params` binds the statement's named parameters ($company), never pasted, a missing one 400", false,
          s(json!({"sql": st(), "params": obj()}), &["sql"]), panel_store::h_db_query),
        r("GET", "/api/db/named", "dbNamed", "Every named query of the store (one file each in tools/research/mcp-rhai/sql/queries): name, description, parameters read from the prepared statement, and its sql", false, None, panel_store::h_db_named_list),
        r("POST", "/api/db/named", "dbNamedRun", "Run one named query with its parameters bound, never pasted ({name, params: {company: 0, ...}}): {columns, rows, count, capped}; 404 for an unknown name, 400 for a missing parameter", false,
          s(json!({"name": st(), "params": obj()}), &["name"]), panel_store::h_db_named_run),
        r("GET", "/api/overview/history", "overviewHistory", "The overview's history kept by the panel: one sample per game day (tick, date, vehicles, towns, per company money, value, loan, net); ?since=<tick> for the newer ones", false, None, h_history),
        r("GET", "/api/catalogue", "catalogue", "MCP initialize, resources/list and tools/list", true, None, h_catalogue),
        r("GET", "/api/resource", "readResource", "Read one MCP resource by ?uri=", true, None, h_resource),
        r("POST", "/api/rpc", "rpc", "Raw MCP JSON-RPC with the panel's token; a tools/call of openttd.ai.set_setting or openttd.ai.set_settings that changes a freight-line slot (fr_*) names no writer and is refused (422 fr_needs_writer)", true,
          s(json!({"method": st(), "params": obj()}), &["method"]), h_rpc),
        r("POST", "/api/console", "console", "One OpenTTD console line as the operator; returns its output", false,
          s(json!({"command": st(), "wait": {"type": "number"}}), &["command"]), h_console),
        r("GET", "/api/content", "content", "What the content server (BaNaNaS) offers whose name matches ?q= (AIs, libraries, NewGRFs ...): id, type, state, name, from the game's own content state", false,
          None, h_content),
        r("POST", "/api/content/download", "contentDownload", "The owner's download of content items by id (an AI and the libraries it needs), through the game's content select and content download", false,
          s(json!({"ids": {"type": "array", "items": {"type": "integer"}}, "wait": {"type": "number"}}), &["ids"]), h_content_download),
        r("POST", "/api/pause", "pause", "Pause or resume through the approval-gated MCP tool", true,
          s(json!({"pause": {"type": "boolean"}}), &["pause"]), h_pause),
        r("POST", "/api/approve", "approve", "The owner's approval of a pending decision, which the game server carries out at once (ADR 0038): the decision it recorded (200), 422 when the game rejected it, 409 for an approval not pending, 404 for one it does not hold, 504 when no decision came. Needs the owner secret (signed in through POST /api/owner/login, the cookie ?owner=<secret> sets, or the X-Panel-Owner header), else 401; an owner-only tool (openttd.ai.set_settings, a Battlebrain live decision) is refused with 403 and the console command to type, whoever asks; `by` names the approver it declares (recorded in plan.directive_outcome, not checked)", true,
          s(json!({"id": {"type": "integer"}, "by": st()}), &["id"]), h_approve),
        r("POST", "/api/deny", "deny", "Deny a pending decision: the game server withdraws it and enacts nothing (ADR 0038), so no owner secret is needed. 200 with the approval now denied, 409 for an approval not pending, 404 for one the server does not hold, 504 when the server did not mark it denied", true,
          s(json!({"id": {"type": "integer"}}), &["id"]), h_deny),
        r("GET", "/api/approvals", "approvals", "The game server's whole approval queue for the approvals list: every approval (pending, decided, expired) with its tool, exact arguments, ticks and the rule that matched it (matched_rule), marked owner_only (decided at the game's console alone, ADR 0038) and decidable (pending, not expired); the core's standing rules (id, effect, tool glob, JSON matcher, created_via, hits, persisted) and rules_supported, false while the core keeps none", true, None, h_approvals),
        r("POST", "/api/approve/many", "approveMany", "The owner approves the approvals a person picked, one by one through the same core as POST /api/approve: one result per id with the status /api/approve answers (200, 403 owner-only, 404, 409, 422, 504), and how many went through. Needs the owner secret", true,
          s(json!({"ids": {"type": "array", "items": {"type": "integer"}}}), &["ids"]), h_approve_many),
        r("POST", "/api/deny/many", "denyMany", "The owner denies the approvals a person picked, one by one through the same core as POST /api/deny: one result per id, and how many were withdrawn. Needs the owner secret", true,
          s(json!({"ids": {"type": "array", "items": {"type": "integer"}}}), &["ids"]), h_deny_many),
        r("POST", "/api/approve/all", "approveAll", "The owner approves every waiting approval a tool glob (and a JSON matcher over its arguments) takes: `prismatic_mcp approve all [<glob> [<matcher>]] --via panel` to the core, which never lets it decide an owner-only tool. 501 while the core keeps no rules, 422 with the line it refused with. Needs the owner secret", true,
          s(json!({"tool": st(), "match": obj()}), &[]), h_approve_all),
        r("POST", "/api/deny/all", "denyAll", "The owner denies every waiting approval a tool glob (and a JSON matcher) takes: `prismatic_mcp deny all [<glob> [<matcher>]]`. 501 while the core keeps no rules. Needs the owner secret", true,
          s(json!({"tool": st(), "match": obj()}), &[]), h_deny_all),
        r("POST", "/api/rules", "ruleAdd", "The owner's standing rule, as Claude's always allow / always deny: effect allow or deny, a tool glob (* and ?) over the MCP tool name or action type, an optional JSON matcher over the arguments (plain keys match a subset, $.a.b and $.list[*] keys are JSON paths, a string is a glob, \"*\" any present value), persist to keep it across restarts. Deny wins over allow; the core applies it to waiting and new approvals and records approved_via with the rule. `prismatic_mcp rule allow|deny <glob> [<matcher>] [--persist] --via panel`; an allow rule never decides an owner-only tool. 200 with the rule, 400 for a bad glob or matcher, 422 when the core refused the line, 501 while it keeps no rules. Needs the owner secret", true,
          s(json!({"effect": {"type": "string", "enum": ["allow", "deny"]}, "tool": st(), "match": obj(), "persist": {"type": "boolean"}}), &["effect", "tool"]), h_rule_add),
        r("POST", "/api/rules/remove", "ruleRemove", "The owner removes standing rule `id` (`prismatic_mcp rule remove <id>`): 200 once the core no longer lists it, 404 for a rule it does not hold, 501 while it keeps no rules. Needs the owner secret", true,
          s(json!({"id": {"type": "integer"}}), &["id"]), h_rule_remove),
        r("POST", "/api/approvers", "approvers", "The owner says who may approve from inside the game (ADR 0038): {add: <client id>} registers a connected client's key, {remove: <key prefix>} drops one, {} lists them (`prismatic_mcp approvers ...`); /api/console refuses every approvers line. 422 when the game printed a refusal. Needs the owner secret", true,
          s(json!({"add": {"type": "integer"}, "remove": st()}), &[]), h_approvers),
        r("POST", "/api/auto-approve", "autoApprove", "The owner's full-auto switch: on needs the owner secret (as /api/approve), off is open to anyone; it never approves an owner-only tool", false,
          s(json!({"on": {"type": "boolean"}}), &["on"]), h_auto),
        r("POST", "/api/owner/login", "ownerLogin", "The owner signs in: the owner secret (from the file mcp-panel named at start, $PRISMATIC_PANEL_OWNER_SECRET_FILE or ~/.config/prismatic/panel-owner-secret) sets this browser's HttpOnly cookie, as ?owner= does, and answers 204; a wrong secret 401 (after a delay), five wrong ones within a minute 429", false,
          s(json!({"secret": {"type": "string"}}), &["secret"]), h_owner_login),
        r("POST", "/api/owner/logout", "ownerLogout", "Sign this browser out of the owner's routes (its cookie is cleared); 204", false,
          Some(obj()), h_owner_logout),
        r("POST", "/api/owner/once", "ownerOnce", "A one-time sign-in link for the owner's browser: `?once=<nonce>` (answered as `once` and `url`) sets the owner's cookie once within 60 s and redirects (303) to the page without the query; a spent or unknown nonce answers 401. Needs the owner secret", false,
          Some(obj()), h_owner_once),
        r("POST", "/api/start-ai", "startAi", "Start a company AI with an optional settings string", false,
          s(json!({"ai": st(), "settings": st()}), &[]), h_start_ai),
        r("GET", "/api/bots", "bots", "The AIs the owner can run as their bot, their settings (from openttd.ai.list, else info.nut) and the running AI companies", true,
          None, h_bots),
        r("GET", "/api/bots/choices", "botChoices", "What a bot setting naming a town, industry or cargo may name, from openttd://world/graph (towns by population, industries with their cargos, every cargo)", true,
          None, h_bot_choices),
        r("POST", "/api/bots/planned", "botPlanned", "Plan the owner's bot as the panel does when it starts one: mcp-plan's portfolio within the loan limit, then the settings chosen from it, published as plan.portfolio and plan.bot_settings beside the panel.ai sentences; with start: true also start `ai` with those settings (a new AI company). track: N prices the N best rail lines on the map first (needs AlgoGlyph; the bot start uses 16, 0 here unless given). Answers the settings and the two events' data; 502 with that data and no bot started when the planner failed. It plans also when the stack runs with --no-plan-bot, which turns off only the bot start's plan", true,
          s(json!({"ai": st(), "start": {"type": "boolean"}, "track": {"type": "integer", "minimum": 0, "maximum": 16}}), &["ai"]), h_bot_planned),
        r("POST", "/api/bots/start", "botStart", "Start the owner's bot: an AI with a settings map", false,
          s(json!({"ai": st(), "settings": obj()}), &["ai"]), h_bot_start),
        r("POST", "/api/bots/setting", "botSetting", "Change one setting of a running bot (openttd.ai.set_setting, approval-gated). A freight-line slot (fr_*, fr2_* .. fr16_*) is sent only when `writer` names who sends it and the company's planner key planner.<company> (/api/rhai/shared) is unset or names that writer: else 422 fr_needs_writer (no writer) or 409 fr_planner_held, and nothing reaches the game", true,
          s(json!({"company": {"type": "integer"}, "name": st(), "value": {"type": "integer"}, "writer": st()}), &["company", "name", "value"]), h_bot_setting),
        r("GET", "/api/bridge/companies", "bridgeCompanies", "Every company as the bridge's GET /companies answers it (CompanyOut: money, loan, income from the vehicles' profit, company value, vehicle counts), so Prismatic decides from this game with OPENTTD_BRIDGE_URL=<panel>/api/bridge (src/panel_decision.rs); each company's `history` (prismatic-history/v1, I0085) names the bus endpoint and the read-only store query of its samples, to fit from on /api/db/query", true,
          None, h_bridge_companies),
        r("GET", "/api/bridge/observation", "bridgeObservation", "The latest answer of GET /api/bridge/companies as the observation a directive is bound to (research/agent-loop.md step 1): its number, game tick and date, the digest of the companies' identities and the age limit in game days; POST /api/decision/pull refuses a directive not decided from a current observation", false, None, h_bridge_observation),
        r("POST", "/api/decision/pull", "decisionPull", "Prismatic decides the bot: GET a decision directive {ai, company_id, settings} from url (default PRISMATIC_DECISION_URL), check it against research/prismatic-bridge/decision-contract.schema.json and apply it (settings on the running AI through openttd.ai.set_setting, approval-gated; else start_ai), answering a DecisionPullResult; the fit its working carries (prismatic-fit/v1) is recorded in plan.directive's inputs.stored, or the fallback when there is none", true,
          s(json!({"url": st(), "key": st()}), &[]), h_decision_pull),
        r("GET", "/api/plan", "plan", "A route plan: town pairs ranked by model v2 from the game's payment formula, vehicle speeds and base prices (tools/research/mcp-rhai src/plan.rs), each with its mode of transport (bus, rail, air), plus the best pairs of each mode; ?limit=N&params=JSON overrides any assumption (the answer echoes the effective params); &track=N prices the N best rail lines of each list on the real map with AlgoGlyph track-planning (line_costs) before the portfolio is chosen; &calibrate=1 adds `calibration`: every running line's real profit per vehicle (openttd://vehicles profit_last_year) against this plan's prediction for the same towns or industries, the median ratio per mode, and `params.calibration` to pass back as params (src/calibrate.rs); with track, &yards=N proves up to N transfer yards offline (yards: can the other line still leave from a platform beside the better line's, its tiles blocked; src/yards.rs)", true,
          None, h_plan),
        r("GET", "/api/investment", "investment", "What to build, decided by AlgoGlyph's transport-investment kernel (routes of one or two legs through hubs, ranked by return on investment) over the planner's candidates from the live game, one run per cargo with that cargo's own vehicles, then one portfolio within the budget (default: the loan limit); ?params=JSON as /api/plan. The planner's own portfolio comes alongside for comparison", true,
          None, h_investment),
        r("POST", "/api/actionplan", "actionPlan", "Plan before acting: a set of actions with dependencies ({id, kind, label?, cost?, income?, amount?+step?, repeat?, after?}) turned by AlgoGlyph's action-plan kernel into a validated DAG and executable presses (repay 50k = 5 x repay 10k as one plan) with the cash and loan after each, refused if it would overdraw; cash and loan default to the bot company's newest day in the store (the console's companies line when the store has none; plan.action names which), the loan limits to the game's", true,
          s(json!({"actions": {"type": "array"}, "cash": {"type": "number"}, "loan": {"type": "number"}, "max_loan": {"type": "number"},
                   "loan_step": {"type": "number"}, "company": {"type": "integer"}}), &["actions"]), h_action_plan),
        r("POST", "/api/intent", "intent", "A sentence from the owner (\"najdi nejbližší les + pilu\", \"vybuduj nejlukrativnější trať\", \"find the most productive coal mine\") answered over the live game: nouns from the game's own language files and world graph, orderings nearest / most productive / most profitable, candidates in order, and for build the bot settings that carry out the first (tools/research/mcp-rhai src/intent.rs)", true,
          s(json!({"text": st(), "params": obj()}), &["text"]), h_intent),
        r("GET", "/api/graph", "graph", "The road network as a graph (nodes: junctions, ends, stations, town anchors; edges with length; components), derived from openttd://map, towns and stations by tools/research/mcp-rhai mcp-graph", true,
          None, h_graph),
        r("GET", "/api/instances", "instances", "Every OpenTTD MCP instance in the cluster (panels and GUI clients) from the shared registry, each checked with a live probe", true,
          None, h_instances),
        r("GET", "/api/mesh", "mesh", "The Majordomus mesh this repository's server cooperates with (src/panel_mesh_page.rs, read-only): `mesh` (identity, trust policy, providers, tallies), `peers` (machines, runtimes, sessions, claims) and `nodes` (every discovered node with trust, presence and endpoints), read from the Majordomus server ($MAJORDOMUS_URL, else the one `majordomus serve ensure` names for this checkout) /api/v1/mesh, /peers and /nodes unchanged; 200 also when Majordomus is down (`reachable` false, `errors` per part)", false,
          None, crate::panel_mesh_page::h_mesh),
        // The strategist and the blackboard (src/panel_strategy.rs, research/mechanisms/strategist-and-blackboard.md).
        r("GET", "/api/strategy", "strategy", "The background strategist: enabled, running, cycles, its configuration (PRISMATIC_STRATEGIST, PRISMATIC_STRATEGY_*), strategy.status, strategy.summary, the last cycle with each part's duration and error (plan, track, towns, tuning), the next run, and the current strategy from the blackboard: portfolio (AlgoGlyph knapsack-dp or the built-in DP, named), AlgoGlyph transport-investment, track-planning of the best rail lines, town warnings per rated company, BattleBrain's tuning proposals, a bot settings suggestion (data only, never applied) and the mesh peers' plans. Reads and computes only; the game is never changed", false,
          None, crate::panel_strategy::h_strategy),
        r("POST", "/api/strategy/run", "strategyRun", "Run one strategist cycle now (also with PRISMATIC_STRATEGIST=0): with wait: true the answer is the cycle's record (parts, durations, errors, the plan.strategy event's sequence number), else 202 and the cycle runs in the background; 409 without a game", false,
          s(json!({"wait": {"type": "boolean"}}), &[]), crate::panel_strategy::h_strategy_run),
        r("GET", "/api/blackboard", "blackboard", "The panel's blackboard, the shared working state of this instance: one key (?key=, 404 when absent) or every key under ?prefix=, each with its value, writer (by), revision, wall time (written_at), game date and its last values (history; ?history=0 leaves them out). strategy.* is the strategist's, mesh.* what it read from the Majordomus mesh. In the panel's memory: a restarted panel starts empty", false,
          None, crate::panel_strategy::h_blackboard),
        r("POST", "/api/blackboard", "blackboardWrite", "Write a blackboard key for scripts, pages and services to share ({key, value, by?}); the reserved strategy.* and mesh.* keys are refused (403). Published as a blackboard.write event", false,
          s(json!({"key": st(), "value": {}, "by": st()}), &["key", "value"]), crate::panel_strategy::h_blackboard_write),
        r("GET", "/api/battle/contestants", "battleContestants", "The LAN battle's contestants as tools/research/ai-battle/battle.json declares them (src/panel_battle.rs, read-only, read on every request): `file`, `count` and `contestants` in the file's order, each as the file has it (ai, label, source, group) with `settings` made explicit (\"\" for the AI's defaults); the live arena (rhai/arena.rhai) plays this lineup, so it is written in one place. 404 when the file is missing", false,
          None, crate::panel_battle::h_contestants),
        r("POST", "/api/instances/call", "instancesCall", "One MCP JSON-RPC call on a named instance of the cluster (a panel's server or a GUI client)", true,
          s(json!({"name": st(), "method": st(), "params": obj()}), &["name", "method"]), h_instances_call),
        r("GET", "/api/news", "news", "The news the owner's game window has shown, newest first (openttd.view.news.list on the GUI client); ?limit=N", true,
          None, h_news),
        r("POST", "/api/news", "newsShow", "Show a news message in the owner's game window, from the bot, a script or another AI (openttd.view.news.show): text, optional style normal|thin|small|company, a tile, x/y, town or vehicle for its viewport, or a company (and title) for the manager's face", true,
          s(json!({"text": st(), "style": {"type": "string", "enum": ["normal", "thin", "small", "company"]}, "company": {"type": "integer"},
                   "title": {"type": "string", "enum": ["none", "launch", "trouble", "merger", "bankrupt"]}, "tile": {"type": "integer"},
                   "x": {"type": "integer"}, "y": {"type": "integer"}, "town": {"type": "integer"}, "vehicle": {"type": "integer"}}), &["text"]), h_news_show),
        r("GET", "/api/ui/classes", "uiClasses", "Window class numbers to names, derived from src/window_type.h (for openttd.ui.windows)", false,
          None, h_ui_classes),
        r("POST", "/api/ask", "ask", "Ask the player a yes/no question in the game window (openttd.view.ask); the answer arrives as event ask.answered and in GET /api/ask: yes, no (also a closed dialog), or replaced by a newer question", true,
          s(json!({"question": st(), "caption": st()}), &["question"]), h_ask),
        r("GET", "/api/ask", "askAnswers", "Every question asked in the game window, newest first, with its answer (yes, no, replaced, or null while open)", true,
          None, h_ask_answers),
        r("POST", "/api/news/close", "newsClose", "Close the news window in the owner's game", true, None, h_news_close),
        r("POST", "/api/view/attach", "viewAttach", "Attach the GUI client the owner watches, by its MCP token file", false,
          s(json!({"token_file": st()}), &["token_file"]), h_view_attach),
        r("POST", "/api/view/tool", "viewTool", "Call a tool on the attached GUI client's own MCP server (openttd.view.* and the rest)", false,
          s(json!({"name": st(), "arguments": obj()}), &["name"]), h_view_tool),
        r("POST", "/api/rhai", "rhai", "Run a Rhai script against this panel, in the panel's own process (tools/research/mcp-rhai)", false,
          s(json!({"script": st(), "timeout": {"type": "number"}}), &["script"]), h_rhai),
        // The script library as a workspace (src/panel_scripts.rs, the Scripts page /scripts).
        r("GET", "/api/scripts", "scripts", "Every Rhai script of the library ($MCP_RHAI_PATH, else tools/research/mcp-panel/rhai): its header comment as documentation, the libraries it imports, the scripts importing it (a library), and its service state", false, None, crate::panel_scripts::h_scripts),
        r("GET", "/api/script", "script", "One script (?name=): its text, documentation, imports, modification time and the parser's verdict (error with line and column, or null)", false, None, crate::panel_scripts::h_script),
        r("POST", "/api/script/check", "scriptCheck", "Parse a text as Rhai without running or saving it: the error with its line and column, or null", false,
          s(json!({"text": st()}), &["text"]), crate::panel_scripts::h_script_check),
        r("POST", "/api/script/save", "scriptSave", "Save a script into the library (only text that parses; 422 with the error otherwise). A running service on it, or on a script importing it, reloads within a second. Committing stays the owner's", false,
          s(json!({"name": st(), "text": st()}), &["name", "text"]), crate::panel_scripts::h_script_save),
        r("GET", "/api/rhai/functions", "rhaiFunctions", "Every function a Rhai script of this panel can call (src/rhai_catalogue.rs, I0088): host functions with the doc of src/main.rs, one per MCP tool with its description and input schema, one per OpenAPI operation with its route, one per console command, and every non-private function of each library script with its /// comments; derived at most every 30 s (?fresh=1 derives now)", false, None, crate::rhai_catalogue::h_functions),
        // Memory the scripts and services share (src/rhai_shared.rs, I0091): every write a rhai.shared event.
        r("GET", "/api/rhai/shared", "rhaiShared", "The memory Rhai scripts and services share: one key (?key=, 404 when absent) or every key under ?prefix=, each with its value, revision, declared writer and time. In the panel's memory: a restarted panel starts empty; the store keeps every write as a rhai.shared event", false, None, crate::rhai_shared::h_shared),
        r("POST", "/api/rhai/shared", "rhaiSharedSet", "Write a shared key. With rev, only if the key is at that revision (0: only if it does not exist); 409 with the current revision otherwise. Published as a rhai.shared event", false,
          s(json!({"key": st(), "value": {}, "writer": st(), "rev": {"type": "integer", "minimum": 0}}), &["key", "value"]), crate::rhai_shared::h_shared_set),
        r("POST", "/api/rhai/shared/delete", "rhaiSharedDelete", "Delete a shared key (with rev, only at that revision); 409 when it is absent or at another revision. Published as a rhai.shared event", false,
          s(json!({"key": st(), "writer": st(), "rev": {"type": "integer", "minimum": 0}}), &["key"]), crate::rhai_shared::h_shared_delete),
        // The debugger (src/rhai_debug.rs, I0090): a script run as a session that pauses at breakpoints.
        r("POST", "/api/rhai/debug", "rhaiDebugStart", "Run a Rhai script as a debug session: it pauses at the given breakpoint lines; answers the session once it pauses or finishes (status, line, column, depth, the variables in scope, the call stack, printed lines, the result). A paused script never pauses the game", false,
          s(json!({"script": st(), "breakpoints": {"type": "array", "items": {"type": "integer", "minimum": 1}}}), &["script"]), crate::rhai_debug::h_debug_start),
        r("GET", "/api/rhai/debug", "rhaiDebug", "A debug session's state (?id=), or every session without one", false, None, crate::rhai_debug::h_debug),
        r("POST", "/api/rhai/debug/command", "rhaiDebugCommand", "Steer a paused debug session: continue, step (into a function), over, out (of the function), stop; breakpoints may be replaced with any command (an empty command only sets them). Answers the session once it pauses again or finishes", false,
          s(json!({"id": {"type": "integer"}, "command": st(), "breakpoints": {"type": "array", "items": {"type": "integer", "minimum": 1}}}), &["id"]), crate::rhai_debug::h_debug_command),
        // Evidence with provenance (src/evidence.rs; research/agent-loop.md, Retrieve).
        r("GET", "/api/evidence", "evidence", "Passages of this repository that answer ?q= (k results, default 5): each with its path, lines, source class (implementation, test, policy, design, knowledge, documentation, candidate, record), the commit the index was built at, the file's digest, its text and its freshness now (current, with its new lines when it moved; stale; deleted); `insufficient evidence` when no passage covers half the question's terms. Indexed from the tracked files, without .ai/local, vendored trees, binaries or secret-looking paths", false, None, h_evidence),
        r("GET", "/api/services", "services", "The scripts running as services in this panel: running, started, reloads, the last line's sequence number and the files watched for hot reload", false, None, crate::panel_scripts::h_services),
        r("POST", "/api/services/start", "serviceStart", "Start a script as a service in the background (rail_monitor, rail_steward, ...); it reloads when its file or a library it imports changes", false,
          s(json!({"name": st()}), &["name"]), crate::panel_scripts::h_service_start),
        r("POST", "/api/services/stop", "serviceStop", "Stop a service at its next operation; its log stays readable", false,
          s(json!({"name": st()}), &["name"]), crate::panel_scripts::h_service_stop),
        r("GET", "/api/services/scores", "serviceScores", "The services judged by what followed their actions (src/service_eval.rs, research/mechanisms/service-self-evaluation.md): per service and action the outcomes the horizon later (?horizon= game months, default 3; ?scope=run for the newest run with an action), a detector by its resolution and the others by the company's share of the field against its own pre-action trend, a Jeffreys posterior and the verdict (insufficient, poor, unclear, helps); the coverage of deterioration episodes derived from the samples (threshold derived from the store, declared fallback); the baselines; the eval.* keys service_eval.rhai keeps. Associative, never causal; it changes nothing", false, None, crate::service_eval::h_scores),
        r("GET", "/api/services/log", "serviceLog", "A service's printed lines after ?since=<seq> (?name=): [{seq, text}], whether it runs, the last sequence number", false, None, crate::panel_scripts::h_service_log),
    ]
}

// ---------------------------------------------------------------------- derived routes and OpenAPI

/// A route derived from a live MCP catalogue.
#[derive(Clone, Debug, PartialEq)]
pub struct Derived {
    pub method: &'static str,
    pub path: String,
    pub op_id: String,
    pub summary: String,
    pub schema: Option<Value>,
    pub item: Value,
}

/// `re.sub(r"\W", "_", name)`.
pub fn word(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// One REST route per MCP tool (POST /api/tools/<name>, the tool's inputSchema as the body), per tool
/// of the attached GUI client (POST /api/client/<name>) and per MCP resource (GET
/// /api/resources/<path of openttd://...>), from the live catalogues.
pub fn derive_routes(tools: &[Value], client_tools: &[Value], resources: &[Value]) -> Vec<Derived> {
    let schema_of = |t: &Value| Some(t.get("inputSchema").filter(|s| truthy(Some(s))).cloned().unwrap_or_else(obj));
    let desc = |t: &Value, name: &str| t.get("description").and_then(Value::as_str).unwrap_or(name).to_string();
    let mut out = Vec::new();
    for t in tools {
        let Some(name) = t["name"].as_str() else { continue };
        out.push(Derived { method: "POST", path: format!("/api/tools/{name}"), op_id: format!("tool_{}", word(name)),
                           summary: desc(t, name), schema: schema_of(t), item: t.clone() });
    }
    // The attached GUI client's own tools (openttd.ui.*, openttd.input.*, openttd.view.* and whatever it
    // gains), one route each, from its live tools/list: the owner's window is a surface like the server.
    for t in client_tools {
        let Some(name) = t["name"].as_str() else { continue };
        out.push(Derived { method: "POST", path: format!("/api/client/{name}"), op_id: format!("client_{}", word(name)),
                           summary: format!("GUI client: {}", desc(t, name)), schema: schema_of(t), item: t.clone() });
    }
    for r in resources {
        let Some(uri) = r["uri"].as_str() else { continue };
        let path = uri.split_once("://").map_or(uri, |(_, p)| p);
        let summary = [r.get("description"), r.get("name")].into_iter().flatten().find(|v| truthy(Some(v)))
            .and_then(Value::as_str).unwrap_or(uri).to_string();
        out.push(Derived { method: "GET", path: format!("/api/resources/{path}"), op_id: format!("resource_{}", word(path)),
                           summary, schema: None, item: r.clone() });
    }
    out
}

/// The query parameters a declared GET route documents, each with whether its handler requires it
/// (answers 400 or an error without it), so a client such as the hub's check knows which reads it
/// may call with no input.
fn query_params(path: &str) -> &'static [(&'static str, bool)] {
    match path {
        "/api/events" => &[("since", false), ("kinds", false), ("limit", false)],
        "/api/resource" => &[("uri", true)],
        "/api/news" => &[("limit", false)],
        "/api/plan" => &[("limit", false), ("params", false), ("track", false), ("yards", false), ("calibrate", false)],
        "/api/investment" => &[("params", false)],
        "/api/content" => &[("q", true)],
        "/api/result" => &[("name", true)],
        // panel_bench::h_bench: one record by id; without it, every record.
        "/api/bench" => &[("id", false)],
        "/api/db/query" => &[("sql", false)],
        // panel_realworld::h_scenario answers 400 without a place name.
        "/api/realworld/scenario" => &[("name", true)],
        // panel_town::h_live: one company's view; without it, the first company with a rating.
        "/api/town/live" => &[("company", false)],
        // The script workspace (panel_scripts.rs): one script, and a service's lines after a seq.
        "/api/script" => &[("name", true)],
        "/api/services/log" => &[("name", true), ("since", false)],
        "/api/services/scores" => &[("horizon", false), ("scope", false)],
        "/api/rhai/shared" => &[("key", false), ("prefix", false)],
        "/api/rhai/debug" => &[("id", false)],
        "/api/evidence" => &[("q", true), ("k", false)],
        _ => &[],
    }
}

/// The required query parameters of a declared GET route that a request lacks (query_params), so
/// the dispatcher answers 400 before any handler runs, as it does for a POST's missing fields.
pub fn missing_query(path: &str, query: &Query) -> Vec<&'static str> {
    query_params(path).iter().filter(|(n, req)| *req && query.get(n).map_or(true, str::is_empty)).map(|(n, _)| *n).collect()
}

/// The OpenAPI 3.1 document of the declared routes, the streaming routes, and the derived routes.
pub fn openapi_doc(routes: &[Route], derived: &[Derived], panel_port: u16) -> Value {
    let mut paths = Map::new();
    let mut op = |method: &str, path: &str, op_id: &str, summary: &str, schema: &Option<Value>, tag: &str, item: Option<&Value>| {
        let mut o = json!({"operationId": op_id, "summary": summary, "tags": [tag],
                           "responses": {"200": {"description": "OK", "content": {"application/json": {"schema": obj()}}}}});
        if let Some(sc) = schema {
            o["requestBody"] = json!({"required": true, "content": {"application/json": {"schema": sc}}});
        }
        let q = if method == "GET" { query_params(path) } else { &[] };
        if !q.is_empty() {
            o["parameters"] = Value::Array(q.iter().map(|(n, req)| json!({"name": n, "in": "query", "required": req, "schema": {"type": "string"}})).collect());
        }
        if let Some(a) = item.and_then(|i| i.get("annotations")).filter(|a| truthy(Some(a))) {
            o["x-mcp-annotations"] = a.clone();
        }
        let entry = paths.entry(path.to_string()).or_insert_with(|| json!({}));
        entry[method.to_ascii_lowercase()] = o;
    };
    for r in routes {
        op(r.method, r.path, r.op_id, r.summary, &r.schema, "panel", None);
    }
    for d in derived {
        let tag = if d.path.starts_with("/api/client/") { "gui client tools" } else if d.method == "POST" { "mcp tools" } else { "mcp resources" };
        op(d.method, &d.path, &d.op_id, &d.summary, &d.schema, tag, Some(&d.item));
    }
    paths.insert("/api/rhai/stream".into(), json!({"post": {"operationId": "rhaiStream", "tags": ["panel"],
        "summary": "Run a Rhai script in the panel's process and stream its output line by line (SSE: out, err, exit)",
        "requestBody": {"required": true, "content": {"application/json": {"schema": {
            "type": "object", "properties": {"script": {"type": "string"}}, "required": ["script"]}}}},
        "responses": {"200": {"description": "text/event-stream"}}}}));
    paths.insert("/api/view/screenshot".into(), json!({"get": {"operationId": "viewScreenshot", "tags": ["view"],
        "summary": "What the attached GUI client shows right now, as PNG", "responses": {"200": {"description": "image/png"}}}}));
    paths.insert("/play".into(), json!({"get": {"operationId": "playPage", "tags": ["play"],
        "summary": "Play the running game in the browser: the real OpenTTD client in WebAssembly, fullscreen (research/play-in-browser.md)",
        "responses": {"200": {"description": "text/html"}}}}));
    paths.insert("/api/play".into(), json!({"get": {"operationId": "playState", "tags": ["play"],
        "summary": "Whether the WebAssembly client is built, where, and the bridge it joins through",
        "responses": {"200": {"description": "application/json"}}}}));
    paths.insert("/play/{file}".into(), json!({"get": {"operationId": "playFile", "tags": ["play"],
        "summary": "A file of the built WebAssembly client: openttd.js, openttd.wasm or openttd.data",
        "parameters": [{"name": "file", "in": "path", "required": true, "schema": {"type": "string", "enum": ["openttd.js", "openttd.wasm", "openttd.data"]}}],
        "responses": {"200": {"description": "the file"}, "404": {"description": "not built"}}}}));
    // Not a read: a player's commands travel over it (readOnlyHint false keeps the hub from probing it).
    paths.insert("/ws/game".into(), json!({"get": {"operationId": "playBridge", "tags": ["play"], "x-mcp-annotations": {"readOnlyHint": false},
        "summary": "WebSocket upgrade: raw bytes to and from the game server (127.0.0.1:<game port>), the panel's own pages only (Origin), subprotocol binary",
        "responses": {"101": {"description": "switching protocols"}, "403": {"description": "foreign origin"}, "503": {"description": "too many bridges"}}}}));
    paths.insert("/api/events/stream".into(), json!({"get": {"operationId": "eventStream", "tags": ["events"],
        "summary": "Server-sent events: every event as it is published (?since=, ?kinds=)",
        "responses": {"200": {"description": "text/event-stream"}}}}));
    json!({"openapi": "3.1.0",
           "info": {"title": "OpenTTD MCP panel (RESEARCH-ONLY)", "version": "1",
                    "description": "Declared once in tools/research/mcp-rhai/src/panel.rs (routes()) and derived from the live \
                                    MCP catalogues: every MCP tool and resource of the running game, and every tool \
                                    of the attached GUI client (/api/client/...), is a route here."},
           "servers": [{"url": format!("http://127.0.0.1:{panel_port}")}], "paths": paths})
}

/// The derived routes of the running game, or none when the catalogue cannot be read.
fn live_derived(g: &Game) -> Vec<Derived> {
    if !g.mcp_up.load(Ordering::SeqCst) {
        return vec![];
    }
    let list = |method: &str, key: &str| -> Result<Vec<Value>, String> {
        Ok(g.rpc(method, None)?.get("result").and_then(|r| r.get(key)).and_then(Value::as_array).cloned().unwrap_or_default())
    };
    match (list("tools/list", "tools"), list("resources/list", "resources")) {
        (Ok(t), Ok(r)) => derive_routes(&t, &g.client_tools(), &r),
        _ => vec![],
    }
}

/// What is wrong with a request body against its declared JSON schema: not an object, a required
/// field missing, a field of the wrong type, a value outside its enum. [] when it fits. One check for
/// every POST route, declared or derived, so a bad call answers 400 and never reaches a handler.
pub fn body_errors(schema: Option<&Value>, body: &Value) -> Vec<String> {
    let Some(schema) = schema.filter(|s| truthy(Some(s))) else { return vec![] };
    let Some(b) = body.as_object() else { return vec!["the body must be a JSON object".into()] };
    let mut errs: Vec<String> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str)
        .filter(|k| !b.contains_key(*k)).map(|k| format!("missing {k}")).collect();
    for (k, spec) in schema["properties"].as_object().into_iter().flatten() {
        let (Some(v), true) = (b.get(k), spec.is_object()) else { continue };
        let want = spec.get("type").and_then(Value::as_str);
        let fits = match want {
            Some("string") => Some(v.is_string()),
            Some("integer") => Some(v.is_i64() || v.is_u64()),
            Some("number") => Some(v.is_number()),
            Some("boolean") => Some(v.is_boolean()),
            Some("object") => Some(v.is_object()),
            Some("array") => Some(v.is_array()),
            _ => None,
        };
        if fits == Some(false) {
            errs.push(format!("{k} must be {}", want.unwrap_or("")));
        } else if let Some(e) = spec.get("enum").and_then(Value::as_array) {
            if !e.contains(v) {
                errs.push(format!("{k} must be one of {}", py_repr(&Value::Array(e.clone()))));
            }
        }
    }
    errs
}

// ---------------------------------------------------------------------- Rhai in process

pub enum ScriptLine {
    Out(String),
    Err(String),
    Exit(i32),
}

/// Run a script in this process, in the engine mcp-rhai runs it in (functions derived from the panel
/// at `base`, the hub from MCP_HUB), every printed line sent as it is printed, the result value as a
/// last `Out` line, an error as `Err("mcp-rhai: ...")`, then `Exit` with mcp-rhai's exit status.
/// Setting `abort` stops the script at its next operation.
pub fn spawn_script(base: String, script: String, abort: Arc<AtomicBool>, tx: mpsc::Sender<ScriptLine>) -> thread::JoinHandle<i32> {
    let token = run_token_for(&base);
    spawn_script_as(base, token, script, abort, tx)
}

/// [`spawn_script`] holding `token`: the run token when the caller may change the game, else none,
/// so a script sent through `POST /api/rhai` has its caller's authority and no more.
pub fn spawn_script_as(base: String, token: Option<String>, script: String, abort: Arc<AtomicBool>, tx: mpsc::Sender<ScriptLine>) -> thread::JoinHandle<i32> {
    thread::spawn(move || {
        let hub = std::env::var("MCP_HUB").ok().filter(|h| !h.is_empty());
        let mut engine = rhai_host::host_as(&base, hub.as_deref(), token);
        let out = tx.clone();
        engine.on_print(move |s| for l in s.split('\n') { let _ = out.send(ScriptLine::Out(l.to_string())); });
        let out = tx.clone();
        engine.on_debug(move |s, src, pos| {
            let line = match src { Some(src) => format!("{src} @ {pos:?} | {s}"), None => format!("{pos:?} | {s}") };
            let _ = out.send(ScriptLine::Out(line));
        });
        let stop = abort.clone();
        engine.on_progress(move |_| stop.load(Ordering::SeqCst).then(|| rhai::Dynamic::from("stopped")));
        let code = match engine.eval::<rhai::Dynamic>(&script) {
            Ok(v) => {
                if !v.is_unit() {
                    for l in v.to_string().split('\n') { let _ = tx.send(ScriptLine::Out(l.to_string())); }
                }
                0
            }
            Err(e) => {
                let _ = tx.send(ScriptLine::Err(format!("mcp-rhai: {e}")));
                1
            }
        };
        let _ = tx.send(ScriptLine::Exit(code));
        code
    })
}

// ---------------------------------------------------------------------- HTTP

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Query,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// One HTTP/1.1 request from a socket: the request line, headers and a Content-Length body.
pub fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 { return None; }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break p + 4; }
        if buf.len() > 1 << 16 { return None; }
    };
    let head = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
    let length: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok()).unwrap_or(0).min(64 << 20);
    while buf.len() < end + length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 { break; }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body = buf[end..buf.len().min(end + length)].to_vec();
    let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
    Some(Request { method, path: path.to_string(), query: Query::parse(qs), headers, body })
}

fn send_json(stream: TcpStream, code: u16, v: &Value) {
    // 204 carries no body at all (RFC 9110 15.3.5), so a browser's fetch reads none.
    if code == 204 { return hub::respond(stream, 204, "application/json", b""); }
    hub::respond(stream, code, "application/json", py_json(v).as_bytes());
}

fn sse_head(stream: &mut TcpStream) -> std::io::Result<()> {
    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\nCache-Control: no-store\r\n{}\r\n", crate::web_access::headers());
    stream.write_all(head.as_bytes())
}

/// Server-sent events from the bus: every event after ?since= (default: from now on) that passes
/// ?kinds=, a keep-alive comment every 15 s of silence.
fn stream_events(g: &Game, mut stream: TcpStream, q: &Query) {
    let mut since = match q.int("since", g.bus.seq() as i64) {
        Ok(s) => s.max(0) as u64,
        Err(e) => return send_json(stream, 502, &json!({"error": e})),
    };
    let kinds = kinds_of(q.str_or("kinds", ""));
    if sse_head(&mut stream).is_err() { return; }
    loop {
        g.bus.wait_past(since, Duration::from_secs(15));
        let (batch, next) = g.bus.stream_step(since, &kinds);
        since = next;
        let mut text = String::new();
        if batch.is_empty() { text.push_str(": keep-alive\n\n"); }
        for e in &batch {
            text.push_str(&format!("id: {}\nevent: {}\ndata: {}\n\n", e["seq"], e["kind"].as_str().unwrap_or(""), py_json(e)));
        }
        if stream.write_all(text.as_bytes()).and_then(|_| stream.flush()).is_err() { return; }
    }
}

/// Run a Rhai script and stream every line it prints as it is printed (SSE: event "out", "err", then
/// "exit" with the status). A closed connection stops the script.
fn stream_rhai(g: &Game, mut stream: TcpStream, body: &Value, who: &Who) {
    let script = py_str(body.get("script").or(Some(&json!(""))));
    if sse_head(&mut stream).is_err() { return; }
    let (tx, rx) = mpsc::channel();
    let abort = Arc::new(AtomicBool::new(false));
    let runner = spawn_script_as(g.panel_url(), g.script_token(who), script.clone(), abort.clone(), tx);
    g.event("rhai", &format!("{} (streamed)", head(&script, 80)));
    for line in rx {
        let (kind, data) = match line {
            ScriptLine::Out(l) => { g.to_owner_console(&format!("[rhai] {l}"), "rhai"); ("out", py_json(&json!(l))) }
            ScriptLine::Err(l) => { g.to_owner_console(&format!("[rhai] {l}"), "error"); ("err", py_json(&json!(l))) }
            ScriptLine::Exit(c) => ("exit", c.to_string()),
        };
        if stream.write_all(format!("event: {kind}\ndata: {data}\n\n").as_bytes()).and_then(|_| stream.flush()).is_err() {
            abort.store(true, Ordering::SeqCst);
            break;
        }
    }
    let _ = runner.join();
}


/// The front-end packages the pages load, from the panel's npm ci (node_modules).
fn vendor(path: &str) -> Option<(&'static str, &'static str)> {
    Some(match path {
        "/vendor/flowbite.min.js" => ("flowbite/dist/flowbite.min.js", "text/javascript"),
        "/vendor/echarts.min.js" => ("echarts/dist/echarts.min.js", "text/javascript"),
        "/vendor/swagger-ui/swagger-ui.css" => ("swagger-ui-dist/swagger-ui.css", "text/css"),
        "/vendor/swagger-ui/swagger-ui-bundle.js" => ("swagger-ui-dist/swagger-ui-bundle.js", "text/javascript"),
        _ => return None,
    })
}

/// The page key of a path that names a page file: "/battle" -> "battle"; anything else (steps,
/// dots, upper case, a deeper path) is None.
fn page_key(path: &str) -> Option<&str> {
    let key = path.strip_prefix('/')?;
    (!key.is_empty() && key.len() <= 32 && key.bytes().all(|b| b.is_ascii_lowercase())).then_some(key)
}

/// What the attached GUI client shows right now, as PNG (openttd.view.screenshot).
fn screenshot(g: &Game) -> Result<Vec<u8>, String> {
    let shot = g.view_call("openttd.view.screenshot", json!({}))?;
    let f = PathBuf::from(shot["path"].as_str().ok_or_else(|| format!("no path in {}", py_json(&shot)))?);
    let mtime = |f: &Path| std::fs::metadata(f).and_then(|m| m.modified()).ok();
    let before = mtime(&f);
    // The client captures on its drawing thread; the file appears within a frame.
    for _ in 0..40 {
        if let Some(m) = mtime(&f) {
            if before.map_or(true, |b| m > b) { break; }
        }
        thread::sleep(Duration::from_millis(50));
    }
    std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))
}

pub struct Panel {
    pub game: Arc<Game>,
    pub routes: Vec<Route>,
}

/// `&`, `<`, `>` and `"` as entities, for text put into a page.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A page's title and description from the live catalogue, and whether the item exists:
/// /tools the catalogue itself, /tool/<name> a server tool, /client/<name> a GUI client tool,
/// /resource/<path> the resource openttd://<path>.
fn page_meta(g: &Arc<Game>, path: &str) -> (String, String, bool) {
    let find = |list: Vec<Value>, key: &str, want: &str| list.into_iter().find(|t| t[key] == json!(want))
        .map(|t| t["description"].as_str().unwrap_or("").to_string());
    if path == "/tools" {
        return ("MCP catalogue".into(), "Every tool, GUI client tool and resource of the game's MCP server, each with its own page".into(), true);
    }
    if let Some(name) = path.strip_prefix("/tool/") {
        let name = percent_decode(name);
        return match find(g.server_tools(), "name", &name) { Some(d) => (name, d, true), None => (format!("{name}: not in the catalogue"), String::new(), false) };
    }
    if let Some(name) = path.strip_prefix("/client/") {
        let name = percent_decode(name);
        return match find(g.client_tools(), "name", &name) { Some(d) => (name, d, true), None => (format!("{name}: no GUI client offers it"), String::new(), false) };
    }
    let rest = percent_decode(path.strip_prefix("/resource/").unwrap_or(""));
    let uri = format!("openttd://{rest}");
    let resources = g.rpc("resources/list", None).ok()
        .and_then(|r| r.pointer("/result/resources").and_then(Value::as_array).cloned()).unwrap_or_default();
    match find(resources, "uri", &uri) { Some(d) => (uri, d, true), None => (format!("{uri}: not in the catalogue"), String::new(), false) }
}

impl Panel {
    pub fn handle(&self, mut stream: TcpStream) {
        // The browser's OpenTTD (WASM) joins the game through this bridge (src/panel_play.rs). It is
        // told apart before the request is read, because the WebSocket handshake needs it unread.
        if crate::panel_play::is_game_upgrade(&stream) {
            return crate::panel_play::bridge(stream, self.game.args.game_port);
        }
        let Some(mut req) = read_request(&mut stream) else { return };
        // Who may ask, from where (src/web_access.rs, ADR 0038): a preflight or a refusal is answered here.
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        // The address the request came from, for the governance audit (Who): the panel's own header,
        // never one the client sent.
        req.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(PEER_HEADER));
        if let Some(p) = peer { req.headers.push((PEER_HEADER.to_string(), p.to_string())); }
        if let Err((code, why)) = crate::web_access::gate(&req.method, &req.headers, peer, req.query.get("token")) {
            return if code == 204 { hub::respond(stream, 204, "text/plain", b"") } else { send_json(stream, code, &json!({"error": why})) };
        }
        let g = &self.game;
        // The page opened once as ?once=<nonce> (a one-time link, POST /api/owner/once) or ?owner=<secret>
        // keeps the secret in an HttpOnly cookie, which its one API client sends with every request
        // (src/web_access.rs carries the header on). Either answers 303 to the page without the
        // query, so neither stays in the address bar or the history.
        if req.method == "GET" && (req.query.get("once").is_some() || req.query.get("owner").is_some()) {
            let cookie = match req.query.get("once") {
                Some(n) => {
                    if !spend_once(&mut g.owner_nonces.lock().unwrap(), n, Instant::now()) {
                        return send_json(stream, 401, &json!({"error": "that sign-in link is spent, expired or unknown: ask for a new one (POST /api/owner/once)",
                                                             "refused": "owner_once"}));
                    }
                    let who = Who::of(g.owner_secret.as_deref(), g.args.panel_port, &req.headers);
                    g.governance("owner_session", json!(false), json!(true), "GET ?once=", &Who { credential: "token", role: "owner", ..who });
                    g.owner_secret.as_deref().map(|s| owner_set_cookie(g.args.panel_port, s))
                }
                None => owner_cookie(g.owner_secret.as_deref(), g.args.panel_port, req.query.get("owner")),
            };
            crate::web_access::set_headers(&format!("{}{}Location: {}\r\n", crate::web_access::headers(), cookie.unwrap_or_default(), req.path));
            return hub::respond(stream, 303, "text/plain", b"");
        }
        let file = |stream: TcpStream, f: PathBuf, ctype: &str| match std::fs::read(&f) {
            Ok(b) => hub::respond(stream, 200, ctype, &b),
            Err(_) => send_json(stream, 404, &json!({"error": "not found"})),
        };
        let body = match req.method.as_str() {
            "GET" => Value::Null,
            "POST" => {
                let text = if req.body.is_empty() { &b"{}"[..] } else { &req.body[..] };
                match serde_json::from_slice::<Value>(text) {
                    Ok(v) => v,
                    Err(_) => return send_json(stream, 400, &json!({"error": "body is not JSON"})),
                }
            }
            _ => return send_json(stream, 501, &json!({"error": format!("unsupported method {}", req.method)})),
        };
        let path = req.path.as_str();
        if req.method == "POST" && path == "/api/rhai/stream" {
            if !body.is_object() { return send_json(stream, 400, &json!({"error": "invalid body: the body must be a JSON object"})); }
            let who = Who::of_run(g.owner_secret.as_deref(), g.run_token.as_deref().ok(), g.args.panel_port, &req.headers);
            return stream_rhai(g, stream, &body, &who);
        }
        if req.method == "GET" {
            match path {
                "/" | "/index.html" => return file(stream, statics().join("index.html"), "text/html; charset=utf-8"),
                "/ui" => return file(stream, statics().join("ui.html"), "text/html; charset=utf-8"),
                "/world" => return file(stream, statics().join("world.html"), "text/html; charset=utf-8"),
                "/play" => return file(stream, statics().join("play.html"), "text/html; charset=utf-8"),
                "/api/play" => {
                    let dir = crate::panel_play::play_dir(&here().join("../../.."));
                    return send_json(stream, 200, &json!({"built": crate::panel_play::play_built(&dir), "dir": dir.display().to_string(),
                        "game_port": g.args.game_port, "bridge": "/ws/game", "max_bridges": crate::panel_play::MAX_BRIDGES}));
                }
                "/docs" => return hub::respond(stream, 200, "text/html; charset=utf-8", hub::swagger_page("OpenTTD MCP API").as_bytes()),
                "/openapi.json" => return send_json(stream, 200, &openapi_doc(&self.routes, &live_derived(g), g.args.panel_port)),
                "/api/view/screenshot" => return match screenshot(g) {
                    Ok(png) => hub::respond(stream, 200, "image/png", &png),
                    Err(e) => send_json(stream, 503, &json!({"error": e})),
                },
                "/api/events/stream" => return stream_events(g, stream, &req.query),
                _ => {}
            }
            // One page per tool, GUI client tool and resource, and the catalogue of them: stable
            // addresses (/tool/<name>, /client/<name>, /resource/<path>, /tools) whose title and
            // description come from the live catalogue, so a link opens to its own page.
            // The duels: /results lists them, /result/<name> is one, titled from its own record.
            if path == "/results" || path.starts_with("/result/") {
                let (title, description, found) = panel_results::page_meta(path);
                return match std::fs::read_to_string(statics().join("results.html")) {
                    Ok(t) => {
                        let html = t.replace("__TITLE__", &html_escape(&title)).replace("__DESCRIPTION__", &html_escape(&description));
                        hub::respond(stream, if found { 200 } else { 404 }, "text/html; charset=utf-8", html.as_bytes())
                    }
                    Err(_) => send_json(stream, 404, &json!({"error": "static/results.html is missing"})),
                };
            }
            // The benchmark records: /bench is every bench, /bench/<group>/<name> one, titled from the records.
            if path == "/bench" || path.starts_with("/bench/") {
                let (title, description, found) = panel_bench::page_meta(&percent_decode(path));
                return match std::fs::read_to_string(statics().join("bench.html")) {
                    Ok(t) => {
                        let html = t.replace("__TITLE__", &html_escape(&title)).replace("__DESCRIPTION__", &html_escape(&description));
                        hub::respond(stream, if found { 200 } else { 404 }, "text/html; charset=utf-8", html.as_bytes())
                    }
                    Err(_) => send_json(stream, 404, &json!({"error": "static/bench.html is missing"})),
                };
            }
            // The script workspace: an editor for the Rhai library, its services and their logs
            // (static/scripts.html, src/panel_scripts.rs); /scripts/<name> opens one script.
            if path == "/scripts" || path.starts_with("/scripts/") {
                let name = percent_decode(path.strip_prefix("/scripts/").unwrap_or(""));
                let (title, description) = if name.is_empty() {
                    ("Scripts".to_string(), "The Rhai library: edit with highlighting and live checks, run once, or run as a service that reloads on every change".to_string())
                } else {
                    (format!("{name}.rhai"), crate::panel_scripts::list(&crate::rhai_host::rhai_dir()).into_iter()
                        .find(|s| s["name"] == json!(name)).and_then(|s| s["doc"].as_str().map(|d| d.lines().next().unwrap_or("").to_string())).unwrap_or_default())
                };
                return match std::fs::read_to_string(statics().join("scripts.html")) {
                    Ok(t) => {
                        let html = t.replace("__TITLE__", &html_escape(&title)).replace("__DESCRIPTION__", &html_escape(&description))
                            .replace("__IMPORTMAP__", crate::panel_npm::import_map_tag(&here().join("node_modules")));
                        hub::respond(stream, 200, "text/html; charset=utf-8", html.as_bytes())
                    }
                    Err(_) => send_json(stream, 404, &json!({"error": "static/scripts.html is missing"})),
                };
            }
            // The real-world places: /realworld lists them, /realworld/<name> is one, titled from its snapshot.
            if path == "/realworld" || path.starts_with("/realworld/") {
                let (title, description, found) = panel_realworld::page_meta(path);
                return match std::fs::read_to_string(statics().join("realworld.html")) {
                    Ok(t) => {
                        let html = t.replace("__TITLE__", &html_escape(&title)).replace("__DESCRIPTION__", &html_escape(&description));
                        hub::respond(stream, if found { 200 } else { 404 }, "text/html; charset=utf-8", html.as_bytes())
                    }
                    Err(_) => send_json(stream, 404, &json!({"error": "static/realworld.html is missing"})),
                };
            }
            // The town authority: the derived model and the live towns with advice (src/panel_town.rs).
            if path == "/towns" {
                return match std::fs::read_to_string(statics().join("towns.html")) {
                    Ok(t) => {
                        let html = t.replace("__TITLE__", "Towns")
                            .replace("__DESCRIPTION__", "What a town's local authority rewards, punishes and refuses, derived from the engine sources, and the live towns with advice per company");
                        hub::respond(stream, 200, "text/html; charset=utf-8", html.as_bytes())
                    }
                    Err(_) => send_json(stream, 404, &json!({"error": "static/towns.html is missing"})),
                };
            }
            // The store's page: queries kept in the address, numeric columns charted (static/data.html).
            if path == "/data" {
                return match std::fs::read_to_string(statics().join("data.html")) {
                    Ok(t) => {
                        let html = t.replace("__TITLE__", "Data")
                            .replace("__DESCRIPTION__", "Read-only queries on this instance's DuckDB store: runs, events and samples");
                        hub::respond(stream, 200, "text/html; charset=utf-8", html.as_bytes())
                    }
                    Err(_) => send_json(stream, 404, &json!({"error": "static/data.html is missing"})),
                };
            }
            if path == "/tools" || path.starts_with("/tool/") || path.starts_with("/client/") || path.starts_with("/resource/") {
                let (title, description, found) = page_meta(g, path);
                return match std::fs::read_to_string(statics().join("tool.html")) {
                    Ok(t) => {
                        let html = t.replace("__TITLE__", &html_escape(&title)).replace("__DESCRIPTION__", &html_escape(&description));
                        hub::respond(stream, if found { 200 } else { 404 }, "text/html; charset=utf-8", html.as_bytes())
                    }
                    Err(_) => send_json(stream, 404, &json!({"error": "static/tool.html is missing"})),
                };
            }
            // Any file of an installed npm package, for ES modules a page imports through its
            // import map (src/panel_npm.rs; the Scripts page's CodeMirror, I0089).
            if let Some((f, ctype)) = crate::panel_npm::npm_file(&here().join("node_modules"), path) {
                return file(stream, f, ctype);
            }
            if let Some((rel, ctype)) = vendor(path) {
                return match std::fs::read(here().join("node_modules").join(rel)) {
                    Ok(b) => hub::respond(stream, 200, ctype, &b),
                    Err(_) => send_json(stream, 404, &json!({"error": "run npm ci in tools/research/mcp-panel"})),
                };
            }
            // A page is its file: /<key> serves static/<key>.html when it exists (a lower-case key,
            // no path steps), so a new page of the shell's registry needs no route of its own here.
            if let Some(key) = page_key(path) {
                let f = statics().join(format!("{key}.html"));
                if f.is_file() { return file(stream, f, "text/html; charset=utf-8"); }
            }
            if let Some(name) = path.strip_prefix("/play/") {
                let dir = crate::panel_play::play_dir(&here().join("../../.."));
                return match crate::panel_play::play_file(&dir, name) {
                    Some((f, ctype)) => file(stream, f, ctype),
                    None => send_json(stream, 404, &json!({"error": "not built: see research/play-in-browser.md"})),
                };
            }
            if let Some(rel) = path.strip_prefix("/static/") {
                let f = statics().join(rel);
                // Only files under static/: no parent steps, no absolute path.
                if !rel.split('/').any(|p| p == "..") && !rel.starts_with('/') && f.is_file() {
                    let ctype = match f.extension().and_then(|e| e.to_str()) { Some("css") => "text/css", Some("js") => "text/javascript", _ => "application/octet-stream" };
                    return file(stream, f, ctype);
                }
                return send_json(stream, 404, &json!({"error": "not found"}));
            }
        }
        let (code, out) = self.dispatch(&req, body);
        send_json(stream, code, &out);
    }

    /// The declared routes, then the derived ones: (status, JSON answer).
    pub fn dispatch(&self, req: &Request, body: Value) -> (u16, Value) {
        let g = &self.game;
        let (method, path) = (req.method.as_str(), req.path.as_str());
        let hit = self.routes.iter().find(|r| r.method == method && r.path == path);
        if method == "POST" {
            let mut schema = hit.and_then(|r| r.schema.clone());
            if hit.is_none() && (path.starts_with("/api/tools/") || path.starts_with("/api/client/")) {
                let name = path.splitn(4, '/').nth(3).unwrap_or("");
                let cat = if path.starts_with("/api/client/") { g.client_tools() } else { g.server_tools() };
                schema = cat.iter().find(|t| t["name"] == json!(name)).and_then(|t| t.get("inputSchema").cloned());
            }
            let errs = body_errors(schema.as_ref(), &body);
            if !errs.is_empty() {
                return (400, json!({"error": format!("invalid body: {}", errs.join("; "))}));
            }
            if let Err((code, mut why)) = owner_gate(g.owner_secret.as_deref(), g.args.panel_port, method, path, &body, &req.headers) {
                g.event("operator", &format!("refused {path}: no owner secret"));
                // What the page's sign-in box needs: where to sign in, and where the secret lives.
                why["sign_in"] = json!("/api/owner/login");
                why["secret_file"] = json!(g.owner_file);
                return (code, why);
            }
        } else if hit.is_some() {
            let missing = missing_query(path, &req.query);
            if !missing.is_empty() {
                let names: Vec<String> = missing.iter().map(|n| format!("?{n}=")).collect();
                return (400, json!({"error": format!("missing required query parameter: {}", names.join(", "))}));
            }
        }
        let up = g.mcp_up.load(Ordering::SeqCst);
        let result: Result<(u16, Value), String> = (|| {
            if let Some(r) = hit {
                if r.needs_mcp && !up {
                    return Ok((503, json!({"error": "MCP server not up yet"})));
                }
                let who = Who::of_run(g.owner_secret.as_deref(), g.run_token.as_deref().ok(), g.args.panel_port, &req.headers);
                let input = Input { query: req.query.clone(), body: if method == "POST" { body.clone() } else { json!({}) }, who };
                return (r.handler)(g, &input);
            }
            if !up {
                return Ok((503, json!({"error": "MCP server not up yet"})));
            }
            if method == "POST" {
                if let Some(name) = path.strip_prefix("/api/client/") {
                    return Ok(match g.view_call(name, or_empty(&body)) {
                        Ok(v) => (200, v),
                        Err(e) => (503, json!({"error": e})),
                    });
                }
                if let Some(name) = path.strip_prefix("/api/tools/") {
                    // A raw tool call names no writer: it sets no freight-line slot (panel_bot.rs).
                    if let Some(r) = panel_bot::raw_call_refusal(g, name, &body) {
                        return Ok(r);
                    }
                    let out = g.tool(name, or_empty(&body))?;
                    return Ok((rpc_status(&out), out));
                }
            }
            if method == "GET" {
                if let Some(rest) = path.strip_prefix("/api/resources/") {
                    let out = g.resource(&format!("openttd://{rest}"))?;
                    return Ok((rpc_status(&out), out));
                }
            }
            Ok((404, json!({"error": "not found"})))
        })();
        // Surface, never hide, a failing call. A route that needs the owner's window answers 503 on a
        // stack without one (headless, or the client restarting): the state is temporary, not a broken route.
        result.unwrap_or_else(|e| (if e.contains("no GUI client attached") { 503 } else { 502 }, json!({"error": e})))
    }
}

/// Serve the panel on `listener`, one thread per connection, until the process ends.
pub fn serve(listener: TcpListener, panel: Arc<Panel>) {
    for stream in listener.incoming().flatten() {
        let p = panel.clone();
        thread::spawn(move || p.handle(stream));
    }
}

/// Is something already listening on this loopback port? A second listener can bind the same
/// loopback port on macOS, and calls then reach either server at random (seen 2026-11-16).
pub fn port_busy(port: u16) -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)).is_ok()
}

pub fn free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok().map(|a| a.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// I0094: MCP_PANEL_DIR names the panel's directory; unset or empty, it is this checkout's.
    #[test]
    fn the_panel_directory_comes_from_mcp_panel_dir_else_the_build_checkout() {
        let built = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../mcp-panel"));
        assert_eq!(dir_from(None), built);
        assert_eq!(dir_from(Some("".into())), built, "an empty value is no value");
        assert_eq!(dir_from(Some("/srv/panel".into())), PathBuf::from("/srv/panel"));
        assert!(built.join("static").is_dir(), "the default holds static/");
    }

    #[test]
    fn a_failed_read_is_retried_once_and_a_command_only_when_it_never_left() {
        use ureq::ErrorKind::{ConnectionFailed, Io};
        assert!(rpc_retryable("resources/read", Io, 0));
        assert!(rpc_retryable("tools/list", Io, 0));
        assert!(!rpc_retryable("resources/read", Io, 1), "once");
        assert!(rpc_retryable("tools/call", ConnectionFailed, 0));
        assert!(!rpc_retryable("tools/call", Io, 0), "a command the server may have received is not sent twice");
    }

    #[test]
    fn the_history_keeps_one_sample_a_day_and_restarts_with_a_new_world() {
        let st = |t: i64, date: &str| json!({"game_tick": t, "game_date": date, "data": {"num_vehicles": 3, "num_towns": 9, "paused": false}});
        let cs = json!({"data": {"companies": [{"id": 0, "name": "A", "money": 500, "company_value": 900, "loan": 300}]}});
        let s0 = sample_of(&st(10, "1950-01-01"), &cs).unwrap();
        assert_eq!(s0["companies"][0]["net"], json!(200.0));
        assert_eq!(s0["vehicles"], json!(3), "an older server: the pool count");
        let newer = json!({"game_tick": 1, "game_date": "1950-01-01", "data": {"num_vehicles": 40, "fleet": {"train": 2, "road": 3, "ship": 0, "aircraft": 0, "total": 5}}});
        let s1 = sample_of(&newer, &cs).unwrap();
        assert_eq!(s1["vehicles"], json!(5), "the fleet, not the pool's wagons, parts and effects");
        assert_eq!(s1["fleet"]["road"], json!(3));
        assert!(sample_of(&json!({}), &cs).is_none());
        let mut h = VecDeque::new();
        push_sample(&mut h, s0);
        push_sample(&mut h, sample_of(&st(40, "1950-01-01"), &cs).unwrap());
        assert_eq!(h.len(), 1, "a second read of the same day replaces the first");
        assert_eq!(h[0]["t"], json!(40));
        push_sample(&mut h, sample_of(&st(80, "1950-01-02"), &cs).unwrap());
        assert_eq!(h.len(), 2);
        push_sample(&mut h, sample_of(&st(5, "1950-01-01"), &cs).unwrap());
        assert_eq!(h.len(), 1, "a lower tick is a new world");
        for d in 0..(HISTORY_MAX + 5) { push_sample(&mut h, sample_of(&st(100 + d as i64, &format!("d{d}")), &cs).unwrap()); }
        assert_eq!(h.len(), HISTORY_MAX);
    }

    #[test]
    fn page_paths_decode_and_page_text_is_escaped() {
        assert_eq!(percent_decode("openttd.ai.set_setting"), "openttd.ai.set_setting");
        assert_eq!(percent_decode("map%2Fways"), "map/ways");
        assert_eq!(percent_decode("trailing%2"), "trailing%2");
        assert_eq!(percent_decode("%C3%A9%2"), "é%2");
        assert_eq!(html_escape("<a href=\"x\">&</a>"), "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;");
    }

    #[test]
    fn a_content_state_line_is_parsed_with_names_that_hold_commas() {
        let a = content_line("10884238, AI, Not selected, AdmiralAI, 4C4D4441, 4CCD92FB8F8F01045145BE99A28E14A6").unwrap();
        assert_eq!(a, json!({"id": 10884238, "type": "AI", "state": "Not selected", "name": "AdmiralAI"}));
        let b = content_line("[2026-11-16 23:00:00] 11431842, AI, Selected, NoCAB, Bleeding Edge, 42414342, 7B5D").unwrap();
        assert_eq!(b["name"], "NoCAB, Bleeding Edge");
        assert!(content_line("id, type, state, name").is_none());
    }

    #[test]
    fn a_server_without_a_token_file_is_reached_through_the_panel_that_owns_it() {
        let recs = vec![
            json!({"kind": "panel", "name": "panel-1", "url": "http://127.0.0.1:1/", "mcp": "http://127.0.0.1:2/mcp"}),
            json!({"kind": "panel", "name": "panel-3", "url": "http://127.0.0.1:3", "mcp": "http://127.0.0.1:4/mcp"}),
        ];
        let owned = json!({"kind": "server", "endpoint": "http://127.0.0.1:2/mcp"});
        let stray = json!({"kind": "server", "endpoint": "http://127.0.0.1:9/mcp"});
        assert_eq!(owning_panel(&recs, &owned).as_deref(), Some("http://127.0.0.1:1"));
        assert_eq!(owning_panel(&recs, &stray), None);
        assert_eq!(owning_panel(&recs, &json!({"kind": "server"})), None);
    }

    fn no_op(_: &Arc<Game>, _: &Input) -> Result<(u16, Value), String> { Ok((200, json!({}))) }

    #[test]
    fn a_get_without_its_required_query_is_refused_before_its_handler() {
        assert_eq!(missing_query("/api/script", &Query::parse("")), vec!["name"]);
        assert_eq!(missing_query("/api/script", &Query::parse("name=")), vec!["name"]);
        assert!(missing_query("/api/script", &Query::parse("name=ttd")).is_empty());
        assert_eq!(missing_query("/api/services/log", &Query::parse("since=3")), vec!["name"]);
        assert!(missing_query("/api/events", &Query::parse("")).is_empty(), "optional parameters only");
        assert!(missing_query("/api/status", &Query::parse("")).is_empty());
    }

    #[test]
    fn the_route_table_is_unique_and_each_post_but_close_declares_its_body() {
        let rs = routes();
        let mut seen = HashSet::new();
        for r in &rs {
            assert!(seen.insert((r.method, r.path)), "{} {} declared twice", r.method, r.path);
            assert!(r.path.starts_with("/api/"));
            assert!(!r.op_id.is_empty() && !r.summary.is_empty());
            if r.method == "POST" && r.path != "/api/news/close" {
                assert!(r.schema.is_some(), "{} has no body schema", r.path);
            }
            if r.method == "GET" { assert!(r.schema.is_none()); }
        }
        let ids: HashSet<&str> = rs.iter().map(|r| r.op_id).collect();
        assert_eq!(ids.len(), rs.len(), "operationIds are unique");
        // server.py's table, route for route, plus the routes added after the Python panel was
        // retired: GET /api/content, POST /api/content/download, GET /api/overview/history, and the
        // decision contract's GET /api/bridge/companies and POST /api/decision/pull, and the duels
        // (GET /api/results, GET /api/result, POST and GET /api/duel), and the README scoreboard
        // (GET and POST /api/scoreboard), and the calibration store (GET and POST /api/calibration),
        // and the bus tap (GET /api/bus, GET /api/db/tables, GET and POST /api/db/query; ADR 0033),
        // and the plan events' routes (POST /api/bus/publish, POST /api/bots/planned; I0081), and the
        // script workspace (GET /api/scripts, GET /api/script, POST /api/script/check and /save, GET
        // /api/services, POST /api/services/start and /stop, GET /api/services/log; panel_scripts.rs),
        // and the real-world places (GET /api/realworld, GET /api/realworld/scenario, POST /api/realworld/compile),
        // and the town authority (GET /api/town/model, GET /api/town/live; panel_town.rs),
        // and the Rhai function catalogue (GET /api/rhai/functions; rhai_catalogue.rs, I0088), and the
        // shared memory (GET and POST /api/rhai/shared, POST /api/rhai/shared/delete; rhai_shared.rs, I0091).
        // And the debugger (POST /api/rhai/debug, GET /api/rhai/debug, POST /api/rhai/debug/command; I0090).
        // And the observation a directive is bound to (GET /api/bridge/observation; agent loop step 1).
        // And evidence retrieval (GET /api/evidence; research/agent-loop.md, Retrieve).
        // And the store's named queries (GET and POST /api/db/named; sql/queries).
        // And the strategist and its blackboard (GET /api/strategy, POST /api/strategy/run, GET and
        // POST /api/blackboard; panel_strategy.rs), and the LAN battle's contestants the live arena
        // plays (GET /api/battle/contestants; panel_battle.rs), and the Rust benchmark records
        // (GET /api/bench; panel_bench.rs).
        // And Deny beside Approve (POST /api/deny; ADR 0038), and the owner's sign-in and sign-out
        // (POST /api/owner/login, /api/owner/logout). And the approvals list with its bulk decisions
        // and standing rules (GET /api/approvals; POST /api/approve/many, /api/deny/many,
        // /api/approve/all, /api/deny/all, /api/rules, /api/rules/remove).
        // And the in-game approvers (POST /api/approvers), and the one-time sign-in link (POST /api/owner/once).
        // And the Rust benchmark records (GET /api/bench), and the service scores (GET /api/services/scores).
        assert_eq!(rs.len(), 95);
        for (m, p) in [("GET", "/api/status"), ("POST", "/api/console"), ("POST", "/api/rhai"), ("GET", "/api/news"), ("POST", "/api/news"),
                       ("GET", "/api/content"), ("POST", "/api/content/download"),
                       ("GET", "/api/ask"), ("POST", "/api/ask"), ("POST", "/api/instances/call"), ("GET", "/api/ui/classes"),
                       ("GET", "/api/bridge/companies"), ("POST", "/api/decision/pull"),
                       ("GET", "/api/scoreboard"), ("POST", "/api/scoreboard")] {
            assert!(rs.iter().any(|r| r.method == m && r.path == p), "{m} {p}");
        }
        assert!(rs.iter().filter(|r| !r.needs_mcp).all(|r| ["/api/status", "/api/log", "/api/events", "/api/console", "/api/auto-approve", "/api/owner/login", "/api/owner/logout", "/api/owner/once",
            "/api/start-ai", "/api/bots/start", "/api/ui/classes", "/api/view/attach", "/api/view/tool", "/api/rhai",
            "/api/content", "/api/content/download", "/api/overview/history", "/api/results", "/api/result", "/api/duel", "/api/scoreboard", "/api/calibration",
            "/api/bus", "/api/bus/publish", "/api/db/tables", "/api/db/query", "/api/db/named",
            "/api/scripts", "/api/script", "/api/script/check", "/api/script/save",
            "/api/services", "/api/services/start", "/api/services/stop", "/api/services/log", "/api/services/scores", "/api/rhai/functions", "/api/rhai/shared", "/api/rhai/shared/delete", "/api/rhai/debug", "/api/rhai/debug/command", "/api/bridge/observation", "/api/evidence",
            "/api/realworld", "/api/realworld/scenario", "/api/realworld/compile", "/api/town/model", "/api/town/live", "/api/mesh",
            "/api/strategy", "/api/strategy/run", "/api/blackboard", "/api/battle/contestants", "/api/bench"].contains(&r.path)));
    }

    /// A store with two runs of one instance: the older run's days must not feed the books.
    #[cfg(feature = "store")]
    fn store_with_books() -> crate::bus_sinks::db::Store {
        use crate::bus_sinks::{db, sample_rows};
        let mut st = db::Store::open_in_memory("panel-8800").unwrap();
        st.insert_run(&json!("old"), Some(1), 10, (None, None, None)).unwrap();
        let day = |t: i64, date: &str, c0: (f64, f64), c1: (f64, f64)| json!({"t": t, "date": date, "companies": [
            {"id": 0, "name": "A", "money": c0.0, "loan": c0.1, "value": 1.0, "net": 0.0, "fleet": 0},
            {"id": 1, "name": "B", "money": c1.0, "loan": c1.1, "value": 1.0, "net": 0.0, "fleet": 2}]});
        st.insert_samples(&sample_rows(&day(90000, "1960-01-01", (9.0, 9.0), (9.0, 9.0)), 20)).unwrap();
        st.insert_run(&json!("new"), Some(4242), 1000, (None, None, None)).unwrap();
        st.insert_samples(&sample_rows(&day(22376, "1950-10-13", (300000.0, 300000.0), (50000.0, 0.0)), 1001)).unwrap();
        st.insert_samples(&sample_rows(&day(22456, "1950-10-14", (298290.0, 300000.0), (51000.0, 10000.0)), 1002)).unwrap();
        st
    }

    #[cfg(feature = "store")]
    #[test]
    fn an_action_plan_takes_its_books_from_the_newest_stored_day_of_the_latest_run() {
        let st = store_with_books();
        let r = st.reader().unwrap();
        let mut inp = json!({"actions": [{"id": "r", "kind": "repay_loan", "amount": 50000}]}).as_object().cloned().unwrap();
        let mut books = books_from_caller(&inp);
        let stored = books_from_store(&mut inp, &mut books, "panel-8800", |q| r.run(q));
        assert_eq!((inp["cash"].as_f64(), inp["loan"].as_f64()), (Some(298290.0), Some(300000.0)), "company 0's newest day");
        assert_eq!(books["cash"], json!({"store": "samples", "company": 0, "game_date": "1950-10-14", "game_tick": 22456}));
        assert_eq!(books["loan"]["store"], "samples");
        assert_eq!(books["max_loan"], "missing");
        assert_eq!(books["trailing"]["days"], 2, "the older run's day is not in the window");
        let s = stored.to_json();
        assert!(s["query"].as_str().unwrap().contains("FROM samples"), "{s}");
        assert_eq!((s["name"].clone(), s["params"].clone()), (json!("kernel_window"), json!({"instance": "panel-8800", "days": ACTION_BOOK_DAYS})));
        assert_eq!(s["rows"], 4, "two days of two companies");
        // The event carries the statement and its row count.
        let e = action_event(&json!({}), &Value::Object(inp.clone()), &books, &Map::new(), &stored);
        assert_eq!(e["inputs"]["stored"]["rows"], 4);
        assert_eq!(e["inputs"]["live"]["books"]["cash"]["game_tick"], 22456);
        // A named company gets its own books; a loan the caller gave stays the caller's.
        let mut inp = json!({"actions": [], "company": 1, "loan": 7}).as_object().cloned().unwrap();
        let mut books = books_from_caller(&inp);
        books_from_store(&mut inp, &mut books, "panel-8800", |q| r.run(q));
        assert_eq!((inp["cash"].as_f64(), inp["loan"].as_i64()), (Some(51000.0), Some(7)));
        assert_eq!((books["cash"]["company"].clone(), books["loan"].clone()), (json!(1), json!("caller")));
    }

    #[test]
    fn without_a_store_the_books_stay_missing_for_the_console_and_the_event_says_fallback() {
        let mut inp = json!({"actions": []}).as_object().cloned().unwrap();
        let mut books = books_from_caller(&inp);
        let stored = books_from_store(&mut inp, &mut books, "panel-8800", |_| Err("store off: the store is not open".into()));
        assert!(inp.get("cash").is_none() && inp.get("loan").is_none(), "the console fills them, as before I0086");
        assert_eq!(books["cash"], "missing");
        let e = action_event(&json!({}), &Value::Object(inp), &books, &Map::new(), &stored);
        assert_eq!(e["inputs"]["stored"]["fallback"], "the store gave no samples: store off: the store is not open");
        // A store with no day of the asked company is a fallback too, never a silent default.
        let empty = json!({"columns": ["company", "game_tick", "game_date", "money", "loan", "value", "net", "vehicles"], "rows": []});
        let mut inp = json!({"actions": [], "company": 3}).as_object().cloned().unwrap();
        let stored = books_from_store(&mut inp, &mut books_from_caller(&json!({}).as_object().cloned().unwrap()), "panel-8800", |_| Ok(empty));
        assert_eq!(stored.to_json()["fallback"], "no stored day of company 3");
        // Both given: the store is not read at all.
        let mut inp = json!({"actions": [], "cash": 1, "loan": 0}).as_object().cloned().unwrap();
        let stored = books_from_store(&mut inp, &mut json!({}), "panel-8800", |_| panic!("not read"));
        assert_eq!(stored.to_json()["fallback"], "the caller gave cash and loan");
    }

    #[test]
    fn the_panels_decisions_publish_their_choice_as_data() {
        // plan.action: the kernel's answer, and where cash, loan and the limit came from.
        let books = json!({"cash": {"console": "companies", "company": 1}, "loan": "caller", "max_loan": "console: setting difficulty.max_loan"});
        let input = json!({"actions": [{"id": "r", "kind": "repay_loan", "amount": 50000}], "cash": 90000, "loan": 50000,
                           "max_loan": 300000, "loan_step": 10000});
        let mut out = Map::new();
        out.insert("ok".into(), json!(true));
        out.insert("result".into(), json!({"steps": [{"press": "repay"}, {"press": "repay"}]}));
        out.insert("runner".into(), json!("/x/algoglyph"));
        let e = action_event(&json!({"actions": []}), &input, &books, &out, &panel_events::Stored::Fallback("the caller gave cash and loan".into()));
        assert_eq!(e["decision"], "algoglyph.action_plan");
        assert_eq!(e["choice"]["ok"], true);
        assert_eq!(e["choice"]["steps"].as_array().unwrap().len(), 2);
        assert_eq!(e["choice"]["kernel_input"], input);
        assert_eq!(e["inputs"]["live"]["books"], books);
        assert_eq!(e["version"]["params"], json!({"loan_step": 10000, "max_loan": 300000}));
        assert_eq!(e["inputs"]["stored"]["fallback"], "the caller gave cash and loan");
        // plan.intent: the reading and the first candidates only.
        let cands: Vec<Value> = (0..8).map(|n| json!({"name": format!("mine {n}"), "production": n})).collect();
        let answer = json!({"intent": {"action": "find", "ordering": "most productive", "subjects": [{"name": "coal mine"}]},
                            "answer": {"how": "by production", "candidates": cands}, "reads": [{"uri": "openttd://towns", "ok": true}],
                            "vocabulary_size": 900});
        let mut params = Map::new();
        params.insert("budget".into(), json!(300000));
        let e = intent_event("find the most productive coal mine", "POST /api/intent", &params, &answer);
        assert_eq!(e["decision"], "intent.answer");
        assert_eq!(e["choice"]["candidates"], 8);
        assert_eq!(e["choice"]["first"].as_array().unwrap().len(), INTENT_CANDIDATES);
        assert_eq!(e["choice"]["intent"]["action"], "find");
        assert_eq!(e["inputs"]["live"]["text"], "find the most productive coal mine");
        assert_eq!(e["inputs"]["live"]["reads"][0]["uri"], "openttd://towns");
        assert_eq!(e["version"]["params"]["request"], json!({"budget": 300000}));
        // plan.calibration: the reading, the game it came from, and the factors of every stored reading
        // with it; the stored input is the query that read them, or the fallback when the store is off.
        let out = json!({"params": {"x": 1}, "calibration": {"lines": [{"mode": "rail", "ratio": 0.8}, {"mode": "bus", "why": "far"}], "modes": {}}});
        let game = json!({"seed": 3, "map": "256x256"});
        let rows = vec![json!(["panel-1", 9, 200, 3, "256x256", "[{\"mode\":\"rail\",\"ratio\":0.6}]"])];
        let before = panel_bot::calibration(&calibrate::Scope::All, &game, |_| Ok(panel_store::Across {
            columns: json!(["instance", "seq", "ts_ms", "seed", "map", "lines"]), rows: rows.clone(), stores: vec![] }));
        let e = calibration_event(&out, &game, &before);
        assert_eq!(e["decision"], "calibration.record");
        assert_eq!(e["choice"]["reading"], out["calibration"]);
        assert_eq!(e["version"]["params"]["min_age_days"], json!(calibrate::MIN_AGE_DAYS), "the rule's own constants, not a copy");
        assert_eq!(e["version"]["params"]["candidates"], json!(calibrate::CANDIDATES));
        assert_eq!(e["version"]["params"]["effective"], json!({"x": 1}));
        assert_eq!(e["choice"]["readings"], 2, "the stored one and this one");
        assert_eq!(e["choice"]["factors"], json!({"rail": 0.7, "default": 0.7}), "median of 0.6 and 0.8");
        assert_eq!(e["inputs"]["live"]["game"], game, "what the seed scope compares");
        let mut stored = calibrate::readings_query(&calibrate::Scope::All, &game).unwrap().record();
        stored["rows"] = json!(1);
        assert_eq!(e["inputs"]["stored"], stored);
        let off = panel_bot::calibration(&calibrate::Scope::All, &game, |_| Err("store off: the store is not open".into()));
        let e = calibration_event(&out, &game, &off);
        assert_eq!(e["choice"]["readings"], 1);
        let young = json!({"calibration": {"lines": [{"mode": "bus", "why": "no vehicle has a whole last year yet"}]}});
        assert_eq!(calibration_event(&young, &game, &before)["choice"]["readings"], 1, "a reading that measured nothing adds none");
        assert_eq!(e["choice"]["factors"], json!({"rail": 0.8, "default": 0.8}), "this reading alone");
        assert_eq!(e["inputs"]["stored"], json!({"fallback": "uncalibrated: store off: the store is not open"}));
        // plan.investment: AlgoGlyph's portfolio beside the planner's.
        let out = json!({"params": {"budget": 1}, "portfolio": {"chosen": [1, 2, 3]},
                         "investment": {"candidates": 12, "portfolio": {"solver": "knapsack-dp", "chosen": [{"cargo": "Coal", "mode": "rail", "roi": 1.5}]}}});
        let e = investment_event(&json!({"budget": 1}), &out);
        assert_eq!(e["decision"], "algoglyph.transport_investment");
        assert_eq!(e["choice"]["solver"], "knapsack-dp");
        assert_eq!(e["choice"]["chosen"][0]["cargo"], "Coal");
        assert_eq!(e["choice"]["planner_chosen"], 3);
        assert_eq!(e["inputs"]["live"]["candidates"], 12);
        for e in [action_event(&json!({}), &input, &books, &Map::new(), &panel_events::Stored::Fallback("x".into())), calibration_event(&json!({}), &game, &off), investment_event(&json!({}), &json!({}))] {
            for f in ["decision", "version", "inputs", "choice"] { assert!(e.get(f).is_some(), "{f}"); }
            assert!(crate::panel_events::refuse_external("plan.kernel", &e).is_none(), "the payload contract");
        }
    }

    #[test]
    fn openapi_is_derived_from_the_table_and_the_live_catalogue() {
        let tools = vec![json!({"name": "openttd.decision.validate", "description": "Validate", "inputSchema": {"type": "object", "properties": {"action": {"type": "object"}}},
                                "annotations": {"readOnlyHint": true}}),
                         json!({"name": "openttd.no_schema", "inputSchema": {}})];
        let client = vec![json!({"name": "openttd.view.state", "description": "View"})];
        let resources = vec![json!({"uri": "openttd://game/state", "name": "state"}), json!({"uri": "openttd://map", "description": "Map"})];
        let derived = derive_routes(&tools, &client, &resources);
        assert_eq!(derived.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(),
                   ["/api/tools/openttd.decision.validate", "/api/tools/openttd.no_schema", "/api/client/openttd.view.state",
                    "/api/resources/game/state", "/api/resources/map"]);
        assert_eq!(derived[0].op_id, "tool_openttd_decision_validate");
        assert_eq!(derived[1].schema, Some(json!({"type": "object"})), "an empty inputSchema becomes a bare object");
        assert_eq!(derived[2].summary, "GUI client: View");
        assert_eq!(derived[3].summary, "state");
        let rs = routes();
        let doc = openapi_doc(&rs, &derived, 18800);
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["servers"][0]["url"], "http://127.0.0.1:18800");
        let paths = doc["paths"].as_object().unwrap();
        for r in &rs {
            let op = &paths[r.path][r.method.to_ascii_lowercase()];
            assert_eq!(op["operationId"], r.op_id);
            assert_eq!(op["tags"], json!(["panel"]));
            assert_eq!(op.pointer("/requestBody/content/application~1json/schema"), r.schema.as_ref());
        }
        let v = &paths["/api/tools/openttd.decision.validate"]["post"];
        assert_eq!(v["requestBody"]["content"]["application/json"]["schema"], tools[0]["inputSchema"]);
        assert_eq!(v["x-mcp-annotations"], json!({"readOnlyHint": true}));
        assert_eq!(v["tags"], json!(["mcp tools"]));
        assert_eq!(paths["/api/client/openttd.view.state"]["post"]["tags"], json!(["gui client tools"]));
        assert_eq!(paths["/api/resources/map"]["get"]["tags"], json!(["mcp resources"]));
        assert!(paths["/api/resources/map"]["get"].get("requestBody").is_none());
        assert_eq!(paths["/api/events"]["get"]["parameters"].as_array().unwrap().len(), 3);
        assert_eq!(paths["/api/plan"]["get"]["parameters"][2]["name"], "track");
        assert_eq!(page_key("/battle"), Some("battle"));
        for bad in ["/", "/Battle", "/../x", "/a/b", "/x.html", "/battle2"] { assert_eq!(page_key(bad), None, "{bad}"); }
        for p in ["/api/rhai/stream", "/api/view/screenshot", "/api/events/stream", "/play", "/api/play", "/play/{file}", "/ws/game"] { assert!(paths.contains_key(p)); }
        assert_eq!(paths["/ws/game"]["get"]["x-mcp-annotations"]["readOnlyHint"], json!(false), "the game bridge is no read");
        // GET and POST on one path are two operations of one path item.
        assert!(paths["/api/news"].get("get").is_some() && paths["/api/news"].get("post").is_some());
        // + the declared-by-hand paths: three streams/binaries and the four Play routes (panel_play.rs).
        assert_eq!(paths.len(), rs.iter().map(|r| r.path).collect::<HashSet<_>>().len() + 7 + derived.len());
        // The hub reads the same document into the same operations.
        let ops = hub::ops_from_openapi("panel-1", &doc);
        assert!(ops.iter().any(|o| o.id == "tool_openttd_decision_validate" && o.read_only));
        // A read whose handler answers 400 without its query parameter says so, and the hub's check
        // (which calls only reads that need no input) leaves it out; an optional one stays optional.
        let query = |id: &str| ops.iter().find(|o| o.id == id).unwrap().query.clone();
        assert_eq!(query("content"), vec![("q".to_string(), true)]);
        assert_eq!(query("result"), vec![("name".to_string(), true)]);
        assert_eq!(query("readResource"), vec![("uri".to_string(), true)]);
        assert_eq!(query("script"), vec![("name".to_string(), true)]);
        assert_eq!(query("serviceLog"), vec![("name".to_string(), true), ("since".to_string(), false)]);
        assert_eq!(query("realWorldScenario"), vec![("name".to_string(), true)]);
        // A path template's input is not a query parameter: the check skips it by its path.
        assert!(ops.iter().any(|o| o.id == "playFile" && o.path.contains('{')));
        assert!(query("events").iter().all(|(_, required)| !required));
        assert!(ops.iter().any(|o| o.id == "eventStream" && o.stream));
        let _ = no_op;
    }

    #[test]
    fn bodies_are_checked_against_their_schema() {
        let rs = routes();
        let schema = |p: &str| rs.iter().find(|r| r.method == "POST" && r.path == p).unwrap().schema.clone();
        let news = schema("/api/news");
        assert!(body_errors(news.as_ref(), &json!({"text": "x", "style": "thin", "x": 3})).is_empty());
        assert_eq!(body_errors(news.as_ref(), &json!({})), ["missing text"]);
        assert_eq!(body_errors(news.as_ref(), &json!({"text": 1})), ["text must be string"]);
        assert_eq!(body_errors(news.as_ref(), &json!({"text": "x", "style": "huge"})),
                   ["style must be one of ['normal', 'thin', 'small', 'company']"]);
        assert_eq!(body_errors(news.as_ref(), &json!({"text": "x", "x": true})), ["x must be integer"], "a boolean is no integer");
        assert_eq!(body_errors(news.as_ref(), &json!({"text": "x", "x": 1.5})), ["x must be integer"]);
        assert_eq!(body_errors(news.as_ref(), &json!([1])), ["the body must be a JSON object"]);
        let console = schema("/api/console");
        assert!(body_errors(console.as_ref(), &json!({"command": "date", "wait": 0})).is_empty());
        assert_eq!(body_errors(console.as_ref(), &json!({"command": "date", "wait": "1"})), ["wait must be number"]);
        assert!(body_errors(None, &json!([1])).is_empty(), "no schema, no check");
        assert!(body_errors(Some(&json!({})), &json!("x")).is_empty(), "an empty schema checks nothing");
        let derived = json!({"type": "object", "properties": {"pause": {"type": "boolean"}, "w": {"type": ["integer", "null"]}}, "required": ["pause"]});
        assert_eq!(body_errors(Some(&derived), &json!({"pause": 1, "w": "x"})), ["pause must be boolean"]);
    }

    #[test]
    fn the_bus_numbers_filters_and_streams_events() {
        let bus = Bus::new();
        assert_eq!(bus.seq(), 0);
        bus.publish("game.month", json!({"date": "1950-02-01"}));
        bus.publish("panel.console", json!({"text": "getdate"}));
        bus.publish("gamex", json!(null));
        bus.publish("company.founded", json!({"id": 1}));
        assert_eq!(bus.seq(), 4);
        let kinds = |v: Vec<Value>| v.iter().map(|e| e["kind"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(kinds(bus.since(0, &kinds_of("game"), 500)), ["game.month"], "a kind takes its children, not a longer name");
        assert_eq!(kinds(bus.since(0, &kinds_of("panel.console,company"), 500)), ["panel.console", "company.founded"]);
        assert_eq!(kinds(bus.since(2, &[], 500)), ["gamex", "company.founded"]);
        assert_eq!(bus.since(0, &[], 2).len(), 2);
        let e = &bus.since(0, &[], 1)[0];
        assert_eq!(e["seq"], 1);
        assert!(e["time"].as_str().unwrap().len() >= 19);
        assert_eq!(e["data"]["date"], "1950-02-01");
        let (batch, next) = bus.stream_step(1, &kinds_of("company"));
        assert_eq!((kinds(batch), next), (vec!["company.founded".to_string()], 4));
        let (batch, next) = bus.stream_step(4, &[]);
        assert!(batch.is_empty() && next == 4);
        // A waiting stream wakes when an event is published.
        let bus = Arc::new(bus);
        let b = bus.clone();
        let t = thread::spawn(move || b.wait_past(4, Duration::from_secs(5)));
        thread::sleep(Duration::from_millis(50));
        bus.publish("game.paused", json!({}));
        assert_eq!(t.join().unwrap(), 5);
        assert_eq!(bus.wait_past(5, Duration::from_millis(10)), 5, "nothing new: back after the timeout");
        for _ in 0..BUS_KEEP { bus.publish("x", json!(null)); }
        assert_eq!(bus.since(0, &[], usize::MAX).len(), BUS_KEEP, "the newest 5000 are kept");
    }

    #[test]
    fn game_events_are_derived_by_difference() {
        let snap = |tick: i64, date: &str, paused: bool, vehicles: i64, companies: Value, decisions: Value| Snapshot::of(
            &json!({"game_date": date, "game_tick": tick, "data": {"paused": paused, "num_vehicles": vehicles, "num_towns": 9}}),
            &json!({"data": {"companies": companies}}), &json!({"data": {"decisions": decisions}})).unwrap();
        let a = snap(100, "1950-01-31", false, 2, json!([{"id": 0, "name": "A", "money": 5}]), json!([{"decision_id": "d1", "status": "pending"}]));
        let first = derive_game_events(None, &a);
        assert_eq!(first.iter().map(|e| e.0).collect::<Vec<_>>(), ["game.observed", "company.observed"]);
        assert_eq!(first[0].1["companies"], 1);
        let b = snap(200, "1950-02-01", true, 3, json!([{"id": 0, "name": "B", "money": 7}, {"id": 1, "name": "C"}]),
                     json!([{"decision_id": "d1", "status": "approved"}, {"decision_id": "d2"}]));
        let ev = derive_game_events(Some(&a), &b);
        assert_eq!(ev.iter().map(|e| e.0).collect::<Vec<_>>(),
                   ["game.month", "game.paused", "vehicles.count", "company.founded", "company.renamed", "decision.updated", "decision.recorded"]);
        assert_eq!(ev[0].1["companies"]["0"]["money"], 7);
        assert_eq!(ev[4].1, json!({"id": 0, "from": "A", "to": "B"}));
        let c = snap(10, "1950-02-01", true, 3, json!([{"id": 1, "name": "C"}]), json!([{"decision_id": "d1", "status": "approved"}, {"decision_id": "d2"}]));
        assert_eq!(derive_game_events(Some(&b), &c).iter().map(|e| e.0).collect::<Vec<_>>(), ["game.new_world", "company.removed"]);
        assert!(derive_game_events(Some(&c), &c).is_empty());
        assert!(Snapshot::of(&json!({"error": "x"}), &json!({}), &json!({})).is_none());
    }

    #[test]
    fn console_lines_are_read_like_server_py_reads_them() {
        let tok = "0123456789abcdef0123456789ABCDEF00";
        let (line, found) = redact(&format!("[prismatic-mcp] Bearer token (keep it secret): {tok} end"), None);
        assert_eq!(line, "[prismatic-mcp] Bearer token (held by the panel): <REDACTED> end");
        assert_eq!(found.as_deref(), Some(tok));
        assert_eq!(redact(&format!("echo {tok}"), Some(tok)).0, "echo <REDACTED>");
        assert!(find_token("Bearer token (x): abc").is_none(), "too short for a token");
        // The game prints only where the token is; the panel reads it from that file.
        let dir = std::env::temp_dir().join(format!("mcp-panel-token-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prismatic_mcp_token_8731.json");
        std::fs::write(&file, format!("{{\"agent_token\":\"{tok}\",\"endpoint\":\"http://127.0.0.1:8731/mcp\"}}\n")).unwrap();
        let said = format!("Token written to {} (mode 0600)", file.display());
        assert_eq!(find_token_file(&said), Some(file.to_str().unwrap()));
        let (line, found) = redact(&said, None);
        assert_eq!(found.as_deref(), Some(tok), "the token comes from the file the line names");
        assert_eq!(line, said, "a line without the token is shown as it is");
        assert!(find_token_file("MCP server stopped: the token file /x could not be written with mode 0600.").is_none());
        assert!(find_token_file("Token written to  (mode 0600)").is_none());
        assert_eq!(redact("Token written to /no/such/file (mode 0600)", None).1, None, "an unreadable file yields no token");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(plan_request("dbg: [prismatic-plan] 61206 najdi les"), Some((61206, "najdi les".into())));
        assert_eq!(plan_request("[prismatic-plan] x [prismatic-plan] 7 b"), Some((7, "b".into())));
        assert_eq!(plan_request("[prismatic-plan] 7 "), None);
        assert_eq!(ai_log("[2026] dbg: [script:4] [1] [W] PrismaticAI: low cash"), Some((1, "warning".into(), "PrismaticAI: low cash".into())));
        assert_eq!(ai_log("[script:4] [1] [X] no"), None);
        assert_eq!(max_loan_of("Current value for 'difficulty.max_loan' is '300000' (min: 0)"), Some(300000));
        assert_eq!(max_loan_of("max_loan is: 5000"), Some(5000));
        assert_eq!(max_loan_of("max_loan unknown"), None);
        assert_eq!(company_row("#:1(Blue) Company Name: 'Prismatic & Co.'  Year Founded: 1950  Money: -5694  Loan: 100000  Value: 1"),
                   Some((1, -5694, 100000)));
        assert!(is_approval_line("prismatic_mcp approve 999999"));
        assert!(is_approval_line("  MCP   Auto-Approve on"));
        assert!(is_approval_line("mcp auto_approve"));
        assert!(!is_approval_line("prismatic_mcp approved"));
        assert!(!is_approval_line("prismatic_mcp status"));
        assert!(!is_approval_line("mcpapprove 1"));
        assert!(!is_approval_line("prismatic_mcp deny 3"));
        // The deadline's standing approve verdict is consent in advance; keep enacts nothing.
        assert!(is_approval_line("prismatic_mcp approval_deadline 30 approve"));
        assert!(!is_approval_line("prismatic_mcp approval_deadline 30 keep"));
    }

    #[test]
    fn json_is_written_as_python_writes_it() {
        assert_eq!(py_json(&json!({"mcp_up": true, "a": [1, 2.5, null], "s": "les + pilu"})),
                   r#"{"a": [1, 2.5, null], "mcp_up": true, "s": "les + pilu"}"#);
        // Non-ASCII as \uXXXX (ensure_ascii), astral characters as a surrogate pair.
        let bs = '\\';
        let text: String = [0xe9, 0x1f642].iter().filter_map(|c| char::from_u32(*c)).collect();
        assert_eq!(py_json(&json!(text)), format!("\"{bs}u00e9{bs}ud83d{bs}ude42\""));
        assert_eq!(py_repr(&json!(["Forest", "it's"])), r#"['Forest', "it's"]"#);
        assert_eq!(py_str(Some(&json!(null))), "None");
        assert_eq!(py_int(&json!("5")), Ok(5));
        assert_eq!(py_int(&json!(true)), Ok(1));
        assert!(py_int(&json!("x")).is_err());
    }

    #[test]
    fn queries_are_parsed_as_parse_qs_parses_them() {
        let q = Query::parse("since=4&kinds=game%2Cpanel.console&params=%7B%22modes%22%3A%5B%22bus%22%5D%7D&track=&x=a+b&since=9");
        assert_eq!(q.get("since"), Some("4"));
        assert_eq!(q.get("kinds"), Some("game,panel.console"));
        assert_eq!(q.get("params"), Some(r#"{"modes":["bus"]}"#));
        assert_eq!(q.get("track"), None, "blank values are dropped");
        assert_eq!(q.int("track", 0), Ok(0));
        assert_eq!(q.get("x"), Some("a b"));
        assert!(Query::parse("limit=abc").int("limit", 1).is_err());
        assert_eq!(Query::parse("a=%zz%4").get("a"), Some("%zz%4"));
    }

    #[test]
    fn window_classes_come_from_the_header() {
        let header = "x\nenum class WindowClass : uint16_t {\n\tNone, ///< none\n\t/** main\n * - 0 = #MainWidgets\n */\n\tMainWindow = WindowClass::None,\n\
                      \tMainToolbar, // bar\n\tStatusbar,\n\tInvalid = 0xFFFF, ///< Invalid window.\n};\n";
        let v = window_classes(header).unwrap();
        assert_eq!(v["0"], json!({"name": "MainWindow", "aliases": ["None", "MainWindow"]}));
        assert_eq!(v["1"]["name"], "MainToolbar");
        assert_eq!(v["2"]["name"], "Statusbar");
        assert_eq!(v["65535"]["name"], "Invalid");
        let real = std::fs::read_to_string(here().join("../../../src/window_type.h")).unwrap();
        let v = window_classes(&real).unwrap();
        assert!(v.as_object().unwrap().len() > 100);
        assert_eq!((v["1"]["name"].as_str(), v["2"]["name"].as_str()), (Some("MainToolbar"), Some("Statusbar")));
    }

    #[test]
    fn rpc_status_maps_jsonrpc_errors() {
        assert_eq!(rpc_status(&json!({"game_tick": 3})), 200);
        assert_eq!(rpc_status(&json!({"error": {"jsonrpc": "2.0", "error": {"code": -32602, "message": "unknown"}}})), 404);
        assert_eq!(rpc_status(&json!({"error": {"jsonrpc": "2.0", "error": {"code": -32603}}})), 502);
        assert_eq!(rpc_status(&json!({"error": "refused by the tool"})), 200);
        assert_eq!(rpc_status(&json!({"ok": false, "error": {"code": 1}})), 200);
    }

    #[test]
    fn the_command_line_is_server_pys() {
        let a = parse_args(&["--bin", "/x/openttd", "--panel-port=18800", "--seed", "-3", "--auto-approve", "--no-plan-bot", "--ai", ""]
            .map(String::from)).unwrap();
        assert_eq!((a.bin.as_str(), a.panel_port, a.mcp_port, a.game_port, a.seed, a.ai.as_str(), a.auto_approve, a.plan_bot),
                   ("/x/openttd", 18800, 8731, 3979, -3, "", true, false));
        assert_eq!(a.load, None, "a new world unless --load names a savegame");
        let l = parse_args(&["--bin", "x", "--load", "/s/game.sav"].map(String::from)).unwrap();
        assert_eq!(l.load.as_deref(), Some("/s/game.sav"));
        assert!(parse_args(&["--panel-port".into(), "1".into()]).is_err(), "--bin is required");
        assert!(parse_args(&["--bin".into(), "x".into(), "--nope".into()]).is_err());
    }

    #[test]
    fn a_record_is_mcp_instance_v1() {
        let schema: Value = serde_json::from_str(&std::fs::read_to_string(here().join("../schema/mcp-instance.v1.schema.json")).unwrap()).unwrap();
        // The entries mcp-panel and h_view_attach write, bus and store included (ADR 0033).
        for r in [
            registry_record("panel-1", &json!({"kind": "panel", "role": "dedicated server + panel", "url": "http://127.0.0.1:1",
                "mcp": "http://127.0.0.1:2/mcp", "game_port": 3, "bus": "tcp://127.0.0.1:4", "store": "/x/panel-1.duckdb"})),
            registry_record("client-1", &json!({"kind": "client", "role": "GUI client (owner view)", "token_file": "/x/t.json",
                "panel": "http://127.0.0.1:1"})),
        ] {
            for k in schema["required"].as_array().unwrap() { assert!(r.get(k.as_str().unwrap()).is_some(), "{k}"); }
            for k in r.as_object().unwrap().keys() { assert!(schema["properties"].get(k).is_some(), "{k} is not in the schema"); }
            assert_eq!(r["schema"], schema["properties"]["schema"]["const"]);
            assert!(r.get("token").is_none() && r.get("agent_token").is_none());
        }
        for k in ["bus", "store"] {
            assert!(schema["properties"][k]["description"].as_str().is_some_and(|d| d.contains("ADR 0033")), "{k} is described");
        }
    }

    #[test]
    fn a_stop_kills_and_reaps_every_child_and_what_it_started() {
        static KIDS: Children = Children::new();
        let dir = std::env::temp_dir().join(format!("mcp-panel-children-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let grandchild_file = dir.join("grandchild.pid");
        let gf = grandchild_file.clone();
        // A child that started one of its own and waits on it, as mcp-plan --track waits on node.
        let h = thread::spawn(move || {
            let mut cmd = Command::new("/bin/sh");
            cmd.arg("-c").arg("sleep 60 & echo $! > \"$0\"; wait").arg(&gf);
            run_in(&KIDS, &mut cmd, None, Duration::from_secs(120), None)
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let grandchild = loop {
            if let Some(p) = std::fs::read_to_string(&grandchild_file).ok().and_then(|t| t.trim().parse::<i32>().ok()) { break p; }
            assert!(Instant::now() < deadline, "the child did not start");
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(KIDS.running(), 1);
        let started = Instant::now();
        assert_eq!(KIDS.stop(Duration::from_secs(3)), 1);
        assert_eq!(KIDS.running(), 0, "reaped");
        let r = h.join().unwrap().unwrap();
        assert_eq!(r.code, -1, "ended by a signal");
        assert!(started.elapsed() < Duration::from_secs(3), "SIGTERM was enough: {:?}", started.elapsed());
        // The grandchild got the signal through the process group (launchd or init reaps it).
        let deadline = Instant::now() + Duration::from_secs(5);
        // SAFETY: kill(pid, 0) only probes whether the process exists.
        while unsafe { libc::kill(grandchild, 0) } == 0 {
            assert!(Instant::now() < deadline, "grandchild {grandchild} survived the stop");
            thread::sleep(Duration::from_millis(20));
        }
        let mut cmd = Command::new("/bin/true");
        let refused = run_in(&KIDS, &mut cmd, None, Duration::from_secs(5), None).err().unwrap_or_default();
        assert!(refused.contains("stopping"), "nothing starts after a stop: {refused}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_script_runs_in_process_and_streams_its_lines() {
        // No panel at this address: the derived functions are absent, the built-ins remain.
        let (tx, rx) = mpsc::channel();
        let h = spawn_script("http://127.0.0.1:9".into(), "print(\"a\\nb\"); 40 + 2".into(), Arc::new(AtomicBool::new(false)), tx);
        assert_eq!(h.join().unwrap(), 0);
        let lines: Vec<String> = rx.iter().map(|l| match l { ScriptLine::Out(s) => format!("out {s}"), ScriptLine::Err(s) => format!("err {s}"), ScriptLine::Exit(c) => format!("exit {c}") }).collect();
        assert_eq!(lines, ["out a", "out b", "out 42", "exit 0"]);
        let (tx, rx) = mpsc::channel();
        assert_eq!(spawn_script("http://127.0.0.1:9".into(), "bad(".into(), Arc::new(AtomicBool::new(false)), tx).join().unwrap(), 1);
        assert!(matches!(rx.recv().unwrap(), ScriptLine::Err(s) if s.starts_with("mcp-rhai: Syntax error")));
        let (tx, _rx) = mpsc::channel();
        let abort = Arc::new(AtomicBool::new(true));
        assert_eq!(spawn_script("http://127.0.0.1:9".into(), "loop { }".into(), abort, tx).join().unwrap(), 1, "abort stops a script");
    }

    /// A GUI client's MCP server: tools/list, and tools/call answered per tool, as the game answers.
    fn fake_client() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/mcp", l.local_addr().unwrap());
        thread::spawn(move || for mut c in l.incoming().flatten() {
            let Some(req) = read_request(&mut c) else { continue };
            let auth = req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("Authorization")).map_or(String::new(), |(_, v)| v.clone());
            let call: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let text = |v: Value| json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [{"type": "text", "text": v.to_string()}]}});
            let out = if auth != "Bearer t0k" {
                json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32001, "message": "bad token"}})
            } else if call["method"] == "tools/list" {
                json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [{"name": "openttd.view.state", "inputSchema": {"type": "object"}}]}})
            } else {
                match call["params"]["name"].as_str().unwrap_or("") {
                    "openttd.view.state" => text(json!({"ok": true, "x": 64, "args": call["params"]["arguments"]})),
                    "openttd.view.ask.answers" => text(json!({"asks": [{"id": 2, "question": "r", "answer": null},
                                                                       {"id": 1, "question": "q", "answer": "yes"}]})),
                    other => text(json!({"ok": false, "error": format!("unknown tool {other}")})),
                }
            };
            hub::respond(c, 200, "application/json", out.to_string().as_bytes());
        });
        endpoint
    }

    #[test]
    fn the_owners_window_is_reached_through_its_token_file() {
        let dir = std::env::temp_dir().join(format!("mcp-panel-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let token = dir.join("view-token.json");
        std::fs::write(&token, json!({"agent_token": "t0k", "endpoint": fake_client()}).to_string()).unwrap();
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", "1"].map(String::from)).unwrap();
        let g = Game::new(args);
        let p = Panel { game: g.clone(), routes: routes() };
        let post = |path: &str, body: Value| p.dispatch(&Request { method: "POST".into(), path: path.into(), query: Query::default(),
                                                                    headers: vec![], body: vec![] }, body);
        let get = |path: &str| p.dispatch(&Request { method: "GET".into(), path: path.into(), query: Query::default(), headers: vec![], body: vec![] }, Value::Null);
        // Without a client, a route that needs one says so with 503.
        assert_eq!(post("/api/view/tool", json!({"name": "openttd.view.state"})).0, 503);
        assert!(g.client_tools().is_empty());
        *g.view_token.lock().unwrap() = Some(token.display().to_string());
        let (code, out) = post("/api/view/tool", json!({"name": "openttd.view.state", "arguments": {"a": 1}}));
        assert_eq!((code, out["ok"].clone(), out["args"].clone()), (200, json!(true), json!({"a": 1})));
        assert_eq!(post("/api/view/tool", json!({"name": "openttd.view.nope"})).1["ok"], json!(false));
        assert_eq!(post("/api/view/tool", json!({})).0, 400, "the body is checked first");
        // The client's tools are derived routes once the game's MCP is up.
        assert_eq!(post("/api/client/openttd.view.state", json!({})).0, 503, "MCP not up yet");
        g.mcp_up.store(true, Ordering::SeqCst);
        assert_eq!(post("/api/client/openttd.view.state", json!({})).1["x"], 64);
        assert_eq!(g.client_tools().len(), 1);
        assert_eq!(derive_routes(&[], &g.client_tools(), &[])[0].path, "/api/client/openttd.view.state");
        // An answered question is published once, an open one not at all.
        let since = g.bus.seq();
        assert_eq!(get("/api/ask").0, 200);
        assert_eq!(get("/api/ask").0, 200);
        let answered = g.bus.since(since, &kinds_of("ask"), 10);
        assert_eq!(answered.len(), 1);
        assert_eq!(answered[0]["data"]["question"], "q");
        // The game's own tools need its token: without one the call fails as a 502, never a hang.
        assert_eq!(post("/api/tools/openttd.game.set_pause", json!({"pause": true})).0, 502);
        assert_eq!(get("/api/resources/game/state").0, 502);
        assert_eq!(get("/api/nothing").0, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A panel without a game server, for the routes that answer without one. Its MCP counts as
    /// up, so a route that needs the game is dispatched and meets the missing server itself.
    fn bare_panel(port: &str) -> (Arc<Game>, Panel) {
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", port].map(String::from)).unwrap();
        let g = Game::new(args);
        g.mcp_up.store(true, Ordering::SeqCst);
        (g.clone(), Panel { game: g, routes: routes() })
    }

    fn post_to(p: &Panel, path: &str, body: Value) -> (u16, Value) {
        p.dispatch(&Request { method: "POST".into(), path: path.into(), query: Query::default(), headers: vec![], body: vec![] }, body)
    }

    fn post_as(p: &Panel, path: &str, body: Value, headers: &[(&str, &str)]) -> (u16, Value) {
        let headers = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        p.dispatch(&Request { method: "POST".into(), path: path.into(), query: Query::default(), headers, body: vec![] }, body)
    }

    #[test]
    fn console_lines_are_read_as_the_game_reads_them_and_no_spelling_approves() {
        assert_eq!(console_tokens(r#""prismatic_mcp" appr""ove 1"#), ["prismatic_mcp", "approve", "1"]);
        assert_eq!(console_tokens(r#"  echo "a b"  \"c "#), ["echo", "a b", "\"c"]);
        assert!(console_tokens("   ").is_empty());
        // Every spelling the game resolves to an approval, and every command that runs lines unread.
        for line in [r#""prismatic_mcp" approve 1"#, "/prismatic_mcp approve 1", "prismaticmcp approve 1", "prismatic__mcp approve 1",
                     r#"prismatic_mcp appr""ove 1"#, "PRISMATIC_MCP APPROVE 1", "p approve 1", r#"alias a "prismatic_mcp %+""#,
                     "alias a b", "/alias a b", "exec scripts/x.scr", "schedule on-next-calendar-month x.scr", "e_x_e_c x.scr",
                     "companies\nprismatic_mcp approve 1", "companies\rx", "echo \u{0}"] {
            assert!(console_refusal(line).is_some(), "{line:?} must be refused");
            assert!(is_approval_line(line), "{line:?}");
        }
        assert!(console_refusal("alias a b").unwrap().contains("cannot check"));
        assert!(console_refusal("x\ny").unwrap().contains("control"));
        assert!(console_refusal("prismatic_mcp approve 1").unwrap().contains("game's own console"));
        for line in ["prismatic_mcp deny 3", "prismatic_mcp status", "companies", r#"echo "hello world""#,
                     "start_ai PrismaticAI aggressiveness=50", "prismatic_mcp approved", "mcpapprove 1", "content state x"] {
            assert_eq!(console_refusal(line), None, "{line:?} must pass");
        }
    }

    #[test]
    fn the_console_writer_sends_one_line_only() {
        let (g, p) = bare_panel("5");
        let err = g.console("start_ai X a=1\nprismatic_mcp approve 1").unwrap_err();
        assert!(err.contains("control character"), "{err}");
        // The check comes before the pipe: a clean line meets the closed console instead.
        assert!(g.console("companies").unwrap_err().contains("console is closed"));
        // A value carrying a second line is refused by every route that builds a console line.
        let (code, out) = post_to(&p, "/api/start-ai", json!({"ai": "X", "settings": "a=1\nprismatic_mcp approve 1"}));
        assert!(code >= 400 && py_str(out.get("error")).contains("control character"), "{code} {out}");
        let (code, out) = post_to(&p, "/api/console", json!({"command": r#"alias go "prismatic_mcp %+""#}));
        assert_eq!((code, out["refused"].clone()), (403, json!("console_line")), "{out}");
    }

    #[test]
    fn an_approval_answers_by_its_status_before_any_error_field() {
        // An executed decision carries "error": "": still 200, the page shows success.
        assert_eq!(approve_status(&json!({"status": "executed", "error": ""})), 200);
        assert_eq!(approve_status(&json!({"status": "rejected", "error": "not enough money"})), 422);
        assert_eq!(approve_status(&json!({"refused": "owner_only", "error": "x"})), 403);
        assert_eq!(approve_status(&json!({"error": "approved #1, but ...", "pending_decision": true})), 504);
        assert_eq!(approve_status(&json!({"error": "approval #1 is consumed, not pending", "approval": {}})), 409);
        assert_eq!(approve_status(&json!({"error": "the game server holds no approval #1"})), 404);
        assert_eq!(approve_status(&json!({"error": ""})), 502);
        assert_eq!(approve_status(&json!({})), 502);
    }

    #[test]
    fn owner_only_approvals_are_refused_by_every_panel_channel() {
        let (g, p) = bare_panel("6");
        let secret = g.owner_secret.clone().expect("a secret from /dev/urandom");
        let owner = [(OWNER_HEADER, secret.as_str())];
        // The game server's queue, as openttd://approvals answers it; the panel's own requests are announced.
        let pend = |id: i64, tool: &str| {
            g.test_approvals.lock().unwrap().push(json!({"id": id, "tool": tool, "args": {"company": 1}, "state": "pending", "expired": false, "created_tick": 0, "expires_tick": 9}));
            g.announced.lock().unwrap().insert(id);
        };
        let pending = |id: i64| g.pending_json().iter().any(|p| p["id"] == json!(id));
        pend(41, "openttd.ai.set_settings");
        pend(42, "openttd.game.set_pause");
        assert!(owner_only("openttd.ai.set_settings") && !owner_only("openttd.ai.set_setting") && !owner_only("openttd.game.set_pause"));
        // The owner's own Approve, secret and all: refused with the console command, still pending.
        let (code, out) = post_as(&p, "/api/approve", json!({"id": 41}), &owner);
        assert_eq!((code, out["refused"].clone(), out["console"].clone()), (403, json!("owner_only"), json!("prismatic_mcp approve 41")), "{out}");
        assert!(py_str(out.get("error")).contains("prismatic_mcp approve 41"));
        assert!(pending(41), "a refused approval stays pending for the console");
        // Any other channel that reaches Game::approve: the same refusal.
        assert_eq!(g.approve(41, json!({"channel": "auto-approve"})).unwrap()["refused"], "owner_only");
        // Auto-approve (as --auto-approve sets it) passes the owner-only one over and tries the rest.
        g.auto_approve.store(true, Ordering::SeqCst);
        assert_eq!(g.auto_approve_once(), vec![42]);
        assert!(pending(41));
        g.auto_approve.store(false, Ordering::SeqCst);
        assert!(g.auto_approve_once().is_empty(), "off approves nothing");
        // Auto-approve takes only the panel's own requests, never another MCP client's.
        g.test_approvals.lock().unwrap().push(json!({"id": 44, "tool": "openttd.game.set_pause", "args": {}, "state": "pending", "expired": false}));
        g.auto_approve.store(true, Ordering::SeqCst);
        assert!(!g.auto_approve_once().contains(&44), "not announced by the panel");
        g.auto_approve.store(false, Ordering::SeqCst);
        // Not held, or not pending: 404 and 409, never a 200 with an error inside.
        assert_eq!(g.approve(99, json!({"channel": "api"})).unwrap()["error"], "the game server holds no approval #99");
        g.test_approvals.lock().unwrap().push(json!({"id": 45, "tool": "openttd.game.set_pause", "args": {}, "state": "consumed", "expired": false}));
        assert_eq!(approve_status(&g.approve(45, json!({"channel": "api"})).unwrap()), 409);
        // An ordinary approval with the secret goes through to the console (closed here).
        pend(43, "openttd.game.set_pause");
        let (code, out) = post_as(&p, "/api/approve", json!({"id": 43}), &owner);
        assert!(code == 502 && py_str(out.get("error")).contains("console is closed"), "{code} {out}");
    }

    /// A matcher reaches the core as one console token that the game's tokenizer reads back as the
    /// same JSON, whatever its strings hold; nothing in it can end the line or add a command.
    #[test]
    fn a_matcher_is_one_console_token_the_game_reads_back_as_the_same_json() {
        for m in [
            json!({"name": "max_*"}),
            json!({"text": "say \"hi\" twice"}),
            json!({"path": "C:\\ai\\", "end": "\\"}),
            json!({"msg": "a b  c", "semi": "x; prismatic_mcp approve 1", "nl": "one\ntwo\r", "tab": "\t"}),
            json!({"town": "Brno-Královo Pole", "emoji": "\u{1F682}", "zh": "火车"}),
            json!({"$.params[*]": 255, "$.a.b": "*", "list": [1, "two", {"three": null}], "ok": true}),
        ] {
            let token = matcher_token(&m).unwrap();
            let line = format!("prismatic_mcp rule allow openttd.cmd.* {token} --via panel");
            assert!(!line.chars().any(char::is_control) && !line.contains(';'), "{line}");
            let words = console_tokens(&line);
            assert_eq!(words.len(), 7, "one token for the matcher: {words:?}");
            assert_eq!((words[0].as_str(), words[3].as_str(), words[5].as_str(), words[6].as_str()),
                       ("prismatic_mcp", "openttd.cmd.*", "--via", "panel"));
            assert_eq!(serde_json::from_str::<Value>(&words[4]).unwrap(), m, "{}", words[4]);
        }
        assert!(matcher_token(&json!("name")).is_err() && matcher_token(&json!([1])).is_err(), "only an object");
        assert!(matcher_token(&json!({"k": "x".repeat(MATCHER_MAX)})).is_err(), "no longer than the console holds");
    }

    /// Every line the owner routes send: a bulk approval and every rule end with `--via panel`, so
    /// the core records them as the panel's and never lets them decide an owner-only tool.
    #[test]
    fn the_owner_routes_lines_approve_and_add_rules_only_via_the_panel() {
        assert_eq!(approve_all_line(None, None).unwrap(), "prismatic_mcp approve all --via panel");
        assert_eq!(approve_all_line(Some("openttd.cmd.*"), None).unwrap(), "prismatic_mcp approve all openttd.cmd.* --via panel");
        assert_eq!(approve_all_line(None, Some(&json!({"company": 1}))).unwrap(), r#"prismatic_mcp approve all * "{\"company\":1}" --via panel"#);
        assert_eq!(approve_all_line(Some(" "), Some(&json!({}))).unwrap(), "prismatic_mcp approve all --via panel", "blank is none");
        assert_eq!(deny_all_line(Some("command.Build*"), None).unwrap(), "prismatic_mcp deny all command.Build* --via panel");
        assert_eq!(rule_line("allow", "openttd.game.set_pause", None, false).unwrap(), "prismatic_mcp rule allow openttd.game.set_pause --via panel");
        assert_eq!(rule_line("deny", "openttd.cmd.*", Some(&json!({"name": "max_*"})), true).unwrap(),
                   r#"prismatic_mcp rule deny openttd.cmd.* "{\"name\":\"max_*\"}" --persist --via panel"#);
        for line in [approve_all_line(Some("x?"), Some(&json!({"a": 1}))).unwrap(), rule_line("allow", "*", Some(&json!({"a": "b c"})), true).unwrap()] {
            assert!(line.ends_with(VIA_PANEL), "{line}");
        }
        assert!(rule_line("maybe", "x", None, false).is_err());
        for bad in ["a b", "x;y", "x\"", "", &"x".repeat(129), "openttd.cmd.*\napprove"] {
            assert!(rule_glob(bad).is_err() && rule_line("deny", bad, None, false).is_err(), "{bad:?}");
        }
        // An allow rule naming an owner-only tool outright is refused; a deny rule for one is fine.
        for owner in ["openttd.ai.set_settings", "ai.set_settings"] {
            assert!(rule_line("allow", owner, None, false).unwrap_err().contains("owner's alone"));
            assert!(rule_line("deny", owner, None, false).is_ok());
        }
        assert_eq!(rule_remove_line(7), "prismatic_mcp rule remove 7");
        assert_eq!(bulk_ids(&json!({"ids": [3, 1, 3]})).unwrap(), vec![3, 1], "each once, in order");
        for bad in [json!({}), json!({"ids": []}), json!({"ids": ["1"]}), json!({"ids": (0..=BULK_MAX as i64).collect::<Vec<_>>()})] {
            assert!(bulk_ids(&bad).is_err(), "{bad}");
        }
        // One decision line per channel; an older core gets neither flag.
        assert_eq!((via_of(&json!({"channel": "auto-approve"})), via_of(&json!({"channel": "api"}))), ("auto", "panel"));
        assert_eq!(decide_line(true, &[7], None, None), "prismatic_mcp approve 7");
        assert_eq!(decide_line(false, &[7, 9], Some("panel"), Some("42")), "prismatic_mcp deny 7 9 --via panel --nonce 42");
        // The core's result line, found by its nonce among other lines (a date prefix included).
        let lines: Vec<String> = ["Rule #1 added.", "[approval-core] {\"nonce\":\"41\",\"ok\":true}",
            "[2026-11-23 04:00:00] [approval-core] {\"nonce\":42,\"ok\":false,\"error\":\"x is not a tool glob\"}"].map(String::from).to_vec();
        assert_eq!(core_result(&lines, "42").unwrap()["error"], "x is not a tool glob");
        assert_eq!(core_result(&lines, "41").unwrap()["ok"], true);
        assert!(core_result(&lines, "43").is_none() && core_result(&["[approval-core] not json".into()], "1").is_none());
        let (a, b) = (next_nonce(), next_nonce());
        assert!(a != b && a.chars().all(|c| c.is_ascii_digit()), "{a} {b}");
        let r = per_id(&[1, 2, 3, 4, 5, 6], &json!({"decided": [1, 5], "refused": [{"id": 2, "error": "not pending"}, 6], "skipped": [{"id": 3, "reason": "expired"}],
                                                    "rejected": [{"id": 5, "error": "not enough cash"}]}), true);
        assert_eq!(r.iter().map(|(id, c, _)| (*id, *c)).collect::<Vec<_>>(), [(1, 200), (2, 409), (3, 409), (4, 504), (5, 422), (6, 409)]);
        assert!(py_str(r[1].2.get("error")).contains("not pending") && py_str(r[2].2.get("error")).contains("expired"));
        assert!(py_str(r[4].2.get("error")).contains("not enough cash") && py_str(r[5].2.get("error")).contains("no reason given"));
        assert_eq!(per_id(&[5], &json!({"decided": [5]}), false)[0].2["status"], "denied");
        // Who may approve in the game.
        assert_eq!(approvers_line(&json!({})).unwrap(), "prismatic_mcp approvers");
        assert_eq!(approvers_line(&json!({"add": 2})).unwrap(), "prismatic_mcp approvers add 2");
        assert_eq!(approvers_line(&json!({"remove": "AbCd+/=_-123"})).unwrap(), "prismatic_mcp approvers remove AbCd+/=_-123");
        for bad in [json!({"add": 0}), json!({"add": "2"}), json!({"remove": "short"}), json!({"remove": "abcdefgh ; approve"})] {
            assert!(approvers_line(&bad).is_err(), "{bad}");
        }
    }

    /// The core's matcher and glob, mirrored, and the broad allow rule the core refuses.
    #[test]
    fn the_matcher_is_the_cores_and_a_broad_allow_rule_is_refused_as_the_core_refuses_it() {
        let args = json!({"company": 1, "name": "max_trains", "value": 40, "params": [255, 0, "goal"], "nested": {"a": {"name": "x"}}});
        for (m, want) in [
            (json!({}), true), (json!({"company": 1}), true), (json!({"company": 2}), false), (json!({"company": "1"}), true),
            (json!({"name": "max_*"}), true), (json!({"name": "max_?rains"}), true), (json!({"name": "min_*"}), false),
            (json!({"value": {"$gte": 0, "$lt": 100}}), true), (json!({"value": {"$gt": 40}}), false), (json!({"company": {"$in": [2, 1]}}), true),
            (json!({"company": {"$ne": 1}}), false), (json!({"$.params[0]": 255}), true), (json!({"$.params[*]": "*"}), true),
            (json!({"$.params[*]": 255}), false), (json!({"$..name": "*"}), true), (json!({"$.missing": "*"}), false),
            (json!({"$['name']": "max_trains"}), true), (json!({"$.params[x]": 1}), false), (json!({"params": [255, 0, "go*"]}), true),
            (json!({"missing": "*"}), false), (json!({"$": "*"}), true), (json!({"$ne": null}), true), (json!({"$.nested.*.name": "x"}), true),
        ] {
            assert_eq!(core_match(&m, &args), want, "{m}");
        }
        assert!(core_glob("openttd.cmd.*", "openttd.cmd.BuildRoad") && core_glob("a\\*b", "a*b") && !core_glob("a\\*b", "axb") && core_glob("?", "火"));
        assert!(has_wildcard("openttd.*") && !has_wildcard("openttd.ai.set_setting"));
        // Broad: no value constrained, whatever keys it names (the core's IsBroadMatcher).
        for broad in [None, Some(Value::Null), Some(json!({})), Some(json!({"$": "*"})), Some(json!({"$ne": null})), Some(json!({"company": "*"})),
                      Some(json!({"$.*": "*"})), Some(json!({"$..name": "**"})), Some(json!({"a": {"b": "*"}})), Some(json!({"x": {"$ne": null}})),
                      Some(json!({"$['a b']": "*", "$.l[*]": "***"}))] {
            assert!(broad_matcher(broad.as_ref()), "{broad:?}");
            assert!(rule_line("allow", "openttd.ai.*", broad.as_ref(), false).unwrap_err().contains("approves whatever they ask"), "{broad:?}");
            assert!(rule_line("deny", "openttd.ai.*", broad.as_ref(), false).is_ok(), "a broad deny rule is fine");
            assert!(rule_line("allow", "openttd.ai.set_setting", broad.as_ref(), false).is_ok(), "one tool is fine");
        }
        // Narrowing: a literal, a glob with another character, an empty string, a comparison, $in, an array.
        for narrow in [json!({"company": 0}), json!({"name": "a*"}), json!({"n": {"$lt": 5}}), json!({"c": {"$in": [1]}}), json!({"c": ""}),
                       json!({"c": "?"}), json!({"c": [1]}), json!({"c": true}), json!({"a": {"b": "x"}}), json!({"x": {"$ne": null, "$gt": 1}}),
                       json!({"company": "*", "name": "max_*"}), json!(5)] {
            assert!(!broad_matcher(Some(&narrow)), "{narrow}");
        }
        assert!(rule_line("allow", "openttd.ai.*", Some(&json!({"company": 1})), false).is_ok());
        assert!(rule_line("allow", "openttd.ai.*", Some(&json!({"company": "*"})), false).is_err(), "a glob with a broad matcher is refused");
        assert!(rule_line("allow", "openttd.ai.set_setting", Some(&json!({"company": "*"})), false).is_ok(), "one tool with a broad matcher stays allowed");
    }

    /// Real `[approval-core] {...}` lines of the core (src/fixtures/approval-core-lines.txt, written
    /// from the real core's output): each is found by its nonce and every id it names is answered.
    #[test]
    fn the_real_cores_result_lines_are_read() {
        let text = include_str!("fixtures/approval-core-lines.txt");
        let lines: Vec<String> = text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).map(String::from).collect();
        assert!(lines.len() >= 8, "the fixture holds the core's lines");
        let failed: Vec<&String> = lines.iter().filter(|l| l.contains(r#""ok":false"#)).collect();
        assert!(failed.iter().all(|l| { let r = core_result(std::slice::from_ref(l), &serde_json::from_str::<Value>(&l[l.find('{').unwrap()..]).unwrap()["nonce"].as_str().unwrap().to_string()).unwrap(); !r["error"].as_str().unwrap_or_default().is_empty() }),
                "a refused line names why");
        for line in &lines {
            let raw: Value = serde_json::from_str(&line[line.find(CORE_RESULT).expect("a result line") + CORE_RESULT.len()..]).unwrap();
            let nonce = raw["nonce"].as_str().map(str::to_string).unwrap_or_else(|| raw["nonce"].to_string());
            let r = core_result(std::slice::from_ref(line), &nonce).unwrap_or_else(|| panic!("not read: {line}"));
            assert!(r["ok"].is_boolean(), "{line}");
            let ids: Vec<i64> = ["decided", "refused", "skipped", "rejected"].iter()
                .flat_map(|k| r[*k].as_array().cloned().unwrap_or_default()).filter_map(|e| entry_of(&e).map(|(id, _)| id)).collect();
            for (id, code, out) in per_id(&ids, &r, true) {
                assert!(code != 504, "#{id} of {line} is not answered: {out}");
                // Each refused, skipped or rejected id answers with its status and the core's reason.
                for (k, want) in [("refused", 409), ("skipped", 409), ("rejected", 422)] {
                    if let Some((_, why)) = r[k].as_array().into_iter().flatten().filter_map(entry_of).find(|(e, _)| *e == id) {
                        assert_eq!(code, want, "#{id} under {k} in {line}");
                        assert!(py_str(out.get("error")).contains(&why), "#{id}: {out} names {why:?}");
                    }
                }
            }
        }
    }

    /// /api/console refuses every line that would manage the standing rules, carry the core's
    /// `--via` marker or deny in bulk, in every spelling the game resolves; one denial stays open.
    #[test]
    fn the_console_refuses_rules_via_and_bulk_denial_in_every_spelling() {
        for line in [
            "prismatic_mcp rule allow openttd.cmd.*", "prismatic_mcp RULE deny x", "prismatic_mcp ru_le remove 1",
            "prismatic_mcp \"rules\"", "/prismatic_mcp rules", "prismatic_mcp r-ule allow x", "prismatic_mcp Ru__Les",
            "prismatic_mcp \"ru\"\"le\" allow *", "prismatic_mcp deny 3 --via panel", "prismatic_mcp deny 3 --VIA panel",
            "prismatic_mcp deny 3 -_via panel", "prismatic_mcp deny all", "prismatic_mcp deny ALL openttd.*",
            "prismatic_mcp de_ny \"all\"", "prismatic_mcp approve all --via panel",
            "prismatic_mcp rule allow * --persist", "prismatic_mcp approvers add 2", "prismatic_mcp APPROVERS", "prismatic_mcp app_rovers remove abcdefgh",
            "prismatic_mcp \"approver\"s", "/prismatic_mcp approv-ers", "prismatic_mcp deny 3 --nonce 1", "prismatic_mcp deny 3 --NON_CE 1",
        ] {
            assert!(console_refusal(line).is_some(), "{line}");
        }
        for open in ["prismatic_mcp deny 3", "say \"rule the rails\"", "prismatic_mcp status", "echo allowed"] {
            assert_eq!(console_refusal(open), None, "{open}");
        }
        let (_, p) = bare_panel("9");
        let (code, out) = post_to(&p, "/api/console", json!({"command": "prismatic_mcp rule allow *"}));
        assert_eq!((code, out["refused"].clone()), (403, json!("console_line")));
    }

    /// The owner's bulk and rule routes: each needs the owner secret, reads the core's rules back,
    /// and answers 501 while the core keeps none.
    #[test]
    fn the_bulk_and_rule_routes_are_the_owners_and_answer_what_the_core_did() {
        let (g, p) = bare_panel("9");
        let secret = g.owner_secret.clone().unwrap();
        let owner = [(OWNER_HEADER, secret.as_str())];
        for path in OWNER_ROUTES {
            assert_eq!(post_to(&p, path, json!({"ids": [1], "id": 1, "effect": "allow", "tool": "x"})).0, 401, "{path}");
        }
        *g.test_console.lock().unwrap() = Some(vec![]);
        let sent = || g.test_console.lock().unwrap().clone().unwrap();
        // A core without rules: the list says so, and the rule routes answer 501, sending nothing.
        g.test_approvals.lock().unwrap().extend([
            json!({"id": 41, "tool": "openttd.ai.set_settings", "args": {"company": 1}, "state": "pending", "expired": false}),
            json!({"id": 42, "tool": "openttd.game.set_pause", "args": {"pause": true}, "state": "pending", "expired": false,
                   "matched_rule": {"id": 1, "effect": "allow", "decides": true}}),
            json!({"id": 43, "tool": "openttd.game.set_pause", "args": {}, "state": "pending", "expired": true}),
        ]);
        let list = approvals_view(&g.queue_data());
        assert_eq!((list["rules_supported"].clone(), g.keeps_rules()), (json!(false), false), "an older core takes no --via");
        for path in ["/api/rules", "/api/rules/remove", "/api/approve/all", "/api/deny/all"] {
            assert_eq!(post_as(&p, path, json!({"effect": "allow", "tool": "x", "id": 1}), &owner).0, 501, "{path}");
        }
        assert!(sent().is_empty());
        // The list: every row marked, the matched rule kept.
        let (code, list) = p.dispatch(&Request { method: "GET".into(), path: "/api/approvals".into(), query: Query::default(), headers: vec![], body: vec![] }, Value::Null);
        assert_eq!(code, 200);
        let rows = list["approvals"].as_array().unwrap();
        assert_eq!(rows.iter().map(|a| (a["owner_only"].clone(), a["decidable"].clone())).collect::<Vec<_>>(),
                   [(json!(true), json!(true)), (json!(false), json!(true)), (json!(false), json!(false))]);
        assert_eq!(rows[1]["matched_rule"]["id"], 1);
        // A core with rules.
        *g.test_rules.lock().unwrap() = Some(vec![]);
        assert_eq!(approvals_view(&g.queue_data())["rules_supported"], true);
        let (code, out) = post_as(&p, "/api/rules", json!({"effect": "allow", "tool": "openttd.game.*", "match": {"pause": true}, "persist": true}), &owner);
        assert_eq!((code, out["rule"]["id"].clone(), out["rule"]["persisted"].clone()), (200, json!(1), json!(true)), "{out}");
        assert_eq!(out["rule"]["match"], json!({"pause": true}), "the matcher arrived as JSON");
        assert!(sent().last().unwrap().contains(" --persist --via panel --nonce ") && py_str(out.get("sent")) == *sent().last().unwrap());
        let (code, out) = post_as(&p, "/api/rules", json!({"effect": "deny", "tool": "bad.*"}), &owner);
        assert_eq!((code, out["error"].clone()), (422, json!("bad.* is not a tool glob")), "the core's refusal: {out}");
        assert!(py_str(out.get("sent")).contains(" --via panel --nonce "), "{out}");
        assert_eq!(post_as(&p, "/api/rules", json!({"effect": "allow", "tool": "a b"}), &owner).0, 400);
        assert_eq!(post_as(&p, "/api/rules", json!({"effect": "allow", "tool": "openttd.ai.set_settings"}), &owner).0, 400);
        assert_eq!(post_as(&p, "/api/rules", json!({"effect": "allow", "tool": "x", "match": "nope"}), &owner).0, 400);
        let (code, out) = post_as(&p, "/api/rules/remove", json!({"id": 1}), &owner);
        assert_eq!((code, out["removed"].clone()), (200, json!(1)), "{out}");
        assert!(sent().last().unwrap().starts_with("prismatic_mcp rule remove 1 --nonce "));
        assert_eq!(post_as(&p, "/api/rules/remove", json!({"id": 1}), &owner).0, 404);
        let (code, out) = post_as(&p, "/api/approve/all", json!({"tool": "openttd.game.*"}), &owner);
        assert_eq!((code, out["status"].clone()), (200, json!("approved")), "{out}");
        assert!(sent().last().unwrap().starts_with("prismatic_mcp approve all openttd.game.* --via panel --nonce "));
        let (code, _) = post_as(&p, "/api/deny/all", json!({"match": {"company": 1}}), &owner);
        assert_eq!(code, 200);
        assert!(sent().last().unwrap().starts_with(r#"prismatic_mcp deny all * "{\"company\":1}" --via panel --nonce "#));
        assert_eq!(post_as(&p, "/api/approve/all", json!({"tool": "x y"}), &owner).0, 400);
        // A rule line the core never answers: 504, never a 200.
        let silent = core_line(&g, "prismatic_mcp status").unwrap();
        assert_eq!(silent.0, 504);
        // Bulk over ids: one answer per id as /api/approve and /api/deny give it.
        let (code, out) = post_as(&p, "/api/approve/many", json!({"ids": [41, 99, 43]}), &owner);
        assert_eq!(code, 200);
        assert_eq!(out["results"].as_array().unwrap().iter().map(|r| r["code"].as_u64().unwrap()).collect::<Vec<_>>(), [403, 404, 409]);
        assert_eq!((out["done"].clone(), out["failed"].clone()), (json!(0), json!(3)));
        let (code, out) = post_as(&p, "/api/deny/many", json!({"ids": [99, 43]}), &owner);
        assert_eq!((code, out["failed"].clone()), (200, json!(2)), "{out}");
        assert_eq!(post_as(&p, "/api/approve/many", json!({"ids": []}), &owner).0, 400);
        // A core with rules: the picked ids go in ONE line, via the panel, and its result names each.
        g.test_approvals.lock().unwrap().extend([
            json!({"id": 44, "tool": "openttd.game.set_pause", "args": {}, "state": "pending", "expired": false}),
            json!({"id": 45, "tool": "openttd.game.set_pause", "args": {}, "state": "pending", "expired": false}),
            json!({"id": 46, "tool": "openttd.game.set_pause", "args": {}, "state": "pending", "expired": false}),
            json!({"id": 47, "tool": "openttd.game.set_pause", "args": {}, "state": "pending", "expired": false}),
        ]);
        let (code, out) = post_as(&p, "/api/approve/many", json!({"ids": [42, 44, 41]}), &owner);
        assert_eq!(code, 200);
        assert_eq!(out["results"].as_array().unwrap().iter().map(|r| (r["id"].as_i64().unwrap(), r["code"].as_u64().unwrap())).collect::<Vec<_>>(),
                   [(41, 403), (42, 200), (44, 200)], "{out}");
        assert!(sent().last().unwrap().starts_with("prismatic_mcp approve 42 44 --via panel --nonce "), "{:?}", sent().last());
        let (code, out) = post_as(&p, "/api/deny/many", json!({"ids": [45]}), &owner);
        assert_eq!((code, out["done"].clone()), (200, json!(1)));
        assert!(sent().last().unwrap().starts_with("prismatic_mcp deny 45 --via panel --nonce "));
        // One decision: the panel's channel, or auto-approve's.
        let _ = g.deny(46);
        assert_eq!(sent().last().unwrap(), "prismatic_mcp deny 46 --via panel");
        let _ = g.approve(47, json!({"channel": "auto-approve"}));
        assert_eq!(sent().last().unwrap(), "prismatic_mcp approve 47 --via auto");
        // The in-game approvers, the owner's.
        let (code, out) = post_as(&p, "/api/approvers", json!({"add": 2}), &owner);
        assert_eq!((code, sent().last().unwrap().as_str()), (200, "prismatic_mcp approvers add 2"), "{out}");
        let (code, out) = post_as(&p, "/api/approvers", json!({"add": 404}), &owner);
        assert_eq!((code, py_str(out.get("error")).starts_with("No connected client #404")), (422, true), "{out}");
        assert_eq!(post_as(&p, "/api/approvers", json!({"remove": "short"}), &owner).0, 400);
        assert_eq!(post_as(&p, "/api/approvers", json!({}), &owner).0, 200);
        assert_eq!(bulk_answer(vec![(1, 200, json!({})), (2, 409, json!({}))])["done"], 1);
    }

    /// POST /api/deny goes through the server's one core like Approve, needs no owner secret, and
    /// answers 404 for an approval the server does not hold and 409 for one not pending; a pending
    /// one reaches the console (closed here, so 502). A denial reads back as 200.
    #[test]
    fn denying_needs_no_secret_and_answers_what_the_server_holds() {
        let (g, p) = bare_panel("7");
        let row = |id: i64, state: &str, expired: bool| json!({"id": id, "tool": "openttd.game.set_pause", "args": {}, "state": state, "expired": expired});
        g.test_approvals.lock().unwrap().extend([row(51, "pending", false), row(52, "consumed", false), row(53, "pending", true)]);
        let (code, out) = post_to(&p, "/api/deny", json!({"id": 99}));
        assert_eq!((code, out["error"].clone()), (404, json!("the game server holds no approval #99")), "no secret asked: {out}");
        let (code, out) = post_to(&p, "/api/deny", json!({"id": 52}));
        assert_eq!((code, out["error"].clone()), (409, json!("approval #52 is consumed, not pending")));
        let (code, out) = post_to(&p, "/api/deny", json!({"id": 53}));
        assert_eq!((code, out["error"].clone()), (409, json!("approval #53 is expired, not pending")));
        let (code, out) = post_to(&p, "/api/deny", json!({"id": 51}));
        assert!(code == 502 && py_str(out.get("error")).contains("console is closed"), "{code} {out}");
        assert_eq!(approve_status(&json!({"status": "denied", "approval_id": 51})), 200);
        assert_eq!(approve_status(&json!({"error": "denied #51 ...", "pending_decision": true})), 504);
    }

    #[test]
    fn approving_and_auto_approve_on_need_the_owner_secret() {
        let s = Some("s3cret");
        let h = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        let gate = |path: &str, body: Value, headers: &[(&str, &str)]| owner_gate(s, 8800, "POST", path, &body, &h(headers));
        let refused = gate("/api/approve", json!({"id": 1}), &[]).unwrap_err();
        assert_eq!((refused.0, refused.1["refused"].clone()), (401, json!("owner_secret")));
        assert!(!py_str(refused.1.get("error")).contains("s3cret"), "the refusal never shows the secret");
        assert!(gate("/api/approve", json!({"id": 1}), &[("X-Panel-Owner", "s3creT")]).is_err());
        assert!(gate("/api/approve", json!({"id": 1}), &[("X-Panel-Owner", "s3cre")]).is_err(), "a prefix is not the secret");
        assert!(gate("/api/approve", json!({"id": 1}), &[("x-panel-owner", " s3cret ")]).is_ok());
        assert!(gate("/api/approve", json!({"id": 1}), &[("Cookie", "a=b; mcp_owner_8800=s3cret")]).is_ok());
        assert!(gate("/api/approve", json!({"id": 1}), &[("Cookie", "mcp_owner_8801=s3cret")]).is_err(), "another panel's cookie");
        assert!(gate("/api/approve", json!({"id": 1}), &[("Authorization", "Bearer s3cret")]).is_err(), "the web token is not the owner");
        assert!(gate("/api/auto-approve", json!({"on": true}), &[]).is_err());
        assert!(gate("/api/auto-approve", json!({"on": false}), &[]).is_ok(), "switching off is open");
        assert!(gate("/api/pause", json!({"pause": true}), &[]).is_ok());
        assert!(owner_gate(s, 8800, "GET", "/api/approve", &Value::Null, &[]).is_ok());
        // A panel without a secret has no owner over HTTP at all.
        assert!(owner_gate(None, 8800, "POST", "/api/approve", &json!({"id": 1}), &h(&[("X-Panel-Owner", "")])).is_err());
        // The page's cookie is set only for the secret itself.
        let c = owner_cookie(s, 8800, Some("s3cret")).unwrap();
        assert!(c.starts_with("Set-Cookie: mcp_owner_8800=s3cret;") && c.contains("HttpOnly") && c.contains("SameSite=Strict"), "{c}");
        assert_eq!(owner_cookie(s, 8800, Some("guess")), None);
        assert_eq!(owner_cookie(None, 8800, Some("")), None);
        let (a, b) = (new_owner_secret().unwrap(), new_owner_secret().unwrap());
        assert!(a.len() == 64 && a.bytes().all(|c| c.is_ascii_hexdigit()) && a != b);
        // Through the router: the switch stays off without the secret, and the secret reaches no event.
        let (g, p) = bare_panel("7");
        let secret = g.owner_secret.clone().unwrap();
        assert_eq!(post_to(&p, "/api/auto-approve", json!({"on": true})).0, 401);
        assert!(!g.auto_approve.load(Ordering::SeqCst));
        assert_eq!(post_to(&p, "/api/approve", json!({"id": 5})).0, 401);
        assert_eq!(post_as(&p, "/api/auto-approve", json!({"on": true}), &[(OWNER_HEADER, &secret)]).0, 200);
        assert!(g.auto_approve.load(Ordering::SeqCst));
        assert_eq!(post_to(&p, "/api/auto-approve", json!({"on": false})).0, 200);
        assert!(!g.auto_approve.load(Ordering::SeqCst));
        let seen = format!("{:?} {:?}", g.bus.since(0, &[], 10_000), g.events.lock().unwrap());
        assert!(seen.contains("refused /api/approve") && !seen.contains(&secret), "{seen}");
    }


    /// A directory of its own for one owner-secret test, removed when the test ends.
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(name: &str) -> TmpDir {
            let d = std::env::temp_dir().join(format!("mcp-panel-owner-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            TmpDir(d)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn mode_of(p: &Path) -> u32 {
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::mode(&std::fs::metadata(p).unwrap().permissions()) & 0o777
    }

    #[test]
    fn the_owner_secret_lives_in_one_0600_file_and_survives_a_restart() {
        // Where it lives: the flag, else the environment, else the home directory's config.
        assert_eq!(owner_secret_path(Some("/f"), Some("/e"), Some("/h")).unwrap(), PathBuf::from("/f"));
        assert_eq!(owner_secret_path(None, Some("/e"), Some("/h")).unwrap(), PathBuf::from("/e"));
        assert_eq!(owner_secret_path(Some(" "), Some(""), Some("/h")).unwrap(), PathBuf::from("/h/.config/prismatic/panel-owner-secret"));
        assert!(owner_secret_path(None, None, None).unwrap_err().contains(OWNER_FILE_ENV));
        // Made once, 0600 in a 0700 directory it creates, then read on every start.
        let tmp = TmpDir::new("file");
        let file = tmp.0.join("prismatic/panel-owner-secret");
        let first = load_owner_secret(&file).unwrap();
        assert!(first.len() == 64 && first.bytes().all(|c| c.is_ascii_hexdigit()), "{first}");
        assert_eq!((mode_of(&file), mode_of(file.parent().unwrap())), (0o600, 0o700));
        assert_eq!(load_owner_secret(&file).unwrap(), first, "a restart reads the same secret");
        // A panel that comes second keeps the first one's secret.
        make_owner_secret_file(&file).unwrap();
        assert_eq!(load_owner_secret(&file).unwrap(), first);
        assert!(make_owner_secret_file(&tmp.0.join("no-such-dir/x")).unwrap_err().contains("no-such-dir"));
        // A file others may read is refused, and so is one that holds no secret.
        std::fs::set_permissions(&file, <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o644)).unwrap();
        let refused = load_owner_secret(&file).unwrap_err();
        assert!(refused.contains("mode 644") && refused.contains("chmod 600") && !refused.contains(&first), "{refused}");
        let short = tmp.0.join("prismatic/short");
        std::fs::write(&short, "abc\n").unwrap();
        std::fs::set_permissions(&short, <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600)).unwrap();
        assert!(load_owner_secret(&short).unwrap_err().contains("holds no owner secret"));
        assert!(load_owner_secret(&tmp.0.join("prismatic")).unwrap_err().contains("prismatic"), "a directory is no secret");
    }

    #[test]
    fn a_panel_names_where_its_owner_secret_lives_and_never_the_secret() {
        let tmp = TmpDir::new("of");
        let args = |extra: &[&str]| parse_args(&[&["--bin", "/x/openttd", "--panel-port", "18801"], extra].concat().iter().map(|a| a.to_string()).collect::<Vec<_>>()).unwrap();
        let file = tmp.0.join("secret");
        let a = args(&["--owner-secret-file", file.to_str().unwrap()]);
        assert_eq!(a.owner_file.as_deref(), file.to_str());
        let o = owner_of(&a, Some("/ignored"), None).unwrap();
        let secret = o.secret.clone().unwrap();
        assert_eq!(o.file.as_deref(), file.to_str());
        assert!(o.note.contains(file.to_str().unwrap()) && o.note.contains("http://127.0.0.1:18801/") && !o.note.contains(&secret), "{}", o.note);
        // The environment names the file when no flag does.
        let env_file = tmp.0.join("from-env");
        let o = owner_of(&args(&[]), env_file.to_str(), None).unwrap();
        assert_eq!(o.file.as_deref(), env_file.to_str());
        // One-shot: kept nowhere, so the owner approves at the console.
        let e = args(&["--ephemeral-owner-secret"]);
        assert!(e.ephemeral_owner);
        let o = owner_of(&e, env_file.to_str(), None).unwrap();
        assert!(o.secret.is_some() && o.file.is_none() && o.note.contains("prismatic_mcp approve") && !o.note.contains(&o.secret.clone().unwrap()));
        assert!(owner_of(&args(&[]), None, None).is_err());
        // The panel reads the file it was given; a file others may read stops it before any game starts.
        let g = Game::new(a);
        assert_eq!(g.owner_secret.as_deref(), Some(secret.as_str()));
        assert_eq!(g.owner_file.as_deref(), file.to_str());
        std::fs::set_permissions(&file, <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o640)).unwrap();
        let g = Game::new(args(&["--owner-secret-file", file.to_str().unwrap()]));
        assert!(g.owner_secret.is_none());
        let stopped = g.start().unwrap_err();
        assert!(stopped.contains("chmod 600") && !g.server_running(), "{stopped}");
        // A test panel without the flag never touches a person's own file: one-shot.
        let (g, _) = bare_panel("18802");
        assert!(g.owner_secret.is_some() && g.owner_file.is_none());
    }

    #[test]
    fn the_owner_signs_in_with_the_secret_and_guessing_is_refused_then_slowed() {
        let mut fails = VecDeque::new();
        let t0 = Instant::now();
        assert!(owner_login(Some("s3cret"), " s3cret ", &mut fails, t0).is_ok());
        let wrong = owner_login(Some("s3cret"), "s3cre", &mut fails, t0).unwrap_err();
        assert_eq!((wrong.0, wrong.1["refused"].clone()), (401, json!("owner_login")));
        assert!(!wrong.1.to_string().contains('6') && !wrong.1.to_string().contains("s3c"), "nothing about the secret: {}", wrong.1);
        assert!(owner_login(None, "", &mut fails, t0).is_err(), "a panel without a secret signs nobody in");
        for _ in 0..3 { let _ = owner_login(Some("s3cret"), "x", &mut fails, t0); }
        let limited = owner_login(Some("s3cret"), "guess", &mut fails, t0).unwrap_err();
        assert_eq!((limited.0, limited.1["refused"].clone()), (429, json!("owner_login_rate")), "a guess waits out the window");
        assert!(owner_login(Some("s3cret"), "s3cret", &mut fails, t0).is_ok(), "the owner's own secret is never locked out");
        assert_eq!(owner_login(Some("s3cret"), "guess", &mut fails, t0 + LOGIN_WINDOW).unwrap_err().0, 401, "the window passes");
        assert_eq!(fails.len(), 1);
        // The cookies: kept a year, HttpOnly, SameSite=Strict, for this panel's port; sign-out clears it.
        let c = owner_set_cookie(8800, "s3cret");
        assert!(c.starts_with("Set-Cookie: mcp_owner_8800=s3cret;") && c.contains("HttpOnly") && c.contains("SameSite=Strict")
            && c.contains("Path=/") && c.contains(&format!("Max-Age={OWNER_COOKIE_MAX_AGE}")), "{c}");
        assert_eq!(owner_cookie(Some("s3cret"), 8800, Some("s3cret")), Some(c));
        let out = owner_clear_cookie(8800);
        assert!(out.starts_with("Set-Cookie: mcp_owner_8800=;") && out.contains("Max-Age=0") && out.contains("HttpOnly"), "{out}");
    }

    #[test]
    fn the_sign_in_routes_set_and_clear_the_owner_cookie() {
        let (g, p) = bare_panel("18803");
        let secret = g.owner_secret.clone().unwrap();
        // Refused an owner's route, the page learns where to sign in and where the secret lives.
        let refused = post_to(&p, "/api/approve", json!({"id": 1}));
        assert_eq!((refused.0, refused.1["sign_in"].clone(), refused.1["secret_file"].clone()), (401, json!("/api/owner/login"), Value::Null));
        crate::web_access::set_headers("");
        assert_eq!(post_to(&p, "/api/owner/login", json!({"secret": "wrong"})).0, 401);
        assert!(!crate::web_access::headers().contains("Set-Cookie"), "a wrong secret sets no cookie");
        assert_eq!(post_to(&p, "/api/owner/login", json!({})).0, 400, "the secret is required");
        let (code, out) = post_to(&p, "/api/owner/login", json!({"secret": secret}));
        assert_eq!((code, out), (204, Value::Null));
        assert!(crate::web_access::headers().contains(&owner_set_cookie(18803, &secret)));
        crate::web_access::set_headers("");
        assert_eq!(post_to(&p, "/api/owner/logout", json!({})).0, 204);
        assert!(crate::web_access::headers().contains("mcp_owner_18803=; HttpOnly"));
        crate::web_access::set_headers("");
        let seen = format!("{:?} {:?}", g.bus.since(0, &[], 10_000), g.events.lock().unwrap());
        assert!(seen.contains("the owner signed in") && seen.contains("wrong secret") && !seen.contains(&secret), "{seen}");
        for _ in 0..5 { let _ = post_to(&p, "/api/owner/login", json!({"secret": "x"})); }
        assert_eq!(post_to(&p, "/api/owner/login", json!({"secret": "y"})).0, 429);
        assert_eq!(post_to(&p, "/api/owner/login", json!({"secret": secret})).0, 204, "the owner is never locked out");
        assert!(format!("{:?}", g.events.lock().unwrap()).contains("too many wrong secrets"));
        // A 204 goes out with no body at all.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        send_json(listener.accept().unwrap().0, 204, &Value::Null);
        let mut raw = String::new();
        client.read_to_string(&mut raw).unwrap();
        assert!(raw.starts_with("HTTP/1.1 204 No Content") && raw.contains("Content-Length: 0") && raw.ends_with("\r\n\r\n"), "{raw}");
    }

    #[test]
    fn a_planner_that_fails_answers_502_and_starts_no_bot() {
        let dir = std::env::temp_dir().join(format!("mcp-panel-planner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let f = dir.join(name);
            std::fs::write(&f, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&f, <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755)).unwrap();
            f
        };
        let failing = script("failing-planner", "echo 'mcp-plan: towns: connection refused' >&2; exit 3");
        let air = json!({"portfolio": {"solver": "fixed", "chosen": [{"kind": "passengers", "mode": "air", "score": 3,
            "a": {"id": 1, "name": "A"}, "b": {"id": 2, "name": "B"}, "options": [{"mode": "air"}]}]}, "reads": []});
        let fixed = script("fixed-planner", &format!("cat <<'JSON'\n{air}\nJSON"));
        let (g, p) = bare_panel("2");
        // PRISMATIC_PLANNER is read by this test only; both cases run here, one after the other.
        std::env::set_var("PRISMATIC_PLANNER", &failing);
        let since = g.bus.seq();
        let (code, out) = post_to(&p, "/api/bots/planned", json!({"ai": "PrismaticAI", "start": true}));
        assert_eq!(code, 502, "{out}");
        assert_eq!(out["started"], Value::Null, "an unplanned bot is not started");
        assert!(out["error"].as_str().unwrap().contains("exit 3: mcp-plan: towns: connection refused"), "{out}");
        assert_eq!(out["bot_settings"]["unplanned"], out["error"]);
        let plan = g.bus.since(since, &kinds_of("plan"), 10);
        let kinds: Vec<&str> = plan.iter().filter_map(|e| e["kind"].as_str()).collect();
        assert_eq!(kinds, ["plan.portfolio", "plan.bot_settings"], "the failure is published as data too");
        assert_eq!(plan[0]["data"]["inputs"]["live"]["planner"]["exit"], 3);
        assert_eq!(plan[0]["data"]["choice"]["error"], out["error"]);

        std::env::set_var("PRISMATIC_PLANNER", &fixed);
        let (code, out) = post_to(&p, "/api/bots/planned", json!({"ai": "PrismaticAI"}));
        std::env::remove_var("PRISMATIC_PLANNER");
        assert_eq!(code, 200, "{out}");
        assert_eq!(out["settings"], "air_a=1,air_b=2,air_planes=2");
        assert!(out["bot_settings"].get("unplanned").is_none());
        assert_eq!(out["started"], Value::Null, "planned, not started");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_bus_answers_its_kinds_while_the_tap_is_down() {
        let (_, p) = bare_panel("3");
        let (code, out) = p.dispatch(&Request { method: "GET".into(), path: "/api/bus".into(), query: Query::default(), headers: vec![], body: vec![] }, Value::Null);
        assert_eq!(code, 200, "{out}");
        assert_eq!(out["endpoint"], Value::Null);
        assert_eq!(out["error"], "the bus tap is not running");
        assert_eq!(out["kinds"], panel_events::table(panel_events::PANEL_BUS));
    }

    #[test]
    fn an_action_plans_books_start_from_what_the_caller_gave() {
        let body = |v: Value| v.as_object().cloned().unwrap();
        assert_eq!(books_from_caller(&body(json!({}))), json!({"cash": "missing", "loan": "missing", "max_loan": "missing"}));
        assert_eq!(books_from_caller(&body(json!({"cash": 5, "max_loan": 9}))), json!({"cash": "caller", "loan": "missing", "max_loan": "caller"}));
    }

    /// A game's MCP server for the live path: openttd://game/state at `tick`, one AI company 1
    /// (openttd://companies read at that tick, with its money, as the game's envelope carries it),
    /// openttd.ai.set_settings offered; every set_settings call is kept in `calls` and answered as
    /// the game answers a gated call, with a pending approval.
    fn fake_game(tick: i64, date: &str, calls: Arc<Mutex<Vec<Value>>>) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let date = date.to_string();
        thread::spawn(move || for mut c in l.incoming().flatten() {
            let Some(req) = read_request(&mut c) else { continue };
            let call: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let res = |v: Value| json!({"jsonrpc": "2.0", "id": 1, "result": {"contents": [{"text": v.to_string()}]}});
            let out = match call["method"].as_str().unwrap_or("") {
                "tools/list" => json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [{"name": "openttd.ai.set_settings"}]}}),
                "resources/read" => match call["params"]["uri"].as_str().unwrap_or("") {
                    "openttd://game/state" => res(json!({"game_tick": tick, "game_date": date})),
                    // The game's own envelope (prismatic_mcp_server.cpp: env["data"] = {companies}), read at
                    // the tick and date of the game, each company with its books.
                    "openttd://companies" => res(json!({"game_tick": tick, "game_date": date, "data": {"companies": [
                        {"id": 1, "is_ai": true, "name": "A", "money": 5_000_000, "loan": 0, "max_loan": 300_000}]}})),
                    // The server's queue the panel reads its pending approvals from (#211): every
                    // set_settings call below is held as approval 40 + its place among the calls.
                    "openttd://approvals" => res(json!({"data": {"approvals": calls.lock().unwrap().iter().enumerate()
                        .filter(|(_, c)| c["name"] == "openttd.ai.set_settings")
                        .map(|(i, c)| json!({"id": 41 + i, "tool": c["name"], "args": c["arguments"], "state": "pending",
                                             "expired": false, "created_tick": tick, "expires_tick": tick + 74}))
                        .collect::<Vec<_>>()}})),
                    other => res(json!({"error": format!("no {other}")})),
                },
                _ => {
                    let mut seen = calls.lock().unwrap();
                    seen.push(call["params"].clone());
                    let answer = if call["params"]["name"] == "openttd.ai.set_settings" {
                        let id = 40 + seen.len();
                        json!({"status": "pending_approval", "approval_id": id,
                               "message": format!("Approval required in OpenTTD: prismatic_mcp approve {id}")})
                    } else {
                        json!({"ok": true, "applied": 17})
                    };
                    json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [{"type": "text", "text": answer.to_string()}]}})
                }
            };
            hub::respond(c, 200, "application/json", out.to_string().as_bytes());
        });
        port
    }

    #[test]
    fn a_rollback_is_applied_through_the_live_path_and_never_in_a_world_behind_its_snapshot() {
        // battlebrain rollback's directive (I0117 round 2), served once like any decision.
        let snap: Value = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../schema/fixtures/battlebrain/valid/snapshot.json")).unwrap()).unwrap();
        let out = crate::battlebrain_contract::rollback_directive(&snap["value"], 1, None, "test").unwrap();
        let directive = out["directive"].clone();
        let key = directive["decision"]["idempotency_key"].as_str().unwrap().to_string();
        let tick = snap["value"]["at"]["tick"].as_i64().unwrap();
        let serve = |d: Value| {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/decision", l.local_addr().unwrap());
            thread::spawn(move || for mut c in l.incoming().flatten() {
                if read_request(&mut c).is_none() { continue }
                hub::respond(c, 200, "application/json", d.to_string().as_bytes());
            });
            url
        };
        let panel_on = |game_port: u16, port: &str| {
            let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", port, "--mcp-port", &game_port.to_string()].map(String::from)).unwrap();
            let g = Game::new(args);
            g.mcp_up.store(true, Ordering::SeqCst);
            *g.token.lock().unwrap() = Some("t".into());
            (g.clone(), Panel { game: g, routes: routes() })
        };
        // The world is the snapshot's or later: one set_settings, every slot cleared, under one approval.
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (_, p) = panel_on(fake_game(tick + 74, "1961-01-02", calls.clone()), "5");
        let (code, res) = post_to(&p, "/api/decision/pull", json!({"url": serve(directive.clone()), "key": key}));
        assert_eq!(code, 200, "{res}");
        let seen = calls.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!(seen[0]["name"], "openttd.ai.set_settings");
        let settings = &seen[0]["arguments"]["settings"];
        assert_eq!(settings["planned_freight_only"], 0);
        assert_eq!(settings["fr_from"], -1);
        assert_eq!(settings[format!("{}from", crate::battlebrain_contract::freight_slot_prefix(crate::battlebrain_contract::FREIGHT_SLOTS))], -1);
        assert!(seen[0]["arguments"]["expires_tick"].as_i64().unwrap() > tick + 74);
        // An earlier save reloaded (its date still before valid_until): refused before any call.
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (g, p) = panel_on(fake_game(tick - 1, "1961-01-01", calls.clone()), "6");
        let since = g.bus.seq();
        // (A key's answer is kept for KEY_TTL, so this pull uses its own decision: a rollback of the rollback.)
        let again = crate::battlebrain_contract::rollback_directive(&snap["value"], 1, Some(&directive["decision"]), "test").unwrap();
        let again_key = again["directive"]["decision"]["idempotency_key"].as_str().unwrap().to_string();
        let (code, res) = post_to(&p, "/api/decision/pull", json!({"url": serve(again["directive"].clone()), "key": again_key}));
        assert_eq!(code, 409, "{res}");
        assert!(res["error"].as_str().unwrap().contains("behind the snapshot"), "{res}");
        assert!(calls.lock().unwrap().is_empty(), "nothing set in a world behind the snapshot");
        let ev = g.bus.since(since, &kinds_of("plan.directive"), 10);
        assert_eq!(ev.last().unwrap()["data"]["choice"]["outcome"], "world_changed");
    }

    /// RESEARCH-ONLY, ignored: the stand-in that tools/research/battlebrain-cutover-check runs the
    /// cutover script against. One real panel per world of $BATTLEBRAIN_STANDIN_WORLDS
    /// (`name=tick,date;...`), each on its own loopback port in front of `fake_game` at that tick;
    /// it writes each panel's URL to $BATTLEBRAIN_STANDIN_DIR/<name>.url and serves until that
    /// directory holds `stop` (at most 15 minutes). The panels share this process's Rhai shared
    /// memory and pull keys, as one panel's calls do. Without the directory it does nothing.
    #[test]
    #[ignore]
    fn the_cutover_rehearsal_stand_in() {
        let Some(dir) = std::env::var_os("BATTLEBRAIN_STANDIN_DIR").map(std::path::PathBuf::from) else { return };
        let worlds = std::env::var("BATTLEBRAIN_STANDIN_WORLDS").unwrap_or_default();
        for w in worlds.split(';').filter(|w| !w.is_empty()) {
            let (name, at) = w.split_once('=').expect("name=tick,date");
            let (tick, date) = at.split_once(',').expect("tick,date");
            let game = fake_game(tick.parse().expect("a tick"), date, Arc::new(Mutex::new(Vec::new())));
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port().to_string();
            let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", &port, "--mcp-port", &game.to_string()].map(String::from)).unwrap();
            let g = Game::new(args);
            g.mcp_up.store(true, Ordering::SeqCst);
            *g.token.lock().unwrap() = Some("t".into());
            std::fs::write(dir.join(format!("{name}.url")), format!("http://127.0.0.1:{port}")).unwrap();
            let p = Arc::new(Panel { game: g, routes: routes() });
            thread::spawn(move || serve(l, p));
        }
        let until = Instant::now() + Duration::from_secs(900);
        while !dir.join("stop").exists() && Instant::now() < until {
            thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn a_directive_decided_without_an_observation_is_refused_and_published() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/decision", l.local_addr().unwrap());
        thread::spawn(move || for mut c in l.incoming().flatten() {
            if read_request(&mut c).is_none() { continue }
            let d = json!({"ai": "PrismaticAI", "settings": "aggressiveness=75", "engine": "test"});
            hub::respond(c, 200, "application/json", d.to_string().as_bytes());
        });
        let (g, p) = bare_panel("4");
        let since = g.bus.seq();
        // The stand-in never reads GET /api/bridge/companies, and this panel has no game to read: the
        // directive is bound to nothing current, so nothing is tried (research/agent-loop.md step 1).
        let (code, out) = post_to(&p, "/api/decision/pull", json!({"url": url}));
        assert_eq!(code, 409, "{out}");
        assert!(out["errors"][0].as_str().unwrap().contains("cannot be read"), "{out}");
        assert!(out.get("applied_rcon").is_none(), "nothing applied: {out}");
        let ev = g.bus.since(since, &kinds_of("plan.directive"), 10);
        assert_eq!(ev.len(), 1, "the refusal is on the bus");
        let choice = &ev[0]["data"]["choice"];
        assert_eq!((choice["accepted"].clone(), choice["outcome"].clone()), (json!(false), json!("stale")));
    }

    /// A freight-line slot reaches the game through the panel only from a request that names its
    /// writer (POST /api/bots/setting {writer}); the raw MCP routes name none, so they set no slot.
    /// Every refusal is answered before the game is asked and is an event; any other setting, and a
    /// named writer while no planner key is set, goes on to the game (none here: its own error).
    #[test]
    fn a_freight_slot_is_sent_only_by_a_named_writer() {
        let (g, p) = bare_panel("5");
        let since = g.bus.seq();
        let (code, out) = post_to(&p, "/api/bots/setting", json!({"company": 0, "name": "fr_from", "value": 3}));
        assert_eq!((code, out["outcome"].as_str()), (422, Some("fr_needs_writer")), "{out}");
        assert!(out["error"].as_str().unwrap().starts_with("fr_from not sent: a change of company 0's freight-line slots names its writer"), "{out}");
        let ev = g.bus.since(since, &kinds_of("panel.ai"), 10);
        assert!(ev.iter().any(|e| e["data"]["text"].as_str().unwrap_or("").contains("fr_from not sent")), "the refusal is an event: {ev:?}");
        assert_eq!(post_to(&p, "/api/bots/setting", json!({"company": 0, "name": "fr_to", "value": 3, "writer": " "})).0, 422, "a blank writer is none");
        // Named, no planner key set: on to the game, which this panel has none of.
        let (code, out) = post_to(&p, "/api/bots/setting", json!({"company": 0, "name": "fr_from", "value": 3, "writer": "owner"}));
        assert_eq!((code, out["fallback"].as_str()), (200, Some("start")), "{out}");
        let (code, out) = post_to(&p, "/api/bots/setting", json!({"company": 0, "name": "rail_a", "value": 3}));
        assert_eq!((code, out["fallback"].as_str()), (200, Some("start")), "not a slot: as before, no writer needed: {out}");
        assert_eq!(post_to(&p, "/api/bots/setting", json!({"company": 0, "name": "fr_from", "value": 3, "writer": 7})).0, 400, "a writer is a string");

        let raw = [("/api/tools/openttd.ai.set_setting", json!({"company": 0, "name": "fr16_cargo", "value": 1})),
                   ("/api/tools/openttd.ai.set_settings", json!({"company": 0, "settings": {"rail_a": 1, "fr2_to": 9}})),
                   ("/api/rpc", json!({"method": "tools/call", "params": {"name": "openttd.ai.set_settings",
                                                                          "arguments": {"company": 0, "settings": {"fr_from": 1}}}}))];
        for (path, body) in raw {
            let (code, out) = post_to(&p, path, body);
            assert_eq!((code, out["outcome"].as_str()), (422, Some("fr_needs_writer")), "{path}: {out}");
            assert!(out["governed"].as_str().unwrap().contains("POST /api/decision/pull"), "{out}");
        }
        // The same routes without a slot go on to the game, as before (no token here: 502).
        assert_eq!(post_to(&p, "/api/tools/openttd.ai.set_setting", json!({"company": 0, "name": "rail_a", "value": 1})).0, 502);
        assert_eq!(post_to(&p, "/api/rpc", json!({"method": "tools/call", "params": {"name": "openttd.ai.set_setting",
                                                                                     "arguments": {"company": 0, "name": "rail_a", "value": 1}}})).0, 502);
        assert_eq!(post_to(&p, "/api/rpc", json!({"method": "tools/list"})).0, 502);
    }

    /// The governance audit's events of a panel since `since`, their data only.
    fn governance_since(g: &Game, since: u64) -> Vec<Value> {
        g.bus.since(since, &kinds_of("governance"), 100).into_iter().map(|e| e["data"].clone()).collect()
    }

    #[test]
    fn who_names_the_credential_kind_and_never_the_credential() {
        let h = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        let cookie = |s: &str| format!("a=1; mcp_owner_8800={s}");
        assert_eq!(Who::of(Some("s3cret"), 8800, &h(&[("Cookie", &cookie("s3cret"))])).credential, "owner-cookie");
        assert_eq!(Who::of(Some("s3cret"), 8800, &h(&[(OWNER_HEADER, "s3cret")])).credential, "token");
        assert_eq!(Who::of(Some("s3cret"), 8800, &h(&[(OWNER_HEADER, "wrong"), ("Cookie", &cookie("nope"))])).credential, "none");
        assert_eq!(Who::of(Some("s3cret"), 8801, &h(&[("Cookie", &cookie("s3cret"))])).credential, "none", "another port's cookie");
        assert_eq!(Who::of(None, 8800, &h(&[(OWNER_HEADER, "")])).credential, "none", "no secret, no owner");
        let w = Who::of(Some("s3cret"), 8800, &h(&[("origin", "http://127.0.0.1:8800"), ("User-Agent", "curl/8"), (PEER_HEADER, "127.0.0.1")]));
        assert_eq!(w, Who { credential: "none", role: "none", origin: Some("http://127.0.0.1:8800".into()), user_agent: Some("curl/8".into()), peer: Some("127.0.0.1".into()) });
        assert_eq!(Who::none(), Who { credential: "none", role: "none", origin: None, user_agent: None, peer: None });
        assert_eq!(Who::of(Some("s3cret"), 8800, &h(&[(OWNER_HEADER, "s3cret")])).role, "owner");
    }

    #[test]
    fn a_console_line_that_sets_the_deadline_and_an_approvers_body_are_governance() {
        assert_eq!(console_governance("prismatic_mcp approval_deadline 7 keep"), Some(("approval_deadline", json!("7 keep"))));
        assert_eq!(console_governance(r#"/Prismatic-MCP "approval_deadline" 3"#), Some(("approval_deadline", json!("3"))));
        for not in ["prismatic_mcp approval_deadline", "prismatic_mcp approvers add 2", "prismatic_mcp status", "echo approval_deadline 3", ""] {
            assert_eq!(console_governance(not), None, "{not}");
        }
        assert_eq!(approvers_change(&json!({"add": 2})), Some(json!({"op": "add", "args": 2})));
        assert_eq!(approvers_change(&json!({"remove": "AbCd1234"})), Some(json!({"op": "remove", "args": "AbCd1234"})));
        assert_eq!(approvers_change(&json!({})), None, "a listing changes nothing");
        assert_eq!(approvers_change(&json!({"add": null})), None);
    }

    #[test]
    fn switching_auto_approve_publishes_who_did_it_in_the_digest_shape() {
        let (g, p) = bare_panel("18811");
        let secret = g.owner_secret.clone().unwrap();
        let since = g.bus.seq();
        g.history.lock().unwrap().push_back(json!({"t": 4242, "date": "1951-03-07"}));
        let on = post_as(&p, "/api/auto-approve", json!({"on": true}), &[(OWNER_HEADER, &secret), ("Origin", "http://127.0.0.1:18811"),
                                                                         ("User-Agent", "check/1"), (PEER_HEADER, "127.0.0.1")]);
        assert_eq!(on.0, 200, "{on:?}");
        let cookie = format!("mcp_owner_18811={secret}");
        assert_eq!(post_as(&p, "/api/auto-approve", json!({"on": false}), &[("Cookie", &cookie)]).0, 200);
        assert_eq!(post_to(&p, "/api/auto-approve", json!({"on": false})).0, 200, "switching off is open to anyone");
        let ev = governance_since(&g, since);
        assert_eq!(ev.len(), 3, "{ev:?}");
        assert_eq!(ev[0], json!({"setting": "auto_approve", "from": false, "to": true, "via": "POST /api/auto-approve", "credential": "token",
                                 "role": "owner", "origin": "http://127.0.0.1:18811", "user_agent": "check/1", "peer": "127.0.0.1",
                                 "game_date": "1951-03-07", "game_tick": 4242}));
        assert_eq!((ev[1]["from"].clone(), ev[1]["to"].clone(), ev[1]["credential"].clone()), (json!(true), json!(false), json!("owner-cookie")));
        assert_eq!((ev[2]["from"].clone(), ev[2]["credential"].clone(), ev[2]["origin"].clone()), (json!(false), json!("none"), Value::Null));
        // The secret reaches no event, whichever way it was sent.
        assert!(!format!("{:?}", g.bus.since(0, &[], 10_000)).contains(&secret));
        assert!(panel_events::declared("governance.changed"));
    }

    #[test]
    fn signing_in_and_out_and_a_console_approver_change_are_governance() {
        let (g, p) = bare_panel("18812");
        let secret = g.owner_secret.clone().unwrap();
        let since = g.bus.seq();
        assert_eq!(post_to(&p, "/api/owner/login", json!({"secret": secret})).0, 204);
        assert_eq!(post_to(&p, "/api/owner/logout", json!({})).0, 204);
        crate::web_access::set_headers("");
        let ev = governance_since(&g, since);
        assert_eq!(ev.len(), 2, "{ev:?}");
        assert_eq!((ev[0]["setting"].clone(), ev[0]["to"].clone(), ev[0]["via"].clone()), (json!("owner_session"), json!(true), json!("POST /api/owner/login")));
        assert_eq!((ev[1]["from"].clone(), ev[1]["to"].clone(), ev[1]["via"].clone()), (json!(true), json!(false), json!("POST /api/owner/logout")));
        assert_eq!((ev[0]["game_date"].clone(), ev[0]["game_tick"].clone()), (Value::Null, Value::Null), "no clock before the first sample");
        // A deadline line reaches no game here (no server runs), so it records nothing: only a sent line is governance.
        let since = g.bus.seq();
        assert_ne!(post_to(&p, "/api/console", json!({"command": "prismatic_mcp approval_deadline 7 keep"})).0, 200);
        assert!(governance_since(&g, since).is_empty());
        assert!(!format!("{:?}", g.bus.since(0, &[], 10_000)).contains(&secret));
    }

    #[test]
    fn a_line_sent_to_the_game_that_changes_governance_is_published() {
        let (g, p) = bare_panel("18813");
        let secret = g.owner_secret.clone().unwrap();
        // A stand-in for the game's console: a process that reads its stdin and keeps nothing.
        let mut child = Command::new("/bin/cat").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        g.console_w.lock().unwrap().stdin = child.stdin.take();
        let since = g.bus.seq();
        let (code, out) = post_as(&p, "/api/console", json!({"command": "prismatic_mcp approval_deadline 7 keep", "wait": 0}),
                                  &[("User-Agent", "mcp-e2e"), (OWNER_HEADER, &secret)]);
        assert_eq!(code, 200, "{out}");
        let (code, out) = post_as(&p, "/api/approvers", json!({"add": 2}), &[(OWNER_HEADER, &secret)]);
        assert_eq!(code, 200, "{out}");
        assert_eq!(post_as(&p, "/api/approvers", json!({}), &[(OWNER_HEADER, &secret)]).0, 200, "a listing");
        let ev = governance_since(&g, since);
        assert_eq!(ev.len(), 2, "{ev:?}");
        assert_eq!((ev[0]["setting"].clone(), ev[0]["from"].clone(), ev[0]["to"].clone(), ev[0]["via"].clone(), ev[0]["user_agent"].clone()),
                   (json!("approval_deadline"), Value::Null, json!("7 keep"), json!("POST /api/console"), json!("mcp-e2e")));
        assert_eq!((ev[1]["setting"].clone(), ev[1]["to"].clone(), ev[1]["via"].clone(), ev[1]["credential"].clone()),
                   (json!("approvers"), json!({"op": "add", "args": 2}), json!("POST /api/approvers"), json!("token")));
        let since = g.bus.seq();
        assert_eq!(post_to(&p, "/api/console", json!({"command": "companies", "wait": 0})).0, 200);
        assert!(governance_since(&g, since).is_empty(), "a line that changes no governance publishes none");
        assert!(!format!("{:?}", g.bus.since(0, &[], 10_000)).contains(&secret));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn the_governance_a_panel_starts_with_is_published_once() {
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", "18814", "--auto-approve"].map(String::from)).unwrap();
        let g = Game::new(args);
        let ev = governance_since(&g, 0);
        assert_eq!(ev.len(), 1, "{ev:?}");
        assert_eq!((ev[0]["setting"].clone(), ev[0]["from"].clone(), ev[0]["to"].clone(), ev[0]["via"].clone(), ev[0]["credential"].clone()),
                   (json!("auto_approve"), json!(false), json!(true), json!("--auto-approve at start"), json!("none")));
        let since = g.bus.seq();
        g.start_governance(Some(" 7:approve "));
        let ev = governance_since(&g, since);
        assert_eq!(ev.len(), 2, "{ev:?}");
        assert_eq!((ev[1]["setting"].clone(), ev[1]["to"].clone(), ev[1]["via"].clone()),
                   (json!("approval_deadline"), json!("7:approve"), json!(format!("{DEADLINE_ENV} at start"))));
        let (g, _) = bare_panel("18815");
        let since = g.bus.seq();
        g.start_governance(Some("  "));
        g.start_governance(None);
        assert!(governance_since(&g, since).is_empty(), "no flag, no deadline: nothing");
        assert!(governance_since(&g, 0).is_empty());
    }

    #[test]
    fn the_panel_records_the_address_a_request_came_from_never_one_it_claims() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", &port.to_string()].map(String::from)).unwrap();
        let g = Game::new(args);
        let p = Arc::new(Panel { game: g.clone(), routes: routes() });
        thread::spawn(move || serve(l, p));
        let since = g.bus.seq();
        let body = r#"{"on": false}"#;
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(c, "POST /api/auto-approve HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Panel-Peer: 10.9.9.9\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut raw = String::new();
        let _ = c.read_to_string(&mut raw);
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        let ev = governance_since(&g, since);
        assert_eq!(ev.len(), 1, "{ev:?}");
        assert_eq!(ev[0]["peer"], json!("127.0.0.1"), "the claimed 10.9.9.9 is dropped");
    }

    /// A 0600 file holding `text`, in a fresh directory of this test.
    fn secret_file(name: &str, text: &str, mode: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("mcp-panel-run-token-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("run-token");
        std::fs::write(&f, text).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(mode)).unwrap();
        f
    }

    #[test]
    fn the_run_token_names_the_stacks_own_callers_and_never_opens_an_owners_route() {
        let f = secret_file("route", "rt0ken-abc\n", 0o600);
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", "18821", "--run-token-file", f.to_str().unwrap()].map(String::from)).unwrap();
        assert_eq!(args.run_token_file.as_deref(), f.to_str());
        let g = Game::new(args);
        g.mcp_up.store(true, Ordering::SeqCst);
        let p = Panel { game: g.clone(), routes: routes() };
        assert_eq!(g.run_token.as_deref(), Ok("rt0ken-abc"), "the file's text, trimmed");
        assert_eq!(run_token_for("http://127.0.0.1:18821/").as_deref(), Some("rt0ken-abc"));
        let run = [(RUN_TOKEN_HEADER, "rt0ken-abc")];
        // The run token is no owner secret: approving and switching auto-approve on stay the owner's.
        assert_eq!(post_as(&p, "/api/approve", json!({"id": 1}), &run).0, 401);
        assert_eq!(post_as(&p, "/api/auto-approve", json!({"on": true}), &run).0, 401);
        assert_eq!(post_as(&p, "/api/approvers", json!({"add": 2}), &run).0, 401);
        let since = g.bus.seq();
        assert_eq!(post_as(&p, "/api/auto-approve", json!({"on": false}), &run).0, 200);
        let ev = governance_since(&g, since);
        assert_eq!((ev[0]["credential"].clone(), ev[0]["role"].clone()), (json!("token"), json!("run")), "{ev:?}");
        let h = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        let owner = g.owner_secret.clone().unwrap();
        assert_eq!(Who::of_run(Some(&owner), Some("rt0ken-abc"), 18821, &h(&[(OWNER_HEADER, &owner), (RUN_TOKEN_HEADER, "rt0ken-abc")])).role, "owner",
                   "the owner secret wins");
        assert_eq!(Who::of_run(Some(&owner), Some("rt0ken-abc"), 18821, &h(&[(RUN_TOKEN_HEADER, "wrong")])).role, "none");
        assert_eq!(Who::of_run(Some(&owner), None, 18821, &h(&[(RUN_TOKEN_HEADER, "rt0ken-abc")])).role, "none", "a panel without a token");
        assert!(!format!("{:?}", g.bus.since(0, &[], 10_000)).contains("rt0ken-abc"), "the token reaches no event");
    }

    #[test]
    fn a_run_token_file_others_may_read_stops_the_panel_at_start() {
        let f = secret_file("wide", "rt0ken-wide", 0o644);
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", "18822", "--run-token-file", f.to_str().unwrap()].map(String::from)).unwrap();
        let g = Game::new(args);
        let why = g.start().unwrap_err();
        assert!(why.contains("readable beyond its owner") && why.contains("chmod 600"), "{why}");
        assert!(!g.server_running(), "no game starts");
        assert!(run_token_for("http://127.0.0.1:18822").is_none());
        // Without --run-token-file the panel makes one of its own for its in-process scripts.
        let (g, _) = bare_panel("18823");
        assert_eq!(g.run_token.as_ref().map(String::len), Ok(64));
        assert_eq!(run_token_for("http://127.0.0.1:18823"), g.run_token.clone().ok());
    }

    #[test]
    fn a_script_the_panel_runs_sends_its_run_token() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx_h, rx_h) = mpsc::channel();
        thread::spawn(move || for mut c in l.incoming().flatten() {
            if let Some(req) = read_request(&mut c) { let _ = tx_h.send((req.path, req.headers)); }
            hub::respond(c, 200, "application/json", b"{}");
        });
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", &port.to_string()].map(String::from)).unwrap();
        let g = Game::new(args);
        let (tx, rx) = mpsc::channel();
        let h = spawn_script(g.panel_url(), "status()".into(), Arc::new(AtomicBool::new(false)), tx);
        assert_eq!(h.join().unwrap(), 0, "{:?}", rx.try_iter().collect::<Vec<_>>().len());
        let seen: Vec<(String, Vec<(String, String)>)> = rx_h.try_iter().collect();
        assert!(seen.iter().any(|(path, _)| path == "/api/status"), "{seen:?}");
        for (path, headers) in &seen {
            let sent = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(RUN_TOKEN_HEADER)).map(|(_, v)| v.clone());
            assert_eq!(sent, g.run_token.clone().ok(), "{path} carries the run token");
        }
    }

    #[test]
    fn the_console_reads_and_chats_for_anyone_and_changes_the_game_only_for_the_owner_or_the_stack() {
        for read in ["companies", "/Clients", "getdate", "list_ai", "setting", "setting max_loan", "prismatic_mcp status", "prismatic_mcp approvals", "help"] {
            assert_eq!(console_authority(read), Authority::Read, "{read}");
        }
        for chat in ["say \"hi\"", "echo x", "sayclient 2 hi", "prismatic_mcp deny 7"] {
            assert_eq!(console_authority(chat), Authority::Chat, "{chat}");
        }
        for owner in ["start_ai PrismaticAI", "/Start-AI x", "setting max_loan 500000", "newgame", "research_coplay allow 1", "prismatic_mcp deny all",
                      "prismatic_mcp status now", "prismatic_mcp approval_deadline 7 keep", "plan build a line", "fake consume 4", "", "rescan_ai"] {
            assert_eq!(console_authority(owner), Authority::Owner, "{owner}");
        }
        assert!(may_change(&Who { role: "owner", ..Who::none() }) && may_change(&Who { role: "run", ..Who::none() }) && !may_change(&Who::none()));
        let (g, p) = bare_panel("18831");
        let secret = g.owner_secret.clone().unwrap();
        let run = g.run_token.clone().unwrap();
        let mut child = Command::new("/bin/cat").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        g.console_w.lock().unwrap().stdin = child.stdin.take();
        let line = |cmd: &str, h: &[(&str, &str)]| post_as(&p, "/api/console", json!({"command": cmd, "wait": 0}), h).0;
        assert_eq!(line("companies", &[]), 200, "a read is open");
        assert_eq!(line("say \"hello\"", &[]), 200, "a chat line is open");
        let (code, out) = post_to(&p, "/api/console", json!({"command": "start_ai PrismaticAI", "wait": 0}));
        assert_eq!((code, out["refused"].clone(), out["needs"].clone(), out["sign_in"].clone()),
                   (401, json!("owner_secret"), json!("owner_or_token"), json!("/api/owner/login")), "{out}");
        assert_eq!(line("start_ai PrismaticAI", &[(OWNER_HEADER, &secret)]), 200, "the owner");
        assert_eq!(line("start_ai PrismaticAI", &[(RUN_TOKEN_HEADER, &run)]), 200, "the stack's own helpers");
        assert_eq!(line("start_ai PrismaticAI", &[(RUN_TOKEN_HEADER, "wrong")]), 401);
        assert_eq!(line("prismatic_mcp approvers add 2", &[(OWNER_HEADER, &secret)]), 403, "the approvers stay the owner's route, never a line");
        assert!(format!("{:?}", g.events.lock().unwrap()).contains("refused console line without a credential: start_ai PrismaticAI"));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn starting_an_ai_without_a_credential_asks_the_owner_through_the_approval_tool() {
        assert_eq!(settings_map("a=1, b = -2").unwrap(), json!({"a": 1, "b": -2}));
        assert_eq!(settings_map("").unwrap(), json!({}));
        assert!(settings_map("a").unwrap_err().contains("not name=value"));
        assert!(settings_map("a=x").unwrap_err().contains("not an integer"));
        // A game without openttd.ai.start (none here: no MCP answers): nothing starts unapproved.
        let (g, p) = bare_panel("18832");
        let (code, out) = post_to(&p, "/api/start-ai", json!({"ai": "X", "settings": "a=1"}));
        assert_eq!((code, out["needs"].clone()), (401, json!("owner_or_token")), "{out}");
        assert!(!g.ai_started.load(Ordering::SeqCst));
        assert!(panel_bot::start_bot(&g, "X", json!({"a": 1})).unwrap_err().contains("no openttd.ai.start"), "no console fallback");
        // With the tool: the request goes to the game's approval; a stand-in MCP holds it pending.
        let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let mcp = l.local_addr().unwrap().port();
        let seen_by = calls.clone();
        thread::spawn(move || for mut c in l.incoming().flatten() {
            let Some(req) = read_request(&mut c) else { continue };
            let call: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let out = if call["method"] == "tools/list" {
                json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [{"name": "openttd.ai.start"}]}})
            } else {
                seen_by.lock().unwrap().push(call["params"].clone());
                let answer = json!({"status": "pending_approval", "approval_id": 9, "message": "Approval required in OpenTTD: prismatic_mcp approve 9"});
                json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [{"type": "text", "text": answer.to_string()}]}})
            };
            hub::respond(c, 200, "application/json", out.to_string().as_bytes());
        });
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", "18833", "--mcp-port", &mcp.to_string()].map(String::from)).unwrap();
        let g = Game::new(args);
        g.mcp_up.store(true, Ordering::SeqCst);
        *g.token.lock().unwrap() = Some("t".into());
        let p = Panel { game: g.clone(), routes: routes() };
        let (code, out) = post_to(&p, "/api/start-ai", json!({"ai": "X", "settings": "a=1"}));
        assert_ne!(code, 401, "{out}");
        assert_eq!(out["approval_id"], json!(9), "{out}");
        let seen = calls.lock().unwrap().clone();
        assert!(seen.iter().any(|c| c["name"] == "openttd.ai.start" && c["arguments"] == json!({"name": "X", "settings": {"a": 1}})), "{seen:?}");
        assert!(format!("{:?}", g.events.lock().unwrap()).contains("sent for approval (no credential)"));
        assert!(!g.ai_started.load(Ordering::SeqCst), "not started: the owner decides");
    }

    #[test]
    fn attaching_a_client_and_a_scripts_authority_follow_the_caller() {
        let (g, p) = bare_panel("18834");
        let run = g.run_token.clone().unwrap();
        let (code, out) = post_to(&p, "/api/view/attach", json!({"token_file": "/nonexistent"}));
        assert_eq!((code, out["needs"].clone()), (401, json!("owner_or_token")), "{out}");
        assert_eq!(post_as(&p, "/api/view/attach", json!({"token_file": "/nonexistent"}), &[(RUN_TOKEN_HEADER, &run)]).0, 400, "past the gate");
        assert_eq!(g.script_token(&Who::none()), None, "a script sent without a credential holds no token");
        assert_eq!(g.script_token(&Who { role: "owner", ..Who::none() }), Some(run.clone()));
        assert_eq!(g.script_token(&Who { role: "run", ..Who::none() }), Some(run.clone()));
        assert_eq!(OWNER_ROUTES, ["/api/approve", "/api/approve/many", "/api/approve/all", "/api/deny/many", "/api/deny/all",
                                  "/api/rules", "/api/rules/remove", "/api/approvers", "/api/owner/once"]);
        for path in OWNER_ROUTES {
            let body = json!({"id": 1, "ids": [1], "effect": "deny", "tool": "x", "add": 2, "minutes": 5});
            assert_eq!(post_as(&p, path, body, &[(RUN_TOKEN_HEADER, &run)]).0, 401, "the run token opens no owner route: {path}");
        }
    }

    #[test]
    fn a_script_sent_without_a_credential_cannot_change_the_game_through_the_console() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", &port.to_string()].map(String::from)).unwrap();
        let g = Game::new(args);
        let mut child = Command::new("/bin/cat").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        g.console_w.lock().unwrap().stdin = child.stdin.take();
        let p = Arc::new(Panel { game: g.clone(), routes: routes() });
        let pp = p.clone();
        thread::spawn(move || serve(l, pp));
        let script = r#"let out = console("start_ai PrismaticAI"); print(out); "#;
        let (code, out) = post_to(&p, "/api/rhai", json!({"script": script, "timeout": 20}));
        assert_eq!(code, 200, "{out}");
        assert!(format!("{:?}", g.events.lock().unwrap()).contains("refused console line without a credential"), "{out}");
        let secret = g.owner_secret.clone().unwrap();
        let (code, out) = post_as(&p, "/api/rhai", json!({"script": script, "timeout": 20}), &[(OWNER_HEADER, &secret)]);
        assert_eq!(code, 200, "{out}");
        assert!(format!("{:?}", g.events.lock().unwrap()).contains("console: start_ai PrismaticAI") || g.bus.since(0, &kinds_of("panel.console"), 100).iter()
                .any(|e| e["data"]["text"] == json!("start_ai PrismaticAI")), "the owner's script reaches the console");
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_one_time_sign_in_link_serves_once_within_a_minute() {
        let mut nonces = HashMap::new();
        let t0 = Instant::now();
        let a = owner_once(&mut nonces, t0).unwrap();
        assert_eq!(a.len(), 32);
        assert!(spend_once(&mut nonces, &a, t0 + Duration::from_secs(59)), "within the minute");
        assert!(!spend_once(&mut nonces, &a, t0 + Duration::from_secs(59)), "never twice");
        let b = owner_once(&mut nonces, t0).unwrap();
        assert!(!spend_once(&mut nonces, &b, t0 + OWNER_ONCE_TTL), "not after the minute");
        assert!(!spend_once(&mut nonces, "never-made", t0));
        let _ = owner_once(&mut nonces, t0);
        let _ = owner_once(&mut nonces, t0 + OWNER_ONCE_TTL + Duration::from_secs(1));
        assert_eq!(nonces.len(), 1, "a spent minute's nonces are dropped");
        assert_eq!(OWNER_COOKIE_MAX_AGE, 43_200, "the owner's cookie lasts 12 hours");
        let (g, p) = bare_panel("18841");
        let secret = g.owner_secret.clone().unwrap();
        let run = g.run_token.clone().unwrap();
        assert_eq!(post_to(&p, "/api/owner/once", json!({})).0, 401);
        assert_eq!(post_as(&p, "/api/owner/once", json!({}), &[(RUN_TOKEN_HEADER, &run)]).0, 401, "the run token makes no sign-in link");
        let (code, out) = post_as(&p, "/api/owner/once", json!({}), &[(OWNER_HEADER, &secret)]);
        assert_eq!(code, 200, "{out}");
        let n = out["once"].as_str().unwrap().to_string();
        assert_eq!((out["url"].clone(), out["expires_s"].clone()), (json!(format!("http://127.0.0.1:18841/?once={n}")), json!(60)));
        assert!(!out.to_string().contains(&secret));
    }

    /// One GET over TCP to a panel: the raw answer.
    fn get_raw(port: u16, target: &str) -> String {
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(c, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut raw = String::new();
        let _ = c.read_to_string(&mut raw);
        raw
    }

    #[test]
    fn opening_the_page_with_a_link_or_the_secret_signs_in_and_redirects_to_it_without_the_query() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let args = parse_args(&["--bin", "/nonexistent/openttd", "--panel-port", &port.to_string()].map(String::from)).unwrap();
        let g = Game::new(args);
        let p = Arc::new(Panel { game: g.clone(), routes: routes() });
        let secret = g.owner_secret.clone().unwrap();
        thread::spawn(move || serve(l, p));
        let n = owner_once(&mut g.owner_nonces.lock().unwrap(), Instant::now()).unwrap();
        let since = g.bus.seq();
        let first = get_raw(port, &format!("/?once={n}"));
        assert!(first.starts_with("HTTP/1.1 303 See Other"), "{first}");
        assert!(first.contains("\r\nLocation: /\r\n"), "{first}");
        assert!(first.contains(&format!("Set-Cookie: mcp_owner_{port}={secret}; HttpOnly; SameSite=Strict; Path=/; Max-Age=43200")), "{first}");
        let ev = governance_since(&g, since);
        assert_eq!((ev[0]["setting"].clone(), ev[0]["via"].clone(), ev[0]["role"].clone()), (json!("owner_session"), json!("GET ?once="), json!("owner")), "{ev:?}");
        let again = get_raw(port, &format!("/?once={n}"));
        assert!(again.starts_with("HTTP/1.1 401") && again.contains("owner_once") && !again.contains("Set-Cookie"), "{again}");
        let owner = get_raw(port, &format!("/scripts?owner={secret}"));
        assert!(owner.starts_with("HTTP/1.1 303") && owner.contains("\r\nLocation: /scripts\r\n") && owner.contains("Set-Cookie: mcp_owner_"), "{owner}");
        let wrong = get_raw(port, "/?owner=wrong");
        assert!(wrong.starts_with("HTTP/1.1 303") && wrong.contains("\r\nLocation: /\r\n") && !wrong.contains("Set-Cookie"), "{wrong}");
    }
}
