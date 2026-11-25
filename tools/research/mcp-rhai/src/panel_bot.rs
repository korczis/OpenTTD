//! RESEARCH-ONLY: the owner's bot in the control panel (src/panel.rs), ported from
//! tools/research/mcp-panel/server.py: the AIs the owner can run with their settings, what a setting
//! may name, starting a bot, changing a running bot's setting, and the routes the planner's portfolio
//! hands the bot when the panel starts it.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::panel::{py_int, py_json, py_str, run, Game};
use crate::calibrate::{self, Scope};
use crate::panel_events::{self, Stored};
use crate::panel_store::Across;
use crate::store_sql::Named;
use crate::duel_outcomes;

fn ai_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Documents/OpenTTD/ai")
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The text between `open` and the first `close` after it, for every occurrence: (inside, rest).
/// A string literal at the start of `s`, `\"` escapes kept as written: (inside, rest).
fn quoted(s: &str) -> Option<(&str, &str)> {
    let s = s.strip_prefix('"')?;
    let mut escaped = false;
    let end = s.char_indices().find(|&(_, c)| {
        let close = c == '"' && !escaped;
        escaped = c == '\\' && !escaped;
        close
    })?.0;
    Some((&s[..end], &s[end + 1..]))
}

/// The value of `key = value` in a Squirrel table, as server.py's `field()` reads it: the first `key`
/// at a word boundary followed by `=`, its value a quoted string or the text up to a comma or line
/// end, stripped of blanks and quotes.
pub fn field(block: &str, key: &str) -> Option<String> {
    let mut from = 0;
    while let Some(p) = block[from..].find(key) {
        let at = from + p;
        from = at + key.len();
        if block[..at].chars().next_back().map_or(false, is_word) {
            continue;
        }
        let Some(rest) = block[at + key.len()..].trim_start().strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        if let Some((inside, mut tail)) = quoted(rest) {
            // `"fr" + "2" + "_from"` (a loop unrolled by `unroll_loops`) is one string; a term that
            // is not a literal cannot be resolved here, so the value is unknown.
            let mut value = inside.to_string();
            while let Some(next) = tail.trim_start().strip_prefix('+') {
                let (part, after) = quoted(next.trim_start())?;
                value.push_str(part);
                tail = after;
            }
            return Some(value);
        }
        let end = rest.find([',', '\n']).unwrap_or(rest.len());
        if end == 0 {
            continue;
        }
        return Some(rest[..end].trim().trim_matches('"').to_string());
    }
    None
}

fn field_int(block: &str, key: &str) -> Option<i64> {
    field(block, key)?.parse().ok()
}

/// `AddLabels("name", {_0 = "off", _1 = "on"})` of an info.nut, by setting name.
fn labels(text: &str) -> BTreeMap<String, Map<String, Value>> {
    let mut out = BTreeMap::new();
    let mut from = 0;
    while let Some(p) = text[from..].find("AddLabels(") {
        let at = from + p + "AddLabels(".len();
        from = at;
        let Some((name, rest)) = quoted(text[at..].trim_start()).filter(|(n, _)| !n.is_empty()) else { continue };
        let Some(rest) = rest.trim_start().strip_prefix(',').map(str::trim_start).and_then(|r| r.strip_prefix('{')) else { continue };
        // The body ends at the first `}` that a `)` follows.
        let Some(end) = rest.match_indices('}').map(|(i, _)| i).find(|i| rest[i + 1..].trim_start().starts_with(')')) else { continue };
        let mut map = Map::new();
        let body = &rest[..end];
        let mut b = 0;
        while let Some(q) = body[b..].find('_') {
            let s = &body[b + q + 1..];
            b += q + 1;
            let n = s.chars().enumerate().take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && *c == '-')).count();
            if n == 0 || &s[..n] == "-" { continue; }
            let Some((label, _)) = s[n..].trim_start().strip_prefix('=').map(str::trim_start).and_then(quoted) else { continue };
            map.insert(s[..n].to_string(), json!(label));
        }
        out.insert(name.to_string(), map);
    }
    out
}

/// Walk `s` as Squirrel source: `f(byte index, char, outside a literal)` for every char, a string
/// or char literal (with `\` escapes) counted as inside. `f` returning false stops the walk.
fn walk_code(s: &str, mut f: impl FnMut(usize, char, bool) -> bool) {
    let (mut quote, mut escaped) = (None::<char>, false);
    for (i, c) in s.char_indices() {
        let outside = quote.is_none();
        match quote {
            Some(_) if escaped => escaped = false,
            Some(_) if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            None if c == '"' || c == '\'' => quote = Some(c),
            _ => {}
        }
        if !f(i, c, outside) { return; }
    }
}

/// `text` without `//` and `/* */` comments (each replaced by one blank), literals left as they are,
/// so a brace or quote in a comment cannot end a loop body or open a string.
pub fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let (mut quote, mut escaped) = (None::<char>, false);
    while let Some((i, c)) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if escaped { escaped = false; } else if c == '\\' { escaped = true; } else if c == q { quote = None; }
            continue;
        }
        let rest = &text[i..];
        if rest.starts_with("//") || rest.starts_with("/*") {
            let end = if rest.starts_with("//") { rest.find('\n').unwrap_or(rest.len()) } else { rest[2..].find("*/").map_or(rest.len(), |e| e + 4) };
            out.push(' ');
            while chars.peek().map_or(false, |&(k, _)| k < i + end) { chars.next(); }
            continue;
        }
        if c == '"' || c == '\'' { quote = Some(c); }
        out.push(c);
    }
    out
}

/// The most text unrolling may write, so nested loops cannot blow the panel's memory.
const UNROLL_LIMIT: usize = 4 << 20;

/// `text` with every `for (local i = a; i <= b; i++) { ... }` (or `<`) written out once per `i`,
/// the loop variable outside literals replaced by its value as a string literal, so settings an
/// info.nut declares in a loop (PrismaticAI's fr2_* to fr16_*) are read by name. Nested loops are
/// unrolled too. A loop of another shape is left as it is. Comments must be stripped first
/// (`strip_comments`).
pub fn unroll_loops(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(p) = rest.find("for (local ") {
        out.push_str(&rest[..p]);
        let after = &rest[p..];
        match unroll_one(after) {
            Some((written, used)) => { out.push_str(&written); rest = &after[used..]; }
            None => { out.push_str("for (local "); rest = &after["for (local ".len()..]; }
        }
    }
    out.push_str(rest);
    out
}

/// One `for (local ...)` loop at the start of `s`, unrolled, and how many bytes of `s` it took.
fn unroll_one(s: &str) -> Option<(String, usize)> {
    let head_end = s.find(')')?;
    let head = &s["for (local ".len()..head_end];
    let parts: Vec<&str> = head.split(';').map(str::trim).collect();
    let [init, cond, step] = parts[..] else { return None };
    let (var, start) = init.split_once('=')?;
    let var = var.trim();
    if var.is_empty() || !var.chars().all(is_word) { return None; }
    let start: i64 = start.trim().parse().ok()?;
    let (end, inclusive) = if let Some(e) = cond.strip_prefix(var).map(str::trim_start).and_then(|c| c.strip_prefix("<=")) {
        (e.trim().parse::<i64>().ok()?, true)
    } else {
        (cond.strip_prefix(var)?.trim_start().strip_prefix('<')?.trim().parse::<i64>().ok()?, false)
    };
    if step != format!("{var}++") { return None; }
    let open = head_end + 1 + s[head_end + 1..].find('{')?;
    if !s[head_end + 1..open].trim().is_empty() { return None; }
    // The body's closing brace, braces inside literals not counted.
    let (mut depth, mut close) = (0usize, None);
    walk_code(&s[open..], |i, c, outside| {
        if outside && c == '{' { depth += 1; }
        if outside && c == '}' {
            depth -= 1;
            if depth == 0 { close = Some(open + i); return false; }
        }
        true
    });
    let close = close?;
    let body = &s[open + 1..close];
    let last = if inclusive { end } else { end.checked_sub(1)? };
    let count = last.checked_sub(start)?;
    if !(0..=1000).contains(&count) { return None; }
    let mut written = String::new();
    for i in start..=last {
        written.push_str(&substitute(body, var, &format!("\"{i}\"")));
        if written.len() > UNROLL_LIMIT { return None; }
    }
    let written = unroll_loops(&written);
    if written.len() > UNROLL_LIMIT { return None; }
    Some((written, close + 1))
}

/// `body` with the identifier `var` outside literals replaced by `with`.
fn substitute(body: &str, var: &str, with: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut skip_to = 0usize;
    walk_code(body, |i, c, outside| {
        if i < skip_to { return true; }
        if outside && body[i..].starts_with(var)
            && (i == 0 || !is_word(body[..i].chars().next_back().unwrap()))
            && !body[i + var.len()..].chars().next().map_or(false, is_word) {
            out.push_str(with);
            skip_to = i + var.len();
            return true;
        }
        out.push(c);
        true
    });
    out
}

/// Settings as an AI's info.nut declares them, in openttd.ai.list's shape (flags, labels, value =
/// the default, editable), so the panel draws one kind of control for either source.
pub fn info_settings(text: &str) -> Vec<Value> {
    let text = &unroll_loops(&strip_comments(text));
    let labels = labels(text);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = text[from..].find("AddSetting({") {
        let at = from + p + "AddSetting({".len();
        let Some(end) = text[at..].find("});") else { break };
        let block = &text[at..at + end];
        from = at + end;
        let flags_text = field(block, "flags").unwrap_or_default();
        let flags: Vec<&str> = [("boolean", "CONFIG_BOOLEAN"), ("in_game", "CONFIG_INGAME"), ("developer", "CONFIG_DEVELOPER")]
            .iter().filter(|(_, t)| flags_text.contains(t)).map(|(f, _)| *f).collect();
        // A name that is not a literal (or a loop unrolled into one) is skipped, not shown bare.
        let Some(name) = field(block, "name") else { continue };
        let name = Some(name);
        let default = field_int(block, "custom_value");
        let (mut mn, mut mx) = (field_int(block, "min_value"), field_int(block, "max_value"));
        if flags.contains(&"boolean") {
            (mn, mx) = (Some(0), Some(1));
        }
        let step = field_int(block, "step_size").filter(|s| *s != 0).unwrap_or(1);
        let lab = name.as_ref().and_then(|n| labels.get(n)).cloned().unwrap_or_default();
        out.push(json!({"name": name, "description": field(block, "description"), "min": mn, "max": mx, "step": step,
                        "default": default, "flags": flags, "labels": lab, "value": default, "editable": true,
                        "ingame": flags.contains(&"in_game")}));
    }
    out
}

/// The name an info.nut gives its AI: `GetName() { return "..."; }`.
pub fn info_name(text: &str) -> Option<String> {
    let mut from = 0;
    while let Some(p) = text[from..].find("GetName()") {
        let at = from + p + "GetName()".len();
        from = at;
        let Some(rest) = text[at..].trim_start().strip_prefix('{') else { continue };
        let Some(rest) = rest.trim_start().strip_prefix("return") else { continue };
        if let Some((name, _)) = quoted(rest.trim_start()).filter(|(n, _)| !n.is_empty()) {
            return Some(name.to_string());
        }
    }
    None
}

fn data_of(v: Value) -> Value {
    match v {
        Value::Object(mut m) if m.contains_key("data") => m.remove("data").unwrap_or(Value::Null),
        other => other,
    }
}

fn key(v: &Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())
}

/// The AIs the owner can run as their representative, with their settings, and the running AI
/// companies with the value each setting has now. One shape whatever the source: from
/// openttd.ai.list when the game offers it (values read from the running scripts), else from each
/// AI's info.nut (defaults only, and no live change possible).
pub fn bots(g: &Arc<Game>) -> Result<Value, String> {
    let tools = g.tool_names();
    let mut infos: Vec<PathBuf> = std::fs::read_dir(ai_dir()).map(|d| d.filter_map(|e| e.ok().map(|e| e.path().join("info.nut")))
        .filter(|p| p.is_file()).collect()).unwrap_or_default();
    infos.sort();
    let mut ais = Vec::new();
    for info in infos {
        let text = String::from_utf8_lossy(&std::fs::read(&info).unwrap_or_default()).into_owned();
        let dir = info.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        ais.push(json!({"ai": info_name(&text).unwrap_or_else(|| dir.clone()), "dir": dir, "settings": info_settings(&text)}));
    }
    let mut companies = BTreeMap::new();
    for c in g.resource("openttd://companies")?.pointer("/data/companies").and_then(Value::as_array).cloned().unwrap_or_default() {
        if c.get("is_ai").map_or(false, |v| v.as_bool().unwrap_or(false)) {
            companies.insert(key(&c["id"]), c);
        }
    }
    let mut running = Vec::new();
    let mut source = "info.nut (openttd.ai.list not in tools/list yet)";
    let live_list = tools.contains("openttd.ai.list");
    if live_list {
        let data = g.tool("openttd.ai.list", json!({}))?;
        let data = if data.is_object() { data_of(data) } else { json!({}) };
        source = "mcp:openttd.ai.list";
        // An AI installed but without an info.nut under ~/Documents/OpenTTD/ai still appears, with no settings to set.
        let known: HashSet<String> = ais.iter().map(|a| py_str(a.get("ai"))).collect();
        for a in data["available"].as_array().into_iter().flatten() {
            if !known.contains(&py_str(a.get("name"))) {
                ais.push(json!({"ai": a.get("name"), "dir": null, "settings": []}));
            }
        }
        for c in data["companies"].as_array().into_iter().flatten() {
            let base = companies.get(&key(&c["company"])).cloned().unwrap_or(json!({}));
            let mut settings = c.get("settings").cloned().unwrap_or(json!([]));
            for st in settings.as_array_mut().into_iter().flatten() {
                let ingame = st["flags"].as_array().map_or(false, |f| f.contains(&json!("in_game")));
                if st.is_object() { st["ingame"] = json!(ingame); }
            }
            running.push(json!({"id": c.get("company"), "name": base.get("name").cloned().unwrap_or(json!("")), "money": base.get("money"),
                                "ai": c.get("ai"), "running": c.get("running"), "settings": settings}));
        }
    } else {
        for c in companies.values() {
            running.push(json!({"id": c["id"], "name": c.get("name").cloned().unwrap_or(json!("")), "money": c.get("money"),
                                "ai": null, "running": true, "settings": []}));
        }
    }
    Ok(json!({"source": source, "ais": ais, "running": running,
              "live": {"list": live_list, "set_setting": tools.contains("openttd.ai.set_setting")}}))
}

/// What a town, industry or cargo setting may name: towns from openttd://towns (by population), and
/// from openttd://world/graph where the game serves it, industries with what they produce and accept
/// and every cargo they name. A kind the game cannot list is empty, and the panel then shows a number
/// box for it instead of a picker with nothing in it.
pub fn bot_choices(g: &Arc<Game>) -> Result<Value, String> {
    let mut towns = Vec::new();
    let mut industries = Vec::new();
    let mut cargos: BTreeMap<i64, String> = BTreeMap::new();
    let t = data_of(g.resource("openttd://towns")?);
    for n in t["towns"].as_array().into_iter().flatten() {
        towns.push(json!({"id": n.get("id"), "name": n.get("name").cloned().unwrap_or_else(|| json!(format!("town {}", py_str(n.get("id"))))),
                          "population": n.get("population").cloned().unwrap_or(json!(0))}));
    }
    let gr = data_of(g.resource("openttd://world/graph")?);
    let had_towns = !towns.is_empty();
    for n in gr["nodes"].as_array().into_iter().flatten() {
        let id = py_str(n.get("id"));
        let (kind, idx) = id.split_once(':').unwrap_or((id.as_str(), ""));
        let Ok(idx) = idx.parse::<i64>() else { continue };
        let names = |k: &str| -> Vec<Value> { n[k].as_array().into_iter().flatten().map(|p| p.get("name").cloned().unwrap_or(Value::Null)).collect() };
        if kind == "town" && !had_towns {
            towns.push(json!({"id": idx, "name": n.get("name").cloned().unwrap_or_else(|| json!(format!("town {idx}"))),
                              "population": n.get("population").cloned().unwrap_or(json!(0))}));
        } else if kind == "industry" {
            industries.push(json!({"id": idx, "name": n.get("name").cloned().unwrap_or_else(|| json!(format!("industry {idx}"))),
                                   "produces": names("produces"), "accepts": names("accepts")}));
            for c in n["produces"].as_array().into_iter().flatten().chain(n["accepts"].as_array().into_iter().flatten()) {
                if let Some(id) = c.get("cargo").filter(|v| !v.is_null()).and_then(|v| py_int(v).ok()) {
                    cargos.insert(id, c.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("cargo {id}")));
                }
            }
        }
    }
    towns.sort_by(|a, b| b["population"].as_f64().unwrap_or(0.0).total_cmp(&a["population"].as_f64().unwrap_or(0.0)));
    industries.sort_by(|a, b| py_str(a.get("name")).cmp(&py_str(b.get("name"))));
    let mut cargo: Vec<(i64, String)> = cargos.into_iter().collect();
    cargo.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(json!({"town": towns, "industry": industries, "cargo": cargo.into_iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>()}))
}

pub fn start_bot(g: &Arc<Game>, ai: &str, settings: Value) -> Result<Value, String> {
    let mut flat = Vec::new();
    for (k, v) in settings.as_object().into_iter().flatten() {
        flat.push(format!("{k}={}", py_int(v)?));
    }
    let flat = flat.join(",");
    // A new company goes through the game's approval (openttd.ai.start), never past it: a binary
    // without that tool starts no bot from here (the console line would found one unapproved).
    if !g.tool_names().contains("openttd.ai.start") {
        return Err(format!("this game binary has no openttd.ai.start, so no bot starts here without the owner's approval \
                            (start_ai {ai} {flat} at the game's own console)").replace("  ", " "));
    }
    g.tool("openttd.ai.start", json!({"name": ai, "settings": settings}))
}

pub fn set_bot_setting(g: &Arc<Game>, company: &Value, name: &str, value: &Value) -> Result<Value, String> {
    if !g.tool_names().contains("openttd.ai.set_setting") {
        return Ok(json!({"error": "this game binary has no openttd.ai.set_setting: a running bot's settings cannot change; start a bot with the new settings instead",
                         "fallback": "start"}));
    }
    g.tool("openttd.ai.set_setting", json!({"company": py_int(company)?, "name": name, "value": py_int(value)?}))
}

/// Where a freight-line slot is sent from, named in every refusal of one.
pub const GOVERNED_PATHS: &str = "a freight-line slot is sent by the company's planner as POST /api/bots/setting \
    {company, name, value, writer}, or as a live Battlebrain decision through POST /api/decision/pull";

/// The refusal of a request that changes freight-line slots (fr_*, fr2_* .. fr16_*) of `company`
/// on behalf of `writer` (None: it names none). `names` are the settings it changes, the others
/// no concern here; `holder` reads a company's planner key (rhai_shared::planner_of). None when it
/// names no slot, or names a writer that may send them (fr_author::change_refusal). Else
/// (422 fr_needs_writer or 409 fr_planner_held, the answer). A company that is no integer reaches
/// no AI, and the game refuses that call itself (AiCompany), so it is let through to it.
pub fn slot_refusal(company: &Value, names: &[String], writer: Option<&str>, holder: &dyn Fn(i64) -> Option<Value>) -> Option<(u16, Value)> {
    let slots: Vec<&str> = names.iter().map(String::as_str).filter(|n| crate::panel_decision::is_freight_slot_setting(n)).collect();
    if slots.is_empty() {
        return None;
    }
    let c = company.as_i64()?;
    let r = crate::rhai_shared::fr_author::change_refusal(holder(c).as_ref(), c, writer)?;
    let status = if r["code"] == crate::rhai_shared::fr_author::NO_WRITER { 422 } else { 409 };
    Some((status, json!({"error": format!("{} not sent: {}", slots.join(", "), py_str(r.get("why"))),
                         "outcome": r["code"], "refusal": r, "settings": slots, "governed": GOVERNED_PATHS})))
}

/// Answer a refusal from slot_refusal and say it as an event, so the owner sees what was not asked.
fn refused(g: &Arc<Game>, (status, answer): (u16, Value)) -> (u16, Value) {
    g.event("ai", &format!("{} ({})", py_str(answer.get("error")), py_str(answer.get("outcome"))));
    (status, answer)
}

/// POST /api/bots/setting {company, name, value, writer?}: set_bot_setting, except that a
/// freight-line slot is refused before the game is asked unless the request names its `writer`
/// and the company's planner key is unset or names that writer (slot_refusal). Every other
/// setting is sent as before, with or without a writer.
pub fn bot_setting(g: &Arc<Game>, body: &Value) -> Result<(u16, Value), String> {
    let name = py_str(body.get("name"));
    let writer = body.get("writer").and_then(Value::as_str);
    if let Some(r) = slot_refusal(&body["company"], std::slice::from_ref(&name), writer, &crate::rhai_shared::planner_of) {
        return Ok(refused(g, r));
    }
    Ok((200, set_bot_setting(g, &body["company"], &name, &body["value"])?))
}

/// The settings of a company's AI that an MCP call changes: `name` of openttd.ai.set_setting, the
/// keys of openttd.ai.set_settings' `settings`; none for any other tool.
pub fn settings_of_call(tool: &str, args: &Value) -> Vec<String> {
    match tool {
        "openttd.ai.set_setting" => vec![py_str(args.get("name"))],
        "openttd.ai.set_settings" => args["settings"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A raw MCP call through the panel (POST /api/tools/<tool>, POST /api/rpc tools/call) names no
/// writer, so it changes no freight-line slot: refused as a writer-less /api/bots/setting is.
/// None for every other call. The decision executor (src/panel_decision.rs) and set_bot_setting
/// call the game themselves (Game::tool), never through these routes.
pub fn raw_call_refusal(g: &Arc<Game>, tool: &str, args: &Value) -> Option<(u16, Value)> {
    slot_refusal(&args["company"], &settings_of_call(tool, args), None, &crate::rhai_shared::planner_of).map(|r| refused(g, r))
}

/// Settings in the order they were set, a key set again keeping its place (a Python dict).
#[derive(Default)]
pub struct Settings(pub Vec<(String, Value)>);

impl Settings {
    fn set(&mut self, k: &str, v: Value) {
        match self.0.iter_mut().find(|(n, _)| n == k) {
            Some(e) => e.1 = v,
            None => self.0.push((k.to_string(), v)),
        }
    }
    fn has(&self, k: &str) -> bool {
        self.0.iter().any(|(n, _)| n == k)
    }
    pub fn iter(&self) -> impl Iterator<Item = &(String, Value)> {
        self.0.iter()
    }
}

/// A world-graph id ("industry:12") or a plain number, as the number (the crate's one rule,
/// battlebrain_contract::graph_number).
fn num(v: &Value) -> Option<i64> {
    crate::battlebrain_contract::graph_number(v)
}

/// `d.get(k, default)`: the value when the key is there (even null), else the default.
fn get_or<'a>(v: &'a Value, k: &str, default: &'a Value) -> &'a Value {
    v.get(k).unwrap_or(default)
}

/// Freight lines handed to the bot at most (fr_*, fr2_* .. fr16_*); bridge 2026-11-17 declares as many.
pub const MAX_FREIGHT_LINES: usize = crate::battlebrain_contract::FREIGHT_SLOTS;

/// Freight lines the planned bot may plan itself once the handed ones are built (bridge own_lines; it
/// defaults to 0 there, so a bot started without a plan, a duel's baseline, stays unplanned). Held-start
/// pairs over ten years, own_lines 8 against 0: seed 31337 -1.0%, 4242 +45.2%, 20208 +30.7% (knowledge
/// the-bots-rail-pathfinder-lays-plain-track-only). They cost value until about month 50, so a 36-month
/// duel shows their cost and not their return. `PRISMATIC_BOT_FROM_DUELS=1` derives the value from the
/// stored duel outcomes instead (src/duel_outcomes.rs, I0087); this constant stays the default until
/// that rule wins held-start pairs on three seeds.
pub const OWN_LINES: usize = 8;

/// What the bot is handed, as settings and as the choices of the two plan.* events (I0081).
pub struct Planned {
    pub settings: Settings,
    /// `choice` of plan.portfolio: the planner's portfolio as it was read.
    pub portfolio: Value,
    /// `choice` of plan.bot_settings: every line handed over, in its slot, and why others were not.
    pub bot_settings: Value,
}

/// One end of a candidate line: its id and name.
fn end(v: &Value) -> Value {
    json!({"id": v.get("id"), "name": v.get("name")})
}

/// A planner candidate as the events carry it.
fn line_of(c: &Value) -> Value {
    let empty = json!({});
    let mut l = json!({"kind": c.get("kind"), "mode": c.get("mode"), "cargo": c.get("cargo"), "cargo_id": c.get("cargo_id"),
           "from": end(get_or(c, "producer", get_or(c, "a", &empty))), "to": end(get_or(c, "acceptor", get_or(c, "b", &empty))),
           "distance": c.get("distance"), "score": c.get("score")});
    // The planner's monthly profit of the option it chose (its mode's, and its cargo's when the option
    // names one), and the vehicles that option buys, when it gave them: replan.rhai pays a passenger
    // pair back from them, as no freight price names that line.
    let chosen = c["options"].as_array().into_iter().flatten()
        .find(|o| o["mode"] == c["mode"] && (o["cargo_id"].is_null() || o["cargo_id"] == c["cargo_id"]));
    if let Some(m) = chosen.and_then(|o| o["monthly_profit"].as_f64()) {
        l["monthly_profit"] = json!(m);
    }
    if let Some(n) = chosen.and_then(|o| o["vehicles"]["count"].as_f64()) {
        l["vehicles"] = json!(n);
    }
    l
}

/// Settings as a JSON object and as the `start_ai` argument they make.
pub fn settings_json(s: &Settings) -> (Value, String) {
    (Value::Object(s.iter().cloned().collect()), s.iter().map(|(k, v)| format!("{k}={}", py_str(Some(v)))).collect::<Vec<_>>().join(","))
}

/// Why the priced lines without a route have none: each track-planning status other than found (a
/// refusal's text cut to 160 characters) with how many lines answered it, the most frequent first.
/// The January 1954 round of seed 20261122 said only "0 with a route", and its cause had to be inferred.
fn why_unpriced(priced: &[Value]) -> String {
    let mut seen: Vec<(String, usize)> = Vec::new();
    for p in priced.iter().filter(|p| p["status"] != json!("found")) {
        let why: String = p["status"].as_str().unwrap_or("no status").chars().take(160).collect();
        match seen.iter_mut().find(|(w, _)| *w == why) {
            Some((_, n)) => *n += 1,
            None => seen.push((why, 1)),
        }
    }
    seen.sort_by(|a, b| b.1.cmp(&a.1));
    seen.iter().map(|(w, n)| format!("{n}x {w}")).collect::<Vec<_>>().join("; ")
}

/// Whether the bot can build a rail line whose option is `opt`. PrismaticAI lays rail over buildable
/// land, level crossings, bridges and tunnels (rail.nut FindRailPath), so any line track-planning
/// found a route for is the bot's. When lines were priced (`priced_any`), an unpriced one is not
/// handed over blind either: on seed 20208 an unpriced passenger line needed a bridge over a river
/// and failed. Without AlgoGlyph (nothing priced) every line is tried as before.
fn can_build(priced_any: bool, opt: &Value) -> bool {
    match opt.get("track").filter(|t| t.as_object().map_or(false, |m| !m.is_empty())) {
        None => !priced_any,
        Some(t) => t["status"] == json!("found"),
    }
}

fn score(c: &Value) -> f64 {
    c["score"].as_f64().unwrap_or(0.0)
}

/// The option of a candidate's own mode.
fn option_of(c: &Value) -> Value {
    c["options"].as_array().into_iter().flatten().find(|o| o["mode"] == c["mode"]).cloned().unwrap_or(json!({}))
}

/// The planner's candidates in the order the bot is handed them: the portfolio's, then the spares
/// (the freight and passenger-rail lists, best score first), and whether any line was priced.
fn in_order(out: &Value) -> (Vec<Value>, Vec<Value>, bool) {
    let chosen: Vec<Value> = out.pointer("/portfolio/chosen").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut spare: Vec<Value> = out["freight"].as_array().into_iter().flatten()
        .chain(out.pointer("/by_mode/rail").and_then(Value::as_array).into_iter().flatten())
        .filter(|c| !chosen.contains(c) && score(c) > 0.0).cloned().collect();
    spare.sort_by(|a, b| score(b).total_cmp(&score(a)));
    let priced_any = out.pointer("/line_costs/priced").and_then(Value::as_array).map_or(false, |p| !p.is_empty());
    (chosen, spare, priced_any)
}

/// A freight rail candidate the bot can build, as (producer, acceptor, cargo); None for any other.
fn freight_line(c: &Value, priced_any: bool) -> Option<(i64, i64, i64)> {
    if c["kind"] != json!("freight") || c["mode"] != json!("freight_rail") || !can_build(priced_any, &option_of(c)) {
        return None;
    }
    Some((num(&c["producer"]["id"])?, num(&c["acceptor"]["id"])?, py_int(&c["cargo_id"]).ok()?))
}

/// The companies with a rail station at a candidate's producer (mcp-plan's `producer.rail_stations`,
/// cargo.rs `rail_claims`); none when the planner did not say.
fn holders(c: &Value) -> Vec<i64> {
    c["producer"]["rail_stations"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect()
}

/// The companies a plan is for: every running company of the panel's AI `ai` (of any AI when `ai`
/// is empty) whose AI declares the freight-line slots, so that the plan can steer it; sorted.
pub fn plan_targets(running: &Value, ai: &str) -> Vec<i64> {
    let mut ids: Vec<i64> = running.as_array().into_iter().flatten()
        .filter(|r| r["running"] != json!(false) && (ai.is_empty() || r["ai"] == ai))
        .filter(|r| r["settings"].as_array().into_iter().flatten()
            .any(|s| s["name"].as_str().is_some_and(crate::panel_decision::is_freight_slot_setting)))
        .filter_map(|r| r["id"].as_i64()).collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Which company of `companies` (the running companies of one AI) is handed each producer, so that
/// no two of them build from one mine (research/incident-2026-11-22-sibling-industries.md). The
/// producers of every freight line the bot can build, by their best line's score (the lower id
/// first on a tie), go round the companies: each to the one with the fewest lines so far (the lower
/// id on a tie), never beyond MAX_FREIGHT_LINES. A producer where a company has a rail station goes
/// to that company only, and to none when another company holds it too (rail.nut
/// ForeignRailStationAt refuses it to everyone else). A producer no company is handed is left to a
/// company not started yet (`settings_for` with None).
pub fn assign_producers(out: &Value, companies: &[i64]) -> BTreeMap<i64, i64> {
    let (chosen, spare, priced_any) = in_order(out);
    let mut best: BTreeMap<i64, (f64, Vec<i64>)> = BTreeMap::new();
    for c in chosen.iter().chain(spare.iter()) {
        let Some((producer, _, _)) = freight_line(c, priced_any) else { continue };
        let e = best.entry(producer).or_insert((f64::MIN, holders(c)));
        e.0 = e.0.max(score(c));
    }
    let mut order: Vec<(i64, f64, Vec<i64>)> = best.into_iter().map(|(p, (s, h))| (p, s, h)).collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let (mut handed, mut lines) = (BTreeMap::new(), BTreeMap::<i64, usize>::new());
    for (producer, _, held) in order {
        let count = |c: &i64| lines.get(c).copied().unwrap_or(0);
        let pick = companies.iter().copied()
            .filter(|c| held.iter().all(|h| h == c) && count(c) < MAX_FREIGHT_LINES)
            .min_by_key(|c| (count(c), *c));
        if let Some(c) = pick {
            handed.insert(producer, c);
            *lines.entry(c).or_insert(0) += 1;
        }
    }
    handed
}

/// Why the freight line `c` from `producer` is not for `company` (None: a company not started yet),
/// or None when it is: another company has a rail station at the producer, or `assigned` hands the
/// producer to another company of the AI, or to none.
fn not_for(company: Option<i64>, producer: i64, c: &Value, assigned: &BTreeMap<i64, i64>) -> Option<String> {
    let name = py_str(c["producer"].get("name"));
    if let Some(h) = holders(c).into_iter().find(|h| Some(*h) != company) {
        return Some(format!("company {h} already has a rail station at {name}"));
    }
    match (assigned.get(&producer), company) {
        (Some(a), Some(me)) if *a == me => None,
        (None, None) => None,
        (Some(a), _) => Some(format!("{name} is handed to company {a} of the same AI")),
        (None, Some(_)) => Some(format!("{name} is handed to no company of the AI: each it may go to has its lines")),
    }
}

/// The bot's routes from the planner's portfolio within the budget, chosen from mcp-plan's answer:
/// up to sixteen freight rail lines (fr_*, fr2_* .. fr16_*), one passenger rail pair (rail_a,
/// rail_b) and one air pair (air_a, air_b, air_planes), with the events that explain each choice and
/// the same choice as data. Plans for a company not started yet that shares the map with no
/// company of its AI: `settings_for` with no company and no producer handed out.
pub fn settings_from_plan(out: &Value, event: &mut dyn FnMut(String)) -> Planned {
    settings_for(out, None, &BTreeMap::new(), event)
}

/// `settings_from_plan` for `company` (None: a company not started yet), whose freight lines are
/// only those from a producer `assigned` hands it (`assign_producers`) and no other company has a
/// rail station at (`not_for`); a portfolio line left out is in `dropped` with the reason.
pub fn settings_for(out: &Value, company: Option<i64>, assigned: &BTreeMap<i64, i64>, event: &mut dyn FnMut(String)) -> Planned {
    let empty = json!({});
    let pf = out.get("portfolio").filter(|v| v.is_object()).unwrap_or(&empty);
    let (chosen, spare, priced_any) = in_order(out);
    let (mut dropped, mut spares_used) = (Vec::new(), 0usize);
    let (mut passenger_rail, mut air) = (Value::Null, Value::Null);
    event(format!("planner portfolio by {}: {} investment(s), total score {}, {}k of {}k",
                  pf.get("solver").map_or("no solver".to_string(), |v| py_str(Some(v))), chosen.len(),
                  py_str(pf.get("total_score")), py_str(pf.get("investment_k")), py_str(pf.get("budget_k"))));
    let priced: Vec<Value> = out.pointer("/line_costs/priced").and_then(Value::as_array).cloned().unwrap_or_default();
    if !priced.is_empty() {
        let found: Vec<&Value> = priced.iter().filter(|p| p["status"] == json!("found")).collect();
        event(format!("planner: {} rail line(s) priced on the map by track-planning, {} with a route: {}{}", priced.len(), found.len(),
                      found.iter().map(|p| format!("{} pieces {}", py_str(p.get("pieces")), p["money"].as_f64().unwrap_or(0.0).round() as i64))
                          .collect::<Vec<_>>().join(", "),
                      if found.len() == priced.len() { String::new() } else { format!("; {} without a route dropped their rail option ({})", priced.len() - found.len(), why_unpriced(&priced)) }));
    }
    let portfolio = json!({"solver": pf.get("solver"), "chosen": chosen.iter().map(line_of).collect::<Vec<_>>(),
                           "total_score": pf.get("total_score"), "investment_k": pf.get("investment_k"), "budget_k": pf.get("budget_k"),
                           "priced": {"lines": priced.len(), "with_route": priced.iter().filter(|p| p["status"] == json!("found")).count()}});
    // The portfolio first; a rail line the bot cannot build (`can_build`) is replaced by the next
    // priced or unpriced line of its kind that it can, from the planner's lists (best score first).
    let mut settings = Settings::default();
    let (mut freight_lines, mut producers) = (0usize, HashSet::new());
    // The freight lines chosen (portfolio first, then spares), set in order of score after the loop.
    let mut lines: Vec<(f64, i64, i64, i64, Value)> = Vec::new();
    let mut pair_score: Option<f64> = None;
    for c in chosen.iter().chain(spare.iter()) {
        let mode = c["mode"].as_str().unwrap_or("");
        let in_chosen = chosen.contains(c);
        let opt = option_of(c);
        let rail = mode == "rail" || mode == "freight_rail";
        if rail && !can_build(priced_any, &opt) {
            if in_chosen {
                let line = format!("{} - {}", py_str(get_or(c, "producer", get_or(c, "a", &empty)).get("name")),
                                   py_str(get_or(c, "acceptor", get_or(c, "b", &empty)).get("name")));
                let why = if opt.get("track").map_or(false, |t| t.as_object().map_or(false, |m| !m.is_empty())) {
                    format!("has no route on the map ({})", py_str(opt["track"].get("status")))
                } else {
                    "was not priced on the map, so it is not handed to the bot blind".to_string()
                };
                event(format!("planner: the {mode} line {line} {why}; the next line it can build goes instead"));
                dropped.push(json!({"line": line_of(c), "why": why}));
            }
            continue;
        }
        if !in_chosen && !rail {
            continue;
        }
        let freight = c["kind"] == json!("freight");
        if freight && mode == "freight_rail" && freight_lines < MAX_FREIGHT_LINES {
            // Up to MAX_FREIGHT_LINES freight lines, one per producer (a producer's cargo is counted
            // once): fr_* is built at once, the rest when money allows. After eight lines the best run
            // on seed 31337 held 308074 idle at month 36 with its loan repaid (research/results
            // duel-20261117T041705), so a longer list is handed over and built as money comes in. The
            // game ignores a setting the AI does not declare (ScriptConfig::SetSetting), so an older bot
            // uses the first ones.
            let Some((producer, acceptor, cargo)) = freight_line(c, priced_any) else { continue };
            if let Some(why) = not_for(company, producer, c, assigned) {
                if in_chosen {
                    event(format!("planner: the freight line {} - {} is not handed over: {why}", py_str(c["producer"].get("name")),
                                  py_str(c["acceptor"].get("name"))));
                    dropped.push(json!({"line": line_of(c), "why": why}));
                }
                continue;
            }
            if !producers.insert(producer) {
                continue;
            }
            freight_lines += 1;
            if !in_chosen { spares_used += 1; }
            lines.push((score(c), producer, acceptor, cargo, json!({"cargo": c.get("cargo"), "from": c["producer"].get("name"),
                "to": c["acceptor"].get("name"), "distance": c.get("distance"), "profit": opt.get("monthly_profit"), "score": c.get("score")})));
        } else if !freight && mode == "rail" && !settings.has("rail_a") {
            settings.set("rail_a", c["a"]["id"].clone());
            settings.set("rail_b", c["b"]["id"].clone());
            pair_score = Some(score(c));
            if !in_chosen { spares_used += 1; }
            passenger_rail = json!({"line": line_of(c), "spare": !in_chosen});
            event(format!("planner: passengers by rail {} - {}, score {}", py_str(c["a"].get("name")), py_str(c["b"].get("name")), py_str(c.get("score"))));
        } else if !freight && mode == "air" && !settings.has("air_a") {
            let count = opt.pointer("/vehicles/count").filter(|v| crate::panel::py_int(v).map_or(false, |n| n != 0))
                .and_then(|v| py_int(v).ok()).unwrap_or(2);
            settings.set("air_a", c["a"]["id"].clone());
            settings.set("air_b", c["b"]["id"].clone());
            settings.set("air_planes", json!(count.clamp(1, 8)));
            air = json!({"line": line_of(c), "planes": count.clamp(1, 8)});
            event(format!("planner: air {} - {}, {count} plane(s), score {}", py_str(c["a"].get("name")), py_str(c["b"].get("name")), py_str(c.get("score"))));
        }
    }
    // The bot builds its lines in the order it is given, borrowing for each: the best score first.
    // On seed 31337 a passenger pair scored 113003 was built second, before freight lines scored up
    // to 240547, and the company was a third behind at month 24 (research/results
    // duel-20261117T032759). Freight lines go in order of score, and rail_after tells the bot how many
    // of them score above the passenger pair, to build before it.
    lines.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut freight = Vec::new();
    for (i, (_, producer, acceptor, cargo, e)) in lines.iter().enumerate() {
        let prefix = crate::battlebrain_contract::freight_slot_prefix(i + 1);
        freight.push(json!({"slot": prefix, "from": producer, "to": acceptor, "cargo_id": cargo, "cargo": e.get("cargo"),
                            "from_name": e.get("from"), "to_name": e.get("to"), "distance": e.get("distance"),
                            "monthly_profit": e.get("profit"), "score": e.get("score")}));
        settings.set(&format!("{prefix}from"), json!(producer));
        settings.set(&format!("{prefix}to"), json!(acceptor));
        settings.set(&format!("{prefix}cargo"), json!(cargo));
        event(format!("planner: {} by rail {} -> {}, {} tiles, profit {}/month, score {}{}", py_str(e.get("cargo")),
                      py_str(e.get("from")), py_str(e.get("to")), py_str(e.get("distance")), py_str(e.get("profit")), py_str(e.get("score")),
                      if i == 0 { String::new() } else { format!(" (line {}, built when the money allows)", i + 1) }));
    }
    if !lines.is_empty() && OWN_LINES > 0 {
        settings.set("own_lines", json!(OWN_LINES));
        event(format!("planner: after these lines the bot may plan up to {OWN_LINES} more itself (own_lines)"));
    }
    let mut rail_after = Value::Null;
    if let Some(ps) = pair_score {
        let after = lines.iter().filter(|f| f.0 > ps).count();
        settings.set("rail_after", json!(after));
        rail_after = json!(after);
        event(format!("planner: the passenger rail pair (score {ps}) is built after {after} freight line(s) that score more"));
    }
    let (as_object, flat) = settings_json(&settings);
    let bot_settings = json!({"settings": as_object, "start_ai": flat, "freight": freight, "passenger_rail": passenger_rail, "air": air,
                              "rail_after": rail_after, "own_lines": settings.0.iter().find(|(k, _)| k == "own_lines").map(|(_, v)| v.clone()),
                              "dropped": dropped, "spares_used": spares_used});
    Planned { settings, portfolio, bot_settings }
}

/// The parameters of the bot's settings rule, for its `version`.
pub fn bot_params(extra: &str, from_duels: &duel_outcomes::Derivation) -> Value {
    let mut p = json!({"max_freight_lines": MAX_FREIGHT_LINES, "own_lines": OWN_LINES, "PRISMATIC_BOT_SETTINGS": extra});
    if from_duels.record["why"] != duel_outcomes::OFF {
        p["from_duels"] = json!({"switch": duel_outcomes::SWITCH, "min_months": duel_outcomes::MIN_MONTHS, "near": duel_outcomes::NEAR,
                                 "own_lines_max": duel_outcomes::OWN_LINES_MAX, "rule": "own-lines-by-handed-v1"});
    }
    p
}

/// Apply what the stored duel outcomes derived (I0087) to the plan's settings, with a sentence for
/// each value it changed. Nothing changes when the derivation fell back to the constants.
pub fn apply_from_duels(p: &mut Planned, d: &duel_outcomes::Derivation, event: &mut dyn FnMut(String)) {
    let why = |k: &str| py_str(d.record["derived"][k].get("why"));
    if let Some(o) = d.own_lines {
        let before = p.bot_settings["own_lines"].clone();
        p.settings.set("own_lines", json!(o));
        p.bot_settings["own_lines"] = json!(o);
        event(format!("planner: own_lines {o} from stored duel outcomes (constant {}): {}", py_str(Some(&before)), why("own_lines")));
    }
    if let (Some(r), true) = (d.rail_after, p.settings.has("rail_after")) {
        let before = p.bot_settings["rail_after"].clone();
        p.settings.set("rail_after", json!(r));
        p.bot_settings["rail_after"] = json!(r);
        event(format!("planner: rail_after {r} from stored duel outcomes (scores gave {}): {}", py_str(Some(&before)), why("rail_after")));
    }
    let (as_object, flat) = settings_json(&p.settings);
    p.bot_settings["settings"] = as_object;
    p.bot_settings["start_ai"] = json!(flat);
}

/// The data of plan.portfolio: the planner's answer read for the bot. `request` is what the planner
/// was asked (its arguments and parameters), `planner` how it ran, `cal` the calibration read for it:
/// its store query and rows, or the fallback `uncalibrated` with the reason.
/// What answered the planner's AlgoGlyph calls, for plan.portfolio's `inputs.live.planner`: mcp-plan's
/// `algoglyph` summary when MCP_ALGOGLYPH_SERVE asked for one flow serve (`{runner, pid, calls,
/// fallbacks, gone}`, src/flow_serve.rs), else the per-call runner, else none (I0086).
pub fn kernel_runner(out: &Value, algoglyph: bool) -> Value {
    match out.get("algoglyph") {
        Some(a) => a.clone(),
        None if algoglyph => json!({"runner": "npm run flow -- kernel, one process per call (MCP_ALGOGLYPH_SERVE unset)"}),
        None => json!({"runner": "none (MCP_ALGOGLYPH_DIR unset)"}),
    }
}

pub fn portfolio_event(out: &Value, p: &Planned, request: Value, planner: Value, cal: &Calibration) -> Value {
    panel_events::plan("bot.portfolio", json!({"request": request, "effective": out.get("params")}),
                       json!({"planner": planner, "reads": out.get("reads"), "calibration": cal.record}), &cal.stored, p.portfolio.clone())
}

/// What the planned bot's calibration read (I0082): the factors it plans with (None: uncalibrated),
/// what its plan.* events name as their stored input, and the record they carry in `inputs.live`.
pub struct Calibration {
    pub factors: Option<Value>,
    pub stored: Stored,
    pub record: Value,
    /// The readings read, each with its measured lines (`calibrate::readings_of`).
    pub readings: Vec<Value>,
}

/// Why the plan runs on the model alone when nobody asked for calibration: the A/B did not favour it.
pub const CALIBRATION_OFF: &str = "calibration is off (PRISMATIC_PLAN_CALIBRATION unset or off, the default: the three-seed held-start A/B averaged about zero, knowledge the-planner-overpredicts-what-a-line-earns; ADR 0034 point 6)";

/// The calibration of `scope` for a game (`calibrate::game_of`), read with `read` (every store,
/// `panel_store::every_store`): the factors of every reading it returns, or the fallback
/// `uncalibrated` with its reason when calibration is off, the scope is not one, the store is off or
/// it holds no reading. Never a silent default (ADR 0034 point 2).
pub fn calibration(scope: &Scope, game: &Value, read: impl FnOnce(&Named) -> Result<Across, String>) -> Calibration {
    let name = match scope {
        Scope::Off => "off".to_string(),
        Scope::All => "all".to_string(),
        Scope::Seed => "seed".to_string(),
        Scope::Unknown(v) => v.clone(),
    };
    let fallback = |why: String, extra: Value| {
        let mut record = json!({"scope": name, "used": false, "fallback": "uncalibrated", "why": why, "game": game});
        if let (Some(r), Some(x)) = (record.as_object_mut(), extra.as_object()) { r.extend(x.clone()); }
        Calibration { factors: None, stored: Stored::Fallback(format!("uncalibrated: {why}")), record, readings: Vec::new() }
    };
    match scope {
        Scope::Off => return fallback(CALIBRATION_OFF.into(), json!({})),
        Scope::Unknown(v) => {
            let file = if v.ends_with(".json") { "; no file is read any more: calibration comes from the stores' plan.calibration events (I0082)" } else { "" };
            return fallback(format!("PRISMATIC_PLAN_CALIBRATION={v} is none of off, all, seed{file}"), json!({}));
        }
        _ => {}
    }
    let query = match calibrate::readings_query(scope, game) {
        Ok(q) => q,
        Err(why) => return fallback(why, json!({})),
    };
    let across = match read(&query) {
        Ok(a) => a,
        Err(why) => return fallback(why, query.record()),
    };
    let answer = json!({"columns": across.columns, "rows": across.rows});
    let readings = calibrate::readings_of(&answer);
    if readings.is_empty() {
        let mut read = query.record();
        read["rows"] = json!(across.rows.len());
        read["stores"] = json!(across.stores);
        return fallback(format!("the store holds no calibration reading for this scope ({} row(s) from {} store(s))",
                                across.rows.len(), across.stores.len()), read);
    }
    let f = calibrate::factors(&readings);
    let used: Vec<Value> = readings.iter().map(|r| json!({"instance": r["instance"], "seq": r["seq"], "ts_ms": r["ts_ms"],
                                                          "seed": r["seed"], "map": r["map"], "lines": r["lines"].as_array().map_or(0, Vec::len)}))
        .collect();
    let mut record = query.record();
    if let (Some(r), Some(x)) = (record.as_object_mut(), json!({"scope": name, "used": true, "rows": across.rows.len(),
            "stores": across.stores, "game": game, "readings": used, "lines": f["lines"], "modes": f["modes"],
            "factors": f["params"]["calibration"], "rule": calibrate::RULE}).as_object()) {
        r.extend(x.clone());
    }
    Calibration {
        factors: Some(f["params"]["calibration"].clone()),
        stored: Stored::Query { query, rows: across.rows.len() },
        record,
        readings,
    }
}

/// plan.bot_settings names the calibration its portfolio was planned with: its query and rows, or
/// the fallback, when a run asked for calibration (off, the history of the settings themselves is
/// I0087's, and the event says so).
pub fn with_calibration(data: &mut Value, cal: &Calibration) {
    data["inputs"]["live"]["calibration"] = cal.record.clone();
    // The settings' own history (I0087), when it was read, stays the stored input; the calibration's
    // query and rows are in inputs.live.calibration either way.
    if cal.record["scope"] != "off" && data["inputs"]["stored"].get("query").is_none() {
        data["inputs"]["stored"] = cal.stored.to_json();
    }
}

/// The data of plan.bot_settings, with the run's extra settings (PRISMATIC_BOT_SETTINGS) applied to
/// `p.settings` and recorded: the settings the bot is started with.
pub fn bot_settings_event(p: &mut Planned, extra_text: &str, portfolio_seq: Option<u64>, from_duels: &duel_outcomes::Derivation) -> Value {
    let from_run = match extra_settings(extra_text) {
        Ok(extra) => {
            for (k, v) in &extra { p.settings.set(k, json!(v)); }
            json!({"settings": extra.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<String, Value>>()})
        }
        Err(e) => json!({"refused": e}),
    };
    let mut choice = p.bot_settings.clone();
    let (as_object, flat) = settings_json(&p.settings);
    choice["settings"] = as_object;
    choice["start_ai"] = json!(flat);
    choice["from_run"] = from_run;
    panel_events::plan("bot.settings", bot_params(extra_text, from_duels),
                       json!({"portfolio_seq": portfolio_seq, "rule": "panel_bot.rs:settings_from_plan", "from_duels": from_duels.record}),
                       &from_duels.stored, choice)
}

/// The bot's routes from the planner's portfolio within the budget (difficulty.max_loan), read live
/// with mcp-plan when the panel plans its bot; no settings without a planner or a portfolio.
pub fn planned_air(g: &Arc<Game>) -> Settings {
    if !g.args.plan_bot {
        // Unplanned (--no-plan-bot): no portfolio, but the run's own settings still start the bot.
        // Without this, PRISMATIC_BOT_SETTINGS reached only a planned bot, and the owner's unplanned
        // co-play bot of 2026-11-23 started on the defaults (max_routes 4) instead of its evaluated
        // settings. A malformed list is refused whole and said, as for a planned start.
        let extra = std::env::var("PRISMATIC_BOT_SETTINGS").unwrap_or_default();
        return unplanned_settings(&extra, &mut |text| g.event("ai", &text));
    }
    // With AlgoGlyph present the best rail lines are priced on the real map (track-planning) before
    // the portfolio is chosen, so the bot builds what pays on this terrain; as many lines of each list
    // as the bot takes freight lines (at most two per producer, linecost.rs).
    planned(g, MAX_FREIGHT_LINES).map(|p| p.settings).unwrap_or_default()
}

/// The settings an unplanned bot starts with: the run's extra settings (PRISMATIC_BOT_SETTINGS), in
/// order (a later name wins), each said through `say`; none, and the refusal said, for a malformed list.
pub fn unplanned_settings(extra: &str, say: &mut dyn FnMut(String)) -> Settings {
    let mut s = Settings::default();
    match extra_settings(extra) {
        Ok(list) => for (k, v) in list {
            say(format!("bot setting from the run (unplanned): {k}={v}"));
            s.set(&k, json!(v));
        },
        Err(e) => say(format!("bot settings from the run refused: {e}")),
    }
    s
}

/// Plan the bot: ask mcp-plan for its portfolio (pricing the `track` best rail lines on the map when
/// AlgoGlyph is present), choose the settings from it, and publish both decisions, as sentences
/// (panel.ai) and as data (plan.portfolio, plan.bot_settings). None without a planner binary. When
/// the planner fails (it cannot run, exits non-zero or answers no JSON) both events are still
/// published, `portfolio.error` and `bot_settings.unplanned` say why, and the settings are empty.
/// The settings are those of a company not started yet (the bot start's start_ai); each running
/// company of the AI has a plan of its own in `bot_settings.companies` (`company_plans`), and no two
/// of them, nor the new one, are handed one producer (`assign_producers`).
/// PRISMATIC_PLANNER names another planner binary than the sibling mcp-plan (RESEARCH-ONLY: a
/// planner under test, or a failing one to prove that a failure is reported).
pub fn planned(g: &Arc<Game>, track: usize) -> Option<Planned> {
    let exe = std::env::var_os("PRISMATIC_PLANNER").filter(|v| !v.is_empty()).map(PathBuf::from)
        .unwrap_or_else(|| crate::panel::sibling("mcp-plan"));
    if !exe.exists() {
        return None;
    }
    let budget = g.max_loan().filter(|b| *b != 0);
    let mut params = match budget { Some(b) => json!({"budget": b}), None => json!({}) };
    // Back-propagation (I0082): with PRISMATIC_PLAN_CALIBRATION `all` or `seed` the plan starts from
    // what earlier games earned per mode, read from the stores' plan.calibration events, instead of
    // the model alone (src/calibrate.rs). Off unless a run asks for it (a duel's calibrated: true), so
    // its A/B decides whether it pays; every outcome is published, the fallback with its reason.
    let scope = calibrate::scope(std::env::var("PRISMATIC_PLAN_CALIBRATION").ok().as_deref());
    let game = if scope == Scope::Seed {
        calibrate::game_of(g.args.seed, &g.resource("openttd://game/state").unwrap_or(Value::Null))
    } else {
        json!({"seed": g.args.seed, "map": null})
    };
    let cal = calibration(&scope, &game, crate::panel_store::every_store);
    if let Some(f) = &cal.factors {
        g.event("ai", &format!("planner: calibration from the store ({}, {} reading(s) in {} row(s)): {f}",
                               cal.record["scope"].as_str().unwrap_or(""), cal.record["readings"].as_array().map_or(0, Vec::len),
                               cal.record["rows"]));
        params["calibration"] = f.clone();
    } else if scope != Scope::Off {
        g.event("ai", &format!("planner: calibration asked for ({}), but {}; the plan runs uncalibrated",
                               cal.record["scope"].as_str().unwrap_or(""), py_str(cal.record.get("why"))));
    }
    let algoglyph = std::env::var("MCP_ALGOGLYPH_DIR").map_or(false, |d| !d.is_empty());
    let track = if algoglyph { track } else { 0 };
    // The running companies of the AI, each planned on its own (`company_plans`); a producer where
    // any other company has a rail station is no candidate (mcp-plan --for). None readable: the plan
    // is for a company not started yet only, and every station holds its producer.
    let targets = bots(g).map_or_else(|_| Vec::new(), |b| plan_targets(&b["running"], &g.args.ai));
    let plan_for = targets.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    // Forty candidates, so up to sixteen freight lines with distinct producers can be found (ten gave
    // three on seed 20208, thirty six on seed 31337).
    let mut argv = vec!["--panel".to_string(), g.panel_url(), "--limit".into(), "40".into(), "--params".into(), py_json(&params)];
    if track > 0 {
        argv.extend(["--track".to_string(), track.to_string()]);
    }
    argv.extend(["--for".to_string(), plan_for]);
    let request = json!({"budget": budget, "params": params, "limit": 40, "track": track, "for": targets});
    let mut planner = json!({"exe": "mcp-plan", "args": argv[2..].to_vec(), "budget_from": "console: setting difficulty.max_loan",
                             "algoglyph": algoglyph});
    let out = match run(Command::new(&exe).args(&argv), None, Duration::from_secs(600), None) {
        Ok(r) if r.code == 0 => match serde_json::from_str::<Value>(&r.stdout) {
            Ok(v) => Ok(v),
            Err(e) => Err(format!("planner not reachable for the bot: {e}")),
        },
        Ok(r) => {
            planner["exit"] = json!(r.code);
            planner["stderr"] = json!(r.stderr.trim());
            Err(format!("planner failed for the bot: exit {}{}", r.code,
                        r.stderr.trim().lines().last().map(|l| format!(": {l}")).unwrap_or_default()))
        }
        Err(e) => Err(format!("planner not reachable for the bot: {e}")),
    };
    let out = match out {
        Ok(v) => v,
        Err(e) => {
            g.event("ai", &e);
            planner["error"] = json!(e);
            let mut p = settings_from_plan(&json!({}), &mut |_| {});
            p.portfolio["error"] = json!(e);
            let seq = g.plan("plan.portfolio", portfolio_event(&json!({}), &p, request, planner, &cal));
            // No plan, no settings: the bot starts unplanned, and the run's extra settings are not
            // applied either; the event names them as asked and not applied.
            let extra = std::env::var("PRISMATIC_BOT_SETTINGS").unwrap_or_default();
            p.bot_settings["unplanned"] = json!(e);
            if !extra.trim().is_empty() {
                p.bot_settings["from_run"] = json!({"not_applied": extra, "why": "no plan: the bot starts unplanned"});
            }
            let unplanned = duel_outcomes::from_duels(duel_outcomes::enabled(), 0, OWN_LINES as i64, None, |_| unreachable!("nothing handed over"));
            let mut data = panel_events::plan("bot.settings", bot_params(&extra, &unplanned),
                                              json!({"portfolio_seq": seq, "from_duels": unplanned.record}),
                                              &unplanned.stored, p.bot_settings.clone());
            with_calibration(&mut data, &cal);
            g.plan("plan.bot_settings", data);
            return Some(p);
        }
    };
    let assigned = assign_producers(&out, &targets);
    let mut p = settings_for(&out, None, &assigned, &mut |text| g.event("ai", &text));
    planner["kernel_runner"] = kernel_runner(&out, algoglyph);
    let seq = g.plan("plan.portfolio", portfolio_event(&out, &p, request, planner, &cal));
    // own_lines and rail_after from the stored duel outcomes (I0087), when the run turns the switch on;
    // otherwise, and when the stores give no pair, the constants above, with the reason in the event.
    let handed = p.bot_settings["freight"].as_array().map_or(0, Vec::len);
    let from_duels = duel_outcomes::from_duels(duel_outcomes::enabled(), handed, OWN_LINES as i64, p.bot_settings["rail_after"].as_i64(),
                                               crate::panel_store::every_store);
    if from_duels.record["used"] == true {
        apply_from_duels(&mut p, &from_duels, &mut |text| g.event("ai", &text));
    } else if duel_outcomes::enabled() {
        g.event("ai", &format!("planner: own_lines and rail_after from stored duel outcomes asked for, but {}; the constants stay",
                               py_str(from_duels.record.get("why"))));
    }
    // Extra settings for this bot from the run that started the stack (mcp-duel --bot-settings, the
    // duel form): an A/B of a bot feature without a code change. A malformed list is refused whole.
    let extra = std::env::var("PRISMATIC_BOT_SETTINGS").unwrap_or_default();
    match extra_settings(&extra) {
        Ok(list) => for (k, v) in list { g.event("ai", &format!("bot setting from the run: {k}={v}")); },
        Err(e) => g.event("ai", &format!("bot settings from the run refused: {e}")),
    }
    let mut data = bot_settings_event(&mut p, &extra, seq, &from_duels);
    // The settings above are for a company not started yet (start_ai), which no planner key holds;
    // each running company of the AI gets its own plan beside them, under its own key.
    let refusals = crate::rhai_shared::fr_refusals(PLANNED_WRITER);
    let companies = company_plans(&out, &targets, &assigned, &refusals, &extra, seq, &from_duels, &mut |text| g.event("ai", &text));
    data["choice"]["companies"] = json!(companies);
    with_calibration(&mut data, &cal);
    p.bot_settings = data["choice"].clone();
    g.plan("plan.bot_settings", data);
    Some(p)
}

/// The planned-settings path's name for the planner key `planner.<company>` (src/fr_author.rs).
pub const PLANNED_WRITER: &str = "panel_bot";

/// The refusals (rhai_shared::fr_refusals) of `company`: each company has a plan of its own, so
/// only its own planner key holds that plan's freight-line slots.
pub fn held_for(held: &[Value], company: i64) -> Vec<Value> {
    held.iter().filter(|h| h["company"].as_i64() == Some(company)).cloned().collect()
}

/// One plan per running company of the AI (`targets`), from the one planner answer `out`: the
/// freight lines `assigned` hands it (`settings_for`), the stored duel outcomes and the run's extra
/// settings as the plan of a new company gets them, no freight-line slot while its own planner key
/// names another writer (`hold_freight_slots`), and `company` named. One sentence each.
#[allow(clippy::too_many_arguments)]
pub fn company_plans(out: &Value, targets: &[i64], assigned: &BTreeMap<i64, i64>, refusals: &[Value], extra: &str,
                     seq: Option<u64>, from_duels: &duel_outcomes::Derivation, event: &mut dyn FnMut(String)) -> Vec<Value> {
    targets.iter().map(|&company| {
        let mut q = settings_for(out, Some(company), assigned, &mut |_| {});
        if from_duels.record["used"] == true {
            apply_from_duels(&mut q, from_duels, &mut |_| {});
        }
        let mut choice = bot_settings_event(&mut q, extra, seq, from_duels)["choice"].take();
        let held = held_for(refusals, company);
        let withheld = hold_freight_slots(&mut q.settings, &mut choice, &held);
        let from: Vec<String> = choice["freight"].as_array().into_iter().flatten().map(|l| py_str(l.get("from_name"))).collect();
        let why: Vec<&str> = held.iter().filter_map(|h| h["why"].as_str()).collect();
        event(format!("planner: company {company}: {} freight line(s){}, {} portfolio line(s) left out{}", from.len(),
                      if from.is_empty() { String::new() } else { format!(" from {}", from.join(", ")) },
                      choice["dropped"].as_array().map_or(0, Vec::len),
                      if withheld.is_empty() { String::new() } else { format!("; {} freight-line setting(s) not handed over: {}", withheld.len(), why.join("; ")) }));
        choice["company"] = json!(company);
        choice
    }).collect()
}

/// Take every freight-line slot (fr_*, fr2_* .. fr16_*) out of `settings` while `held` names a
/// refusal, and record it in plan.bot_settings' `choice`: its settings and start_ai without them,
/// and `held` {by: the refusals, withheld: the setting names}. Answers the names withheld; with no
/// refusal it changes nothing.
pub fn hold_freight_slots(settings: &mut Settings, choice: &mut Value, held: &[Value]) -> Vec<String> {
    if held.is_empty() {
        return Vec::new();
    }
    let is_slot = crate::panel_decision::is_freight_slot_setting;
    let withheld: Vec<String> = settings.iter().map(|(k, _)| k.clone()).filter(|k| is_slot(k)).collect();
    settings.0.retain(|(k, _)| !is_slot(k));
    let (as_object, flat) = settings_json(settings);
    choice["settings"] = as_object;
    choice["start_ai"] = json!(flat);
    choice["held"] = json!({"by": held, "withheld": withheld});
    withheld
}

/// "name=value,name=value" with setting names of lower-case letters, digits and _ and whole values.
pub fn extra_settings(text: &str) -> Result<Vec<(String, i64)>, String> {
    text.split(',').map(str::trim).filter(|p| !p.is_empty()).map(|p| {
        let (k, v) = p.split_once('=').ok_or_else(|| format!("{p}: name=value"))?;
        let (k, v) = (k.trim(), v.trim());
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(format!("{k}: a setting name (a-z, 0-9, _)"));
        }
        Ok((k.to_string(), v.parse::<i64>().map_err(|_| format!("{k}: a whole number, not {v}"))?))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unplanned bot starts with the run's settings, a later name winning, and a malformed list
    /// is refused whole and said.
    #[test]
    fn an_unplanned_bot_starts_with_the_runs_settings() {
        let mut said = Vec::new();
        let s = unplanned_settings("aggressiveness=80,max_routes=16,max_routes=30, line_budget=30000", &mut |t| said.push(t));
        assert_eq!(s.0, vec![("aggressiveness".to_string(), json!(80)), ("max_routes".to_string(), json!(30)), ("line_budget".to_string(), json!(30000))]);
        assert_eq!(said.len(), 4, "{said:?}");
        assert!(said[2].starts_with("bot setting from the run (unplanned): max_routes=30"), "{said:?}");
        let mut said = Vec::new();
        assert!(unplanned_settings("max_routes=lots", &mut |t| said.push(t)).0.is_empty());
        assert!(said[0].starts_with("bot settings from the run refused: max_routes"), "{said:?}");
        assert!(unplanned_settings("", &mut |_| panic!("nothing to say")).0.is_empty());
    }

    #[test]
    fn the_portfolio_event_names_what_answered_the_kernel_calls() {
        let serve = json!({"runner": "flow serve, one process for every call", "pid": 7, "calls": 33, "fallbacks": 0, "gone": null});
        assert_eq!(kernel_runner(&json!({"algoglyph": serve.clone()}), true), serve);
        assert!(kernel_runner(&json!({}), true)["runner"].as_str().unwrap().contains("one process per call"));
        assert!(kernel_runner(&json!({}), false)["runner"].as_str().unwrap().starts_with("none"));
    }

    const INFO: &str = r#"class X extends AIInfo {
	function GetName()        { return "PrismaticAI"; }
	function GetSettings() {
		AddSetting({
			name = "aggressiveness",
			description = "How aggressively (0 = passive, 100 = maximal), e.g. 'aggressiveness=75'.",
			min_value = 0,
			max_value = 100,
			custom_value = 50,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "intercity",
			description = "1: connect towns; 0: one town only.",
			min_value = 0, max_value = 5, custom_value = 1, step_size = 0,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddLabels("intercity", {_0 = "off", _1 = "on"});
		AddSetting({name = "reserve", description = "Cash", min_value = 0, max_value = 500000, custom_value = 20000, step_size = 5000, flags = 0});
	}
}"#;

    #[test]
    fn settings_are_read_from_info_nut() {
        assert_eq!(info_name(INFO).as_deref(), Some("PrismaticAI"));
        let s = info_settings(INFO);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], json!({"name": "aggressiveness", "description": "How aggressively (0 = passive, 100 = maximal), e.g. 'aggressiveness=75'.",
                                "min": 0, "max": 100, "step": 1, "default": 50, "flags": ["in_game"], "labels": {}, "value": 50,
                                "editable": true, "ingame": true}));
        assert_eq!((s[1]["min"].clone(), s[1]["max"].clone(), s[1]["step"].clone()), (json!(0), json!(1), json!(1)), "a boolean is 0..1");
        assert_eq!(s[1]["flags"], json!(["boolean", "in_game"]));
        assert_eq!(s[1]["labels"], json!({"0": "off", "1": "on"}));
        assert_eq!((s[2]["step"].clone(), s[2]["flags"].clone(), s[2]["ingame"].clone()), (json!(5000), json!([]), json!(false)));
        let real = ai_dir().join("PrismaticAI/info.nut");
        if let Ok(text) = std::fs::read_to_string(real) {
            let s = info_settings(&text);
            assert!(s.iter().all(|s| s["name"].is_string() && s["default"].is_i64()));
            let names: Vec<&str> = s.iter().filter_map(|s| s["name"].as_str()).collect();
            assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len(), "every name once");
            assert!(!names.contains(&"fr"), "no bare name from a loop");
        }
    }

    #[test]
    fn settings_declared_in_a_loop_are_read_by_name() {
        let text = r#"
		AddSetting({name = "fr1_from", description = "line 1", min_value = -1, max_value = 9, custom_value = -1, flags = 0});
		for (local i = 2; i <= 3; i++) {
			AddSetting({
				name = "fr" + i + "_from",
				description = "Producing industry of line " + i + " (id; -1: none).",
				min_value = -1, max_value = 65535, custom_value = -1, flags = CONFIG_INGAME,
			});
		}
		for (local n = 0; n < 2; n++) { AddSetting({name = "x" + n, custom_value = 0, flags = 0}); }
		AddSetting({name = "y" + unknown, custom_value = 0, flags = 0});"#;
        let s = info_settings(text);
        let names: Vec<&str> = s.iter().filter_map(|s| s["name"].as_str()).collect();
        assert_eq!(names, vec!["fr1_from", "fr2_from", "fr3_from", "x0", "x1"], "unrolled; an unresolvable name is skipped");
        assert_eq!(s[2]["description"], json!("Producing industry of line 3 (id; -1: none)."));
        assert_eq!((s[1]["min"].clone(), s[1]["max"].clone(), s[1]["ingame"].clone()), (json!(-1), json!(65535), json!(true)));
        assert_eq!(unroll_loops("for (local i = 0; i < n; i++) { f(i); }"), "for (local i = 0; i < n; i++) { f(i); }", "another shape stays");
    }

    #[test]
    fn lines_without_a_route_say_why_the_most_frequent_reason_first() {
        let refused = format!("error: track-planning refused: [\"terrain.slopes[0] says tile (1, 0) has Slope 0, its heights give 2: {}\"]", "x".repeat(200));
        let priced = vec![json!({"status": "unreachable"}), json!({"status": refused}), json!({"status": "found"}), json!({"status": refused}), json!({})];
        let why = why_unpriced(&priced);
        let cut: String = refused.chars().take(160).collect();
        assert_eq!(why, format!("2x {cut}; 1x unreachable; 1x no status"));
        assert_eq!(why_unpriced(&[json!({"status": "found"})]), "");
    }

    #[test]
    fn a_loop_survives_comments_escapes_nesting_and_huge_bounds() {
        let text = r#"
		for (local i = 1; i <= 2; i++) { // a comment with a } and a " in it
			/* and { another " } */
			AddSetting({name = "a" + i, description = "say \"hi\" {" + i + "}", custom_value = 0, flags = 0});
			for (local j = 0; j < 2; j++) { AddSetting({name = "b" + i + "_" + j, custom_value = 0, flags = 0}); }
		}
		for (local k = -2; k <= 9223372036854775807; k++) { AddSetting({name = "huge" + k, custom_value = 0, flags = 0}); }
		for (local k = -9223372036854775808; k < -9223372036854775808; k++) { AddSetting({name = "min" + k, custom_value = 0, flags = 0}); }"#;
        let s = info_settings(text);
        let names: Vec<&str> = s.iter().filter_map(|s| s["name"].as_str()).collect();
        assert_eq!(names, vec!["a1", "b1_0", "b1_1", "a2", "b2_0", "b2_1"], "comments ignored, nested unrolled, huge loops left alone");
        assert_eq!(s[0]["description"], json!(r#"say \"hi\" {1}"#), "escapes kept as written");
        assert_eq!(strip_comments("x = \"// not a comment\"; // gone\ny"), "x = \"// not a comment\";  \ny");
    }

    #[test]
    fn extra_bot_settings_are_parsed_or_refused_whole() {
        assert_eq!(extra_settings("own_lines=8, line_budget=50000").unwrap(), vec![("own_lines".to_string(), 8), ("line_budget".to_string(), 50000)]);
        assert!(extra_settings("").unwrap().is_empty());
        assert!(extra_settings("own_lines").is_err());
        assert!(extra_settings("Own=1").is_err());
        assert!(extra_settings("own_lines=eight").is_err());
    }

    #[test]
    fn the_bot_gets_the_portfolio_with_rail_it_can_build() {
        let rail = |a: i64, score: f64, status: &str| json!({"kind": "passengers", "mode": "rail", "score": score,
            "a": {"id": a, "name": format!("T{a}")}, "b": {"id": a + 1, "name": "B"},
            "options": [{"mode": "rail", "track": {"status": status}}]});
        let fr = |p: i64, score: f64| json!({"kind": "freight", "mode": "freight_rail", "score": score, "cargo": "Coal", "cargo_id": 1,
            "distance": 40, "producer": {"id": format!("industry:{p}"), "name": "Mine"}, "acceptor": {"id": "industry:99", "name": "Plant"},
            "options": [{"mode": "freight_rail", "monthly_profit": 500, "track": {"status": "found"}}]});
        let air = json!({"kind": "passengers", "mode": "air", "score": 3.0, "a": {"id": 4, "name": "A"}, "b": {"id": 5, "name": "B"},
                         "options": [{"mode": "air", "vehicles": {"count": 12}}]});
        let out = json!({"portfolio": {"solver": "knapsack-dp", "chosen": [rail(1, 9.0, "no_route"), fr(7, 8.0), air.clone()], "total_score": 20,
                                       "investment_k": 250, "budget_k": 300},
                         "line_costs": {"priced": [{"status": "found", "pieces": 30, "money": 12345.6}, {"status": "no_route"}]},
                         "freight": [fr(7, 8.0), fr(8, 5.0), fr(7, 4.0)], "by_mode": {"rail": [rail(1, 9.0, "no_route"), rail(10, 2.0, "found")]}});
        let mut events = Vec::new();
        let planned = settings_from_plan(&out, &mut |e| events.push(e));
        let s = &planned.settings;
        let flat: Vec<String> = s.iter().map(|(k, v)| format!("{k}={}", py_str(Some(v)))).collect();
        // Freight lines are set after the loop, in order of score; the passenger pair (score 2) waits
        // for both freight lines (8 and 5), so rail_after is 2.
        assert_eq!(flat, ["air_a=4", "air_b=5", "air_planes=8", "rail_a=10", "rail_b=11", "fr_from=7", "fr_to=99", "fr_cargo=1",
                          "fr2_from=8", "fr2_to=99", "fr2_cargo=1", "own_lines=8", "rail_after=2"]);
        assert!(events.iter().any(|e| e == "planner: the passenger rail pair (score 2) is built after 2 freight line(s) that score more"));
        // A spare that scores above a portfolio line is built first.
        let order = json!({"portfolio": {"chosen": [fr(7, 3.0), rail(10, 4.0, "found")]}, "freight": [fr(7, 3.0), fr(8, 6.0)]});
        let s2 = settings_from_plan(&order, &mut |_| {}).settings;
        let get = |k: &str| s2.iter().find(|(n, _)| n.as_str() == k).map(|(_, v)| py_str(Some(v)));
        assert_eq!(get("fr_from").as_deref(), Some("8"));
        assert_eq!(get("fr2_from").as_deref(), Some("7"));
        assert_eq!(get("rail_after").as_deref(), Some("1"), "only the line scoring 6 is above the pair's 4");
        assert_eq!(events[0], "planner portfolio by knapsack-dp: 3 investment(s), total score 20, 250k of 300k");
        assert_eq!(events[1], "planner: 2 rail line(s) priced on the map by track-planning, 1 with a route: 30 pieces 12346; 1 without a route dropped their rail option (1x no_route)");
        assert!(events[2].starts_with("planner: the rail line T1 - B has no route on the map (no_route)"));
        assert!(events.iter().any(|e| e.ends_with("(line 2, built when the money allows)")));
        let none = settings_from_plan(&json!({}), &mut |_| {});
        assert!(none.settings.0.is_empty());
        assert_eq!(none.bot_settings["settings"], json!({}));
    }

    /// The planner answer of `the_bot_gets_the_portfolio_with_rail_it_can_build`, as one fixture.
    fn answer() -> Value {
        let rail = |a: i64, score: f64, status: &str| json!({"kind": "passengers", "mode": "rail", "score": score,
            "a": {"id": a, "name": format!("T{a}")}, "b": {"id": a + 1, "name": "B"},
            "options": [{"mode": "rail", "track": {"status": status}}]});
        let fr = |p: i64, score: f64| json!({"kind": "freight", "mode": "freight_rail", "score": score, "cargo": "Coal", "cargo_id": 1,
            "distance": 40, "producer": {"id": format!("industry:{p}"), "name": "Mine"}, "acceptor": {"id": "industry:99", "name": "Plant"},
            "options": [{"mode": "freight_rail", "monthly_profit": 500, "track": {"status": "found"}}]});
        json!({"portfolio": {"solver": "knapsack-dp", "chosen": [rail(1, 9.0, "no_route"), fr(7, 8.0)], "total_score": 17,
                             "investment_k": 250, "budget_k": 300},
               "line_costs": {"priced": [{"status": "found", "pieces": 30, "money": 12345.6}, {"status": "no_route"}]},
               "freight": [fr(7, 8.0), fr(8, 5.0)], "by_mode": {"rail": [rail(1, 9.0, "no_route"), rail(10, 2.0, "found")]},
               "params": {"budget": 300000, "cost_weight": 1}, "reads": [{"uri": "openttd://towns", "ok": true}]})
    }

    /// A chosen line keeps the monthly profit and the vehicles of the option it chose: the one of its
    /// mode and, where the option names one, of its cargo; nothing where the planner gave none.
    #[test]
    fn a_chosen_line_keeps_its_options_profit_and_vehicles() {
        let c = json!({"mode": "freight_rail", "cargo_id": 2, "options": [
            {"mode": "bus", "monthly_profit": 9},
            {"mode": "freight_rail", "cargo_id": 1, "monthly_profit": 100, "vehicles": {"count": 9.0}},
            {"mode": "freight_rail", "cargo_id": 2, "monthly_profit": 250.0, "vehicles": {"count": 3.0}}]});
        let l = line_of(&c);
        assert_eq!((l["monthly_profit"].clone(), l["vehicles"].clone()), (json!(250.0), json!(3.0)));
        let bus = line_of(&json!({"mode": "bus", "options": [{"mode": "bus", "monthly_profit": -4.0}]}));
        assert_eq!((bus["monthly_profit"].clone(), bus.get("vehicles")), (json!(-4.0), None), "a loss is kept; replan reads it as unpriced");
        assert!(line_of(&json!({"mode": "air", "options": [{"mode": "rail", "monthly_profit": 5}]})).get("monthly_profit").is_none());
    }

    #[test]
    fn the_plan_events_of_settings_from_plan_carry_the_choice_as_data() {
        let out = answer();
        let mut p = settings_from_plan(&out, &mut |_| {});
        assert_eq!(p.portfolio, json!({"solver": "knapsack-dp", "total_score": 17, "investment_k": 250, "budget_k": 300,
            "priced": {"lines": 2, "with_route": 1},
            "chosen": [
                {"kind": "passengers", "mode": "rail", "cargo": null, "cargo_id": null, "from": {"id": 1, "name": "T1"},
                 "to": {"id": 2, "name": "B"}, "distance": null, "score": 9.0},
                {"kind": "freight", "mode": "freight_rail", "cargo": "Coal", "cargo_id": 1, "from": {"id": "industry:7", "name": "Mine"},
                 "to": {"id": "industry:99", "name": "Plant"}, "distance": 40, "score": 8.0, "monthly_profit": 500.0}]}));
        let request = json!({"budget": 300000, "params": {"budget": 300000}, "limit": 40, "track": 16});
        let off = calibration(&Scope::Off, &json!({"seed": 3, "map": null}), |_| panic!("calibration off reads no store"));
        let e = portfolio_event(&out, &p, request.clone(), json!({"exe": "mcp-plan"}), &off);
        assert_eq!(e["decision"], "bot.portfolio");
        assert_eq!(e["version"]["params"], json!({"request": request, "effective": {"budget": 300000, "cost_weight": 1}}));
        assert_eq!(e["version"]["revision"], panel_events::REVISION);
        assert_eq!(e["inputs"]["live"]["reads"], json!([{"uri": "openttd://towns", "ok": true}]), "the planner's own list of reads");
        assert!(e["inputs"]["stored"]["fallback"].as_str().unwrap().starts_with("uncalibrated: calibration is off"));
        assert_eq!(e["inputs"]["live"]["calibration"]["fallback"], "uncalibrated");
        assert_eq!(e["choice"], p.portfolio);

        let off = duel_outcomes::from_duels(false, 2, 8, Some(2), |_| unreachable!());
        let b = bot_settings_event(&mut p, "own_lines=0", Some(41), &off);
        assert_eq!(b["decision"], "bot.settings");
        assert_eq!(b["version"]["params"], json!({"max_freight_lines": 16, "own_lines": 8, "PRISMATIC_BOT_SETTINGS": "own_lines=0"}));
        assert_eq!(b["inputs"]["live"]["portfolio_seq"], 41);
        assert_eq!(b["inputs"]["stored"]["fallback"], json!(format!("constants: {}", duel_outcomes::OFF)));
        assert_eq!(b["inputs"]["live"]["from_duels"]["used"], false);
        let c = &b["choice"];
        assert_eq!(c["settings"], json!({"rail_a": 10, "rail_b": 11, "fr_from": 7, "fr_to": 99, "fr_cargo": 1, "fr2_from": 8, "fr2_to": 99,
                                         "fr2_cargo": 1, "own_lines": 0, "rail_after": 2}));
        assert_eq!(c["start_ai"], "rail_a=10,rail_b=11,fr_from=7,fr_to=99,fr_cargo=1,fr2_from=8,fr2_to=99,fr2_cargo=1,own_lines=0,rail_after=2");
        assert_eq!(c["own_lines"], 8, "what the rule chose; from_run says what the run changed");
        assert_eq!(c["from_run"], json!({"settings": {"own_lines": 0}}));
        assert_eq!(c["rail_after"], 2);
        assert_eq!(c["freight"][0], json!({"slot": "fr_", "from": 7, "to": 99, "cargo_id": 1, "cargo": "Coal", "from_name": "Mine",
                                           "to_name": "Plant", "distance": 40, "monthly_profit": 500, "score": 8.0}));
        assert_eq!(c["freight"][1]["slot"], "fr2_");
        assert_eq!(c["passenger_rail"]["line"]["from"], json!({"id": 10, "name": "T10"}));
        assert_eq!(c["passenger_rail"]["spare"], true);
        assert_eq!(c["air"], Value::Null);
        assert_eq!(c["spares_used"], 2, "the passenger pair and the second freight line came from the lists");
        assert_eq!(c["dropped"].as_array().unwrap().len(), 1);
        assert_eq!(c["dropped"][0]["why"], "has no route on the map (no_route)");
        assert_eq!(py_str(p.settings.iter().find(|(k, _)| k == "own_lines").map(|(_, v)| v)), "0", "the bot starts with the run's value");
        let refused = bot_settings_event(&mut settings_from_plan(&out, &mut |_| {}), "own_lines=eight", None, &off);
        assert!(refused["choice"]["from_run"]["refused"].as_str().unwrap().contains("whole number"));
        assert_eq!(refused["choice"]["settings"]["own_lines"], 8);
    }

    /// An answer of every store to `readings_query`, as `panel_store::every_store` gives it.
    fn across(rows: Vec<Value>) -> Across {
        Across { columns: json!(["instance", "seq", "ts_ms", "seed", "map", "lines"]), stores: vec![json!({"store": "panel-1.duckdb", "via": "this panel", "rows": rows.len()})],
                 rows }
    }

    #[test]
    fn the_calibration_reads_the_store_and_falls_back_with_its_reason() {
        let game = json!({"seed": 3, "map": "256x256"});
        let rows = vec![json!(["panel-1", 9, 200, 3, "256x256", "[{\"mode\":\"bus\",\"ratio\":0.2},{\"mode\":\"rail\",\"ratio\":0.6}]"]),
                        json!(["calibration-import", 1, 100, null, null, "[{\"mode\":\"bus\",\"ratio\":0.4}]"]),
                        json!(["panel-1", 4, 50, 3, "256x256", "[{\"mode\":\"rail\",\"why\":\"far\"}]"])];
        let mut asked = None;
        let c = calibration(&Scope::All, &game, |q| { asked = Some(q.clone()); Ok(across(rows.clone())) });
        let want = calibrate::readings_query(&Scope::All, &game).unwrap();
        assert_eq!(asked.as_ref(), Some(&want), "the query of src/calibrate.rs");
        assert_eq!(c.factors, Some(json!({"bus": 0.3, "rail": 0.6, "default": 0.4})), "bus: median of 0.2 and 0.4");
        let mut stored = want.record();
        stored["rows"] = json!(3);
        assert_eq!(c.stored.to_json(), stored);
        assert_eq!((c.record["query"].clone(), c.record["name"].clone()), (json!(want.sql()), json!("calibration_readings")));
        assert_eq!(c.record["used"], true);
        assert_eq!(c.record["readings"], json!([{"instance": "panel-1", "seq": 9, "ts_ms": 200, "seed": 3, "map": "256x256", "lines": 2},
                                               {"instance": "calibration-import", "seq": 1, "ts_ms": 100, "seed": null, "map": null, "lines": 1}]),
                   "the rows it used; a reading that measured nothing is not one");
        assert_eq!(c.record["stores"][0]["via"], "this panel");

        // The store off: no calibration, and the event says why.
        let off = calibration(&Scope::All, &game, |_| Err("store off: built without the store feature".into()));
        assert_eq!(off.factors, None);
        assert_eq!(off.stored.to_json(), json!({"fallback": "uncalibrated: store off: built without the store feature"}));
        assert_eq!((off.record["fallback"].clone(), off.record["used"].clone()), (json!("uncalibrated"), json!(false)));
        assert_eq!(off.record["name"], "calibration_readings", "the query it could not run");
        // Too little history: an empty store, or only readings that measured nothing.
        let empty = calibration(&Scope::All, &game, |_| Ok(across(vec![rows[2].clone()])));
        assert_eq!(empty.factors, None);
        assert!(empty.stored.to_json()["fallback"].as_str().unwrap().starts_with("uncalibrated: the store holds no calibration reading"));
        assert_eq!((empty.record["rows"].clone(), empty.record["params"]["limit"].clone()), (json!(1), json!(calibrate::QUERY_READINGS)));
        // The seed scope without a map size never queries; a file name is refused, never read.
        let blind = calibration(&Scope::Seed, &json!({"seed": 3, "map": null}), |_| panic!("no query without a map"));
        assert!(blind.record["why"].as_str().unwrap().contains("map size"));
        let file = calibration(&calibrate::scope(Some("old/calibration.json")), &game, |_| panic!("a file name reads nothing"));
        assert!(file.record["why"].as_str().unwrap().contains("no file is read"));
        assert_eq!(file.factors, None);
        // Off, the default: the model alone, said so.
        let none = calibration(&calibrate::scope(None), &game, |_| panic!("off reads nothing"));
        assert_eq!(none.stored.to_json()["fallback"], json!(format!("uncalibrated: {CALIBRATION_OFF}")));
    }

    #[test]
    fn plan_bot_settings_names_the_calibration_query_and_its_rows() {
        let out = answer();
        let mut p = settings_from_plan(&out, &mut |_| {});
        let game = json!({"seed": 3, "map": "256x256"});
        let used = calibration(&Scope::All, &game, |_| Ok(across(vec![json!(["panel-1", 9, 200, 3, "256x256", "[{\"mode\":\"bus\",\"ratio\":0.2}]"])])));
        let off = duel_outcomes::from_duels(false, 2, 8, Some(2), |_| unreachable!());
        let mut b = bot_settings_event(&mut p, "", Some(7), &off);
        with_calibration(&mut b, &used);
        let mut stored = calibrate::readings_query(&Scope::All, &game).unwrap().record();
        stored["rows"] = json!(1);
        assert_eq!(b["inputs"]["stored"], stored);
        assert_eq!(b["inputs"]["live"]["calibration"]["readings"][0]["seq"], 9);
        assert_eq!(b["inputs"]["live"]["portfolio_seq"], 7);
        let mut b = bot_settings_event(&mut p, "", Some(7), &off);
        with_calibration(&mut b, &calibration(&Scope::Off, &game, |_| unreachable!()));
        assert_eq!(b["inputs"]["stored"]["fallback"], json!(format!("constants: {}", duel_outcomes::OFF)), "off: the constants, said so");
        assert_eq!(b["inputs"]["live"]["calibration"]["scope"], "off");
    }

    /// A store's answer to `duel_outcomes::outcomes_query`: one held-start ten-year pair handing over
    /// two lines, own_lines 0 against 8, the 8 ahead by 50% against a 0.2% rival spread.
    fn outcome_rows() -> Across {
        let duel = |seq: i64, a: f64, b: f64| json!(["panel-9", seq, seq * 10, "plan.duel", json!({"version": {"params": {"opponent": "PrismaticAI"}},
            "inputs": {"live": {"record": format!("research/results/d{seq}.json")}},
            "choice": {"seed": 5, "months": 120, "held_start_tick": 1400, "a": {"value": a}, "b": {"value": b}}}).to_string()]);
        let settings = |seq: i64, own: i64| json!(["panel-9", seq, seq * 10, "plan.bot_settings",
            json!({"choice": {"freight": [{}, {}], "settings": {"own_lines": own, "rail_after": 2}}}).to_string()]);
        // Two runs of one instance: each verdict joins the settings published before it.
        let rows = vec![settings(1, 0), duel(2, 1_000_000.0, 50_000.0), settings(3, 8), duel(4, 1_500_000.0, 50_100.0)];
        Across { columns: json!(["instance", "seq", "ts_ms", "kind", "data"]),
                 stores: vec![json!({"store": "panel-9.duckdb", "via": "this panel", "rows": rows.len()})], rows }
    }

    #[test]
    fn own_lines_from_stored_duels_reach_the_bot_and_plan_bot_settings_names_query_rule_and_outcomes() {
        let out = answer();
        let mut p = settings_from_plan(&out, &mut |_| {});
        assert_eq!(p.bot_settings["own_lines"], 8, "the constant");
        let handed = p.bot_settings["freight"].as_array().unwrap().len();
        let d = duel_outcomes::from_duels(true, handed, OWN_LINES as i64, p.bot_settings["rail_after"].as_i64(), |_| Ok(outcome_rows()));
        assert_eq!((d.own_lines, d.rail_after), (Some(16), None), "the 8 paid beyond the spread: twice it; no rail_after arm differs");
        let mut said = Vec::new();
        apply_from_duels(&mut p, &d, &mut |t| said.push(t));
        assert_eq!(said.len(), 1);
        assert!(said[0].starts_with("planner: own_lines 16 from stored duel outcomes (constant 8): "), "{}", said[0]);
        let b = bot_settings_event(&mut p, "", Some(3), &d);
        let mut stored = duel_outcomes::outcomes_query().record();
        stored["rows"] = json!(4);
        assert_eq!(b["inputs"]["stored"], stored);
        let live = &b["inputs"]["live"]["from_duels"];
        assert_eq!((live["used"].clone(), live["rule"].clone()), (json!(true), json!(duel_outcomes::RULE)));
        assert_eq!(live["outcomes"].as_array().unwrap().len(), 2);
        assert_eq!(live["derived"]["own_pairs"][0]["verdict"], "pays");
        assert_eq!(b["version"]["params"]["from_duels"]["rule"], "own-lines-by-handed-v1");
        assert_eq!(b["choice"]["own_lines"], 16);
        assert_eq!(b["choice"]["settings"]["own_lines"], 16);
        assert!(b["choice"]["start_ai"].as_str().unwrap().contains("own_lines=16"));
        // The run's own setting still wins over the derived value.
        let mut q = settings_from_plan(&out, &mut |_| {});
        apply_from_duels(&mut q, &d, &mut |_| {});
        assert_eq!(bot_settings_event(&mut q, "own_lines=8", None, &d)["choice"]["settings"]["own_lines"], 8);
        // Off: the constants and the same parameters as before I0087, so the version's digest is unchanged.
        let off = duel_outcomes::from_duels(false, handed, 8, Some(2), |_| unreachable!());
        let mut r = settings_from_plan(&out, &mut |_| {});
        let e = bot_settings_event(&mut r, "", None, &off);
        assert_eq!(e["version"]["params"], json!({"max_freight_lines": 16, "own_lines": 8, "PRISMATIC_BOT_SETTINGS": ""}));
        assert_eq!(e["choice"]["own_lines"], 8);
    }

    /// One author per freight-line slot: no refusal changes nothing (an unset planner key is as
    /// before); a refusal takes every fr_* slot out of the settings and the start_ai line, keeps
    /// the rest, and plan.bot_settings says who holds them.
    #[test]
    fn a_planner_key_of_another_writer_holds_the_planned_freight_slots_and_says_why() {
        let mut p = settings_from_plan(&answer(), &mut |_| {});
        let before = settings_json(&p.settings);
        let mut choice = p.bot_settings.clone();
        assert!(hold_freight_slots(&mut p.settings, &mut choice, &[]).is_empty());
        assert_eq!(settings_json(&p.settings), before, "unset: the settings are exactly as before");
        assert_eq!(choice, p.bot_settings, "unset: the event is exactly as before");
        assert!(before.1.contains("fr_from=7") && before.1.contains("fr2_from=8"), "{}", before.1);

        let refusal = crate::rhai_shared::fr_author::refusal(Some(&json!("battlebrain")), 0, PLANNED_WRITER).unwrap();
        let withheld = hold_freight_slots(&mut p.settings, &mut choice, &[refusal]);
        assert_eq!(withheld, ["fr_from", "fr_to", "fr_cargo", "fr2_from", "fr2_to", "fr2_cargo"]);
        let (_, flat) = settings_json(&p.settings);
        assert!(!flat.contains("fr"), "{flat}");
        assert!(flat.contains("rail_a=10") && flat.contains("own_lines=8"), "the other settings stay: {flat}");
        assert_eq!(choice["start_ai"], json!(flat));
        assert_eq!(choice["held"]["withheld"].as_array().unwrap().len(), 6);
        assert_eq!(choice["held"]["by"][0]["holder"], "battlebrain");
        assert!(choice["held"]["by"][0]["why"].as_str().unwrap().contains("planner.0"));
    }

    /// The panel's check of a change that names its slots: none named, or a company that is no
    /// integer, lets it through; a writer-less one is 422, one whose writer is not the planner 409,
    /// and the planner itself (or anyone while the key is unset) passes.
    #[test]
    fn a_slot_change_needs_a_writer_that_holds_the_planner_key() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let held = |c: i64| (c == 2).then(|| json!("battlebrain"));
        assert!(slot_refusal(&json!(2), &names(&["rail_a", "own_lines"]), None, &held).is_none(), "no slot: no concern");
        assert!(slot_refusal(&json!("2"), &names(&["fr_from"]), None, &held).is_none(), "no integer company: the game refuses it");
        let (status, out) = slot_refusal(&json!(2), &names(&["rail_a", "fr_from", "fr3_to"]), None, &held).unwrap();
        assert_eq!((status, out["outcome"].as_str(), out["settings"].clone()), (422, Some("fr_needs_writer"), json!(["fr_from", "fr3_to"])));
        assert_eq!(out["refusal"]["holder"], "battlebrain");
        let (status, out) = slot_refusal(&json!(2), &names(&["fr_from"]), Some("owner"), &held).unwrap();
        assert_eq!((status, out["outcome"].as_str()), (409, Some("fr_planner_held")));
        assert!(out["error"].as_str().unwrap().starts_with("fr_from not sent: company 2's freight-line slots are planned by battlebrain"), "{out}");
        assert_eq!(out["governed"], GOVERNED_PATHS);
        assert!(slot_refusal(&json!(2), &names(&["fr_from"]), Some("battlebrain"), &held).is_none(), "the planner itself");
        assert!(slot_refusal(&json!(1), &names(&["fr_from"]), Some("owner"), &held).is_none(), "unset: any named writer");
    }

    /// What an MCP call changes on a company's AI, by tool: one name, every key, or nothing.
    #[test]
    fn the_settings_an_mcp_call_changes() {
        assert_eq!(settings_of_call("openttd.ai.set_setting", &json!({"company": 0, "name": "fr_to", "value": 1})), ["fr_to"]);
        let mut every = settings_of_call("openttd.ai.set_settings", &json!({"settings": {"rail_a": 1, "fr_to": 2}}));
        every.sort();
        assert_eq!(every, ["fr_to", "rail_a"]);
        assert!(settings_of_call("openttd.ai.set_settings", &json!({"settings": 3})).is_empty());
        assert!(settings_of_call("openttd.ai.start", &json!({"settings": {"fr_to": 2}})).is_empty(), "a new AI company is no running bot's slot");
    }

    /// Each company has a plan of its own: only its own planner key holds it.
    #[test]
    fn only_a_companys_own_key_holds_its_plan() {
        let r = |c: i64| crate::rhai_shared::fr_author::refusal(Some(&json!("battlebrain")), c, PLANNED_WRITER).unwrap();
        let held = [r(0), r(2), r(5)];
        assert_eq!(held_for(&held, 2), [r(2)]);
        assert!(held_for(&held, 1).is_empty());
    }

    /// The companies a plan is for: running companies of the panel's AI whose AI declares the
    /// freight-line slots, sorted, each once; with no AI named, any AI that declares them.
    #[test]
    fn the_plan_is_for_every_running_company_of_the_ai_that_declares_the_slots() {
        let slots = json!([{"name": "rail_a"}, {"name": "fr_from"}]);
        let running = json!([{"id": 6, "ai": "PrismaticAI", "running": true, "settings": slots},
                             {"id": 0, "ai": "PrismaticAI", "settings": slots},
                             {"id": 2, "ai": "AAAHogEx", "running": true, "settings": [{"name": "rail_a"}]},
                             {"id": 3, "ai": "PrismaticAI", "running": false, "settings": slots},
                             {"id": 4, "ai": "Variant", "running": true, "settings": slots},
                             {"id": 6, "ai": "PrismaticAI", "settings": slots},
                             {"id": 7, "ai": null, "settings": []}]);
        assert_eq!(plan_targets(&running, "PrismaticAI"), [0, 6]);
        assert_eq!(plan_targets(&running, ""), [0, 4, 6], "no AI named: any AI that declares the slots");
        assert!(plan_targets(&json!({"error": "no list"}), "PrismaticAI").is_empty());
    }

    /// A freight line from producer `p` scoring `score`, the rail stations at it held by `held`.
    fn held_line(p: i64, score: f64, held: &[i64]) -> Value {
        json!({"kind": "freight", "mode": "freight_rail", "score": score, "cargo": "Coal", "cargo_id": 1, "distance": 40,
               "producer": {"id": format!("industry:{p}"), "name": format!("Mine {p}"), "rail_stations": held},
               "acceptor": {"id": "industry:99", "name": "Plant"},
               "options": [{"mode": "freight_rail", "monthly_profit": 500, "track": {"status": "found"}}]})
    }

    /// The planner's answer for two PrismaticAI companies 0 and 6 on a map where company 6 has a rail
    /// station at mine 3, AAAHogEx (2) at mine 4, and both siblings at mine 5.
    fn sibling_answer() -> Value {
        let lines = vec![held_line(1, 10.0, &[]), held_line(2, 9.0, &[]), held_line(3, 8.0, &[6]), held_line(4, 7.0, &[2]),
                         held_line(5, 6.0, &[0, 6]), held_line(6, 5.0, &[]), held_line(8, 4.0, &[]), held_line(7, 4.0, &[]),
                         held_line(1, 3.0, &[])];
        json!({"portfolio": {"solver": "knapsack-dp", "chosen": [lines[0].clone(), lines[1].clone(), lines[3].clone()]},
               "line_costs": {"priced": [{"status": "found", "pieces": 30, "money": 1000.0}]}, "freight": lines})
    }

    /// Two companies of one AI never get one producer: the best producer goes to the lower id, the
    /// next to the other, then round by the lines each has (the lower id on a tie, the lower producer
    /// id on equal scores). A producer one company holds goes to it alone; one a rival or both hold
    /// goes to none. The answer does not depend on the order of the planner's lists.
    #[test]
    fn two_companies_of_one_ai_are_handed_different_producers_best_first() {
        let out = sibling_answer();
        let want: BTreeMap<i64, i64> = [(1, 0), (2, 6), (3, 6), (6, 0), (7, 0), (8, 6)].into_iter().collect();
        assert_eq!(assign_producers(&out, &[0, 6]), want);
        assert_eq!(assign_producers(&out, &[6, 0]), want, "the order of the companies does not matter");
        let mut shuffled = out.clone();
        shuffled["freight"].as_array_mut().unwrap().reverse();
        assert_eq!(assign_producers(&shuffled, &[0, 6]), want, "nor the order of the lists");
        // One company: no more than the bot's slots; the rest is left to a company not started yet.
        let many = json!({"freight": (1..=17).map(|p| held_line(p, 100.0 - p as f64, &[])).collect::<Vec<_>>()});
        let one = assign_producers(&many, &[0]);
        assert_eq!(one.len(), MAX_FREIGHT_LINES);
        assert!(!one.contains_key(&17));
        assert!(assign_producers(&many, &[]).is_empty());
    }

    /// Each company's settings hold only the producers it is handed, and a portfolio line it is not
    /// handed is dropped with the reason; a company not started yet gets what no company holds or is
    /// handed.
    #[test]
    fn each_company_is_handed_only_its_own_producers_and_told_why_not_the_others() {
        let out = sibling_answer();
        let assigned = assign_producers(&out, &[0, 6]);
        let froms = |p: &Planned| -> Vec<i64> {
            p.bot_settings["freight"].as_array().unwrap().iter().map(|l| l["from"].as_i64().unwrap()).collect()
        };
        let mut said = Vec::new();
        let zero = settings_for(&out, Some(0), &assigned, &mut |e| said.push(e));
        assert_eq!(froms(&zero), [1, 6, 7]);
        let six = settings_for(&out, Some(6), &assigned, &mut |_| {});
        assert_eq!(froms(&six), [2, 3, 8]);
        let why: Vec<&str> = zero.bot_settings["dropped"].as_array().unwrap().iter().map(|d| d["why"].as_str().unwrap()).collect();
        assert_eq!(why, ["Mine 2 is handed to company 6 of the same AI", "company 2 already has a rail station at Mine 4"]);
        assert!(said.contains(&"planner: the freight line Mine 4 - Plant is not handed over: company 2 already has a rail station at Mine 4".to_string()));
        let flat = settings_json(&zero.settings).1;
        assert!(flat.contains("fr_from=1,") && flat.contains("fr2_from=6,") && flat.contains("fr3_from=7,"), "{flat}");
        // A company not started yet: every producer is held or handed out.
        assert!(froms(&settings_for(&out, None, &assigned, &mut |_| {})).is_empty());
        // Seventeen producers for one company: the last is left to a new company, and the portfolio's
        // line from it says why the running company does not get it.
        let lines: Vec<Value> = (1..=17).map(|p| held_line(p, 100.0 - p as f64, &[])).collect();
        let many = json!({"portfolio": {"chosen": [lines[16].clone()]}, "freight": lines});
        let assigned = assign_producers(&many, &[0]);
        let full = settings_for(&many, Some(0), &assigned, &mut |_| {});
        assert_eq!(full.bot_settings["freight"].as_array().unwrap().len(), MAX_FREIGHT_LINES);
        assert_eq!(full.bot_settings["dropped"][0]["why"], "Mine 17 is handed to no company of the AI: each it may go to has its lines");
        assert_eq!(froms(&settings_for(&many, None, &assigned, &mut |_| {})), [17]);
        // Without holders or companies (a planner that does not say) every line stays, as before.
        assert_eq!(froms(&settings_from_plan(&json!({"freight": [held_line(4, 7.0, &[])]}), &mut |_| {})), [4]);
    }

    /// One plan per company, from one answer: each names its company, holds its own lines, takes the
    /// stored duel outcomes and the run's extra settings, and only its own planner key withholds its
    /// freight-line slots; one sentence each.
    #[test]
    fn every_running_company_gets_a_plan_of_its_own() {
        let out = sibling_answer();
        let assigned = assign_producers(&out, &[0, 6]);
        let d = duel_outcomes::from_duels(true, 2, OWN_LINES as i64, None, |_| Ok(outcome_rows()));
        assert_eq!(d.record["used"], true);
        let refusal = crate::rhai_shared::fr_author::refusal(Some(&json!("battlebrain")), 6, PLANNED_WRITER).unwrap();
        let mut said = Vec::new();
        let plans = company_plans(&out, &[0, 6], &assigned, &[refusal], "line_budget=5", Some(9), &d, &mut |e| said.push(e));
        assert_eq!(plans.len(), 2);
        assert_eq!((plans[0]["company"].clone(), plans[1]["company"].clone()), (json!(0), json!(6)));
        assert_eq!(plans[0]["settings"]["fr_from"], 1);
        assert_eq!(plans[0]["settings"]["own_lines"], 16, "the stored duel outcomes, as for a new company");
        assert_eq!(plans[0]["settings"]["line_budget"], 5, "the run's extra settings too");
        assert!(plans[0]["held"].is_null());
        assert!(plans[1]["settings"].get("fr_from").is_none(), "company 6's key names battlebrain: {}", plans[1]["settings"]);
        assert_eq!(plans[1]["held"]["by"][0]["company"], 6);
        assert_eq!(plans[1]["freight"][0]["from"], 2, "the plan still says what it would hand over");
        assert_eq!(said[0], "planner: company 0: 3 freight line(s) from Mine 1, Mine 6, Mine 7, 2 portfolio line(s) left out");
        assert!(said[1].starts_with("planner: company 6: 3 freight line(s) from Mine 2, Mine 3, Mine 8, 2 portfolio line(s) left out; 9 freight-line setting(s) not handed over: company 6's"), "{}", said[1]);
        let none = duel_outcomes::from_duels(false, 0, OWN_LINES as i64, None, |_| unreachable!());
        assert!(company_plans(&out, &[], &assigned, &[], "", None, &none, &mut |_| {}).is_empty());
    }
}
