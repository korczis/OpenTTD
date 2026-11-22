// The game's advice as data (RESEARCH-ONLY): pure functions over the advice news the owner's window
// shows, so the monitor (rail_monitor.rhai) reacts to them and `cargo test` in tools/research/mcp-rhai
// checks the reading offline (main.rs tests, advice_*). Nothing here calls the game or the panel.
//
// Inputs, all derived from the live game:
//   events     news.shown events (events(since, "news")): the panel publishes every item the GUI
//              client shows (panel.rs watch_news). An advice item carries `advice`, its AdviceType
//              (news_type.h: vehicle_lost, train_stuck, vehicle_unprofitable, vehicle_old, order,
//              autorenew_failed, refit_failed, vehicle_waiting, aircraft_destination_too_far), and
//              `refs`, the objects it names ({vehicle, unit, owner}, {station}, {tile, x, y},
//              {industry}, {town}, {engine}); src/prismatic_mcp/prismatic_mcp_view.cpp news.list.
//   vehicles   openttd://vehicles .vehicles (id, unit, type, owner, tile, x, y, lost, in_depot,
//              stopped, age, max_age, profit_last_year, profit_this_year, orders)
//   stations   openttd://stations .stations (id, name)
//   lines      rail_lib::lines_of: the bot's rail lines with their trains, for a line's label
// The game raises advice only for the window's own company (vehicle.cpp: `owner == _local_company`),
// so the advice the panel sees is the owner's company's. Each function says why in words.

import "rail_lib" as rl;

/// What an advice says about its vehicle, as a player reads it; an advice this list does not know
/// yet reads as its own name.
fn what(advice) {
    let w = #{
        vehicle_lost: "lost",
        train_stuck: "stuck",
        vehicle_unprofitable: "unprofitable",
        vehicle_old: "old",
        order: "has an order problem",
        autorenew_failed: "failed to autorenew",
        refit_failed: "failed to refit",
        vehicle_waiting: "waits in a depot",
        aircraft_destination_too_far: "cannot reach its next airport",
    };
    let out = w[advice];
    if out != () { return out; }
    let t = `${advice}`;
    t.replace("_", " ");
    t
}

/// A vehicle type (openttd://vehicles `type`) as a word.
fn type_word(t) {
    if t == "train" { "train" } else if t == "road" { "road vehicle" } else if t == "ship" { "ship" }
    else if t == "aircraft" { "aircraft" } else { "vehicle" }
}

/// Every advice item in `events` (oldest first, as events() returns them), flat: #{seq, advice,
/// text, date} with each ref's fields beside them (vehicle, unit, owner, station, tile, x, y,
/// industry, town, engine), the first ref winning a field two refs both name.
fn items(events) {
    let out = [];
    for e in events {
        if e.kind != "news.shown" || e.data == () { continue; }
        let d = e.data;
        if d.advice == () { continue; }
        let it = #{ seq: e.seq, advice: d.advice, text: d.text ?? "", date: d.date ?? "" };
        for r in d.refs ?? [] {
            for k in r.keys() { if it[k] == () { it[k] = r[k]; } }
        }
        out.push(it);
    }
    out
}

/// One advice about one vehicle is one thing to act on, however often the game repeats it; an item
/// that names no vehicle is told apart by its text.
fn key_of(it) {
    if it.vehicle != () { `${it.advice}:${it.vehicle}` } else { `${it.advice}:${it.text}` }
}

/// The items grouped by advice and vehicle, in the order each group first appeared: the newest
/// item's fields, with `key`, `count` (items in the group) and `first_seq`.
fn group(items) {
    let by = #{};
    let order = [];
    for it in items {
        let k = key_of(it);
        let g = by[k];
        if g == () {
            g = it;
            g.key = k;
            g.count = 0;
            g.first_seq = it.seq;
            order.push(k);
        }
        g.count += 1;
        g.seq = it.seq;
        g.date = it.date;
        g.text = it.text;
        by[k] = g;
    }
    let out = [];
    for k in order { out.push(by[k]); }
    out
}

/// The vehicle with id `id` in openttd://vehicles, or () when it is gone (sold, replaced).
fn vehicle_by_id(vehicles, id) {
    if id == () { return (); }
    for v in vehicles { if v.id == id { return v; } }
    ()
}

/// The line a vehicle serves: the label of the bot's rail line that runs it (rail_lib lines), else
/// the names of the stations its orders visit, "A - B"; () for a vehicle without station orders.
fn line_of(v, stations, lines) {
    if v == () { return (); }
    for l in lines {
        for t in l.trains { if t.id == v.id { return l.label; } }
    }
    let names = [];
    for id in rl::order_stations(v) {
        let s = rl::station_by_id(stations, id);
        let n = if s == () || s.name == () { `station ${id}` } else { s.name };
        if !names.contains(n) { names.push(n); }
    }
    if names.is_empty() { return (); }
    names.reduce(|a, b| if a == "" { b } else { a + " - " + b }, "")
}

/// What to look at for an advice, in a few words: where a lost or stuck vehicle is now (the
/// vehicle's own tile, else the item's), what an unprofitable one lost, how old an old one is.
fn hint(g, v) {
    let at = if v != () { `tile ${v.tile} (${v.x}, ${v.y})` }
             else if g.tile != () { `tile ${g.tile} (${g.x}, ${g.y})` } else { "its last position" };
    let a = g.advice;
    let t = if v == () { () } else { v.type };
    if a == "vehicle_lost" {
        if t == "train" { return `track broken near ${at}?`; }
        if t == "road" { return `road broken near ${at}?`; }
        if t == "ship" { return `no water route from ${at}?`; }
        if t == "aircraft" { return `no airport it can reach from ${at}?`; }
        return `no route from ${at}?`;
    }
    if a == "train_stuck" { return `held near ${at}: a signal, a full platform or a one-way block?`; }
    if a == "vehicle_unprofitable" && v != () { return `lost ${-v.profit_last_year} last year, ${v.profit_this_year} this year`; }
    if a == "vehicle_old" && v != () { return `${v.age} days old, built for ${v.max_age}: renew it?`; }
    if a == "autorenew_failed" { return "autorenew failed: money short, or no newer engine to renew with?"; }
    if a == "refit_failed" { return "refit at its stop failed: is that cargo offered there?"; }
    if a == "vehicle_waiting" { return `waits in the depot near ${at}: start it or sell it?`; }
    if a == "aircraft_destination_too_far" { return "its next airport is beyond its range"; }
    g.text
}

/// Whether the advice still holds, judged on the vehicle now, so the owner hears only what can
/// still be acted on: a vehicle that is gone needs nothing; a lost one only while the pathfinder
/// still finds no way (`lost`); an unprofitable one while last year's profit is below zero; an old
/// one while it is past its maximum age; a waiting one while it is in a depot. Every other advice
/// (stuck, an order, a failed autorenew or refit, a destination too far) holds when it is raised.
fn actionable(g, v) {
    if v == () { return false; }
    let a = g.advice;
    if a == "vehicle_lost" { return v.lost == true; }
    if a == "vehicle_unprofitable" { return v.profit_last_year < 0; }
    if a == "vehicle_old" { return v.max_age > 0 && v.age > v.max_age; }
    if a == "vehicle_waiting" { return v.in_depot == true; }
    true
}

/// A group joined with its vehicle now: type, unit, owner (the vehicle's, else the item's ref),
/// the line it serves, its tile, profits, lost flag, the hint and whether it is actionable.
fn enrich(g, vehicles, stations, lines) {
    let v = vehicle_by_id(vehicles, g.vehicle);
    let out = g;
    out.type = if v == () { () } else { v.type };
    out.unit = if v == () { g.unit } else { v.unit ?? g.unit };
    out.owner = if v == () { g.owner } else { v.owner };
    out.line = line_of(v, stations, lines);
    if v != () {
        out.tile = v.tile;
        out.x = v.x;
        out.y = v.y;
        out.lost = v.lost;
        out.profit_last_year = v.profit_last_year;
        out.profit_this_year = v.profit_this_year;
    }
    out.exists = v != ();
    out.hint = hint(g, v);
    out.actionable = actionable(g, v);
    out
}

/// The companies the groups are about, in order of first appearance; a group whose owner is not
/// known is left out.
fn companies(groups) {
    let out = [];
    for g in groups { if g.owner != () && !out.contains(g.owner) { out.push(g.owner); } }
    out
}

/// What one group keeps on the blackboard and in the mesh: the fields that say what happened and
/// where, without the raw refs.
fn summary(g) {
    #{ key: g.key, advice: g.advice, vehicle: g.vehicle, unit: g.unit, type: g.type, owner: g.owner,
       line: g.line, tile: g.tile, x: g.x, y: g.y, count: g.count, date: g.date, seq: g.seq,
       actionable: g.actionable, hint: g.hint, text: g.text }
}

/// The per-company tally `prev` (the blackboard's `advice.<company>`, or () for a first one)
/// with `groups` of that company added: `total` advice items, `by_advice` counts, and `recent`, the
/// newest `keep` groups (one per key, the newest replacing an older one), and the date and seq.
fn tally(prev, company, groups, date, keep) {
    let t = if prev == () { #{ company: company, total: 0, by_advice: #{}, recent: [] } } else { prev };
    let by = t.by_advice ?? #{};
    let recent = t.recent ?? [];
    for g in groups {
        if g.owner != company { continue; }
        t.total = (t.total ?? 0) + g.count;
        by[g.advice] = (by[g.advice] ?? 0) + g.count;
        let k = g.key;
        recent = recent.filter(|r| r.key != k);
        recent.push(summary(g));
        t.seq = g.seq;
    }
    while recent.len() > keep { recent.remove(0); }
    t.by_advice = by;
    t.recent = recent;
    t.company = company;
    t.date = date;
    t
}

/// "2 vehicle_lost, 1 train_stuck": the groups of `company` counted by advice, in first appearance.
fn counts(groups, company) {
    let n = #{};
    let order = [];
    for g in groups {
        if g.owner != company { continue; }
        if n[g.advice] == () { n[g.advice] = 0; order.push(g.advice); }
        n[g.advice] += g.count;
    }
    let parts = [];
    for a in order { parts.push(`${n[a]} ${a}`); }
    parts.reduce(|a, b| if a == "" { b } else { a + ", " + b }, "")
}

/// The Majordomus mesh record for a company's new advice, without its body (the caller adds
/// `body: to_json(...)`, a host function): kind observation, task openttd:<instance>:advice:<company>.
fn observation(instance, company, groups, date) {
    #{ kind: "observation", task: `openttd:${instance}:advice:${company}`, scope: [],
       title: `OpenTTD ${instance} ${date}: company ${company + 1} advice: ${counts(groups, company)}` }
}

/// One chat line for a group: "train 8 lost on line COAL A - B: track broken near tile 1234 (18, 19)?"
fn chat_line(g) {
    let who = `${type_word(g.type)} ${g.unit ?? g.vehicle ?? ""}`;
    let on = if g.line == () { "" } else { ` on line ${g.line}` };
    let times = if g.count > 1 { ` (${g.count} times)` } else { "" };
    `${who} ${what(g.advice)}${on}: ${g.hint}${times}`
}

/// The groups the owner should hear about now: actionable, of the owner's company (`owner` (),
/// any: the window shows advice only for its own company), and not said within `repeat_s` seconds
/// before `now_s` (`said` maps a key to the time it was said).
fn to_say(groups, owner, said, now_s, repeat_s) {
    let out = [];
    for g in groups {
        if !g.actionable { continue; }
        if owner != () && g.owner != owner { continue; }
        let last = said[g.key];
        if last != () && now_s - last < repeat_s { continue; }
        out.push(g);
    }
    out
}

/// The chat lines for `groups`, at most `max`, the rest folded into one line that counts them.
fn chat_lines(groups, max) {
    let out = [];
    for i in 0..groups.len().min(max) { out.push(chat_line(groups[i])); }
    if groups.len() > max { out.push(`and ${groups.len() - max} more advice: see the panel's advice view`); }
    out
}
