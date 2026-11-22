// The strategist and the blackboard for scripts (RESEARCH-ONLY): small helpers over the panel's
// routes GET /api/strategy, POST /api/strategy/run, GET and POST /api/blackboard
// (src/panel_strategy.rs, research/mechanisms/strategist-and-blackboard.md), which a script reaches
// as api_strategy, api_strategyRun, api_blackboard and api_blackboardWrite. The strategist owns
// strategy.* and mesh.*; a script writes any other key and reads every key. The pure functions at
// the end (lines, warnings, headline) take the board's values and are checked offline by
// `cargo test` in tools/research/mcp-rhai (main.rs tests).
//
//   import "strategy_lib" as sl;
//   let s = sl::strategy();
//   print(sl::headline(s.summary));
//   for l in sl::lines(s.plan) { print(l); }
//   sl::put("rail.hint", #{ prefer: "coal" }, "my_script");

/// The strategist's state and the current strategy (GET /api/strategy).
fn strategy() { api_strategy() }

/// Run one cycle now and wait for it: the cycle's record (POST /api/strategy/run).
fn run_now() { api_strategyRun(#{ wait: true }) }

/// The current value of a blackboard key, or () when it is absent.
fn get(key) {
    let all = api_blackboard(#{ prefix: key, history: 0 });
    for e in all.entries { if e.key == key { return e.value; } }
    ()
}

/// Every entry under a prefix (without history): [#{key, value, by, revision, written_at, game_date}].
fn entries(prefix) { api_blackboard(#{ prefix: prefix, history: 0 }).entries }

/// One key with its history, newest first, or () when it is absent.
fn history(key) {
    let all = api_blackboard(#{ prefix: key });
    for e in all.entries { if e.key == key { return e.history; } }
    ()
}

/// Write a key as `by`; answers the entry. strategy.* and mesh.* are the strategist's and refused.
fn put(key, value, by) {
    if is_reserved(key) { throw `${key}: strategy.* and mesh.* are the strategist's`; }
    api_blackboardWrite(#{ key: key, value: value, by: by })
}

/// What the peers on the Majordomus mesh plan (mesh.peers): records and session intents.
fn peers() { get("mesh.peers") }

// ---- pure: board values in, text out -----------------------------------------------------------

/// Whether a key is in a namespace only the strategist writes.
fn is_reserved(key) { key.starts_with("strategy.") || key.starts_with("mesh.") }

/// One line per chosen investment of strategy.plan (or strategy.investment): "mode from -> to
/// score S", the cargo first when there is one.
fn lines(plan) {
    let out = [];
    if type_of(plan) != "map" || !("portfolio" in plan) { return out; }
    for c in plan.portfolio.chosen {
        let what = if c.cargo == () { "" } else { `${c.cargo} ` };
        out.push(`${what}${c.mode} ${c.from} -> ${c.to} score ${c.score}`);
    }
    out
}

/// Every town warning of strategy.towns, optionally of one company (() for all): "company C: town
/// (rating R): first advice".
fn warnings(towns, company) {
    let out = [];
    if type_of(towns) != "map" || !("companies" in towns) { return out; }
    for c in towns.companies {
        if company != () && c.company != company { continue; }
        for w in c.warnings {
            let rating = if w.rating == () { "unknown" } else { `${w.rating}` };
            let first = if w.advice.len() > 0 { w.advice[0].what } else { "no advice" };
            out.push(`company ${c.company}: ${w.town} (rating ${rating}): ${first}`);
        }
    }
    out
}

/// strategy.summary as one sentence.
fn headline(summary) {
    if type_of(summary) != "map" || !("cycle" in summary) { return "no strategy yet"; }
    let solver = if summary.solver == () { "no solver" } else { summary.solver };
    `cycle ${summary.cycle} at ${summary.game_date}: ${summary.lines} lines, score ${summary.total_score}, by ${solver}; ${summary.town_warnings} town warning(s)`
}
