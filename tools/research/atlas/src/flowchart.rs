//! The Mermaid flowcharts a document marks for the atlas, read into nodes, groups and typed edges.
//!
//! A diagram joins the atlas with one Mermaid comment, which GitHub and the site ignore:
//!
//! ```text
//! flowchart LR
//!   %% atlas: architecture | How the pieces connect
//! ```
//!
//! The diagram stays the one source: GitHub draws it, the site draws it, and the atlas draws the
//! same nodes and edges as an interactive graph. What each piece means is Mermaid's own syntax:
//!
//! - an edge's stroke is its kind: `-->` solid (command and control), `-.->` dotted (observed data,
//!   reads, publication), `==>` thick (approval, validation, evidence);
//! - `click <node> "<path>"` is the node's page (Mermaid's link syntax; GitHub's strict mode ignores
//!   it), and every node of an atlas diagram must have one that resolves;
//! - `class <nodes> proposed` marks a part that is designed but not built;
//! - a `subgraph` is the node's group.
//!
//! Only the subset of the flowchart grammar the repository's diagrams use is read; a statement
//! the reader does not understand is a finding, never silently dropped.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub group: Option<String>,
    pub classes: Vec<String>,
    pub href: Option<String>,
    pub tooltip: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: &'static str,
    pub label: String,
    pub both: bool,
    pub directed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    pub label: String,
    pub parent: Option<String>,
}

#[derive(Debug, Default)]
pub struct Chart {
    pub id: String,
    pub title: String,
    pub line: usize,
    pub nodes: Vec<Node>,
    pub groups: Vec<Group>,
    pub edges: Vec<Edge>,
    pub problems: Vec<(usize, String)>,
}

/// The kinds an edge's stroke stands for, in legend order: (kind, stroke, Mermaid arrow, meaning).
pub const KINDS: [(&str, &str, &str, &str); 3] = [
    ("control", "solid", "-->", "command and control: a request, a call, a game command"),
    ("data", "dotted", "-.->", "observed data: a read, an event, a publication"),
    ("governance", "thick", "==>", "approval, validation and evidence"),
];

fn kind_of(arrow: &str) -> &'static str {
    if arrow.contains('=') {
        "governance"
    } else if arrow.contains('.') {
        "data"
    } else {
        "control"
    }
}

/// A Mermaid label as text: `<br/>` becomes a line break, entities and Mermaid's `#quot;` decode.
pub fn label_text(raw: &str) -> String {
    let mut s = raw.to_string();
    for br in ["<br/>", "<br />", "<br>"] {
        s = s.replace(br, "\n");
    }
    // Markup first (`<b>`), then entities, so an escaped `&lt;` survives as text.
    let mut out = String::new();
    let mut tag = false;
    for c in s.chars() {
        match c {
            '<' if !tag => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    for (from, to) in [
        ("&lt;", "<"), ("&gt;", ">"), ("&middot;", "·"), ("&quot;", "\""), ("#quot;", "\""),
        ("&nbsp;", " "), ("&mdash;", "—"), ("&rarr;", "→"), ("&amp;", "&"),
    ] {
        out = out.replace(from, to);
    }
    out.lines().map(str::trim).collect::<Vec<_>>().join("\n").trim().to_string()
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn rest(&self) -> &'a str {
        std::str::from_utf8(&self.s[self.i..]).unwrap_or("")
    }
    fn starts(&self, p: &str) -> bool {
        self.rest().starts_with(p)
    }
}

fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// A node reference: `id`, `id["label"]`, `id(["label"])`, `id[("label")]`, `id{label}`, each with
/// an optional `:::class`. Returns (id, label, classes).
fn node_ref(c: &mut Cursor) -> Option<(String, Option<String>, Vec<String>)> {
    c.ws();
    let start = c.i;
    while c.peek().is_some_and(is_id) {
        c.i += 1;
    }
    if c.i == start {
        return None;
    }
    let id = std::str::from_utf8(&c.s[start..c.i]).ok()?.to_string();
    let mut label = None;
    if let Some(open) = c.peek().filter(|b| matches!(b, b'[' | b'(' | b'{' | b'>')) {
        // Openers: up to two of [ ( { / \ ; the label is quoted or runs to the first closer.
        let mut openers = vec![open];
        c.i += 1;
        while let Some(b) = c.peek().filter(|b| matches!(b, b'[' | b'(' | b'{' | b'/' | b'\\')) {
            openers.push(b);
            c.i += 1;
        }
        c.ws();
        let text = if c.peek() == Some(b'"') {
            c.i += 1;
            let s = c.i;
            while c.peek().is_some_and(|b| b != b'"') {
                c.i += 1;
            }
            let t = std::str::from_utf8(&c.s[s..c.i]).ok()?.to_string();
            c.i += 1; // the closing quote
            t
        } else {
            let s = c.i;
            while c.peek().is_some_and(|b| !matches!(b, b']' | b')' | b'}')) {
                c.i += 1;
            }
            std::str::from_utf8(&c.s[s..c.i]).ok()?.trim_end_matches(['/', '\\']).to_string()
        };
        c.ws();
        let mut closed = 0;
        while c.peek().is_some_and(|b| matches!(b, b']' | b')' | b'}' | b'/' | b'\\')) && closed < openers.len() + 1 {
            if matches!(c.peek(), Some(b']' | b')' | b'}')) {
                closed += 1;
            }
            c.i += 1;
            if closed == openers.len() {
                break;
            }
        }
        if closed == 0 {
            return None;
        }
        label = Some(label_text(&text));
    }
    let mut classes = vec![];
    while c.starts(":::") {
        c.i += 3;
        let s = c.i;
        while c.peek().is_some_and(|b| is_id(b) || b == b'-') {
            c.i += 1;
        }
        classes.push(std::str::from_utf8(&c.s[s..c.i]).ok()?.to_string());
    }
    Some((id, label, classes))
}

fn arrow_chars(c: &mut Cursor) -> String {
    let s = c.i;
    while c.peek().is_some_and(|b| matches!(b, b'<' | b'-' | b'=' | b'.' | b'>')) {
        c.i += 1;
    }
    std::str::from_utf8(&c.s[s..c.i]).unwrap_or("").to_string()
}

/// An edge: `-->`, `---`, `-.->`, `==>`, `<-->`, `-- text -->`, `-. text .->`, `== text ==>`, each
/// with an optional `|label|`. Returns (arrow, label).
fn edge(c: &mut Cursor) -> Option<(String, String)> {
    c.ws();
    let save = c.i;
    let mut arrow = arrow_chars(c);
    if arrow.len() < 2 {
        c.i = save;
        return None;
    }
    let mut label = String::new();
    if !arrow.contains('>') && matches!(arrow.trim_start_matches('<'), "--" | "==" | "-.") {
        // `-- text -->`: the label runs to the closing arrow.
        let rest = c.rest();
        let close = ["-->", "---", "==>", "===", ".->", ".-"]
            .iter()
            .filter_map(|p| rest.find(p).map(|at| (at, *p)))
            .min_by_key(|(at, _)| *at);
        if let Some((at, _)) = close {
            label = label_text(rest[..at].trim().trim_matches('"'));
            c.i += at;
            arrow.push_str(&arrow_chars(c));
        }
    }
    c.ws();
    if c.peek() == Some(b'|') {
        c.i += 1;
        let s = c.i;
        while c.peek().is_some_and(|b| b != b'|') {
            c.i += 1;
        }
        label = label_text(std::str::from_utf8(&c.s[s..c.i]).ok()?.trim().trim_matches('"'));
        c.i += 1;
    }
    Some((arrow, label))
}

/// Every flowchart in `markdown` that carries `%% atlas: <id> | <title>`, each with the line of its
/// fence. A chart's problems are findings for the caller to report.
pub fn charts(markdown: &str) -> Vec<Chart> {
    let mut out = vec![];
    let mut block: Option<(usize, Vec<(usize, &str)>)> = None;
    for (n, line) in markdown.lines().enumerate() {
        let t = line.trim();
        match &mut block {
            None if t.starts_with("```mermaid") => block = Some((n + 1, vec![])),
            Some((start, body)) if t.starts_with("```") => {
                if let Some(chart) = parse(*start, body) {
                    out.push(chart);
                }
                block = None;
            }
            Some((_, body)) => body.push((n + 1, line)),
            None => {}
        }
    }
    out
}

fn parse(fence: usize, body: &[(usize, &str)]) -> Option<Chart> {
    let header = body.iter().find_map(|(_, l)| l.trim().strip_prefix("%% atlas:"))?;
    let (id, title) = header.split_once('|').unwrap_or((header, ""));
    let mut chart = Chart { id: id.trim().to_string(), title: title.trim().to_string(), line: fence, ..Default::default() };
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let mut order: Vec<String> = vec![];
    let mut stack: Vec<String> = vec![];
    let touch = |nodes: &mut BTreeMap<String, Node>, order: &mut Vec<String>, id: &str, label: Option<String>,
                     classes: Vec<String>, group: Option<String>, line: usize| {
        let node = nodes.entry(id.to_string()).or_insert_with(|| {
            order.push(id.to_string());
            Node { id: id.to_string(), label: id.to_string(), group: group.clone(), classes: vec![], href: None, tooltip: None, line }
        });
        if let Some(l) = label {
            node.label = l;
            node.group = group;
            node.line = line;
        }
        for cl in classes {
            if !node.classes.contains(&cl) {
                node.classes.push(cl);
            }
        }
    };
    for &(line, raw) in body {
        let t = raw.trim();
        if t.is_empty() || t.starts_with("%%") || t.starts_with("flowchart") || t.starts_with("graph ")
            || t.starts_with("classDef ") || t.starts_with("style ") || t.starts_with("linkStyle ")
            || t.starts_with("direction ") || t == "---" || t.starts_with("config:")
        {
            continue;
        }
        if let Some(rest) = t.strip_prefix("subgraph ") {
            let mut c = Cursor { s: rest.as_bytes(), i: 0 };
            let (gid, glabel) = match node_ref(&mut c) {
                Some((gid, l, _)) => {
                    let l = l.unwrap_or_else(|| label_text(c.rest().trim().trim_matches('"')));
                    (gid.clone(), if l.is_empty() { gid } else { l })
                }
                None => {
                    let l = label_text(rest.trim().trim_matches('"'));
                    (l.clone(), l)
                }
            };
            chart.groups.push(Group { id: gid.clone(), label: glabel, parent: stack.last().cloned() });
            stack.push(gid);
            continue;
        }
        if t == "end" {
            stack.pop();
            continue;
        }
        if let Some(rest) = t.strip_prefix("class ") {
            let mut parts = rest.split_whitespace();
            let (ids, cls) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            for id in ids.split(',').filter(|s| !s.is_empty()) {
                touch(&mut nodes, &mut order, id, None, vec![cls.trim_end_matches(';').to_string()], stack.last().cloned(), line);
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("click ") {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let id = parts.next().unwrap_or("").to_string();
            let tail = parts.next().unwrap_or("").trim();
            let tail = tail.strip_prefix("href ").unwrap_or(tail).trim();
            let quoted: Vec<&str> = tail.split('"').skip(1).step_by(2).collect();
            match quoted.first() {
                Some(href) => {
                    touch(&mut nodes, &mut order, &id, None, vec![], stack.last().cloned(), line);
                    let node = nodes.get_mut(&id).expect("touched");
                    node.href = Some(href.to_string());
                    node.tooltip = quoted.get(1).map(|s| s.to_string());
                }
                None => chart.problems.push((line, format!("click {id}: expected a quoted link"))),
            }
            continue;
        }
        // A statement: node-group (edge node-group)*, a node-group being refs joined by `&`.
        let mut c = Cursor { s: t.as_bytes(), i: 0 };
        let mut prev: Vec<String> = vec![];
        let mut pending: Option<(String, String)> = None;
        let mut ok = true;
        loop {
            let mut group = vec![];
            loop {
                match node_ref(&mut c) {
                    Some((id, label, classes)) => {
                        touch(&mut nodes, &mut order, &id, label, classes, stack.last().cloned(), line);
                        group.push(id);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
                c.ws();
                if c.peek() == Some(b'&') {
                    c.i += 1;
                    continue;
                }
                break;
            }
            if !ok {
                break;
            }
            if let Some((arrow, label)) = pending.take() {
                for a in &prev {
                    for b in &group {
                        chart.edges.push(Edge {
                            from: a.clone(), to: b.clone(), kind: kind_of(&arrow), label: label.clone(),
                            both: arrow.starts_with('<'), directed: arrow.contains('>'),
                        });
                    }
                }
            }
            prev = group;
            c.ws();
            if c.peek().is_none() || c.peek() == Some(b';') {
                break;
            }
            match edge(&mut c) {
                Some(e) => pending = Some(e),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            chart.problems.push((line, format!("statement not understood: `{t}`")));
        }
    }
    chart.nodes = order.into_iter().map(|id| nodes.remove(&id).expect("ordered")).collect();
    Some(chart)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "```mermaid\nflowchart LR\n  %% atlas: arch | The pieces\n  owner([Owner]):::owner\n  subgraph game[\"OpenTTD (lockstep)\"]\n    srv[\"Dedicated server<br/>command system\"]\n  end\n  owner -->|watches,<br/>approves| srv\n  srv -.-> a & b\n  a == evidence ==> b\n  b -- reads --> owner\n  x <--> y\n  class a,b proposed\n  click srv \"src/command.cpp\" \"the command system\"\n```\n";

    #[test]
    fn reads_nodes_groups_edges_and_links() {
        let c = charts(SRC).remove(0);
        assert_eq!((c.id.as_str(), c.title.as_str()), ("arch", "The pieces"));
        assert!(c.problems.is_empty(), "{:?}", c.problems);
        let srv = c.nodes.iter().find(|n| n.id == "srv").unwrap();
        assert_eq!(srv.label, "Dedicated server\ncommand system");
        assert_eq!(srv.group.as_deref(), Some("game"));
        assert_eq!(srv.href.as_deref(), Some("src/command.cpp"));
        assert_eq!(srv.tooltip.as_deref(), Some("the command system"));
        assert_eq!(c.groups[0].label, "OpenTTD (lockstep)");
        let owner = c.nodes.iter().find(|n| n.id == "owner").unwrap();
        assert_eq!((owner.label.as_str(), owner.classes.clone()), ("Owner", vec!["owner".to_string()]));
        let kinds: Vec<_> = c.edges.iter().map(|e| (e.from.as_str(), e.to.as_str(), e.kind, e.label.as_str())).collect();
        assert_eq!(kinds, vec![
            ("owner", "srv", "control", "watches,\napproves"),
            ("srv", "a", "data", ""),
            ("srv", "b", "data", ""),
            ("a", "b", "governance", "evidence"),
            ("b", "owner", "control", "reads"),
            ("x", "y", "control", ""),
        ]);
        assert!(c.edges[5].both);
        assert!(c.nodes.iter().find(|n| n.id == "a").unwrap().classes.contains(&"proposed".to_string()));
    }

    #[test]
    fn unannotated_diagrams_are_not_read_and_bad_statements_are_findings() {
        assert!(charts("```mermaid\nflowchart LR\n a --> b\n```\n").is_empty());
        let c = charts("```mermaid\nflowchart LR\n %% atlas: x\n a --> \n```\n").remove(0);
        assert_eq!(c.problems.len(), 1);
    }

    #[test]
    fn labels_lose_markup() {
        assert_eq!(label_text("Tools &amp; resources<br/>read state · gated"), "Tools & resources\nread state · gated");
        assert_eq!(label_text("Command&lt;&gt;::Post"), "Command<>::Post");
    }
}
