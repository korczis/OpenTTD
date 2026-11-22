//! RESEARCH-ONLY. The Research Atlas projection (tools/research/atlas/README.md).
//!
//! Usage: atlas data
//!        atlas render [--check]
//!        atlas --help
//!
//! `data` prints the research-atlas/v1 contract on stdout: every duel and A/B record with its
//! comparability, every LAN battle record's headline and the newest one in full, every Mermaid
//! flowchart marked `%% atlas:` as a typed graph, and the evidence cards. The site's Atlas page reads
//! it (`surfaces.toml` `[[pages.items]]`), and so may any other tool.
//! `render` writes what the repository itself shows from the same projection: the README region
//! between `<!-- atlas:begin -->` and `<!-- atlas:end -->`, and the hero illustrations
//! `docs/atlas/hero-light.svg` and `docs/atlas/hero-dark.svg`. `--check` writes nothing and exits 11
//! while any of them would change.
//! Exit codes: 0 ok, 2 usage, 10 findings (a graph that does not parse or links to nothing, a
//! record that does not read), 11 drift.

mod flowchart;
mod hero;
mod records;
mod repo;

use serde_json::{json, Value};
use std::path::PathBuf;

const EXIT_USAGE: i32 = 2;
const EXIT_FINDINGS: i32 = 10;
const EXIT_DRIFT: i32 = 11;
const SCHEMA: &str = "research-atlas/v1";
const BEGIN: &str = "<!-- atlas:begin -->";
const END: &str = "<!-- atlas:end -->";
const STYLE: &str = "tools/research/noirgfx/style.toml";
const HERO_LIGHT: &str = "docs/atlas/hero-light.svg";
const HERO_DARK: &str = "docs/atlas/hero-dark.svg";
/// The page the site publishes the atlas as (`surfaces.toml` `[[pages.items]] id`).
const PAGE: &str = "atlas.html";

fn usage() -> &'static str {
    "usage: atlas data | atlas render [--check] | atlas --help"
}

fn root() -> PathBuf {
    // tools/research/atlas -> the repository
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("repository root")
}

/// Every flowchart marked for the atlas, from every tracked Markdown document, as graph values;
/// findings name the document and line.
fn graphs(repo: &repo::Repo) -> (Vec<Value>, Vec<String>) {
    let mut out = vec![];
    let mut findings = vec![];
    for path in repo.files.iter().filter(|p| p.ends_with(".md") && !p.starts_with("src/3rdparty/")) {
        let Ok(text) = std::fs::read_to_string(repo.root.join(path)) else { continue };
        if !text.contains("%% atlas:") {
            continue;
        }
        let base = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for chart in flowchart::charts(&text) {
            for (line, problem) in &chart.problems {
                findings.push(format!("{path}:{line}: atlas graph {}: {problem}", chart.id));
            }
            let mut nodes = vec![];
            for n in &chart.nodes {
                // A node's link is relative to its document, like any Markdown link; it must resolve.
                let href = n.href.as_ref().map(|h| {
                    if h.contains("://") {
                        h.clone()
                    } else {
                        let (file, anchor) = h.split_once('#').map(|(f, a)| (f, Some(a))).unwrap_or((h.as_str(), None));
                        let joined = normalise(if base.is_empty() { file.to_string() } else { format!("{base}/{file}") });
                        if !repo.has(&joined) {
                            findings.push(format!("{path}:{}: atlas graph {}: node {} links to {h}, which is not a tracked file", n.line, chart.id, n.id));
                        }
                        match anchor {
                            Some(a) => format!("{joined}#{a}"),
                            None => joined,
                        }
                    }
                });
                if href.is_none() {
                    findings.push(format!("{path}:{}: atlas graph {}: node {} has no `click {} \"<path>\"` line", n.line, chart.id, n.id, n.id));
                }
                let status = if n.classes.iter().any(|c| c == "proposed") { "proposed" } else { "implemented" };
                nodes.push(json!({
                    "id": n.id, "label": n.label, "group": n.group, "classes": n.classes,
                    "href": href, "tooltip": n.tooltip, "status": status, "line": n.line,
                }));
            }
            let groups: Vec<Value> = chart.groups.iter().map(|g| json!({"id": g.id, "label": g.label, "parent": g.parent})).collect();
            let edges: Vec<Value> = chart.edges.iter().map(|e| json!({
                "from": e.from, "to": e.to, "kind": e.kind, "label": e.label, "both": e.both, "directed": e.directed,
            })).collect();
            out.push(json!({
                "id": chart.id, "title": chart.title, "source": path, "line": chart.line,
                "nodes": nodes, "groups": groups, "edges": edges,
            }));
        }
    }
    (out, findings)
}

/// `a/./b/../c` -> `a/c`.
fn normalise(p: String) -> String {
    let mut parts: Vec<&str> = vec![];
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn count(rows: &[Value], f: impl Fn(&Value) -> bool) -> usize {
    rows.iter().filter(|r| f(r)).count()
}

fn link(label: &str, href: &str) -> Value {
    json!({"label": label, "href": href})
}

/// The evidence cards: each value, what it is, how to read it, and the chain that proves it
/// (value -> dataset -> experiment -> source -> validation), plus the atlas view that explores it.
fn cards(rows: &[Value], battle: &Value, graphs: &[Value]) -> Vec<Value> {
    let duels: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "duel").collect();
    let abs = count(rows, |r| r["kind"] == "ab");
    let valid: Vec<&Value> = rows.iter().filter(|r| r["class"] == "valid").collect();
    let ahead = valid.iter().filter(|r| r["a_ahead"] == true).count();
    let no_rail = duels.iter().filter(|r| r["b"]["trains"] == 0).count();
    let fleet = duels.iter().filter(|r| !r["b"]["trains"].is_null()).count();
    let audit = "research/duel-evidence-audit.md";
    let held_rule = ".ai/repo/rules/project/a-tuning-claim-needs-held-start-pairs.v1.md";
    let mut out = vec![
        json!({
            "id": "records", "value": duels.len().to_string(), "label": "duel records",
            "detail": format!("plus {abs} A/B records: each a separate experiment with its own seed, length and bot revision, not one sample"),
            "view": "?view=explorer",
            "chain": [link("dataset", "research/results/"), link("experiment", "research/mechanisms/duels.md"),
                      link("source", "tools/research/mcp-duel"), link("validation", audit)],
        }),
        json!({
            "id": "valid", "value": format!("{} of {}", valid.len(), rows.len()),
            "label": "records that are paired comparisons",
            "detail": "one game, a held start, every month recorded: the only rows a tuning claim may rest on".to_string(),
            "view": "?view=explorer&class=valid",
            "chain": [link("dataset", "research/results/"), link("experiment", audit),
                      link("source", "tools/research/mcp-duel"), link("validation", held_rule)],
        }),
        json!({
            "id": "ahead", "value": format!("{} of {}", ahead, valid.len()),
            "label": "paired records where the planned bot ended ahead",
            "detail": "per record, across seeds, lengths and bot revisions; a tally, not an effect size".to_string(),
            "view": "?view=explorer&class=valid",
            "chain": [link("dataset", "research/results/"), link("experiment", audit),
                      link("source", "tools/research/mcp-duel"), link("validation", held_rule)],
        }),
        json!({
            "id": "rival", "value": format!("{} of {}", no_rail, fleet),
            "label": "duels whose rival ran no train",
            "detail": format!("counted over the duels that record the rival's fleet ({} of {} do not): the planned railway mostly met a rival on buses or autopilot, not another railway", duels.len() - fleet, duels.len()),
            "view": "?view=explorer&confounder=rival+ran+no+train",
            "chain": [link("dataset", "research/results/"), link("experiment", audit),
                      link("source", "tools/research/mcp-duel"), link("validation", audit)],
        }),
    ];
    if let Some(latest) = battle.get("latest").filter(|v| !v.is_null()) {
        let board = latest["leaderboard"].as_array().cloned().unwrap_or_default();
        let best = board.iter().filter(|r| r["ai"] == "PrismaticAI").min_by(|x, y| {
            x["avg_rank"].as_f64().partial_cmp(&y["avg_rank"].as_f64()).unwrap_or(std::cmp::Ordering::Equal)
        });
        let winner = board.first();
        if let (Some(best), Some(winner)) = (best, winner) {
            out.push(json!({
                "id": "battle", "value": format!("{}", best["avg_rank"]),
                "label": "PrismaticAI's best average rank, newest LAN battle",
                "detail": format!("{} of {} contestants over {} matches; {} won with {} points per match",
                    best["key"].as_str().unwrap_or(""), board.len(), latest["matches"],
                    winner["key"].as_str().unwrap_or(""), winner["avg_points"]),
                "view": "?view=battle",
                "chain": [link("dataset", latest["path"].as_str().unwrap_or("")), link("experiment", latest["page"].as_str().unwrap_or("")),
                          link("source", "tools/research/ai-battle/battle.py"),
                          link("validation", ".ai/repo/rules/project/prismatic-ai-is-never-last.v1.md")],
            }));
        }
        let det = latest["determinism"].as_array().cloned().unwrap_or_default();
        let same = det.iter().filter(|r| r["identical"] == true).count();
        out.push(json!({
            "id": "replays", "value": format!("{} of {}", same, det.len()),
            "label": "battle replays identical on another arena",
            "detail": if same < det.len() { "a match does not yet replay the same on another machine: a standing is per host until it does" } else { "every replay matched its original" },
            "view": "?view=battle",
            "chain": [link("dataset", latest["path"].as_str().unwrap_or("")), link("experiment", latest["page"].as_str().unwrap_or("")),
                      link("source", "tools/research/ai-battle/battle.py"),
                      link("validation", ".ai/repo/rules/project/battle-records-are-reproducible.v1.md")],
        }));
    }
    if let Some(g) = graphs.iter().find(|g| g["id"] == "architecture") {
        let n = g["nodes"].as_array().map(Vec::len).unwrap_or(0);
        let e = g["edges"].as_array().map(Vec::len).unwrap_or(0);
        out.push(json!({
            "id": "graph", "value": format!("{n} / {e}"), "label": "parts and connections of the architecture",
            "detail": "drawn from the README's own diagram; every part opens its page, every connection has a kind".to_string(),
            "view": "?view=graph",
            "chain": [link("dataset", g["source"].as_str().unwrap_or("")), link("experiment", "research/mechanisms/README.md"),
                      link("source", "tools/research/atlas/src/flowchart.rs"),
                      link("validation", ".ai/repo/rules/project/published-surfaces-are-derived.v1.md")],
        }));
    }
    out
}

struct Projection {
    data: Value,
    findings: Vec<String>,
}

fn project(repo: &repo::Repo) -> Result<Projection, String> {
    let rows = records::comparisons(repo)?;
    let known = repo.known_commits(&rows.revisions);
    let mut list = rows.rows;
    for r in &mut list {
        let rev = r["revision"].as_str().map(str::to_string);
        r["revision_known"] = json!(rev.is_some_and(|x| known.contains(&x)));
    }
    let battle = records::battles(repo)?;
    let (graphs, findings) = graphs(repo);
    let cards = cards(&list, &battle, &graphs);
    let classes: Vec<Value> = records::COMPARABILITY.iter().map(|(id, meaning)| json!({"id": id, "meaning": meaning})).collect();
    let kinds: Vec<Value> = flowchart::KINDS.iter().map(|(id, stroke, arrow, meaning)| json!({"id": id, "stroke": stroke, "arrow": arrow, "meaning": meaning})).collect();
    let style = std::fs::read_to_string(repo.root.join(STYLE)).map_err(|e| format!("{STYLE}: {e}"))?;
    let pal = hero::palette(&style)?;
    let data = json!({
        "schema": SCHEMA,
        "palette": {"ink": pal.ink, "paper": pal.paper, "signal": pal.signal, "signal_dark": pal.signal_dark},
        "generator": "tools/research/atlas",
        "sources": {
            "comparisons": ["research/results/duel-*.json", "research/results/ab-*.json"],
            "battles": "research/results/battle-*.json",
            "graphs": "tracked Markdown: ```mermaid flowcharts carrying %% atlas:",
            "palette": STYLE,
        },
        "comparability": classes,
        "edge_kinds": kinds,
        "comparisons": list,
        "battles": battle,
        "graphs": graphs,
        "cards": cards,
    });
    Ok(Projection { data, findings })
}

fn md_link(label: &str, href: &str) -> String {
    format!("[{label}]({href})")
}

/// The README region: the evidence cards as a table, the comparability split as a pie, and the
/// legend of the architecture graph's edge kinds.
fn readme_region(p: &Value, pages: Option<&str>) -> String {
    let explore = |view: &str| pages.map(|u| format!("{u}{PAGE}{view}"));
    let mut s = vec![
        BEGIN.to_string(),
        "<!-- generated by tools/research/atlas render; edit the records or the tool, not this block -->".to_string(),
        String::new(),
        "| | What it counts | How to read it | Evidence |".to_string(),
        "|---:|---|---|---|".to_string(),
    ];
    for c in p["cards"].as_array().into_iter().flatten() {
        let chain: Vec<String> = c["chain"].as_array().into_iter().flatten()
            .map(|l| md_link(l["label"].as_str().unwrap_or(""), l["href"].as_str().unwrap_or(""))).collect();
        let mut ev = chain.join(" → ");
        if let Some(u) = explore(c["view"].as_str().unwrap_or("")) {
            ev.push_str(&format!(" · [explore]({u})"));
        }
        s.push(format!("| **{}** | {} | {} | {} |", c["value"].as_str().unwrap_or(""), c["label"].as_str().unwrap_or(""),
            c["detail"].as_str().unwrap_or("").replace('|', "/"), ev));
    }
    let rows = p["comparisons"].as_array().cloned().unwrap_or_default();
    let mut slices = vec![];
    let mut links = vec![];
    for (id, _) in records::COMPARABILITY {
        let n = count(&rows, |r| r["class"] == id);
        if n > 0 {
            slices.push(format!("    \"{id}\" : {n}"));
            if let Some(u) = explore(&format!("?view=explorer&class={id}")) {
                links.push(u);
            }
        }
    }
    s.push(String::new());
    s.push("```mermaid".to_string());
    s.push(format!("pie showData title Duel and A/B records by comparability ({} records)", rows.len()));
    if links.len() == slices.len() && !links.is_empty() {
        s.push(format!("    %% links: {}", links.join(" | ")));
    }
    s.extend(slices);
    s.push("```".to_string());
    s.push(String::new());
    let classes: Vec<String> = records::COMPARABILITY.iter().map(|(id, m)| format!("**{id}**: {m}")).collect();
    s.push(format!("Classes, derived per record by [`atlas`](tools/research/atlas/README.md): {}.", classes.join("; ")));
    s.push(String::new());
    s.push(END.to_string());
    s.join("\n")
}

fn render(repo: &repo::Repo, p: &Value, check: bool) -> Result<bool, String> {
    let readme_path = repo.root.join("README.md");
    let text = std::fs::read_to_string(&readme_path).map_err(|e| format!("README.md: {e}"))?;
    let (Some(b), Some(e)) = (text.find(BEGIN), text.find(END)) else {
        return Err(format!("README.md: markers {BEGIN} / {END} missing"));
    };
    let pages = repo.pages_url();
    let new = format!("{}{}{}", &text[..b], readme_region(p, pages.as_deref()), &text[e + END.len()..]);
    let style = std::fs::read_to_string(repo.root.join(STYLE)).map_err(|e| format!("{STYLE}: {e}"))?;
    let pal = hero::palette(&style)?;
    let outputs = [
        (readme_path, new),
        (repo.root.join(HERO_LIGHT), hero::svg(&pal, false)),
        (repo.root.join(HERO_DARK), hero::svg(&pal, true)),
    ];
    let mut stale = false;
    for (path, want) in outputs {
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if have != want {
            stale = true;
            if check {
                eprintln!("atlas: {} is stale (run tools/research/atlas/atlas render)", path.strip_prefix(&repo.root).unwrap_or(&path).display());
            } else {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                std::fs::write(&path, want).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
    }
    Ok(stale)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{}", include_str!("main.rs").lines().skip(1).take_while(|l| l.starts_with("//!")).map(|l| l.trim_start_matches("//!").strip_prefix(' ').unwrap_or("")).collect::<Vec<_>>().join("\n"));
        return;
    }
    let cmd = args.first().map(String::as_str);
    let check = match (cmd, &args[1..]) {
        (Some("data"), []) => false,
        (Some("render"), []) => false,
        (Some("render"), [f]) if f == "--check" => true,
        _ => {
            eprintln!("atlas: {}", usage());
            std::process::exit(EXIT_USAGE);
        }
    };
    let repo = match repo::Repo::load(root()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("atlas: {e}");
            std::process::exit(EXIT_FINDINGS);
        }
    };
    let p = match project(&repo) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("atlas: {e}");
            std::process::exit(EXIT_FINDINGS);
        }
    };
    for f in &p.findings {
        eprintln!("FAIL atlas {f}");
    }
    if !p.findings.is_empty() {
        std::process::exit(EXIT_FINDINGS);
    }
    if cmd == Some("data") {
        println!("{}", serde_json::to_string(&p.data).expect("json"));
        return;
    }
    match render(&repo, &p.data, check) {
        Ok(stale) if check && stale => std::process::exit(EXIT_DRIFT),
        Ok(_) => {}
        Err(e) => {
            eprintln!("atlas: {e}");
            std::process::exit(EXIT_FINDINGS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalise;

    #[test]
    fn normalises_relative_links() {
        assert_eq!(normalise("research/../tools/./gate.sh".to_string()), "tools/gate.sh");
    }
}
