// Rail monitor (RESEARCH-ONLY): watches the owner's bot's railway in the background and reports what
// is wrong to the Majordomus board, where every agent session reads it (majordomus_blackboard,
// majordomus_peers), as the named claim openttd:<instance>:rail. It also reacts to the game's own
// advice news (a lost or stuck vehicle, one that loses money, ...) as they appear in the owner's
// window. It observes and reports only: it issues no game command and approves nothing.
//
//   tools/research/mcp-rhai/target/release/mcp-rhai tools/research/mcp-panel/rhai/rail_monitor.rhai
//   or from the panel's Scripts page (/scripts) as a service.
//
// What it reports to the board (rail_lib.rhai problems), worst first:
//   crashed      a train crashed (a level crossing, a collision)
//   lost         the pathfinder found no way to the train's destination (openttd://vehicles lost):
//                a broken line or a turn the train may not take; such a train earns nothing
//   unserved     a line the bot built has no train
//   trainless_station  a rail station of the company no train calls at (a line that failed before
//                its train, or was given up half laid; rail_lib trainless_stations)
//   warning      the bot's own rail warnings since the last round: no rail path, a broken line, a
//                refused throat or piece, no depot, a locomotive not bought, a line given up (ai.log)
//   broken_down  a train stopped by a breakdown now (openttd://vehicles broken_down)
//   stopped      a train stopped outside a depot
//   old          a train past its maximum age (autorenew should have replaced it)
// It posts when the report changes, and at least every MONITOR_REPEAT_S (default 600) while
// problems last; MONITOR_EVERY_S (default 60) is the round. MCP_BOARD_DRY=1 prints instead.
//
// Advice (advice_lib.rhai), every ADVICE_EVERY_S (default 2) seconds: the news.shown events with an
// `advice` since the last look, grouped by advice and vehicle, each joined with its vehicle now
// (openttd://vehicles: type, unit, owner, orders and the line they make, profit, lost flag). For
// every company they are about it then
//   - keeps a tally on the shared blackboard as advice.<company> (0-based; every write is a
//     rhai.shared event, so it reaches the bus and the store): counts by advice, the newest groups;
//   - writes an observation record to the Majordomus mesh (task openttd:<instance>:advice:<company>)
//     when the panel has a mesh session ($MAJORDOMUS_URL, $MAJORDOMUS_SESSION); a failure is printed
//     once per distinct reason and never stops the monitor;
//   - says a short line in the game's chat for each new advice that still holds (advice_lib
//     actionable), at most ADVICE_SAY_MAX (default 3) a look, the same vehicle's advice again only
//     after MONITOR_REPEAT_S; only for the owner's company: $MONITOR_OWNER_COMPANY (0-based) when
//     set, else the company the window shows advice for (the game raises advice only for the
//     window's own company). MONITOR_CHAT=0 prints the lines instead of saying them.
// The panel's bus keeps its newest 5000 events (panel.rs BUS_KEEP), so a look every few seconds
// misses no advice while the bus carries fewer than about a thousand events a second.
import "rail_lib" as rl;
import "advice_lib" as al;

fn every_s() { let v = env("MONITOR_EVERY_S"); if v == "" { 60 } else { parse_int(v) } }
fn repeat_s() { let v = env("MONITOR_REPEAT_S"); if v == "" { 600 } else { parse_int(v) } }
fn advice_every_s() { let v = env("ADVICE_EVERY_S"); if v == "" { 2 } else { parse_int(v).max(1) } }
fn say_max() { let v = env("ADVICE_SAY_MAX"); if v == "" { 3 } else { parse_int(v) } }
fn owner_company() { let v = env("MONITOR_OWNER_COMPANY"); if v == "" { () } else { parse_int(v) } }
// How many advice groups a company's tally keeps beside its counts.
const KEEP_RECENT = 20;

fn instance() {
    let store = status().store ?? "";
    if store == "" { return "panel"; }
    let parts = store.split("/");
    let base = parts[parts.len() - 1];
    base.sub_string(0, base.index_of(".").max(0))
}

fn say(text) {
    // The console takes one line; quotes inside the message would end it early.
    text.replace("\"", "'");
    if env("MONITOR_CHAT") == "0" { print(`chat (not said, MONITOR_CHAT=0): ${text}`); return; }
    con_say(`"[monitor] ${text}"`);
}

// The rail round: the problems of every running PrismaticAI company and the board digest. Answers
// the line records it read too, so the advice can name the line a train runs on.
fn rail_round(me, since) {
    let state = read("openttd://game/state");
    let map_w = state.data.map_width;
    let vehicles = read("openttd://vehicles").data.vehicles;
    let stations = read("openttd://stations").data.stations;
    let all = events(0, "ai.log");
    let records = rl::records_from_events(all.events);
    // Warnings only since the last round, so one failure is reported once.
    let fresh = [];
    for e in all.events { if e.seq > since { fresh.push(e); } }
    let found = [];
    for bot in api_bots().running ?? [] {
        if bot.ai != "PrismaticAI" { continue; }
        let lines = rl::lines_of(bot.id, records, vehicles, stations, map_w);
        for p in rl::report(bot.id, lines, vehicles, rl::rail_warnings(fresh), stations) {
            p.text = `${bot.name}: ${p.text}`;
            found.push(p);
        }
    }
    #{ text: rl::digest(me, state.game_date, found, 4), found: found, records: records,
       map_w: map_w, since: all.last_seq }
}

let me = instance();
let since = 0;
let last = "";
let quiet = 0;
// Advice: only what appears after the monitor starts, so a restart does not repeat old news.
let advice_since = status().event_seq;
let said = #{};
let mesh_failed = "";
let records = [];
let map_w = 0;
// Seconds since the start, counted in looks; the rail round runs when it reaches next_round.
let clock = 0;
let next_round = 0;
print(`rail monitor on ${me}: a round every ${every_s()} s, reposted every ${repeat_s()} s while problems last; advice every ${advice_every_s()} s`);
loop {
    if clock >= next_round {
        try {
            let r = rail_round(me, since);
            since = r.since;
            records = r.records;
            map_w = r.map_w;
            // The report without its date, so the same problems a day later are not news.
            let key = r.text.sub_string(r.text.index_of(" rail: "));
            quiet += every_s();
            if key != last || (!r.found.is_empty() && quiet >= repeat_s()) {
                print(r.text);
                board_post(`openttd:${me}:rail`, r.text);
                last = key;
                quiet = 0;
            }
        } catch (err) {
            print(`rail monitor: ${err}`);
        }
        next_round = clock + every_s();
    }
    try {
        let feed = events(advice_since, "news");
        advice_since = feed.last_seq;
        let groups = al::group(al::items(feed.events));
        if !groups.is_empty() {
            let vehicles = read("openttd://vehicles").data.vehicles;
            let stations = read("openttd://stations").data.stations;
            let date = read("openttd://game/state").game_date;
            // The rail lines of every company the advice is about, for a train's line label.
            let owners = [];
            for g in groups {
                let v = al::vehicle_by_id(vehicles, g.vehicle);
                if v != () && !owners.contains(v.owner) { owners.push(v.owner); }
            }
            let lines = [];
            for c in owners { lines += rl::lines_of(c, records, vehicles, stations, map_w); }
            let enriched = [];
            for g in groups { enriched.push(al::enrich(g, vehicles, stations, lines)); }
            for c in al::companies(enriched) {
                let mine = enriched.filter(|g| g.owner == c);
                shared_set(`advice.${c}`, al::tally(shared_get(`advice.${c}`), c, mine, date, KEEP_RECENT));
                let rec = al::observation(me, c, mine, date);
                let body = [];
                for g in mine { body.push(al::summary(g)); }
                rec.body = to_json(body);
                print(`advice: ${rec.title}`);
                try {
                    mesh_record_write(rec);
                } catch (err) {
                    let why = `${err}`;
                    if why != mesh_failed { print(`advice: mesh record not written: ${why}`); mesh_failed = why; }
                }
            }
            let fresh = al::to_say(enriched, owner_company(), said, clock, repeat_s());
            for g in fresh { said[g.key] = clock; }
            for line in al::chat_lines(fresh, say_max()) { say(line); print(`said: ${line}`); }
        }
    } catch (err) {
        print(`advice: ${err}`);
    }
    sleep(advice_every_s() * 1000);
    clock += advice_every_s();
}
