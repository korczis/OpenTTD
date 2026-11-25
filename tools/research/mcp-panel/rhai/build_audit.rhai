// Build audit (RESEARCH-ONLY; research/mechanisms/rhai-scripting.md, research/mcp-hub.md "No
// duplicate buildings"): once a game month, judges what every company built, PrismaticAI and its
// rivals alike, and keeps the result where every agent reads it.
//
// Run it as a service on the Scripts page (/scripts/build_audit) or:
//   curl -X POST 127.0.0.1:8800/api/services/start -d '{"name":"build_audit"}'
//   tools/research/mcp-rhai/target/release/mcp-rhai tools/research/mcp-panel/rhai/build_audit.rhai
//
// What it judges, per company, with the rules hub.audit judges by (the built-in build_audit,
// tools/research/mcp-rhai/src/build_rules.rs), from openttd://stations, openttd://depots,
// openttd://map/ways and openttd://map only:
//   depots      by type, and whether a vehicle leaving one can drive on: open, dead_end (the
//               entrance leads nowhere else), no_exit (the entrance does not lead into it), unknown
//   duplicates  depots of one type within the game's service reach of each other (by road when the
//               ways are known), stations sharing a facility with overlapping catchments
//   stops       road stops drive-through against bay, and the drive-through share
// Every defect (no_exit, dead_end, duplicate_depot, duplicate_station, bay_stop) has a key that
// stays while the defect stays, so a defect is new only once.
//
// Where it goes: the panel's shared memory, one key per company, audit.<company> (company 0 is
// company 1 in the game); every write is a rhai.shared event on the bus and in the DuckDB store,
// where the view build_audit has one row per company and game month. A key of a company that no
// longer has anything to audit is deleted. One chat line, only when the owner's company
// (ttd::owner_company(), co-played with PrismaticAI) has a new defect; the first round after the panel's
// memory is empty is the baseline and says nothing.
//
// Read-only toward the game: it reads resources, writes the panel's memory and chats; it issues no
// command and approves nothing. BUILD_AUDIT_EVERY_S (default 20) is how often it looks at the date;
// BUILD_AUDIT_ROUNDS stops it after that many looks (0 or unset: never).

import "ttd" as ttd;

// Defects kept in full in a key (all are counted and keyed), and named in the chat line.
const KEEP = 30;
const SAY = 3;

fn every_s() { let v = env("BUILD_AUDIT_EVERY_S"); if v == "" { 20 } else { parse_int(v) } }
fn rounds() { let v = env("BUILD_AUDIT_ROUNDS"); if v == "" { 0 } else { parse_int(v) } }

// The items as one text, separated by "; ".
fn joined(items) {
    let out = "";
    for t in items { out += if out == "" { `${t}` } else { `; ${t}` }; }
    out
}

// The shared key of a company's audit.
fn key_of(company) { `audit.${company}` }

// Whether a shared key is one this service writes: audit.<digits>.
fn is_audit_key(key) {
    if !key.starts_with("audit.") { return false; }
    let id = key.sub_string(6);
    if id == "" { return false; }
    for ch in id.chars() { if ch < '0' || ch > '9' { return false; } }
    true
}

// One round, pure: the audit (build_audit's answer), the previous value of each company's key by
// company id as a string (() when absent), the audit keys the panel holds now, the owner's company
// and the game date. Answers the writes, the keys to delete, at most one chat line, and a summary.
fn round(audit, prev, held, owner, date) {
    let writes = [];
    let chat = ();
    let lines = [];
    let present = [];
    for c in audit.companies {
        let before = prev[`${c.company}`];
        let keys = [];
        for d in c.defects { keys.push(d.key); }
        let fresh = [];
        if type_of(before) != "()" {
            let had = before.keys ?? [];
            for d in c.defects { if !had.contains(d.key) { fresh.push(d); } }
        }
        let kept = c.defects.extract(0, global::KEEP.min(c.defects.len()));
        let value = #{ date: date, month: date.sub_string(0, 7), company: c.company,
                       depots: c.depots, exits: c.exits, stops: c.stops,
                       duplicate_depots: c.duplicate_depots, duplicate_stations: c.duplicate_stations,
                       defects: kept, defects_total: c.defects.len(), keys: keys,
                       new_keys: fresh.map(|d| d.key), baseline: type_of(before) == "()",
                       reach: audit.reach, network: audit.network, rails: audit.rails,
                       stops_known: audit.stops_known };
        writes.push(#{ key: key_of(c.company), value: value });
        present.push(key_of(c.company));
        lines.push(`company ${c.company + 1}: ${c.defects.len()} defect(s), ${fresh.len()} new`);
        if c.company == owner && !fresh.is_empty() {
            let named = fresh.extract(0, global::SAY.min(fresh.len())).map(|d| d.what);
            let more = if fresh.len() > global::SAY { ` (+${fresh.len() - global::SAY} more)` } else { "" };
            chat = `build audit, ${fresh.len()} new: ${joined(named)}${more}`;
        }
    }
    let deletes = [];
    for k in held { if is_audit_key(k) && !present.contains(k) { deletes.push(k); } }
    #{ writes: writes, deletes: deletes, chat: chat,
       summary: `build audit ${date}: ${if lines.is_empty() { "nothing built" } else { joined(lines) }}` }
}

fn say(text) {
    // The console takes one line; quotes inside the message would end it early.
    text.replace("\"", "'");
    con_say(`"[build-audit] ${text}"`);
}

let last = "";
let looks = 0;
print(`build audit: a look at the date every ${every_s()} s, an audit each new game month`);
loop {
    try {
        let date = read("openttd://game/state").game_date;
        if date.sub_string(0, 7) != last {
            let audit = build_audit(#{ stations: read("openttd://stations"), depots: read("openttd://depots"),
                                       ways: read("openttd://map/ways"), map: read("openttd://map") });
            let prev = #{};
            for c in audit.companies { prev[`${c.company}`] = shared_get(key_of(c.company)); }
            let r = round(audit, prev, shared_keys("audit."), ttd::owner_company(), date);
            for w in r.writes { shared_set(w.key, w.value); }
            for k in r.deletes { shared_delete(k); }
            if type_of(r.chat) != "()" { say(r.chat); }
            print(r.summary);
            last = date.sub_string(0, 7);
        }
    } catch (err) {
        print(`build audit: ${err}`);
    }
    looks += 1;
    if rounds() > 0 && looks >= rounds() { break; }
    sleep(every_s() * 1000);
}
