//! RESEARCH-ONLY. Kernel contracts derived from the pinned AlgoGlyph source (I0228).
//!
//! AlgoGlyph publishes no typed kernel schema yet (research/battlebrain/algoglyph-contract-proposal.md),
//! and this fork never edits AlgoGlyph. What its source already states is read here instead and
//! written in the proposal's shape (`algoglyph-kernel-contract/v1`), each field with the source line
//! or the test it comes from:
//!
//! - input keys, required keys, enums and descriptions from the kernel's own `input` text (a
//!   DataForm of src/core/solve.ts, or the graph form's doc comment in src/flow/cli.ts) or from the
//!   TypeScript interface a grid kernel's `fromData` reads;
//! - JSON types from the predicates its parser applies (`integer(x)`, `finite(x)`,
//!   `Array.isArray(x)`, `record(x)`, `typeof x === 's'`, `xs.every(...)`, `CONST.includes(x)`,
//!   `x.length === n`), its local helpers included, and the defaults of its destructuring;
//! - output keys and types from the result interface of a grid kernel, or the object a data form's
//!   `solve` returns (literals, `Number.isFinite(v) ? v : null`, a field of a typed call's result);
//! - the input and output keys test/flow-cli.test.ts exercises, by test name.
//!
//! Nothing else is assumed: a field with no evidence is absent and listed in `absent`. Version,
//! exactness and units are stated nowhere in the source, so they are always absent here; the
//! caller's adapter (battlebrain_contract.rs `ADAPTERS`) keeps them with its reasons.
//!
//! The source is read from git at one commit (`git show <commit>:<path>`), never from the working
//! tree, so a dirty checkout cannot shape the artifact.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The contract format, as the proposal names it.
pub const CONTRACT_SCHEMA: &str = "algoglyph-kernel-contract/v1";
/// The format of the committed artifact that holds every derived contract.
pub const ARTIFACT_SCHEMA: &str = "algoglyph-derived-contracts/v1";
/// The committed artifact, relative to the repository root.
pub const ARTIFACT: &str = "tools/research/schema/derived/algoglyph-contracts.json";
/// The runner's kernel list at the pinned commit (its ids and engines), relative to the repository root.
pub const CATALOGUE: &str = "tools/research/schema/fixtures/battlebrain/catalogue.json";
/// Where AlgoGlyph is read from until its submodule lands (feature/algoglyph-submodule), under $HOME.
pub const TRANSITIONAL_ROOT: &str = "dev/algoglyph-wt/main";

/// Why a contract field is absent, the same for every kernel: the source states it nowhere.
pub const ABSENT: &[(&str, &str)] = &[
    ("version", "AlgoGlyph declares no per-kernel version; module.blob names the exact source the contract was derived from"),
    ("exact", "no kernel declares whether its answer is optimal; the caller's adapter states it with its evidence"),
    ("x-unit", "no kernel declares a unit for any field; the caller's adapter maps units"),
];

/// A source tree at one commit.
pub trait Tree {
    fn read(&self, path: &str) -> Option<String>;
    fn blob(&self, path: &str) -> Option<String>;
    fn commit(&self) -> &str;
}

/// An AlgoGlyph git checkout, read at its HEAD commit.
pub struct GitTree {
    root: PathBuf,
    commit: String,
}

impl GitTree {
    pub fn open(root: &Path) -> Result<GitTree, String> {
        let t = GitTree { root: root.to_path_buf(), commit: String::new() };
        match t.git(&["rev-parse", "HEAD"]) {
            Some(c) => Ok(GitTree { commit: c.trim().to_string(), ..t }),
            None => Err(format!("{} is not a git checkout of AlgoGlyph", root.display())),
        }
    }
    fn git(&self, args: &[&str]) -> Option<String> {
        let out = Command::new("git").arg("-C").arg(&self.root).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl Tree for GitTree {
    fn read(&self, path: &str) -> Option<String> {
        self.git(&["show", &format!("{}:{path}", self.commit)])
    }
    fn blob(&self, path: &str) -> Option<String> {
        self.git(&["rev-parse", &format!("{}:{path}", self.commit)]).map(|s| s.trim().to_string())
    }
    fn commit(&self) -> &str {
        &self.commit
    }
}

// ---------------------------------------------------------------------------------------------
// A small scanner of TypeScript text: brackets matched past strings, comments, template literals
// and regular expressions. It reads; it never evaluates.
// ---------------------------------------------------------------------------------------------

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// Whether a `/` at `i` opens a regular expression rather than dividing.
fn regex_allowed(b: &[u8], i: usize) -> bool {
    let prev = b[..i].iter().rev().find(|c| !c.is_ascii_whitespace());
    prev.map_or(true, |c| b"(,=:[!&|?{};+-*%<>~^".contains(c))
}

/// The index past a string, comment, template or regular expression starting at `i`, if one does.
fn skip(b: &[u8], i: usize) -> Option<usize> {
    let at = |k: usize| b.get(k).copied();
    match b[i] {
        q @ (b'\'' | b'"') => {
            let mut j = i + 1;
            while j < b.len() && b[j] != q && b[j] != b'\n' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            Some((j + 1).min(b.len()))
        }
        b'`' => {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'`' {
                if b[j] == b'\\' {
                    j += 2;
                } else if b[j] == b'$' && at(j + 1) == Some(b'{') {
                    j = close(b, j + 1).unwrap_or(b.len());
                } else {
                    j += 1;
                }
            }
            Some((j + 1).min(b.len()))
        }
        b'/' if at(i + 1) == Some(b'/') => Some(b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |p| i + p)),
        b'/' if at(i + 1) == Some(b'*') => Some(find_from(b, i + 2, b"*/").map_or(b.len(), |p| p + 2)),
        b'/' if regex_allowed(b, i) => {
            let (mut j, mut class) = (i + 1, false);
            while j < b.len() && b[j] != b'\n' && (class || b[j] != b'/') {
                match b[j] {
                    b'\\' => j += 1,
                    b'[' => class = true,
                    b']' => class = false,
                    _ => {}
                }
                j += 1;
            }
            j += 1;
            while j < b.len() && b[j].is_ascii_alphabetic() {
                j += 1;
            }
            Some(j.min(b.len()))
        }
        _ => None,
    }
}

fn find_from(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    (from..b.len().saturating_sub(pat.len() - 1)).find(|&k| &b[k..k + pat.len()] == pat)
}

/// The index just past the bracket that closes the one at `open`.
fn close(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        if let Some(j) = skip(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The ranges of `s[from..to]` split at `sep` outside brackets, strings and comments, trimmed.
fn split_top(s: &str, from: usize, to: usize, sep: u8) -> Vec<(usize, usize)> {
    let b = s.as_bytes();
    if from >= to {
        return vec![];
    }
    let (mut out, mut depth, mut start, mut i) = (vec![], 0i32, from, from);
    while i < to {
        if let Some(j) = skip(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push((start, to));
    out.into_iter()
        .map(|(a, z)| {
            let t = &s[a..z];
            let lead = t.len() - t.trim_start().len();
            (a + lead, a + lead + t.trim().len())
        })
        .filter(|(a, z)| z > a)
        .collect()
}

/// `s` split at its first `sep` outside brackets and strings (a ternary's `:` stays in the value).
fn split_first(s: &str, sep: u8) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    let (mut depth, mut i) = (0i32, 0);
    while i < b.len() {
        if let Some(j) = skip(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            c if c == sep && depth == 0 => return Some((s[..i].trim(), s[i + 1..].trim())),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The end of the statement that contains index `from`: its `;` once the brackets opened after
/// `from` are closed, or `to`.
fn statement_end(s: &str, from: usize, to: usize) -> usize {
    let b = s.as_bytes();
    let (mut depth, mut i) = (0i32, from);
    while i < to {
        if let Some(j) = skip(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' if depth <= 0 => return i,
            _ => {}
        }
        i += 1;
    }
    to
}

/// The 1-based line of byte `i`.
fn line_of(s: &str, i: usize) -> usize {
    s.as_bytes()[..i.min(s.len())].iter().filter(|&&c| c == b'\n').count() + 1
}

/// Every index of `pat` in `s[from..to]` where it stands as a whole word (no identifier character
/// or `.` before it, no identifier character after it).
fn word_hits(s: &str, from: usize, to: usize, pat: &str) -> Vec<usize> {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut k = from;
    while let Some(p) = s[k..to].find(pat) {
        let i = k + p;
        let before = i.checked_sub(1).map(|j| b[j]);
        let after = b.get(i + pat.len()).copied();
        if before.map_or(true, |c| !is_ident(c) && c != b'.') && after.map_or(true, |c| !is_ident(c)) {
            out.push(i);
        }
        k = i + pat.len();
    }
    out
}

/// A JS string literal at `i` (quote included), unescaped.
fn string_at(s: &str, i: usize) -> Option<(String, usize)> {
    let b = s.as_bytes();
    let q = *b.get(i)?;
    if !matches!(q, b'\'' | b'"' | b'`') {
        return None;
    }
    let end = skip(b, i)?;
    let raw = &s[i + 1..end - 1];
    if q == b'`' && raw.contains("${") {
        return None;
    }
    let (mut out, mut escaped) = (String::new(), false);
    for c in raw.chars() {
        if escaped {
            out.push(if c == 'n' { '\n' } else { c });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    Some((out, end))
}

/// A literal value (number, string, boolean, null, empty array) as JSON, or None.
fn literal(t: &str) -> Option<Value> {
    let t = t.trim().trim_end_matches(" as const");
    match t {
        "true" => return Some(json!(true)),
        "false" => return Some(json!(false)),
        "null" => return Some(Value::Null),
        "[]" => return Some(json!([])),
        _ => {}
    }
    if let Some((v, end)) = string_at(t, 0) {
        return (end == t.len()).then(|| json!(v));
    }
    let n = t.replace('_', "");
    if let Ok(i) = n.parse::<i64>() {
        return Some(json!(i));
    }
    n.parse::<f64>().ok().filter(|f| f.is_finite() && n.chars().all(|c| c.is_ascii_digit() || ".-+e".contains(c))).map(|f| json!(f))
}

// ---------------------------------------------------------------------------------------------
// Modules: one source file, its imports one hop away.
// ---------------------------------------------------------------------------------------------

struct Module {
    path: String,
    text: String,
    blob: String,
    imports: Vec<(String, Module)>,
}

impl Module {
    fn load(tree: &dyn Tree, path: &str, hops: usize) -> Option<Module> {
        let text = tree.read(path)?;
        let blob = tree.blob(path).unwrap_or_default();
        let mut imports = vec![];
        if hops > 0 {
            let mut k = 0;
            while let Some(p) = text[k..].find("import {") {
                let i = k + p;
                let Some(end) = close(text.as_bytes(), i + 7) else { break };
                let names: Vec<String> = text[i + 8..end - 1]
                    .split(',')
                    .map(|n| n.trim().trim_start_matches("type ").split(" as ").last().unwrap_or("").trim().to_string())
                    .filter(|n| !n.is_empty())
                    .collect();
                if let Some(m) = import_target(path, &text[end..]).and_then(|t| Module::load(tree, &t, hops - 1)) {
                    imports.extend(names.into_iter().map(|n| (n, Module { path: m.path.clone(), text: m.text.clone(), blob: m.blob.clone(), imports: vec![] })));
                }
                k = end;
            }
        }
        Some(Module { path: path.to_string(), text, blob, imports })
    }

    fn at(&self, i: usize) -> String {
        format!("{}:{}", self.path, line_of(&self.text, i))
    }

    /// Where `sub`, a slice of this module's text, stands; the module itself otherwise.
    fn at_slice(&self, sub: &str) -> String {
        let (base, p) = (self.text.as_ptr() as usize, sub.as_ptr() as usize);
        if p >= base && p <= base + self.text.len() {
            self.at(p - base)
        } else {
            self.path.clone()
        }
    }

    /// The module (this one or an import) that declares `name` with `kind` (`const`, `function`,
    /// `interface`, `type`), and the index of its name.
    fn decl(&self, kind: &str, name: &str) -> Option<(&Module, usize)> {
        let here = |m: &Module| -> Option<usize> {
            let pat = format!("{kind} {name}");
            word_hits(&m.text, 0, m.text.len(), &pat).into_iter().next().map(|i| i + kind.len() + 1)
        };
        if let Some(i) = here(self) {
            return Some((self, i));
        }
        self.imports.iter().filter(|(n, _)| n == name).find_map(|(_, m)| here(m).map(|i| (m, i)))
    }
}

/// The module an import's `from '<spec>'` (the text after its braces) names, if it is relative.
fn import_target(path: &str, rest: &str) -> Option<String> {
    let q = rest.find(['\'', '"']).filter(|&q| rest[..q].trim() == "from")?;
    resolve(path, &string_at(rest, q)?.0)
}

fn resolve(from: &str, spec: &str) -> Option<String> {
    if !spec.starts_with('.') {
        return None;
    }
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for seg in spec.split('/') {
        match seg {
            "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(format!("{}.ts", parts.join("/")))
}

// ---------------------------------------------------------------------------------------------
// TypeScript types and the kernel's own shape text, read as JSON Schema.
// ---------------------------------------------------------------------------------------------

/// The members of a TypeScript object type body `s[from..to]`: (name, optional, type text, doc, index).
fn members(s: &str, from: usize, to: usize) -> Vec<(String, bool, String, Option<String>, usize)> {
    let mut out = vec![];
    let mut doc: Option<String> = None;
    let b = s.as_bytes();
    let mut i = from;
    while i < to {
        while i < to && (b[i].is_ascii_whitespace() || b[i] == b';' || b[i] == b',') {
            i += 1;
        }
        if i >= to {
            break;
        }
        if s[i..].starts_with("/**") {
            let end = find_from(b, i, b"*/").map_or(to, |p| p + 2);
            let text: Vec<&str> = s[i + 3..end - 2].lines().map(|l| l.trim().trim_start_matches('*').trim()).filter(|l| !l.is_empty()).collect();
            doc = Some(text.join(" "));
            i = end;
            continue;
        }
        if s[i..].starts_with("//") {
            i = skip(b, i).unwrap_or(to);
            continue;
        }
        // one member: up to a `;` or `,` at depth 0, or a newline at depth 0 not followed by `|`
        let (mut j, mut depth) = (i, 0i32);
        while j < to {
            if let Some(k) = skip(b, j) {
                j = k;
                continue;
            }
            match b[j] {
                b'(' | b'[' | b'{' | b'<' => depth += 1,
                b')' | b']' | b'}' | b'>' if !(b[j] == b'>' && j > 0 && b[j - 1] == b'=') => depth -= 1,
                b';' | b',' if depth == 0 => break,
                b'\n' if depth == 0 && !s[j..to].trim_start().starts_with('|') && !s[i..j].trim_end().ends_with('|') => break,
                _ => {}
            }
            j += 1;
        }
        let m = s[i..j].trim();
        if let Some(colon) = m.find(':') {
            let (name, opt) = match m[..colon].trim() {
                n if n.ends_with('?') => (n.trim_end_matches('?').trim(), true),
                n => (n, false),
            };
            if !name.is_empty() && name.bytes().all(is_ident) {
                out.push((name.to_string(), opt, m[colon + 1..].trim().to_string(), doc.take(), i));
            }
        }
        doc = None;
        i = j + 1;
    }
    out
}

/// The JSON Schema of a TypeScript type. Named types stay named (`x-ts-type`), never expanded,
/// except an indexed access `Name['field']` to an interface of the module.
fn ts_schema(m: &Module, t: &str) -> Value {
    let t = t.trim();
    let alts = split_top(t, 0, t.len(), b'|');
    if alts.len() > 1 {
        let parts: Vec<&str> = alts.iter().map(|(a, z)| &t[*a..*z]).collect();
        if let Some(lits) = parts.iter().map(|p| literal(p)).collect::<Option<Vec<_>>>() {
            return json!({"enum": lits});
        }
        let schemas: Vec<Value> = parts.iter().map(|p| ts_schema(m, p)).collect();
        let simple: Option<Vec<Value>> = schemas.iter().map(|s| (s.as_object().map_or(0, Map::len) == 1).then(|| s.get("type").cloned()).flatten()).collect();
        return match simple {
            Some(types) => json!({"type": types}),
            None => json!({"anyOf": schemas}),
        };
    }
    if let Some(inner) = t.strip_suffix("[]") {
        return json!({"type": "array", "items": ts_schema(m, inner)});
    }
    match t {
        "number" | "string" | "boolean" | "null" => return json!({"type": t}),
        "unknown" | "any" => return json!({}),
        _ => {}
    }
    if let Some(v) = literal(t) {
        return json!({"const": v});
    }
    let b = t.as_bytes();
    if b.first() == Some(&b'(') && close(b, 0) == Some(t.len()) {
        return ts_schema(m, &t[1..t.len() - 1]);
    }
    if b.first() == Some(&b'{') && close(b, 0) == Some(t.len()) {
        return object_type(m, &members(t, 1, t.len() - 1));
    }
    if b.first() == Some(&b'[') && close(b, 0) == Some(t.len()) {
        let items: Vec<Value> = split_top(t, 1, t.len() - 1, b',').iter().map(|(a, z)| ts_schema(m, &t[*a..*z])).collect();
        return json!({"type": "array", "prefixItems": items, "minItems": items.len(), "maxItems": items.len()});
    }
    if t.starts_with("Record<") || t.starts_with("Partial<Record") {
        return json!({"type": "object"});
    }
    if let Some(inner) = t.strip_prefix("Array<").and_then(|r| r.strip_suffix('>')) {
        return json!({"type": "array", "items": ts_schema(m, inner)});
    }
    if let Some((name, field)) = t.split_once("['").and_then(|(n, r)| r.strip_suffix("']").map(|f| (n, f))) {
        if let Some(s) = interface(m, name).and_then(|(im, ms)| ms.into_iter().find(|x| x.0 == field).map(|x| ts_schema(im, &x.2))) {
            return s;
        }
    }
    json!({"x-ts-type": t})
}

fn object_type(m: &Module, ms: &[(String, bool, String, Option<String>, usize)]) -> Value {
    let mut props = Map::new();
    for (name, _, ty, doc, _) in ms {
        let mut s = ts_schema(m, ty);
        if let Some(d) = doc {
            s["description"] = json!(d);
        }
        props.insert(name.clone(), s);
    }
    let req: Vec<&String> = ms.iter().filter(|x| !x.1).map(|x| &x.0).collect();
    json!({"type": "object", "required": req, "properties": props})
}

/// The members of interface `name`, declared in `m` or one of its imports.
fn interface<'a>(m: &'a Module, name: &str) -> Option<(&'a Module, Vec<(String, bool, String, Option<String>, usize)>)> {
    let (im, i) = m.decl("interface", name)?;
    let open = im.text[i..].find('{')? + i;
    let end = close(im.text.as_bytes(), open)?;
    Some((im, members(&im.text, open + 1, end - 1)))
}

/// One key of a shape text: its name, whether it is optional, whether it is one of an alternative
/// (`a? | b: ...`), and its value text.
struct ShapeKey {
    name: String,
    optional: bool,
    alternative: bool,
    value: String,
}

/// The keys of a kernel's shape text `{a, b?: T, c? | d: U, ...}` written for a reader (a DataForm
/// `input`, the graph form's doc comment), and the text after its closing brace. None when the
/// text holds no braced shape. Single quotes are prose (`executor's`), never strings, here.
fn shape_keys(text: &str) -> Option<(Vec<ShapeKey>, String)> {
    let open = text.find('{')?;
    let b = text.as_bytes();
    let (mut depth, mut end, mut quoted) = (0i32, None, false);
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'"' => quoted = !quoted,
            b'{' | b'[' | b'(' if !quoted => depth += 1,
            b'}' | b']' | b')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    let mut parts: Vec<(String, bool)> = vec![];
    let (mut depth, mut start, mut quoted) = (0i32, open + 1, false);
    for i in open + 1..=end {
        let c = b[i];
        match c {
            b'"' => quoted = !quoted,
            b'{' | b'[' | b'(' if !quoted => depth += 1,
            b'}' | b']' | b')' if !quoted && i < end => depth -= 1,
            _ => {}
        }
        if (c == b',' && depth == 0 && !quoted) || i == end {
            parts.push((text[start..i].trim().to_string(), false));
            start = i + 1;
        }
    }
    let mut keys = vec![];
    for (part, _) in parts {
        let mut rest = part.as_str();
        let mut alternative = false;
        loop {
            let n = rest.bytes().take_while(|&c| is_ident(c)).count();
            if n == 0 {
                break;
            }
            let name = &rest[..n];
            let mut after = &rest[n..];
            let optional = after.starts_with('?');
            if optional {
                after = &after[1..];
            }
            let t = after.trim_start();
            if t.is_empty() || t.starts_with(':') || t.starts_with('(') {
                let value = t.strip_prefix(':').unwrap_or(t).trim().to_string();
                keys.push(ShapeKey { name: name.to_string(), optional, alternative, value });
                break;
            } else if let Some(r) = t.strip_prefix('|').or_else(|| t.strip_prefix('+')) {
                // a key followed by `|` (or `+`) has no value of its own; the next key is its alternative
                let alt = t.starts_with('|');
                keys.push(ShapeKey { name: name.to_string(), optional, alternative: alternative || alt, value: String::new() });
                alternative = alternative || alt;
                rest = r.trim_start();
                if alt {
                    if let Some(k) = keys.last_mut() {
                        k.alternative = true;
                    }
                }
            } else {
                break;
            }
        }
    }
    Some((keys, text[end + 1..].trim().trim_start_matches(['.', ':', ' ']).to_string()))
}

/// The JSON Schema a shape text's value states: a nested shape, a list, a primitive, an enum of
/// literals or bare upper-case words, and the rest as a description.
fn shape_value(v: &str) -> Value {
    let v = v.trim();
    if v.is_empty() {
        return json!({});
    }
    if let Some(inner) = v.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        return json!({"description": inner});
    }
    if v.starts_with('{') {
        if let Some((keys, tail)) = shape_keys(v) {
            let mut s = shape_object(&keys);
            if keys.is_empty() {
                s = json!({"type": "object", "description": v});
            } else if !tail.is_empty() {
                s["description"] = json!(tail);
            }
            return s;
        }
    }
    if v.starts_with('[') {
        let b = v.as_bytes();
        let mut depth = 0;
        let end = b.iter().enumerate().find_map(|(i, &c)| {
            match c {
                b'[' | b'{' | b'(' => depth += 1,
                b']' | b'}' | b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
            None
        });
        if let Some(e) = end {
            let inner = v[1..e].trim();
            let mut s = json!({"type": "array"});
            if inner.starts_with('{') {
                s["items"] = shape_value(inner);
            } else {
                let n = inner.split(',').filter(|x| !x.trim().is_empty()).count();
                if n >= 2 && inner.split(',').all(|x| x.trim().bytes().all(is_ident)) {
                    s["minItems"] = json!(n);
                    s["maxItems"] = json!(n);
                }
            }
            let tail = v[e + 1..].trim();
            if let Some(t) = tail.strip_prefix("[]") {
                s = json!({"type": "array", "items": s});
                if !t.trim().is_empty() {
                    s["description"] = json!(t.trim());
                }
            } else if !tail.is_empty() {
                s["description"] = json!(tail);
            }
            return s;
        }
    }
    let alts: Vec<&str> = v.split('|').map(str::trim).collect();
    if alts.len() > 1 {
        let words = |a: &&str| !a.is_empty() && a.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) && a.as_bytes()[0].is_ascii_uppercase();
        if alts.iter().all(words) {
            return json!({"enum": alts});
        }
        let lits: Vec<Value> = alts.iter().filter_map(|a| literal(a)).collect();
        if lits.len() == alts.len() {
            return json!({"enum": lits});
        }
        let mut parts: Vec<Value> = alts.iter().filter(|a| literal(a).is_none()).map(|a| shape_value(a)).collect();
        if !lits.is_empty() {
            parts.insert(0, json!({"enum": lits}));
        }
        return json!({"anyOf": parts});
    }
    let (head, tail) = match v.find(' ') {
        Some(i) => (&v[..i], v[i + 1..].trim()),
        None => (v, ""),
    };
    let mut s = match head.trim_end_matches(['[', ']']) {
        "number" | "string" | "boolean" | "integer" => {
            let mut s = json!({"type": head.trim_end_matches("[]").trim_end_matches("[]")});
            for _ in 0..head.matches("[]").count() {
                s = json!({"type": "array", "items": s});
            }
            s
        }
        _ => return json!({"description": v}),
    };
    if !tail.is_empty() {
        s["description"] = json!(tail.trim_start_matches('(').trim_end_matches(')'));
    }
    s
}

fn shape_object(keys: &[ShapeKey]) -> Value {
    let mut props = Map::new();
    let mut req = vec![];
    for k in keys {
        props.insert(k.name.clone(), shape_value(&k.value));
        if !k.optional && !k.alternative {
            req.push(k.name.clone());
        }
    }
    json!({"type": "object", "required": req, "properties": props})
}

// ---------------------------------------------------------------------------------------------
// What a parser's predicates say about the keys of the value it reads.
// ---------------------------------------------------------------------------------------------

/// What the source says about one key: its JSON types, item types, enum, default, length, nested
/// keys, and the lines saying so.
#[derive(Default, Clone, Debug)]
struct Facts {
    types: BTreeSet<String>,
    items: BTreeSet<String>,
    enumv: Option<Value>,
    default: Option<Value>,
    length: Option<usize>,
    nested: Option<Value>,
    from: BTreeSet<String>,
}

impl Facts {
    fn merge(&mut self, o: Facts) {
        self.types.extend(o.types);
        self.items.extend(o.items);
        self.enumv = self.enumv.take().or(o.enumv);
        self.default = self.default.take().or(o.default);
        self.length = self.length.or(o.length);
        self.from.extend(o.from);
        self.nested = match (self.nested.take(), o.nested) {
            (Some(a), Some(b)) => Some(merge_schema(a, b)),
            (a, b) => a.or(b),
        };
    }
    fn is_empty(&self) -> bool {
        self.types.is_empty() && self.items.is_empty() && self.enumv.is_none() && self.default.is_none() && self.length.is_none() && self.nested.is_none()
    }
    /// The facts as a JSON Schema fragment.
    fn schema(&self) -> Value {
        let mut s = self.nested.clone().unwrap_or_else(|| json!({}));
        let mut types = self.types.clone();
        if types.contains("integer") {
            types.remove("number");
        }
        if self.nested.as_ref().is_some_and(|n| n["type"] == "array") || self.length.is_some() || !self.items.is_empty() {
            types.insert("array".into());
        }
        match types.len() {
            0 => {}
            1 => s["type"] = json!(types.iter().next()),
            _ => s["type"] = json!(types),
        }
        if !self.items.is_empty() {
            let mut it = self.items.clone();
            if it.contains("integer") {
                it.remove("number");
            }
            let t: Vec<&String> = it.iter().collect();
            s["items"] = if t.len() == 1 { json!({"type": t[0]}) } else { json!({"type": t}) };
        }
        if let Some(n) = self.length {
            s["minItems"] = json!(n);
            s["maxItems"] = json!(n);
        }
        if let Some(e) = &self.enumv {
            s["enum"] = e.clone();
        }
        if let Some(d) = &self.default {
            s["default"] = d.clone();
        }
        s
    }
}

/// `b` laid over `a`: `b`'s keys win, `properties` and `required` are merged.
fn merge_schema(mut a: Value, b: Value) -> Value {
    let Some(bo) = b.as_object() else { return a };
    if !a.is_object() {
        return b;
    }
    for (k, v) in bo {
        match k.as_str() {
            "properties" => {
                let ap = a["properties"].as_object().cloned().unwrap_or_default();
                let mut merged = ap.clone();
                for (pk, pv) in v.as_object().into_iter().flatten() {
                    let next = match ap.get(pk) {
                        Some(old) => merge_schema(old.clone(), pv.clone()),
                        None => pv.clone(),
                    };
                    merged.insert(pk.clone(), next);
                }
                a["properties"] = Value::Object(merged);
            }
            "required" => {
                let mut r: BTreeSet<String> = a["required"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
                r.extend(v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)));
                a["required"] = json!(r);
            }
            "items" if a.get("items").is_some() => {
                let old = a["items"].take();
                a["items"] = merge_schema(old, v.clone());
            }
            "x-from" => {
                let mut f: BTreeSet<String> = a["x-from"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
                f.extend(v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)));
                a["x-from"] = json!(f);
            }
            _ => a[k] = v.clone(),
        }
    }
    a
}

/// A value the parser reads: a name it is bound to, and the keys of it that name cannot see
/// (destructured before a `...rest` that was passed on).
#[derive(Clone)]
struct Root {
    name: String,
    hidden: BTreeSet<String>,
}

/// Predicates whose argument then has a JSON type.
const TYPE_CALLS: &[(&str, &str)] = &[
    ("Number.isInteger(", "integer"),
    ("integer(", "integer"),
    ("isInt(", "integer"),
    ("Number.isFinite(", "number"),
    ("finite(", "number"),
    ("isNum(", "number"),
    ("Array.isArray(", "array"),
    ("record(", "object"),
];

/// Calls that are not helpers of the parser (never followed into).
const NOT_HELPERS: &[&str] = &["String", "Number", "Boolean", "unknownKeys", "Object", "JSON", "Math", "Array", "if", "while", "for", "return", "typeof"];

/// The function of `m` (or of the body `ctx`) named `name`: its first parameter and the range of
/// its body (a block or an expression).
fn helper(m: &Module, ctx: (usize, usize), name: &str) -> Option<(String, (usize, usize), usize)> {
    let s = &m.text;
    let b = s.as_bytes();
    let cands = word_hits(s, ctx.0, ctx.1, &format!("const {name}")).into_iter().chain(word_hits(s, 0, s.len(), &format!("const {name}"))).chain(word_hits(s, 0, s.len(), &format!("function {name}")));
    for at in cands {
        let rest = &s[at..];
        let paren = rest.find('(')? + at;
        if s[at..paren].contains('\n') || s[at..paren].contains(';') {
            continue;
        }
        let pclose = close(b, paren)?;
        let param: String = s[paren + 1..pclose - 1].trim().bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
        if param.is_empty() {
            continue;
        }
        let after = &s[pclose..];
        let body_at = if s[at..].starts_with("function") {
            body_brace(s, pclose)?
        } else {
            let arrow = after.find("=>")? + pclose + 2;
            arrow + (s[arrow..].len() - s[arrow..].trim_start().len())
        };
        let end = if b[body_at] == b'{' {
            close(b, body_at)?
        } else {
            split_top(s, body_at, s.len(), b';').first().map(|r| r.1)?
        };
        return Some((param, (body_at, end), at));
    }
    None
}

/// The brace opening a function's body after its parameter list ends at `from`: the last `{` on
/// that line, so a return type such as `: { g: Graph } | string {` is passed over.
fn body_brace(s: &str, from: usize) -> Option<usize> {
    let eol = s[from..].find('\n').map_or(s.len(), |p| p + from);
    s[from..eol].rfind('{').map(|p| p + from)
}

/// The keys a function `f` takes out of its argument before passing the rest on
/// (`const {a, ...rest} = arg; ... {...rest}`): its result names every other key of the argument.
fn passed_on(m: &Module, ctx: (usize, usize), f: &str) -> Option<BTreeSet<String>> {
    let s = &m.text;
    let (p, body, _) = helper(m, ctx, f)?;
    let d = s[body.0..body.1].find("const {")? + body.0;
    let e = close(s.as_bytes(), d + 6)?;
    (s[e..].trim_start().starts_with(&format!("= {p}")) && s[d..e].contains("...")).then_some(())?;
    Some(split_top(s, d + 7, e - 1, b',').iter().map(|(a, z)| s[*a..*z].split([':', '=']).next().unwrap_or("").trim().to_string()).filter(|k| !k.starts_with("...")).collect())
}

/// The keys `root.k` names in `s[from..to]`, in order of first use.
fn keys_of(s: &str, from: usize, to: usize, root: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for i in word_hits(s, from, to, root) {
        let rest = &s[i + root.len()..to];
        let rest = rest.strip_prefix('!').unwrap_or(rest);
        if let Some(r) = rest.strip_prefix("?.").or_else(|| rest.strip_prefix('.')) {
            let k: String = r.bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
            if !k.is_empty() && !["every", "length", "entries", "map", "filter", "some", "includes", "slice"].contains(&k.as_str()) && !out.contains(&k) {
                out.push(k);
            }
        }
    }
    out
}

/// The facts the predicates in body `ctx` of `m` state about each key of `roots`, and (key `""`)
/// about the root itself. `declared` limits the keys to those named elsewhere; None takes every
/// key the body reads.
fn refine(m: &Module, ctx: (usize, usize), roots: &[Root], declared: Option<&[String]>, depth: usize) -> BTreeMap<String, Facts> {
    let s = &m.text;
    let mut roots = roots.to_vec();
    // `const d = raw as T` makes d a name of the same value; `const v = f(raw)` where f passes its
    // argument's other keys on (`const {a, ...rest} = arg; ... {...rest}`) makes v a name of every
    // key but a.
    for r in roots.clone() {
        for i in word_hits(s, ctx.0, ctx.1, "const") {
            let stmt = &s[i..split_top(s, i, ctx.1, b';').first().map_or(i, |x| x.1)];
            let Some((lhs, rhs)) = stmt.strip_prefix("const ").and_then(|t| t.split_once(" = ")) else { continue };
            let lhs = lhs.split(':').next().unwrap_or("").trim();
            if !lhs.bytes().all(is_ident) || lhs.is_empty() {
                continue;
            }
            let rhs = rhs.trim();
            if rhs == r.name || rhs.starts_with(&format!("{} as ", r.name)) {
                roots.push(Root { name: lhs.to_string(), hidden: r.hidden.clone() });
            } else if let Some(hidden) = rhs.strip_suffix(&format!("({})", r.name)).and_then(|f| passed_on(m, ctx, f)) {
                roots.push(Root { name: lhs.to_string(), hidden: hidden.union(&r.hidden).cloned().collect() });
            }
        }
    }
    // names bound to a key: destructuring `const {k, k: a, k = d} = root`, `const a = root.k (?? d)`
    let mut names: Vec<(String, String, Option<Value>, usize)> = vec![];
    for r in &roots {
        names.push((r.name.clone(), String::new(), None, 0));
        for i in word_hits(s, ctx.0, ctx.1, "const") {
            let after = i + 6;
            if s[after..].starts_with('{') {
                let Some(e) = close(s.as_bytes(), after) else { continue };
                let rest = s[e..].trim_start();
                let Some(rhs) = rest.strip_prefix('=') else { continue };
                let rhs = rhs.trim_start();
                if !(rhs.starts_with(&r.name) && rhs.as_bytes().get(r.name.len()).map_or(true, |&c| !is_ident(c))) {
                    continue;
                }
                for (a, z) in split_top(s, after + 1, e - 1, b',') {
                    let ent = &s[a..z];
                    if ent.starts_with("...") {
                        continue;
                    }
                    let (left, def) = match ent.split_once('=') {
                        Some((l, d)) => (l.trim(), literal(d)),
                        None => (ent.trim(), None),
                    };
                    let (k, alias) = match left.split_once(':') {
                        Some((k, al)) => (k.trim(), al.trim()),
                        None => (left, left),
                    };
                    if !r.hidden.contains(k) {
                        names.push((alias.to_string(), k.to_string(), def, a));
                    }
                }
            } else if let Some((lhs, rhs)) = s[after..split_top(s, after, ctx.1, b';').first().map_or(after, |x| x.1)].split_once(" = ") {
                let lhs = lhs.trim();
                let (val, def) = match rhs.split_once(" ?? ") {
                    Some((v, d)) => (v.trim(), literal(d)),
                    None => (rhs.trim(), None),
                };
                if let Some(k) = val.strip_prefix(&format!("{}.", r.name)) {
                    if k.bytes().all(is_ident) && !k.is_empty() && lhs.bytes().all(is_ident) && !r.hidden.contains(k) {
                        names.push((lhs.to_string(), k.to_string(), def, after));
                    }
                }
            }
        }
    }
    let wanted: Vec<String> = match declared {
        Some(d) => d.to_vec(),
        None => {
            let mut all = vec![];
            for r in &roots {
                for k in keys_of(s, ctx.0, ctx.1, &r.name) {
                    if !all.contains(&k) && !r.hidden.contains(&k) {
                        all.push(k);
                    }
                }
            }
            all
        }
    };
    let mut out: BTreeMap<String, Facts> = BTreeMap::new();
    let mut exprs: Vec<(String, String)> = vec![]; // (key, expression)
    for r in &roots {
        exprs.push((String::new(), r.name.clone()));
        for k in &wanted {
            if !r.hidden.contains(k) {
                exprs.push((k.clone(), format!("{}.{k}", r.name)));
                exprs.push((k.clone(), format!("{}?.{k}", r.name)));
            }
        }
    }
    for (alias, k, def, at) in &names {
        if k.is_empty() {
            continue;
        }
        if !wanted.contains(k) {
            continue;
        }
        exprs.push((k.clone(), alias.clone()));
        if let Some(d) = def {
            let f = out.entry(k.clone()).or_default();
            f.default = Some(d.clone());
            f.from.insert(format!("{} default", m.at(*at)));
        }
    }
    for (k, e) in exprs {
        let f = facts_of(m, ctx, &e, depth);
        if !f.is_empty() {
            out.entry(k).or_default().merge(f);
        }
    }
    out
}

/// Whether a predicate starting at `start` (its own negations included in `negated`) says what the
/// value is. In a refusal (`if (cond) return 'message'`) the value passes only where `cond` is
/// false, so a predicate holds when it is negated an odd number of times; anywhere else (a
/// helper's `&&` chain, a ternary) it holds when negated an even number of times.
fn holds(s: &str, ctx: (usize, usize), start: usize, mut negated: usize) -> bool {
    let b = s.as_bytes();
    if s[..start].ends_with('!') {
        negated += 1;
    }
    let guard = word_hits(s, ctx.0, start, "if").into_iter().rev().find_map(|i| {
        let open = i + 2 + (s[i + 2..].len() - s[i + 2..].trim_start().len());
        (b.get(open) == Some(&b'(')).then_some(())?;
        let end = close(b, open)?;
        (end > start).then_some((open, end))
    });
    let mut refusal = false;
    if let Some((open, end)) = guard {
        let then = s[end..].trim_start();
        let then = then.strip_prefix('{').map_or(then, str::trim_start);
        refusal = then.strip_prefix("return").is_some_and(|r| r.trim_start().starts_with(['\'', '"', '`']));
        let mut k = open + 1;
        while let Some(p) = s[k..start].find("!(") {
            let j = k + p;
            if close(b, j + 1).is_some_and(|e| e > start) {
                negated += 1;
            }
            k = j + 2;
        }
    }
    (negated % 2 == 1) == refusal
}

/// The facts the predicates of body `ctx` state about expression `e`.
fn facts_of(m: &Module, ctx: (usize, usize), e: &str, depth: usize) -> Facts {
    let s = &m.text;
    let b = s.as_bytes();
    let mut f = Facts::default();
    for i in word_hits(s, ctx.0, ctx.1, e) {
        let mut after = &s[i + e.len()..ctx.1];
        after = after.strip_prefix('!').unwrap_or(after);
        if (after.starts_with('.') && !after.starts_with(".every(") && !after.starts_with(".length")) || after.starts_with('[') || after.starts_with("?.") {
            continue; // a key or an element of it, not it
        }
        let before = &s[ctx.0..i];
        let at = m.at(i);
        let hit = |f: &mut Facts, what: &str| {
            f.from.insert(format!("{at} {what}"));
        };
        for (call, t) in TYPE_CALLS {
            if before.ends_with(call) && !before[..before.len() - call.len()].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '.' || c == '_') && holds(s, ctx, i - call.len(), 0) {
                f.types.insert(t.to_string());
                hit(&mut f, &format!("{call}{e})"));
            }
        }
        if before.ends_with("typeof ") {
            let a = after.trim_start();
            let unequal = a.starts_with("!==");
            if let Some(r) = a.strip_prefix("===").or_else(|| a.strip_prefix("!==")).filter(|_| holds(s, ctx, i - 7, unequal as usize)) {
                if let Some(t) = string_at(r.trim_start(), 0).map(|x| x.0) {
                    let t = if t == "object" { "object".to_string() } else { t };
                    if ["string", "boolean", "number", "object"].contains(&t.as_str()) {
                        f.types.insert(t.clone());
                        hit(&mut f, &format!("typeof {e} '{t}'"));
                    }
                }
            }
        }
        if let Some(r) = after.strip_prefix(".every(").filter(|_| holds(s, ctx, i, 0)) {
            let arg_end = close(b, i + e.len() + after.len() - r.len() - 1).unwrap_or(ctx.1);
            let arg = &s[i + e.len() + after.len() - r.len()..arg_end.saturating_sub(1)];
            for (call, t) in TYPE_CALLS {
                let bare = call.trim_end_matches('(');
                if arg.trim() == bare || arg.contains(call) {
                    f.items.insert(t.to_string());
                    hit(&mut f, &format!("{e}.every({bare})"));
                }
            }
            if arg.contains("typeof") && arg.contains("'string'") {
                f.items.insert("string".into());
                hit(&mut f, &format!("{e}.every(string)"));
            }
            f.types.insert("array".into());
        }
        if let Some(r) = after.strip_prefix(".length") {
            let unequal = r.trim_start().starts_with("!==") as usize;
            if let Some(n) = r.trim_start().strip_prefix("===").or_else(|| r.trim_start().strip_prefix("!==")).filter(|_| holds(s, ctx, i, unequal)).and_then(|x| x.trim_start().split(|c: char| !c.is_ascii_digit()).next()).and_then(|x| x.parse().ok()) {
                f.length = Some(n);
                hit(&mut f, &format!("{e}.length === {n}"));
            }
        }
        // CONST.includes(e): an enum, when CONST is a literal list of the module or an import
        if let Some(pre) = before.strip_suffix(".includes(") {
            let name: String = pre.bytes().rev().take_while(|&c| is_ident(c)).map(char::from).collect::<String>().chars().rev().collect();
            if let Some(list) = const_list(m, &name).filter(|_| holds(s, ctx, i - ".includes(".len() - name.len(), 0)) {
                f.enumv = Some(list.0);
                f.from.insert(format!("{} {name}", list.1));
            }
        }
        // helper(e, ...): what the helper's own predicates state about its first parameter
        if before.ends_with('(') && depth < 3 {
            let name: String = before[..before.len() - 1].bytes().rev().take_while(|&c| is_ident(c)).map(char::from).collect::<String>().chars().rev().collect();
            let qualified = before[..before.len() - 1 - name.len()].ends_with('.');
            if !name.is_empty() && !qualified && !NOT_HELPERS.contains(&name.as_str()) && !TYPE_CALLS.iter().any(|(c, _)| c.trim_end_matches('(') == name) && holds(s, ctx, i - 1 - name.len(), 0) {
                if let Some((p, body, def)) = helper(m, ctx, &name) {
                    let sub = refine(m, body, &[Root { name: p, hidden: BTreeSet::new() }], None, depth + 1);
                    let mut g = Facts::default();
                    let mut props = Map::new();
                    for (k, kf) in &sub {
                        if k.is_empty() {
                            g.merge(kf.clone());
                        } else {
                            let mut ps = kf.schema();
                            ps["x-from"] = json!(kf.from);
                            props.insert(k.clone(), ps);
                        }
                    }
                    if !props.is_empty() {
                        g.types.insert("object".into());
                        g.nested = Some(json!({"properties": props}));
                    }
                    if !g.is_empty() {
                        g.from.insert(format!("{} {name}", m.at(def)));
                        f.merge(g);
                    }
                }
            }
        }
    }
    // for (const [i, x] of e.entries()) / for (const x of e): x is an item of e
    for i in word_hits(s, ctx.0, ctx.1, "for") {
        let Some(pc) = s[i..ctx.1].find('(').map(|p| p + i) else { continue };
        let Some(pe) = close(b, pc) else { continue };
        let head = &s[pc + 1..pe - 1];
        let Some((bind, over)) = head.split_once(" of ") else { continue };
        let over = over.trim();
        if over != e && over != format!("{e}.entries()") {
            continue;
        }
        let bind = bind.trim().trim_start_matches("const ").trim();
        let item = if over.ends_with(".entries()") { bind.trim_matches(['[', ']']).split(',').nth(1).unwrap_or("").trim() } else { bind };
        if item.is_empty() || !item.bytes().all(is_ident) {
            continue;
        }
        let body_at = s[pe..].find('{').map(|p| p + pe);
        let Some(end) = body_at.and_then(|o| close(b, o)) else { continue };
        let sub = refine(m, (pe, end), &[Root { name: item.to_string(), hidden: BTreeSet::new() }], None, depth + 1);
        let mut item_facts = Facts::default();
        let mut props = Map::new();
        for (k, kf) in &sub {
            if k.is_empty() {
                item_facts.merge(kf.clone());
            } else {
                let mut ps = kf.schema();
                ps["x-from"] = json!(kf.from);
                props.insert(k.clone(), ps);
            }
        }
        let mut item_schema = item_facts.schema();
        if !props.is_empty() {
            item_schema = merge_schema(item_schema, json!({"type": "object", "properties": props}));
        }
        if item_schema.as_object().is_some_and(|o| !o.is_empty()) {
            if !item_facts.from.is_empty() {
                item_schema["x-from"] = json!(item_facts.from);
            }
            f.types.insert("array".into());
            f.nested = Some(merge_schema(f.nested.take().unwrap_or_else(|| json!({})), json!({"items": item_schema})));
            f.from.insert(format!("{} for ... of {e}", m.at(i)));
        }
    }
    f
}

/// A `const NAME = [literals]` of `m` or an import: the literals and where they are declared.
fn const_list(m: &Module, name: &str) -> Option<(Value, String)> {
    if name.is_empty() {
        return None;
    }
    let (cm, i) = m.decl("const", name)?;
    let t = &cm.text;
    let eq = t[i..].find('=')? + i;
    let open = eq + 1 + (t[eq + 1..].len() - t[eq + 1..].trim_start().len());
    if t.as_bytes().get(open) != Some(&b'[') {
        return None;
    }
    let end = close(t.as_bytes(), open)?;
    let lits = split_top(t, open + 1, end - 1, b',').iter().map(|(a, z)| literal(&t[*a..*z])).collect::<Option<Vec<_>>>()?;
    Some((json!(lits), cm.at(i)))
}

// ---------------------------------------------------------------------------------------------
// Outputs: the object a solve returns, or a grid kernel's result interface.
// ---------------------------------------------------------------------------------------------

/// The return type annotation of function `name` of `m` or an import.
fn return_type<'a>(m: &'a Module, name: &str) -> Option<(&'a Module, String, usize)> {
    let (fm, i) = m.decl("function", name)?;
    let t = &fm.text;
    let open = t[i..].find('(')? + i;
    let end = close(t.as_bytes(), open)?;
    let rest = t[end..].trim_start().strip_prefix(':')?;
    let body = rest.find('{')?;
    Some((fm, rest[..body].trim().to_string(), i))
}

/// `id` when the body binds it as `const id = f(...)` and f is declared `): T` (or `): T | string`,
/// a refusal) with T an interface: T's module, name, members, and the evidence.
fn typed_local<'a>(m: &'a Module, ctx: (usize, usize), id: &str) -> Option<(&'a Module, String, Vec<(String, bool, String, Option<String>, usize)>, String)> {
    if id.is_empty() || !id.bytes().all(is_ident) {
        return None;
    }
    let decl = format!("const {id} = ");
    let p = m.text[ctx.0..ctx.1].find(&decl)? + ctx.0 + decl.len();
    let f: String = m.text[p..].bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
    let (fm, ty, at) = return_type(m, &f)?;
    let ty = ty.trim_end_matches("| string").trim().to_string();
    let (im, ms) = interface(fm, &ty)?;
    Some((im, ty.clone(), ms, format!("{f}(): {ty} at {}", fm.at(at))))
}

/// The names a schema's `type` holds: one, a list, or none.
fn type_names(t: &Value) -> Vec<String> {
    match t {
        Value::String(s) => vec![s.clone()],
        Value::Array(xs) => xs.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => vec![],
    }
}

/// The JSON Schema of one returned value, where its text states one.
fn value_schema(m: &Module, ctx: (usize, usize), v: &str, input: &BTreeMap<String, Value>) -> (Value, Option<String>) {
    let v = v.trim();
    let b = v.as_bytes();
    // Number.isFinite(x) ? x : null
    if let Some(r) = v.strip_prefix("Number.isFinite(") {
        if let Some((x, rest)) = r.split_once(')') {
            if rest.trim() == format!("? {x} : null") {
                return (json!({"type": ["number", "null"]}), Some(format!("Number.isFinite({x}) ? {x} : null")));
            }
        }
    }
    let qs = split_top(v, 0, v.len(), b'?');
    if qs.len() == 2 {
        let arms = split_top(v, qs[1].0, qs[1].1, b':');
        if arms.len() == 2 {
            let (a, wa) = value_schema(m, ctx, &v[arms[0].0..arms[0].1], input);
            let (z, wz) = value_schema(m, ctx, &v[arms[1].0..arms[1].1], input);
            if a.get("type").is_some() && z.get("type").is_some() {
                let types: BTreeSet<String> = type_names(&a["type"]).into_iter().chain(type_names(&z["type"])).collect();
                let why = [wa, wz].into_iter().flatten().collect::<Vec<_>>().join("; ");
                return (if types.len() == 1 { json!({"type": types.iter().next()}) } else { json!({"type": types}) }, Some(why));
            }
            return (json!({}), None);
        }
    }
    if let Some(l) = literal(v) {
        let t = match l {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            _ => "array",
        };
        return (json!({"type": t}), Some("literal".into()));
    }
    if b.first() == Some(&b'`') {
        return (json!({"type": "string"}), Some("template literal".into()));
    }
    if b.first() == Some(&b'[') && close(b, 0) == Some(v.len()) {
        return (json!({"type": "array"}), Some("array literal".into()));
    }
    if b.first() == Some(&b'{') && close(b, 0) == Some(v.len()) {
        let o = object_literal(m, ctx, v, input);
        return (o, Some("object literal".into()));
    }
    for (pre, t) in [("String(", "string"), ("Number(", "number"), ("Boolean(", "boolean"), ("Math.", "number")] {
        if v.starts_with(pre) {
            return (json!({"type": t}), Some(pre.trim_end_matches('(').to_string()));
        }
    }
    if [".map(", ".filter(", ".slice("].iter().any(|p| v.contains(p) && close(b, v.rfind(p).unwrap() + p.len() - 1) == Some(v.len())) || v.starts_with("Array.from(") {
        return (json!({"type": "array"}), Some("array method".into()));
    }
    if v.ends_with(".length") {
        return (json!({"type": "integer"}), Some("length".into()));
    }
    // id.field where `const id = f(...)` and f is declared `): T`, T an interface with field
    if let Some((id, field)) = v.split_once('.') {
        if id.bytes().all(is_ident) && field.bytes().all(is_ident) && !field.is_empty() {
            let decl = format!("const {id} = ");
            if let Some(p) = m.text[ctx.0..ctx.1].find(&decl) {
                let rhs = &m.text[ctx.0 + p + decl.len()..];
                let f: String = rhs.bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
                if let Some((fm, ty, at)) = return_type(m, &f) {
                    let ty = ty.trim_end_matches("| string").trim().to_string();
                    if let Some((im, ms)) = interface(fm, &ty) {
                        if let Some(mem) = ms.iter().find(|x| x.0 == field) {
                            return (ts_schema(im, &mem.2), Some(format!("{f}(): {ty} at {}, {ty}.{field} at {}", fm.at(at), im.at(mem.4))));
                        }
                    }
                }
            }
        }
    }
    // f(...) where f is declared `): T`
    let f: String = v.bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
    if !f.is_empty() && v[f.len()..].starts_with('(') && close(b, f.len()) == Some(v.len()) {
        if let Some((fm, ty, at)) = return_type(m, &f) {
            let s = ts_schema(fm, &ty);
            if s.get("type").is_some() || s.get("enum").is_some() {
                return (s, Some(format!("{f}(): {ty} at {}", fm.at(at))));
            }
        }
    }
    // an input key echoed, bare or read off the value the solve was handed
    let echoed = v.split_once('.').filter(|(id, k)| id.bytes().all(is_ident) && input.contains_key(*k) && !m.text[ctx.0..ctx.1].contains(&format!("const {id} "))).map_or(v, |(_, k)| k);
    if let Some(s) = input.get(echoed) {
        let mut s = s.clone();
        if let Some(o) = s.as_object_mut() {
            o.retain(|k, _| k == "type" || k == "items" || k == "enum");
        }
        return (s, Some(format!("echoes input {echoed}")));
    }
    (json!({}), None)
}

/// The schema of an object literal `lit` (text with its braces), keys conditional on a spread
/// `...(c ? {...} : {})` optional.
fn object_literal(m: &Module, ctx: (usize, usize), lit: &str, input: &BTreeMap<String, Value>) -> Value {
    let mut props = Map::new();
    let mut req = vec![];
    if !lit.starts_with('{') {
        return json!({});
    }
    for (a, z) in split_top(lit, 1, lit.len() - 1, b',') {
        let ent = &lit[a..z];
        if let Some(sp) = ent.strip_prefix("...") {
            let sp = sp.trim();
            let inner = sp.strip_prefix('(').and_then(|x| x.strip_suffix(')')).unwrap_or(sp);
            let qs = split_top(inner, 0, inner.len(), b'?');
            if qs.len() == 2 {
                let arm = split_top(inner, qs[1].0, qs[1].1, b':');
                let o = arm.first().map_or(Value::Null, |&(x, y)| object_literal(m, ctx, &inner[x..y], input));
                for (k, v) in o["properties"].as_object().into_iter().flatten() {
                    let mut v = v.clone();
                    v["x-conditional"] = json!(true);
                    props.insert(k.clone(), v);
                }
            } else if let Some((tm, ty, ms, why)) = typed_local(m, ctx, inner) {
                // ...r where `const r = f(...)` and f is declared `): T | string`: T's members
                for mem in &ms {
                    let mut v = ts_schema(tm, &mem.2);
                    if let Some(d) = &mem.3 {
                        v["description"] = json!(d);
                    }
                    v["x-from"] = json!([format!("{} {why}, {ty}.{} at {}", m.at_slice(ent), mem.0, tm.at(mem.4))]);
                    props.insert(mem.0.clone(), v);
                    if !mem.1 {
                        req.push(mem.0.clone());
                    }
                }
            }
            continue;
        }
        let (k, val) = match split_first(ent, b':') {
            Some((k, v)) => (k.trim_matches(['\'', '"']).to_string(), Some(v)),
            None if ent.bytes().all(is_ident) => (ent.to_string(), None),
            None => continue,
        };
        if !k.bytes().all(is_ident) || val == Some("undefined") {
            continue; // JSON drops an undefined value
        }
        let (mut s, why) = match val {
            Some(v) => value_schema(m, ctx, v, input),
            None => {
                // shorthand: a local whose initializer states a type, or an input key echoed
                let local = word_hits(&m.text, ctx.0, ctx.1, &format!("const {k}")).into_iter().find_map(|p| {
                    let rest = &m.text[p + 6 + k.len()..ctx.1];
                    let (ann, eq) = match rest.trim_start().strip_prefix(':') {
                        Some(r) => (r.split('=').next().map(str::trim), r.find('=').map(|q| q + rest.len() - r.len())),
                        None => (None, rest.trim_start().starts_with('=').then(|| rest.len() - rest.trim_start().len())),
                    };
                    let at = p + 6 + k.len() + eq? + 1;
                    let end = statement_end(&m.text, at, ctx.1);
                    let (v, w) = value_schema(m, ctx, &m.text[at..end], input);
                    match ann {
                        Some(t) if v.get("type").is_none() => Some((ts_schema(m, t), Some(format!("const {k}: {t}")))),
                        _ => Some((v, w)),
                    }
                });
                match local {
                    Some(l) => l,
                    None if input.contains_key(&k) => value_schema(m, ctx, &k, input),
                    None => (json!({}), None),
                }
            }
        };
        let mut from = vec![m.at_slice(ent)];
        if let Some(w) = why {
            from[0] = format!("{} {w}", from[0]);
        }
        s["x-from"] = json!(from);
        props.insert(k.clone(), s);
        req.push(k);
    }
    json!({"type": "object", "required": req, "properties": props})
}

/// The schema of what the body `ctx` (a solve) returns last: a literal, or a local object
/// literal with the keys later assigned to it.
fn returned(m: &Module, ctx: (usize, usize), input: &BTreeMap<String, Value>) -> Option<Value> {
    let s = &m.text;
    let b = s.as_bytes();
    // the last `return` at the body's own depth
    let (mut i, mut depth, mut last) = (ctx.0 + 1, 0i32, None);
    while i < ctx.1 - 1 {
        if let Some(j) = skip(b, i) {
            i = j;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'r' if depth == 0 && s[i..].starts_with("return") && !is_ident(b[i - 1]) && !is_ident(b[i + 6]) => last = Some(i),
            _ => {}
        }
        i += 1;
    }
    let r = last? + 6;
    let r = r + (s[r..].len() - s[r..].trim_start().len());
    let r_end = statement_end(s, r, ctx.1);
    // `c ? refusal : record`: a data form answers a string or its record; the record is the output
    let qs = split_top(s, r, r_end, b'?');
    let arms: Vec<usize> = if qs.len() == 2 { split_top(s, qs[1].0, qs[1].1, b':').iter().map(|x| x.0).collect() } else { vec![r] };
    let mut local = None;
    for a in arms {
        if b[a] == b'{' {
            let end = close(b, a)?;
            return Some(object_literal(m, ctx, &s[a..end], input));
        }
        let id: String = s[a..].bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
        let Some(decl) = word_hits(s, ctx.0, ctx.1, &format!("const {id}")).into_iter().next().filter(|_| !id.is_empty()) else { continue };
        let open = s[decl..].find('=')? + decl + 1;
        let open = open + (s[open..].len() - s[open..].trim_start().len());
        if b[open] == b'{' {
            local = Some((id, open));
            break;
        }
    }
    let (id, open) = local?;
    let end = close(b, open)?;
    let mut o = object_literal(m, ctx, &s[open..end], input);
    for at in word_hits(s, end, ctx.1, &format!("{id}.")).into_iter().chain(word_hits(s, end, ctx.1, id.as_str()).into_iter().filter(|&x| s[x + id.len()..].starts_with('.'))) {
        let rest = &s[at + id.len() + 1..ctx.1];
        let k: String = rest.bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
        let after = rest[k.len()..].trim_start();
        if k.is_empty() || !after.starts_with('=') || after.starts_with("==") || o["properties"].get(&k).is_some() {
            continue;
        }
        let v_at = at + id.len() + 1 + k.len() + (rest[k.len()..].len() - after.len()) + 1;
        let v_end = split_top(s, v_at, ctx.1, b';').first().map_or(v_at, |x| x.1);
        let (mut vs, why) = value_schema(m, ctx, &s[v_at..v_end], input);
        vs["x-conditional"] = json!(true);
        vs["x-from"] = json!([format!("{}{}", m.at(at), why.map(|w| format!(" {w}")).unwrap_or_default())]);
        o["properties"][k.as_str()] = vs;
    }
    Some(o)
}

// ---------------------------------------------------------------------------------------------
// What the reference tests exercise (test/flow-cli.test.ts, which test/flow-obligations.test.ts
// requires to run every data form).
// ---------------------------------------------------------------------------------------------

/// The reference test file of the data forms.
pub const REFERENCE_TESTS: &str = "test/flow-cli.test.ts";

#[derive(Default, Debug)]
struct Exercised {
    tests: BTreeSet<String>,
    inputs: BTreeMap<String, BTreeSet<String>>,
    outputs: BTreeMap<String, (BTreeSet<String>, Value)>,
}

fn literal_keys(t: &str) -> Vec<(String, String)> {
    let t = t.trim();
    if !t.starts_with('{') || close(t.as_bytes(), 0) != Some(t.len()) {
        return vec![];
    }
    split_top(t, 1, t.len() - 1, b',')
        .into_iter()
        .map(|(a, z)| {
            let e = &t[a..z];
            let (k, v) = split_first(e, b':').unwrap_or((e, ""));
            (k.to_string(), v.to_string())
        })
        .filter(|(k, _)| !k.is_empty() && k.bytes().all(is_ident))
        .collect()
}

/// The keys of an expected answer `lit` a test compares with, and the JSON type of each literal.
fn read_literal(e: &mut Exercised, lit: &str, label: &str) {
    for (k, v) in literal_keys(lit) {
        let o = e.outputs.entry(k).or_insert_with(|| (BTreeSet::new(), json!({})));
        o.0.insert(label.to_string());
        let t = match literal(&v) {
            Some(Value::String(_)) => "string",
            Some(Value::Number(_)) => "number",
            Some(Value::Bool(_)) => "boolean",
            _ if v.trim_start().starts_with('[') => "array",
            _ => continue,
        };
        if o.1.get("type").is_none() {
            o.1 = json!({"type": t});
        }
    }
}

/// What the reference tests run each kernel on and read from its answer.
fn exercised(m: &Module, ids: &BTreeSet<String>) -> BTreeMap<String, Exercised> {
    let s = &m.text;
    let b = s.as_bytes();
    let mut out: BTreeMap<String, Exercised> = BTreeMap::new();
    for it in word_hits(s, 0, s.len(), "it") {
        if b.get(it + 2) != Some(&b'(') {
            continue;
        }
        let Some((name, _)) = string_at(s, it + 3) else { continue };
        let Some(end) = close(b, it + 2) else { continue };
        let block = (it, end);
        let label = format!("{} {name}", m.at(it));
        for id in ids {
            let pat = format!("('{id}'");
            let mut k = block.0;
            while let Some(p) = s[k..block.1].find(&pat) {
                let call = k + p;
                k = call + pat.len();
                let f: String = s[..call].bytes().rev().take_while(|&c| is_ident(c)).map(char::from).collect::<String>().chars().rev().collect();
                let runs = f == "runKernel" || helper(m, (0, s.len()), &f).is_some_and(|(_, body, _)| s[body.0..body.1].contains("runKernel("));
                if !runs {
                    continue; // not a run of the kernel (a list lookup, a name in a message)
                }
                let args_end = close(b, call).unwrap_or(block.1);
                let stmt_end = statement_end(s, call, block.1);
                let stmt = &s[call..stmt_end];
                if stmt.contains("ok: false") {
                    continue; // a refusal: the answer is not the kernel's result
                }
                // the call's data: an object literal, or JSON.stringify of one, with one spread followed
                let args = split_top(s, call + 1, args_end - 1, b',');
                let e = out.entry(id.clone()).or_default();
                if let Some(&(a, z)) = args.get(1) {
                    let mut data = s[a..z].trim();
                    data = data.strip_prefix("JSON.stringify(").and_then(|x| x.strip_suffix(')')).unwrap_or(data);
                    if !data.starts_with('{') {
                        if let Some(p) = s[block.0..call].rfind(&format!("const {data} = ")) {
                            let at = block.0 + p + "const ".len() + data.len() + " = ".len();
                            if b[at] == b'{' {
                                data = &s[at..close(b, at).unwrap_or(at)];
                            }
                        }
                    }
                    for (key, val) in literal_keys(data) {
                        e.inputs.entry(key).or_default().insert(label.clone());
                        let _ = val;
                    }
                    for (a2, z2) in split_top(data, 1, data.len().saturating_sub(1), b',') {
                        if let Some(sp) = data[a2..z2].strip_prefix("...") {
                            if let Some(p) = s[block.0..call].rfind(&format!("const {} = ", sp.trim())) {
                                let at = block.0 + p + "const ".len() + sp.trim().len() + " = ".len();
                                if b[at] == b'{' {
                                    for (key, _) in literal_keys(&s[at..close(b, at).unwrap_or(at)]) {
                                        e.inputs.entry(key).or_default().insert(label.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                e.tests.insert(label.clone());
                // the answer: a result directly (any helper but runKernel answers its result)
                // or an envelope whose `.result` is cast
                let mut results: Vec<String> = vec![];
                let head = &s[..call];
                let decl = head.rfind("const ").filter(|&d| !s[d..call].contains(';') && !s[d..call].contains('\n'));
                if let Some(d) = decl {
                    let v: String = s[d + 6..].bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
                    if f == "runKernel" {
                        for p in word_hits(s, call, block.1, &format!("{v}.result as")) {
                            let r_decl = s[..p].rfind("const ").unwrap_or(p);
                            let w: String = s[r_decl + 6..].bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
                            let ty_at = p + v.len() + 11;
                            let ty_end = split_top(s, ty_at, block.1, b';').first().map_or(ty_at, |x| x.1);
                            let ty = ts_schema(m, &s[ty_at..ty_end]);
                            for (k2, v2) in ty["properties"].as_object().into_iter().flatten() {
                                let o = e.outputs.entry(k2.clone()).or_insert_with(|| (BTreeSet::new(), json!({})));
                                o.0.insert(label.clone());
                                if v2.get("type").is_some() {
                                    o.1 = v2.clone();
                                }
                            }
                            results.push(w);
                        }
                    } else {
                        results.push(v);
                    }
                }
                // expect(<this call>).toMatchObject({...}) / .toEqual({...})
                let after = &s[args_end..stmt_end];
                for m2 in [").toMatchObject(", ").toEqual("] {
                    if let Some(p) = after.find(m2) {
                        let at = args_end + p + m2.len();
                        if b.get(at) == Some(&b'{') {
                            read_literal(e, &s[at..close(b, at).unwrap_or(at)], &label);
                        }
                    }
                }
                for r in results {
                    for p in word_hits(s, call, block.1, &r) {
                        let rest = &s[p + r.len()..block.1];
                        if let Some(x) = rest.strip_prefix('.') {
                            let k2: String = x.bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
                            if k2.is_empty() || k2 == "result" {
                                continue;
                            }
                            let o = e.outputs.entry(k2.clone()).or_insert_with(|| (BTreeSet::new(), json!({})));
                            o.0.insert(label.clone());
                            let cast = x[k2.len()..].trim_start();
                            if let Some(c) = cast.strip_prefix("as ") {
                                let ty: String = c.chars().take_while(|&ch| ch != ')' && ch != ',' && ch != ';').collect();
                                let t = ts_schema(m, ty.trim());
                                if t.get("type").is_some() && o.1.get("type").is_none() {
                                    o.1 = t;
                                }
                            }
                        } else if rest.trim_start().starts_with(").toMatchObject({") && s[..p].ends_with("expect(") {
                            let at = p + r.len() + rest.find('{').unwrap_or(0);
                            read_literal(e, &s[at..close(b, at).unwrap_or(at)], &label);
                        }
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// One contract per kernel.
// ---------------------------------------------------------------------------------------------

/// The input schema of a shape text, refined by the facts of the parser's body.
fn input_schema(m: &Module, at: usize, text: &str, facts: &BTreeMap<String, Facts>, accepted: Option<(Vec<String>, String)>) -> Value {
    let declared = m.at(at);
    let (keys, tail) = shape_keys(text).unwrap_or_default();
    let mut s = shape_object(&keys);
    s["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    s["description"] = json!(text);
    if !tail.is_empty() {
        s["x-note"] = json!(tail);
    }
    s["x-from"] = json!([format!("{declared} input")]);
    for k in &keys {
        let p = &mut s["properties"][k.name.as_str()];
        let mut from = vec![format!("{declared} declares {}{}", k.name, if k.optional { "?" } else { "" })];
        if let Some(f) = facts.get(&k.name) {
            let merged = merge_schema(p.take(), f.schema());
            *p = merged;
            from.extend(f.from.iter().cloned());
        }
        p["x-from"] = json!(from);
    }
    if let Some((list, where_)) = accepted {
        s["additionalProperties"] = json!(false);
        s["x-from"].as_array_mut().unwrap().push(json!(format!("{where_} unknownKeys refuses every other key")));
        for k in list {
            if s["properties"].get(&k).is_none() {
                s["properties"][k.as_str()] = json!({"x-from": [format!("{where_} accepts {k}")]});
            }
        }
    }
    s
}

/// The keys `unknownKeys(root, [...])` accepts in body `ctx`, and where.
fn accepted_keys(m: &Module, ctx: (usize, usize), root: &str) -> Option<(Vec<String>, String)> {
    let pat = format!("unknownKeys({root}, [");
    let p = m.text[ctx.0..ctx.1].find(&pat)? + ctx.0;
    let open = p + pat.len() - 1;
    let end = close(m.text.as_bytes(), open)?;
    let list = split_top(&m.text, open + 1, end - 1, b',').iter().map(|(a, z)| literal(&m.text[*a..*z]).and_then(|v| v.as_str().map(str::to_string))).collect::<Option<Vec<_>>>()?;
    Some((list, m.at(p)))
}

/// The parameter name and body of method `name(` inside `s[from..to]`.
fn method(s: &str, from: usize, to: usize, name: &str) -> Option<(String, (usize, usize))> {
    let p = word_hits(s, from, to, name).into_iter().find(|&i| s.as_bytes().get(i + name.len()) == Some(&b'('))?;
    let open = p + name.len();
    let end = close(s.as_bytes(), open)?;
    let param: String = s[open + 1..end - 1].trim().bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
    let body = s[end..].find('{')? + end;
    Some((param, (body, close(s.as_bytes(), body)?)))
}

fn finish(mut c: Value, exercised: Option<&Exercised>) -> Value {
    if let Some(x) = exercised {
        c["tests"] = json!(x.tests);
        for (k, tests) in &x.inputs {
            if let Some(p) = c["input"]["properties"].get_mut(k) {
                p["x-exercised"] = json!(tests);
            }
        }
        if c["output"].is_object() {
            for (k, (tests, ty)) in &x.outputs {
                let p = &mut c["output"]["properties"][k.as_str()];
                if p.is_null() {
                    *p = json!({});
                }
                if p.get("type").is_none() {
                    if let Some(t) = ty.get("type") {
                        p["type"] = t.clone();
                        p["x-type-from"] = json!("the test's own reading of the answer");
                    }
                }
                p["x-exercised"] = json!(tests);
            }
        }
    }
    c
}

fn base(id: &str, engine: &str, title: Option<&str>, m: &Module) -> Value {
    let absent: Map<String, Value> = ABSENT.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    json!({"schema": CONTRACT_SCHEMA, "id": id, "engine": engine, "title": title, "version": null, "exact": null,
        "module": {"path": m.path, "blob": m.blob}, "absent": absent})
}

/// The contract of a kernel with a data form (src/core/solve.ts): its `input` text and `solve`.
fn data_form_contract(id: &str, engine: &str, title: Option<&str>, m: &Module) -> Result<Value, String> {
    let s = &m.text;
    let decl = s.find("export const data: DataForm = {").ok_or_else(|| format!("{}: no `export const data: DataForm`", m.path))?;
    let open = decl + s[decl..].find('{').unwrap();
    let end = close(s.as_bytes(), open).ok_or_else(|| format!("{}: the data form does not close", m.path))?;
    let inp = word_hits(s, open, end, "input").into_iter().find(|&i| s[i + 5..].starts_with(':')).ok_or_else(|| format!("{}: the data form has no input", m.path))?;
    let q = inp + 6 + (s[inp + 6..].len() - s[inp + 6..].trim_start().len());
    let (text, _) = string_at(s, q).ok_or_else(|| format!("{}: the data form's input is not a string literal", m.path))?;
    let (param, body) = method(s, open, end, "solve").ok_or_else(|| format!("{}: the data form has no solve", m.path))?;
    let declared: Vec<String> = shape_keys(&text).map(|(k, _)| k.into_iter().map(|k| k.name).collect()).unwrap_or_default();
    let facts = refine(m, body, &[Root { name: param.clone(), hidden: BTreeSet::new() }], Some(&declared), 0);
    let input = input_schema(m, inp, &text, &facts, accepted_keys(m, body, &param));
    let echo: BTreeMap<String, Value> = input["properties"].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), v.clone())).collect();
    let output = returned(m, body, &echo).map(|mut o| {
        o["x-from"] = json!([format!("{} solve returns", m.at(body.0))]);
        o
    });
    let mut c = base(id, engine, title, m);
    c["input"] = input;
    c["output"] = output.unwrap_or(Value::Null);
    Ok(c)
}

/// The contract of a grid kernel: `GridKernel<Data, Result>`, its interfaces, and its `fromData`.
fn grid_contract(id: &str, title: Option<&str>, m: &Module) -> Result<Value, String> {
    let s = &m.text;
    let at = s.find("export const kernel: GridKernel<").ok_or_else(|| format!("{}: no `export const kernel: GridKernel<...>`", m.path))?;
    let args_at = at + "export const kernel: GridKernel<".len();
    let args_end = s[args_at..].find('>').ok_or_else(|| format!("{}: GridKernel<...> does not close", m.path))? + args_at;
    let args: Vec<&str> = s[args_at..args_end].split(',').map(str::trim).collect();
    let (data, result) = (args.first().copied().unwrap_or(""), args.get(1).copied().unwrap_or(""));
    let mut c = base(id, "grid", title, m);
    let Some((im, ms)) = interface(m, data) else {
        return Err(format!("{}: the data interface {data} is not declared", m.path));
    };
    let mut input = object_type(im, &ms);
    input["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    input["x-from"] = json!([format!("{} GridKernel<{data}, {result}>", m.at(at)), format!("{} interface {data}", im.at(ms.first().map_or(0, |x| x.4)))]);
    let declared: Vec<String> = ms.iter().map(|x| x.0.clone()).collect();
    if let Some((param, body)) = method(s, args_end, s.len(), "fromData") {
        let facts = refine(m, body, &[Root { name: param, hidden: BTreeSet::new() }], Some(&declared), 0);
        for mem in &ms {
            let p = &mut input["properties"][mem.0.as_str()];
            let mut from = vec![format!("{} {data}.{}{}", im.at(mem.4), mem.0, if mem.1 { "?" } else { "" })];
            if let Some(f) = facts.get(&mem.0) {
                let merged = merge_schema(p.take(), f.schema());
                *p = merged;
                from.extend(f.from.iter().cloned());
            }
            p["x-from"] = json!(from);
        }
    }
    c["input"] = input;
    c["output"] = match interface(m, result) {
        Some((rm, rs)) => {
            let mut o = object_type(rm, &rs);
            for mem in &rs {
                o["properties"][mem.0.as_str()]["x-from"] = json!([format!("{} {result}.{}", rm.at(mem.4), mem.0)]);
            }
            o["x-from"] = json!([format!("{} GridKernel<{data}, {result}>", m.at(at))]);
            o
        }
        None => Value::Null,
    };
    Ok(c)
}

/// The contract of a graph kernel: the flow's shared graph form (src/flow/cli.ts graphFromInput),
/// its `params` the kernel's own as the runner lists them.
fn graph_contract(id: &str, title: Option<&str>, kernel: &Value, m: &Module, cli: &Module) -> Result<Value, String> {
    let s = &cli.text;
    let f = s.find("export function graphFromInput(").ok_or_else(|| format!("{}: no graphFromInput", cli.path))?;
    let doc_at = s[..f].rfind("/**").ok_or_else(|| format!("{}: graphFromInput has no doc comment", cli.path))?;
    let doc: String = s[doc_at..f].lines().map(|l| l.trim().trim_start_matches("/**").trim_start_matches('*').trim()).collect::<Vec<_>>().join(" ");
    let shape = doc.split('`').nth(1).ok_or_else(|| format!("{}: graphFromInput's doc states no `shape`", cli.path))?.to_string();
    let open = f + s[f..].find('(').unwrap();
    let pend = close(s.as_bytes(), open).unwrap_or(open);
    let param: String = s[open + 1..].bytes().take_while(|&c| is_ident(c)).map(char::from).collect();
    let body_at = body_brace(s, pend).ok_or_else(|| format!("{}: graphFromInput has no body", cli.path))?;
    let body = (body_at, close(s.as_bytes(), body_at).unwrap_or(s.len()));
    let declared: Vec<String> = shape_keys(&shape).map(|(k, _)| k.into_iter().map(|k| k.name).collect()).unwrap_or_default();
    let facts = refine(cli, body, &[Root { name: param, hidden: BTreeSet::new() }], Some(&declared), 0);
    let mut input = input_schema(cli, doc_at, &shape, &facts, None);
    let params: Map<String, Value> = kernel["params"].as_array().into_iter().flatten().filter_map(|p| {
        let k = p["key"].as_str()?;
        let mut s = json!({"default": p["value"], "x-from": [format!("{CATALOGUE} {id}.params")]});
        if let Some(t) = match &p["value"] {
            Value::Number(n) if n.is_i64() => Some("number"),
            Value::Number(_) => Some("number"),
            Value::Bool(_) => Some("boolean"),
            Value::String(_) => Some("string"),
            _ => None,
        } {
            s["type"] = json!(t);
        }
        Some((k.to_string(), s))
    }).collect();
    if input["properties"].get("params").is_some() {
        input["properties"]["params"]["properties"] = Value::Object(params);
    }
    let mut c = base(id, "graph", title, m);
    c["graph"] = json!({"goal": kernel["goal"], "shape": kernel["shape"]});
    c["input"] = input;
    c["output"] = Value::Null;
    c["absent"]["output"] = json!("a graph kernel's result is its own; the flow's graph form states nothing of it, and no result type is declared for it");
    Ok(c)
}

/// The module of a kernel, by AlgoGlyph's own conventions: an engine kernel is
/// src/kernels/<engine>/<id>.ts (src/registry.ts), a custom plate's data form
/// src/plates/<id>/data.ts (src/flow/cli.ts).
pub fn module_path(id: &str, engine: &str) -> String {
    if engine == "custom" {
        format!("src/plates/{id}/data.ts")
    } else {
        format!("src/kernels/{engine}/{id}.ts")
    }
}

/// Where the registry states the module convention `module_path` follows; derivation fails when
/// the registry no longer states it.
fn convention(tree: &dyn Tree) -> Result<Value, String> {
    let mut out = vec![];
    for (file, pat) in [("src/registry.ts", "src/kernels/<engine>/<id>.ts"), ("src/flow/cli.ts", "src/plates/<id>/data.ts")] {
        let text = tree.read(file).ok_or_else(|| format!("{file} is missing"))?;
        let i = text.find(pat).ok_or_else(|| format!("{file} no longer states {pat}: the module convention changed"))?;
        out.push(json!(format!("{file}:{} {pat}", line_of(&text, i))));
    }
    Ok(json!(out))
}

/// Every contract the source at `tree` gives for the kernels of the runner's answer `catalogue`
/// (`{commit, kernels}`, the fixture), as the artifact.
pub fn derive(tree: &dyn Tree, catalogue: &Value) -> Result<Value, String> {
    let commit = catalogue["commit"].as_str().ok_or("the catalogue names no commit")?;
    if commit != tree.commit() {
        return Err(format!("the catalogue ({CATALOGUE}) is of AlgoGlyph {commit}, the source of {}: refresh the catalogue first", tree.commit()));
    }
    let kernels = catalogue["kernels"].as_array().ok_or("the catalogue lists no kernels")?;
    let ids: BTreeSet<String> = kernels.iter().filter_map(|k| k["id"].as_str().map(str::to_string)).collect();
    let tests = Module::load(tree, REFERENCE_TESTS, 0).map(|t| exercised(&t, &ids)).unwrap_or_default();
    let cli = Module::load(tree, "src/flow/cli.ts", 0).ok_or("src/flow/cli.ts is missing")?;
    let mut contracts = vec![];
    for k in kernels {
        let (Some(id), Some(engine)) = (k["id"].as_str(), k["engine"].as_str()) else { continue };
        let title = k["title"].as_str();
        let path = module_path(id, engine);
        let m = Module::load(tree, &path, 1).ok_or_else(|| format!("{id}: its module {path} is missing at {commit}"))?;
        let c = match engine {
            "grid" => grid_contract(id, title, &m)?,
            "graph" => graph_contract(id, title, k, &m, &cli)?,
            _ => data_form_contract(id, engine, title, &m)?,
        };
        contracts.push(finish(c, tests.get(id)));
    }
    contracts.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    Ok(json!({
        "schema": ARTIFACT_SCHEMA,
        "description": format!("RESEARCH-ONLY. GENERATED by `algoglyph-contracts` (tools/research/mcp-rhai/src/algoglyph_contracts.rs, I0228) from the AlgoGlyph source at the commit below; do not edit. Each kernel's contract in the shape research/battlebrain/algoglyph-contract-proposal.md proposes ({CONTRACT_SCHEMA}), with only what the source states: every field names the line or test it comes from (x-from, x-exercised), and what no source states is listed in absent."),
        "source": {"repository": "algoglyph", "commit": commit, "catalogue": CATALOGUE, "reference_tests": REFERENCE_TESTS, "module_convention": convention(tree)?},
        "contracts": contracts,
    }))
}

/// The artifact as committed: pretty JSON with a final newline.
pub fn render(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

/// The AlgoGlyph root: `--algoglyph`, else `$ALGOGLYPH_ROOT`, else the submodule `.gitmodules`
/// declares for AlgoGlyph, else the transitional checkout under $HOME.
pub fn source_root(flag: Option<&str>, repo: &Path, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(f) = flag {
        return PathBuf::from(f);
    }
    if let Some(e) = env("ALGOGLYPH_ROOT").filter(|e| !e.is_empty()) {
        return PathBuf::from(e);
    }
    if let Ok(gm) = std::fs::read_to_string(repo.join(".gitmodules")) {
        let mut path = None;
        for l in gm.lines().map(str::trim) {
            if l.starts_with('[') {
                path = None;
            } else if let Some(p) = l.strip_prefix("path").map(|x| x.trim_start_matches([' ', '='])) {
                path = Some(p.trim().to_string());
            } else if l.starts_with("url") && l.trim_end_matches(".git").ends_with("algoglyph") {
                if let Some(p) = &path {
                    return repo.join(p);
                }
            }
        }
    }
    PathBuf::from(env("HOME").unwrap_or_default()).join(TRANSITIONAL_ROOT)
}

/// The command line: `algoglyph-contracts [--algoglyph DIR] [--check] [--source-optional]`.
/// Writes (or, with --check, compares) the artifact; answers the exit code.
pub fn cli(args: &[String], repo: &Path, env: &dyn Fn(&str) -> Option<String>, out: &mut dyn std::io::Write) -> i32 {
    let mut flag = None;
    let (mut check, mut optional) = (false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--algoglyph" => match it.next() {
                Some(d) => flag = Some(d.clone()),
                None => {
                    let _ = writeln!(out, "algoglyph-contracts: --algoglyph needs a directory");
                    return 2;
                }
            },
            "--check" => check = true,
            "--source-optional" => optional = true,
            "-h" | "--help" => {
                let _ = writeln!(out, "usage: algoglyph-contracts [--algoglyph DIR] [--check] [--source-optional]\n  writes {ARTIFACT} from the AlgoGlyph source (DIR, else $ALGOGLYPH_ROOT, else the algoglyph submodule, else ~/{TRANSITIONAL_ROOT});\n  --check fails (exit 1) when the committed artifact differs from what the source derives;\n  --source-optional reports NOT RUN and exits 0 when no source is found, instead of exit 3");
                return 0;
            }
            other => {
                let _ = writeln!(out, "algoglyph-contracts: unknown argument {other:?} (see --help)");
                return 2;
            }
        }
    }
    let root = source_root(flag.as_deref(), repo, env);
    let tree = match GitTree::open(&root) {
        Ok(t) => t,
        Err(e) => {
            let _ = writeln!(out, "algoglyph-contracts: NOT RUN -- {e}; pass --algoglyph DIR or set ALGOGLYPH_ROOT");
            return if optional { 0 } else { 3 };
        }
    };
    let catalogue: Value = match std::fs::read_to_string(repo.join(CATALOGUE)).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(v) => v,
        None => {
            let _ = writeln!(out, "algoglyph-contracts: {CATALOGUE} is missing or not JSON");
            return 2;
        }
    };
    let derived = match derive(&tree, &catalogue) {
        Ok(v) => render(&v),
        Err(e) => {
            let _ = writeln!(out, "algoglyph-contracts: {e}");
            return 1;
        }
    };
    let path = repo.join(ARTIFACT);
    if check {
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        if committed == derived {
            let _ = writeln!(out, "algoglyph-contracts: {ARTIFACT} is what AlgoGlyph {} derives", tree.commit());
            return 0;
        }
        let line = committed.lines().zip(derived.lines()).position(|(a, b)| a != b).map_or_else(|| committed.lines().count().min(derived.lines().count()) + 1, |i| i + 1);
        let _ = writeln!(out, "algoglyph-contracts: {ARTIFACT} differs from what AlgoGlyph {} derives (first at line {line}); run algoglyph-contracts to regenerate it", tree.commit());
        return 1;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, derived) {
        Ok(()) => {
            let _ = writeln!(out, "algoglyph-contracts: wrote {ARTIFACT} from AlgoGlyph {}", tree.commit());
            0
        }
        Err(e) => {
            let _ = writeln!(out, "algoglyph-contracts: {}: {e}", path.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    /// A fresh directory under the system temp dir.
    fn scratch(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("algoglyph-contracts-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
    }

    /// A git checkout holding `files`, committed.
    fn checkout(dir: &Path, files: &[(&str, &str)]) -> GitTree {
        git(dir, &["init", "-q"]);
        for (p, t) in files {
            let f = dir.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        }
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", "fixture"]);
        GitTree::open(dir).unwrap()
    }

    const REGISTRY: &str = "// an engine-driven one is src/kernels/<engine>/<id>.ts\n";

    const CLI: &str = r#"/** A custom plate's headless data form, next to its plate: src/plates/<id>/data.ts. */
const isInt = (x: unknown, lo: number, hi: number): x is number => Number.isInteger(x) && (x as number) >= lo && (x as number) <= hi;
/**
 * A caller's graph: `{ n, edges: [{ u, v, w? }], s?, params? }`.
 */
export function graphFromInput(raw: unknown, k: GraphKernel): { g: Graph } | string {
  if (!raw || typeof raw !== 'object') return 'the input must be a JSON object';
  const r = raw as Record<string, unknown>;
  if (!isInt(r.n, 1, 9)) return `n must be an integer`;
  if (!Array.isArray(r.edges)) return 'edges must be a list';
  for (const [i, e] of r.edges.entries()) {
    const x = e as Record<string, unknown>;
    if (!isInt(x?.u, 0, 9)) return `edges[${i}]: u`;
  }
  const s = r.s ?? 0;
  if (!isInt(s, 0, 9)) return 's';
  if (r.params !== undefined && (typeof r.params !== 'object' || Array.isArray(r.params))) return 'params must be an object';
  return { g: build(r) };
}
"#;

    const KNAP: &str = r#"export interface KnapData {
  weights: number[];
  values: number[];
  /** The capacity,
   * in units. */
  capacity: number;
  // a comment between members
  note?: string
}
export interface KnapResult {
  best: number;
  /** Indices taken. */
  items: number[];
}
export const kernel: GridKernel<KnapData, KnapResult> = {
  fromData(raw) {
    const d = raw as Partial<KnapData> | null;
    const ints = (xs: unknown, lo: number) => Array.isArray(xs) && xs.every((x) => Number.isInteger(x) && x >= lo);
    if (!d || typeof d !== 'object') return 'object';
    if (!ints(d.weights, 1)) return 'weights';
    if (!Array.isArray(d.values) || !d.values.every((v) => Number.isFinite(v))) return 'values';
    if (!Number.isInteger(d.capacity)) return `capacity`;
    return table(d);
  },
};
"#;

    const LIB: &str = r#"export const DIRS = ['N', 'S'] as const;
export const NOT_A_LIST = 3;
export interface Plan {
  status: 'found' | 'none';
  cost: number;
  steps: Step[];
  pair: [number, string];
  meta: Record<string, unknown>;
  total: { a: number; b?: string };
  list: Array<number>;
  either: number | string;
  mixed: number | { a: number };
  maybe: (string);
  any: unknown;
  one: 1;
  kind: Plan['status'];
  index: { [k: string]: number };
}
export interface Report { score: number; tag?: string }
export function plan(m: unknown, from: End): Plan {
  return x;
}
export function price(p: Plan): number {
  return 1;
}
export function report(x: unknown): Report | string {
  return 'no';
}
export function untyped(x) {
  return 1;
}
"#;

    const ROUTE: &str = r#"// a regex /[{(]/ and a template `${'}'}` must not break the scanner: /* {( */
import { DIRS, NOT_A_LIST, plan, price, report, untyped, type Plan } from './lib';
import { other } from 'external-package';
import { gone } from './missing';
const re = /[{(]/g;
const half = 1 / 2;
const end = (v: unknown, name: string): { x: number } | string => {
  if (!record(v) || !integer(v.x)) return `${name} must be {x}`;
  if (!DIRS.includes(v.dir as string)) return `${name}.dir`;
  return { x: v.x };
};
const pair = (v: unknown) => Array.isArray(v) && v.length === 2 && v.every((c) => integer(c));
function fromGame(raw: Record<string, unknown>): Record<string, unknown> | string {
  const { map, ...rest } = raw;
  return { ...rest, heights: map };
}
function shallow(raw: unknown) {
  const { a } = other;
  return a;
}
function plain(x: unknown) {
  return x;
}
const alias = pair;
export const data: DataForm = {
  input: '{from: {x, dir: N|S}, to?: [x, y], mode?: "fast" | "slow", seed? (default 1), map?: openttd://map data, legs?: [{id, cost?}], flag?: boolean, rows: number[][], alt? | other: string[], a?+b?, tuple?: [a, b][], prose: [ids] trailing words, opts?: {anything goes here}, ratio?: 0.5 | 1 | [lo, hi], n: integer (a count), \'quoted\'} then notes',
  solve(input) {
    if (!record(input)) return 'the input must be an object';
    const raw = fromGame(input);
    const view = shallow(input);
    const same = plain(input);
    const none = nowhere(input);
    if (!alias(input.b)) return 'b';
    if (typeof input.opts !== 'function') return 'opts';
    if (typeof input.opts !== kind) return 'kind';
    if (![1, 2].includes(input.n)) return 'n';
    for (const [i, { id }] of input.legs.entries()) {
      if (!id) return 'id';
    }
    for (const leg of input.legs) {
      count++;
    }
    const extra = unknownKeys(input, ['from', 'to', 'mode', 'seed', 'map', 'legs', 'flag', 'rows', 'alt', 'other', 'a', 'b', 'tuple', 'prose', 'opts', 'ratio', 'n', 'hidden']);
    if (extra) return extra;
    const from = end(raw.from, 'from');
    if (!pair(input.to)) return 'to';
    const { mode = 'fast', seed: sd = 1, ratio = 0.5, n = 1_000 } = input;
    if (typeof mode !== 'string') return 'mode';
    if (!integer(sd)) return 'seed';
    if (typeof raw.map === 'string') return 'hidden behind the rest: no fact';
    for (const [i, l] of input.legs.entries()) {
      if (typeof l.id !== 'string') return `legs[${i}].id`;
    }
    const rows = input.rows ?? [];
    if (rows.length === 3) return 'not three rows';
    if (Array.isArray(input.flag)) return 'flag is no list';
    if (!NOT_A_LIST.includes(input.alt)) return 'alt';
    const p = plan(input.map, from);
    const r = report(input);
    const out: Record<string, unknown> = {
      ...(input.flag ? { money: p.status === 'found' ? price(p) : null, gone: undefined } : {}),
      ...(input.flag ? 1 : 2),
      ...rest,
      status: p.status,
      cost: Number.isFinite(p.cost) ? p.cost : null,
      fin: Number.isFinite(p.cost) ? 0 : null,
      steps: p.steps,
      size: p.steps.length,
      'quoted': 'yes',
      ...r,
      label: `x${1}`,
      when: true,
      list: [1, 2],
      nested: { a: 1, b: String(2) },
      many: p.steps.map((q) => q),
      math: Math.max(1, 2),
      echo: input.mode,
      mode,
      seed: sd,
      priced: price(p),
      odd: untyped(p),
      whole: plan(input.map, from),
      unknown: something(),
      choice: input.flag ? 'a' : 1,
      half: input.flag ? 'a' : something(),
      local,
      typed,
      bare,
      report: r.score,
      nofield: p.nothing,
      novar: q.status,
      method() { return 1; },
      [computed]: 1,
    };
    const local = [1, 2];
    const typed: Plan = other();
    out.extra = 'x';
    out.status === 'found';
    out.steps = [];
    return typeof r === 'string' ? r : out;
  },
};
"#;

    const LIT: &str = r#"export const data: DataForm = {
  input: '{a}',
  solve(raw) {
    if (!record(raw)) return 'object';
    return { a: raw.a };
  },
};
"#;

    const TERNARY: &str = r#"export const data: DataForm = {
  input: "{a}",
  solve(raw) {
    const r = go(raw);
    return typeof r === 'string' ? r : { ...r, b: 1 };
  },
};
"#;

    const TESTS: &str = r#"const ok = async (id: string, data: unknown) => {
  const r = await runKernel(id, JSON.stringify(data));
  return r.result as Record<string, unknown>;
};
describe('x', () => {
  it('knap runs', async () => {
    const data = { weights: [1], values: [2], capacity: 3 };
    const r = await runKernel('knap', JSON.stringify(data));
    const res = r.result as { best: number; items: number[] };
    expect(res.best).toBe(2);
    expect(await runKernel('knap', '{}')).toMatchObject({ ok: false });
    expect(list).toContain('knap');
    const txt = '{"weights": [1]}';
    expect(await runKernel('knap', txt)).toMatchObject({ best: 0 });
    expect(await runKernel('knap')).toMatchObject({ best: 0 });
  });
  it('route answers', async () => {
    const base = { rows: [[1]], from: 1 };
    const r = await ok('route', { ...base, from: { x: 1, dir: 'N' } });
    expect(r.cost as number).toBe(3);
    expect(r.unknown as Thing).toBe(3);
    expect(r).toMatchObject({ status: 'found', odd: 2 });
    expect(await ok('route', { rows: [] })).toMatchObject({ status: 'none', size: 0, half: 'x', when: true, list: [1], obj: {} });
    expect(r.result).toBe(undefined);
  });
  it('route spreads', async () => {
    const notobj = 3;
    expect(await ok('route', { ...missing, ...notobj, rows: [] })).toMatchObject({ status: 'none', deep: { a: b ? c : d } });
  });
  it('paths runs', async () => {
    expect(await ok('paths', 'text')).toEqual({ n: 1 });
  });
});
"#;

    fn fixture(dir: &Path) -> (GitTree, Value) {
        let t = checkout(dir, &[
            ("src/registry.ts", REGISTRY), ("src/flow/cli.ts", CLI), ("src/kernels/grid/knap.ts", KNAP),
            ("src/plates/route/lib.ts", LIB), ("src/plates/route/data.ts", ROUTE), ("src/kernels/numeric/lit.ts", LIT),
            ("src/kernels/stochastic/tern.ts", TERNARY), ("src/kernels/graph/paths.ts", "export const kernel = {};\n"), ("test/flow-cli.test.ts", TESTS),
        ]);
        let cat = json!({"commit": t.commit(), "kernels": [
            {"id": "route", "engine": "custom", "title": "Route"},
            {"id": "knap", "engine": "grid", "title": "Knapsack"},
            {"id": "lit", "engine": "numeric"},
            {"id": "tern", "engine": "stochastic"},
            {"id": "paths", "engine": "graph", "goal": "paths", "shape": "digraph", "params": [{"key": "k", "value": 2}, {"key": "r", "value": 0.5}, {"key": "on", "value": true}, {"key": "mode", "value": "x"}, {"key": "list", "value": [1]}, {"value": 1}]},
            {"engine": "custom"},
        ]});
        (t, cat)
    }

    fn contract<'a>(v: &'a Value, id: &str) -> &'a Value {
        v["contracts"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap_or_else(|| panic!("no contract {id}"))
    }

    #[test]
    fn a_grid_kernel_is_read_from_its_interfaces_and_its_from_data() {
        let (t, cat) = fixture(&scratch("grid"));
        let v = derive(&t, &cat).unwrap();
        let k = contract(&v, "knap");
        let i = &k["input"];
        assert_eq!(i["required"], json!(["weights", "values", "capacity"]));
        assert_eq!(i["properties"]["weights"]["items"]["type"], "integer", "the local helper ints() reads each weight with Number.isInteger");
        assert_eq!(i["properties"]["values"]["items"]["type"], "number");
        assert_eq!(i["properties"]["capacity"]["type"], "integer");
        assert_eq!(i["properties"]["capacity"]["description"], "The capacity, in units.");
        assert_eq!(i["properties"]["note"]["type"], "string");
        assert!(i["properties"]["capacity"]["x-from"].as_array().unwrap().iter().any(|f| f.as_str().unwrap().ends_with("Number.isInteger(d.capacity)")));
        assert_eq!(k["output"]["required"], json!(["best", "items"]));
        assert_eq!(k["output"]["properties"]["items"]["description"], "Indices taken.");
        assert_eq!(k["output"]["properties"]["best"]["x-exercised"][0], "test/flow-cli.test.ts:6 knap runs");
        assert_eq!(k["tests"], json!(["test/flow-cli.test.ts:6 knap runs"]), "a refusal and a name in a list are not runs");
        assert_eq!(i["properties"]["weights"]["x-exercised"][0], "test/flow-cli.test.ts:6 knap runs");
        assert_eq!((k["version"].clone(), k["exact"].clone()), (Value::Null, Value::Null));
        assert_eq!(k["absent"]["x-unit"], json!(ABSENT[2].1));
        assert_eq!(k["module"]["path"], "src/kernels/grid/knap.ts");
        assert_eq!(k["module"]["blob"].as_str().unwrap().len(), 40);
        assert_eq!(v["source"]["module_convention"], json!(["src/registry.ts:1 src/kernels/<engine>/<id>.ts", "src/flow/cli.ts:1 src/plates/<id>/data.ts"]));
        assert_eq!(v["contracts"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect::<Vec<_>>(), ["knap", "lit", "paths", "route", "tern"], "sorted; an entry without an id is skipped");
    }

    #[test]
    fn a_data_form_is_read_from_its_input_text_and_its_predicates() {
        let (t, cat) = fixture(&scratch("form"));
        let v = derive(&t, &cat).unwrap();
        let i = &contract(&v, "route")["input"];
        let p = &i["properties"];
        assert_eq!(i["required"], json!(["from", "rows", "prose", "n"]), "optional keys and the members of an alternative are not required");
        assert_eq!(i["additionalProperties"], false);
        assert_eq!(p["hidden"]["x-from"][0], json!(format!("src/plates/route/data.ts:{} accepts hidden", line_of(ROUTE, ROUTE.find("unknownKeys(input").unwrap()))));
        assert_eq!(i["x-note"], "then notes");
        // nested shapes, enums of bare words, a helper's own predicates, a const list
        assert_eq!(p["from"]["required"], json!(["x", "dir"]));
        assert_eq!(p["from"]["properties"]["x"]["type"], "integer");
        assert_eq!(p["from"]["properties"]["dir"]["enum"], json!(["N", "S"]));
        assert!(p["from"]["properties"]["dir"]["x-from"].as_array().unwrap().iter().any(|f| f.as_str().unwrap().starts_with("src/plates/route/lib.ts:1 DIRS")));
        // a tuple: Array.isArray, length === 2, every(integer)
        assert_eq!((p["to"]["type"].clone(), p["to"]["minItems"].clone(), p["to"]["items"]["type"].clone()), (json!("array"), json!(2), json!("integer")));
        assert_eq!(p["mode"]["enum"], json!(["fast", "slow"]));
        assert_eq!((p["mode"]["type"].clone(), p["mode"]["default"].clone()), (json!("string"), json!("fast")));
        assert_eq!((p["seed"]["type"].clone(), p["seed"]["default"].clone(), p["seed"]["description"].clone()), (json!("integer"), json!(1), json!("default 1")));
        assert_eq!(p["n"]["default"], 1000);
        assert_eq!(p["n"]["type"], "integer");
        assert_eq!(p["ratio"]["anyOf"][0]["enum"], json!([0.5, 1]));
        assert_eq!(p["ratio"]["anyOf"][1]["minItems"], 2);
        assert_eq!(p["ratio"]["default"], 0.5);
        assert_eq!(p["map"]["description"], "openttd://map data");
        assert!(p["map"].get("type").is_none(), "typeof raw.map: raw passes map on to heights, so raw.map is not the input's map");
        assert_eq!(p["legs"]["items"]["properties"]["id"]["type"], "string");
        assert_eq!(p["legs"]["items"]["required"], json!(["id"]));
        assert_eq!(p["flag"]["type"], "boolean", "Array.isArray in a refusal says what flag is not");
        assert_eq!(p["rows"]["items"]["items"]["type"], "number");
        assert!(p["rows"].get("minItems").is_none(), "rows.length === 3 in a refusal says what rows is not");
        assert!(p["alt"].get("enum").is_none(), "NOT_A_LIST is no literal list");
        assert_eq!(p["other"]["items"]["type"], "string");
        assert!(p["a"].is_object() && p["b"].is_object());
        assert_eq!(p["tuple"]["items"]["maxItems"], 2);
        assert!(p["b"].get("type").is_none(), "a helper bound to another name (const alias = pair) is not followed");
        assert!(p["n"].get("enum").is_none(), "[1, 2].includes names no list");
        assert_eq!(p["prose"]["description"], "trailing words");
        assert_eq!(p["opts"]["description"], "{anything goes here}");
        assert_eq!(p["rows"]["x-exercised"].as_array().unwrap().len(), 2, "both tests give rows, one through the spread of its base");
        assert_eq!(contract(&v, "route")["tests"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_data_forms_output_is_the_record_its_solve_returns() {
        let (t, cat) = fixture(&scratch("out"));
        let v = derive(&t, &cat).unwrap();
        let o = &contract(&v, "route")["output"];
        let p = &o["properties"];
        let ty = |k: &str| p[k]["type"].clone();
        assert_eq!(p["status"]["enum"], json!(["found", "none"]), "a field of plan()'s declared Plan");
        assert_eq!(ty("status"), "string", "the test's own reading adds the type");
        assert_eq!(p["status"]["x-type-from"], "the test's own reading of the answer");
        assert_eq!(ty("cost"), json!(["number", "null"]));
        assert_eq!(ty("fin"), json!(["null", "number"]), "a ternary of two literals");
        assert_eq!((ty("steps"), p["steps"]["items"]["x-ts-type"].clone()), (json!("array"), json!("Step")));
        assert_eq!(ty("size"), "integer");
        assert_eq!(ty("quoted"), "string");
        assert_eq!((ty("score"), p["tag"]["type"].clone()), (json!("number"), json!("string")), "...r spreads report()'s declared Report");
        assert_eq!(ty("money"), json!(["null", "number"]));
        assert_eq!(p["money"]["x-conditional"], true);
        assert!(p.get("gone").is_none(), "an undefined value is dropped by JSON");
        for (k, t) in [("label", "string"), ("when", "boolean"), ("list", "array"), ("nested", "object"), ("many", "array"), ("math", "number"), ("priced", "number"), ("local", "array"), ("report", "number"), ("choice", "")] {
            if t.is_empty() {
                assert_eq!(ty(k), json!(["number", "string"]), "{k}");
            } else {
                assert_eq!(ty(k), t, "{k}");
            }
        }
        assert_eq!(p["nested"]["properties"]["b"]["type"], "string");
        assert_eq!((ty("mode"), ty("echo")), (json!("string"), json!("string")), "input keys echoed");
        assert!(ty("seed").is_null(), "sd is the seed only through its destructuring alias, which the output does not follow");
        assert_eq!(ty("odd"), "number", "untyped() has no annotation: the test's reading");
        assert_eq!(p["typed"]["x-ts-type"], "Plan");
        assert!(ty("whole").is_null(), "plan() answers a named Plan, not a JSON type");
        for k in ["unknown", "bare", "nofield", "novar"] {
            assert!(ty(k).is_null(), "{k}: {}", p[k]);
        }
        assert_eq!((ty("half"), p["half"]["x-type-from"].clone()), (json!("string"), json!("the test's own reading of the answer")), "one arm untyped: only the test's reading");
        assert_eq!(p["extra"]["x-conditional"], true);
        assert!(p["steps"].get("x-conditional").is_none(), "an assignment to a key the literal has changes nothing");
        assert!(p.get("method").is_none());
        assert_eq!(p["obj"]["x-exercised"].as_array().unwrap().len(), 1, "a key only a test reads is listed with its test");
        // the plain literal and the ternary arm
        assert_eq!(contract(&v, "lit")["output"]["properties"]["a"]["type"], json!(null));
        assert_eq!(contract(&v, "lit")["input"]["properties"]["a"]["x-from"][0], "src/kernels/numeric/lit.ts:2 declares a");
        assert_eq!(contract(&v, "tern")["output"]["properties"]["b"]["type"], "number");
        // a grid kernel answers its interface; a graph kernel's answer is not derived
        assert!(contract(&v, "paths")["output"].is_null());
    }

    #[test]
    fn a_graph_kernel_reads_the_flows_graph_form_and_its_own_params() {
        let (t, cat) = fixture(&scratch("graph"));
        let v = derive(&t, &cat).unwrap();
        let k = contract(&v, "paths");
        let p = &k["input"]["properties"];
        assert_eq!(k["input"]["required"], json!(["n", "edges"]));
        assert_eq!(p["n"]["type"], "integer");
        assert_eq!(p["edges"]["items"]["properties"]["u"]["type"], "integer", "x?.u is a key of the edge");
        assert_eq!((p["s"]["type"].clone(), p["s"]["default"].clone()), (json!("integer"), json!(0)));
        assert_eq!(p["params"]["type"], "object", "Array.isArray in the refusal says what params is not");
        let params = &p["params"]["properties"];
        assert_eq!((params["k"]["type"].clone(), params["k"]["default"].clone()), (json!("number"), json!(2)));
        assert_eq!((params["r"]["type"].clone(), params["on"]["type"].clone(), params["mode"]["type"].clone()), (json!("number"), json!("boolean"), json!("string")));
        assert!(params["list"].get("type").is_none());
        assert_eq!(k["graph"], json!({"goal": "paths", "shape": "digraph"}));
        assert!(k["absent"]["output"].is_string());
        assert_eq!(k["tests"], json!([format!("test/flow-cli.test.ts:{} paths runs", line_of(TESTS, TESTS.find("it('paths").unwrap()))]));
    }

    #[test]
    fn a_source_that_no_longer_states_what_derivation_reads_is_refused() {
        let base: Vec<(&str, &str)> = vec![("src/registry.ts", REGISTRY), ("src/flow/cli.ts", CLI)];
        let derive_with = |extra: &[(&str, &str)], kernels: Value| -> String {
            let mut files = base.clone();
            files.extend_from_slice(extra);
            let t = checkout(&scratch("refuse"), &files);
            derive(&t, &json!({"commit": t.commit(), "kernels": kernels})).unwrap_err()
        };
        let one = |id: &str, engine: &str| json!([{"id": id, "engine": engine}]);
        assert!(derive_with(&[], one("x", "custom")).contains("src/plates/x/data.ts is missing"));
        assert!(derive_with(&[("src/plates/x/data.ts", "export const y = 1;")], one("x", "custom")).contains("no `export const data: DataForm`"));
        assert!(derive_with(&[("src/plates/x/data.ts", "export const data: DataForm = {")], one("x", "custom")).contains("does not close"));
        assert!(derive_with(&[("src/plates/x/data.ts", "export const data: DataForm = { solve(r) {} };")], one("x", "custom")).contains("has no input"));
        assert!(derive_with(&[("src/plates/x/data.ts", "export const data: DataForm = { input: SHAPE, solve(r) {} };")], one("x", "custom")).contains("not a string literal"));
        assert!(derive_with(&[("src/plates/x/data.ts", "export const data: DataForm = { input: '{a}' };")], one("x", "custom")).contains("has no solve"));
        assert!(derive_with(&[("src/kernels/grid/g.ts", "export const kernel = {};")], one("g", "grid")).contains("GridKernel<...>`"));
        assert!(derive_with(&[("src/kernels/grid/g.ts", "export const kernel: GridKernel<D, R")], one("g", "grid")).contains("does not close"));
        assert!(derive_with(&[("src/kernels/grid/g.ts", "export const kernel: GridKernel<D, R> = {};")], one("g", "grid")).contains("interface D is not declared"));
        // a grid kernel without fromData or a result interface still has its data interface
        let t = checkout(&scratch("grid-bare"), &[("src/registry.ts", REGISTRY), ("src/flow/cli.ts", CLI), ("src/kernels/grid/g.ts", "interface D { a: number }\nexport const kernel: GridKernel<D, R> = {};")]);
        let v = derive(&t, &json!({"commit": t.commit(), "kernels": one("g", "grid")})).unwrap();
        assert_eq!((contract(&v, "g")["input"]["required"].clone(), contract(&v, "g")["output"].clone()), (json!(["a"]), Value::Null));
        // the graph form: gone, undocumented, unshaped
        let graph = |cli: &str| {
            let t = checkout(&scratch("graph-form"), &[("src/registry.ts", REGISTRY), ("src/flow/cli.ts", cli), ("src/kernels/graph/p.ts", "x")]);
            derive(&t, &json!({"commit": t.commit(), "kernels": one("p", "graph")})).unwrap_err()
        };
        let conv = "// src/plates/<id>/data.ts\n";
        assert!(graph(conv).contains("no graphFromInput"));
        assert!(graph(&format!("{conv}export function graphFromInput(raw) {{}}")).contains("no doc comment"));
        assert!(graph(&format!("/** no shape */\n{conv}export function graphFromInput(raw) {{}}")).contains("states no `shape`"));
        assert!(graph(&format!("/** `{{n}}` */\n{conv}export function graphFromInput(raw)\n")).contains("has no body"));
        // the conventions and the catalogue
        let t = checkout(&scratch("conv"), &[("src/registry.ts", "nothing"), ("src/flow/cli.ts", CLI)]);
        assert!(derive(&t, &json!({"commit": t.commit(), "kernels": []})).unwrap_err().contains("no longer states src/kernels/<engine>/<id>.ts"));
        let t = checkout(&scratch("nocli"), &[("src/registry.ts", REGISTRY)]);
        assert!(derive(&t, &json!({"commit": t.commit(), "kernels": []})).unwrap_err().contains("src/flow/cli.ts is missing"));
        let t = checkout(&scratch("noreg"), &[("src/flow/cli.ts", CLI)]);
        assert!(derive(&t, &json!({"commit": t.commit(), "kernels": []})).unwrap_err().contains("src/registry.ts is missing"));
        assert!(derive(&t, &json!({"commit": "0000000000000000000000000000000000000000", "kernels": []})).unwrap_err().contains("refresh the catalogue"));
        assert!(derive(&t, &json!({"kernels": []})).unwrap_err().contains("names no commit"));
        assert!(derive(&t, &json!({"commit": t.commit()})).unwrap_err().contains("lists no kernels"));
        assert!(GitTree::open(&scratch("not-git")).is_err());
        assert_eq!(t.read("no/such/file"), None);
    }

    #[test]
    fn the_scanner_reads_past_strings_comments_templates_and_regexes() {
        let s = "f('}', \"{\", `${ {a: 1} }}`, /[)]/g, a / b /* ) */ // )\n)";
        assert_eq!(close(s.as_bytes(), 1), Some(s.len()));
        assert_eq!(close(b"(a", 0), None);
        assert_eq!(close(br"(`a\`b`, /a\/b/)", 0), Some(16));
        assert!(split_top("x", 1, 0, b',').is_empty());
        assert_eq!(import_target("a.ts", " from 'pkg';"), None);
        assert_eq!(import_target("a/b.ts", " from './c';"), Some("a/c.ts".into()));
        assert_eq!(import_target("a/b.ts", " as x;"), None);
        assert_eq!(split_first("a ? b : c", b':'), Some(("a ? b", "c")));
        assert_eq!(split_first("'a:b'", b':'), None);
        assert_eq!(statement_end("f(a); b;", 0, 8), 4);
        assert_eq!(statement_end("f(a)", 0, 4), 4);
        assert_eq!(string_at("'a\\'b\\n\\", 0).map(|x| x.0), Some("a'b\n".to_string()));
        assert_eq!(string_at("`a${b}`", 0), None);
        assert_eq!(string_at("x", 0), None);
        assert_eq!(literal("-1.5"), Some(json!(-1.5)));
        assert_eq!(literal("1e9x"), None);
        assert_eq!(literal("'a' + b"), None);
        assert_eq!((literal("[]"), literal("null"), literal("false")), (Some(json!([])), Some(Value::Null), Some(json!(false))));
        assert_eq!(resolve("src/a/b.ts", "../c"), Some("src/c.ts".into()));
        assert_eq!(resolve("a.ts", "../../x"), None);
        assert!(shape_keys("no shape").is_none());
        assert!(shape_keys("{a").is_none());
        assert_eq!(shape_value("{}")["description"], "{}");
        assert_eq!(shape_value("{a} and more")["description"], "and more");
        assert_eq!(shape_value("[x, y] is a pair")["description"], "is a pair");
        assert_eq!(shape_value("[x, y][] pairs")["description"], "pairs");
        assert_eq!(shape_value("[x")["description"], "[x");
        assert_eq!(shape_value("string (a name)")["description"], "a name");
        assert_eq!(shape_value("A | b c"), json!({"anyOf": [{"description": "A"}, {"description": "b c"}]}));
        let m = Module { path: "m.ts".into(), text: "interface I { a: number }".into(), blob: String::new(), imports: vec![] };
        assert_eq!(ts_schema(&m, "I['a']"), json!({"type": "number"}));
        assert_eq!(ts_schema(&m, "I['b']"), json!({"x-ts-type": "I['b']"}));
        assert_eq!(ts_schema(&m, "Array<string>"), json!({"type": "array", "items": {"type": "string"}}));
        assert_eq!(ts_schema(&m, "number | { a: number }")["anyOf"][1]["required"], json!(["a"]));
        assert_eq!(ts_schema(&m, "number | string"), json!({"type": ["number", "string"]}));
        assert_eq!(ts_schema(&m, "(string)"), json!({"type": "string"}));
        assert_eq!(ts_schema(&m, "[number, string]")["prefixItems"][1], json!({"type": "string"}));
        assert_eq!(ts_schema(&m, "Record<string, unknown>"), json!({"type": "object"}));
        assert_eq!(ts_schema(&m, "{ [k: string]: number; bare; a: 1 }")["properties"], json!({"a": {"const": 1}}));
        let t = "if (typeof x.a !== kind) return 'k'; if (typeof x.a === 'function') y(); if (Number.isFinite(x.a) && z) w(); if (typeof x.a === 'string') return 'no strings';";
        let fm = Module { path: "f.ts".into(), text: t.into(), blob: String::new(), imports: vec![] };
        let f = facts_of(&fm, (0, t.len()), "x.a", 0);
        assert_eq!(f.types.into_iter().collect::<Vec<_>>(), ["number"], "typeof against a name or an unlisted type states nothing");
        assert_eq!(shape_value("{a"), json!({"description": "{a"}));
        let ctx = (0, m.text.len());
        let mut input = BTreeMap::new();
        input.insert("k".to_string(), json!({"type": ["number", "null"], "x-from": ["x"]}));
        input.insert("flag".to_string(), json!(true));
        assert_eq!(value_schema(&m, ctx, "c ? k : 'a'", &input).0, json!({"type": ["null", "number", "string"]}));
        assert_eq!(value_schema(&m, ctx, "flag", &input).0, json!(true), "a non-object echo is passed on as it is");
        assert_eq!(type_names(&json!(5)), Vec::<String>::new());
        assert_eq!(value_schema(&m, ctx, "Number.isFinite(x", &input).0, json!({}));
        let mut a = Facts::default();
        a.nested = Some(json!({"properties": {"x": {"type": "string"}}}));
        let mut b = Facts::default();
        b.nested = Some(json!({"properties": {"y": {}}, "required": ["y"]}));
        a.merge(b);
        assert_eq!(a.nested.unwrap()["properties"].as_object().unwrap().len(), 2);
        assert_eq!(merge_schema(json!({"required": ["a"]}), json!({"required": ["b"]})), json!({"required": ["a", "b"]}));
        assert_eq!(m.at_slice("elsewhere"), "m.ts");
        assert!(merge_schema(json!(1), json!({"a": 1})).is_object());
        assert_eq!(merge_schema(json!({"a": 1}), json!(2)), json!({"a": 1}));
        assert_eq!(merge_schema(json!({"x-from": ["a"], "items": {"type": "x"}}), json!({"x-from": ["b"], "items": {"min": 1}})), json!({"x-from": ["a", "b"], "items": {"type": "x", "min": 1}}));
        let mut f = Facts::default();
        f.items.insert("number".into());
        f.items.insert("string".into());
        assert_eq!(f.schema()["items"]["type"], json!(["number", "string"]));
    }

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn the_source_root_is_a_parameter_then_the_submodule() {
        let r = scratch("root");
        assert_eq!(source_root(Some("/x"), &r, &env_of(&[("ALGOGLYPH_ROOT", "/y")])), PathBuf::from("/x"));
        assert_eq!(source_root(None, &r, &env_of(&[("ALGOGLYPH_ROOT", "/y")])), PathBuf::from("/y"));
        assert_eq!(source_root(None, &r, &env_of(&[("ALGOGLYPH_ROOT", ""), ("HOME", "/h")])), PathBuf::from("/h").join(TRANSITIONAL_ROOT));
        std::fs::write(r.join(".gitmodules"), "[submodule \"other\"]\n\tpath = third_party/other\n\turl = https://x/other.git\n[submodule \"algoglyph\"]\n\tpath = third_party/algoglyph\n\turl = git@github.com:korczis/algoglyph.git\n").unwrap();
        assert_eq!(source_root(None, &r, &env_of(&[])), r.join("third_party/algoglyph"));
        std::fs::write(r.join(".gitmodules"), "\turl = https://x/algoglyph\n").unwrap();
        assert_eq!(source_root(None, &r, &env_of(&[])), PathBuf::from("").join(TRANSITIONAL_ROOT), "a url without its path names no checkout");
    }

    #[test]
    fn the_command_line_writes_checks_and_refuses() {
        let repo = scratch("cli");
        let src = repo.join("third_party/algoglyph");
        std::fs::create_dir_all(&src).unwrap();
        let (t, cat) = fixture(&src);
        let run = |args: &[&str]| -> (i32, String) {
            let mut out = Vec::new();
            let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            let code = cli(&args, &repo, &|k| (k == "HOME").then(|| "/nonexistent".to_string()), &mut out);
            (code, String::from_utf8(out).unwrap())
        };
        let srcs = src.to_str().unwrap();
        assert_eq!(run(&["--help"]).0, 0);
        assert_eq!(run(&["--bogus"]).0, 2);
        assert_eq!(run(&["--algoglyph"]).0, 2);
        let (code, said) = run(&["--algoglyph", "/nonexistent/algoglyph", "--check"]);
        assert_eq!(code, 3);
        assert!(said.contains("NOT RUN"), "{said}");
        assert_eq!(run(&["--algoglyph", "/nonexistent/algoglyph", "--check", "--source-optional"]).0, 0);
        assert_eq!(run(&["--algoglyph", srcs]).0, 2, "no catalogue yet");
        let fixture_path = repo.join(CATALOGUE);
        std::fs::create_dir_all(fixture_path.parent().unwrap()).unwrap();
        std::fs::write(&fixture_path, serde_json::to_string(&json!({"commit": "0000000000000000000000000000000000000000", "kernels": []})).unwrap()).unwrap();
        assert!(run(&["--algoglyph", srcs]).1.contains("refresh the catalogue"));
        std::fs::write(&fixture_path, serde_json::to_string(&cat).unwrap()).unwrap();
        // the submodule .gitmodules declares is found without --algoglyph
        std::fs::write(repo.join(".gitmodules"), "[submodule \"algoglyph\"]\n\tpath = third_party/algoglyph\n\turl = git@github.com:korczis/algoglyph.git\n").unwrap();
        assert_eq!(run(&["--check"]).0, 1, "nothing committed yet");
        let (code, said) = run(&[]);
        assert_eq!(code, 0, "{said}");
        assert!(said.contains(t.commit()));
        assert_eq!(run(&["--check"]), (0, format!("algoglyph-contracts: {ARTIFACT} is what AlgoGlyph {} derives\n", t.commit())));
        let artifact = repo.join(ARTIFACT);
        let text = std::fs::read_to_string(&artifact).unwrap();
        std::fs::write(&artifact, text.replacen("\"integer\"", "\"number\"", 1)).unwrap();
        assert!(run(&["--check"]).1.contains("first at line"));
        std::fs::write(&artifact, &text[..text.len() / 2]).unwrap();
        assert_eq!(run(&["--check"]).0, 1, "a truncated artifact differs too");
        std::fs::remove_file(&artifact).unwrap();
        std::fs::create_dir_all(&artifact).unwrap();
        assert_eq!(run(&[]).0, 1, "an artifact path that cannot be written");
        assert_eq!(render(&json!({"a": 1})), "{\n  \"a\": 1\n}\n");
    }

    #[test]
    fn the_committed_contracts_are_what_the_pinned_source_derives() {
        let repo = repo();
        let root = source_root(None, &repo, &|k| std::env::var(k).ok());
        let tree = GitTree::open(&root).unwrap_or_else(|e| panic!("NOT RUN: {e}; set ALGOGLYPH_ROOT to an AlgoGlyph checkout at the catalogue's commit"));
        let cat: Value = serde_json::from_str(&std::fs::read_to_string(repo.join(CATALOGUE)).unwrap()).unwrap();
        let derived = render(&derive(&tree, &cat).unwrap());
        let committed = std::fs::read_to_string(repo.join(ARTIFACT)).unwrap();
        assert!(derived == committed, "{ARTIFACT} differs from what AlgoGlyph {} derives: run algoglyph-contracts", tree.commit());
    }

    #[test]
    fn the_committed_contracts_state_what_battlebrain_maps() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(repo().join(ARTIFACT)).unwrap()).unwrap();
        let cat: Value = serde_json::from_str(&std::fs::read_to_string(repo().join(CATALOGUE)).unwrap()).unwrap();
        assert_eq!((v["schema"].as_str(), &v["source"]["commit"]), (Some(ARTIFACT_SCHEMA), &cat["commit"]));
        assert_eq!(v["contracts"].as_array().unwrap().len(), cat["kernels"].as_array().unwrap().len());
        let k = contract(&v, "knapsack-dp");
        assert_eq!(k["input"]["required"], json!(["weights", "values", "capacity"]));
        assert_eq!((k["input"]["properties"]["weights"]["items"]["type"].as_str(), k["input"]["properties"]["values"]["items"]["type"].as_str(), k["input"]["properties"]["capacity"]["type"].as_str()), (Some("integer"), Some("number"), Some("integer")));
        assert_eq!(k["output"]["required"], json!(["best", "items"]));
        let t = contract(&v, "track-planning");
        assert_eq!(t["input"]["required"], json!(["from", "to"]));
        assert_eq!(t["input"]["properties"]["from"]["properties"]["dir"]["enum"], json!(["NE", "SE", "SW", "NW"]));
        assert_eq!(t["output"]["properties"]["status"]["enum"], json!(["found", "unreachable", "budget"]));
        assert_eq!(t["output"]["properties"]["money"]["type"], json!(["null", "number"]));
        assert_eq!(t["output"]["properties"]["cost"]["type"], json!(["number", "null"]), "null, not Infinity, when nothing was found");
        for c in v["contracts"].as_array().unwrap() {
            assert_eq!(c["schema"], CONTRACT_SCHEMA);
            for (_, p) in c["input"]["properties"].as_object().unwrap() {
                assert!(p["x-from"].as_array().is_some_and(|f| !f.is_empty()), "{}: every input key names its source", c["id"]);
            }
            if c["engine"] != "graph" {
                assert!(c["tests"].as_array().is_some_and(|t| !t.is_empty()), "{}: test/flow-cli.test.ts runs every data form", c["id"]);
            }
        }
    }
}
