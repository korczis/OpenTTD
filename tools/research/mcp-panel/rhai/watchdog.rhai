// stall: 180
// Progress watchdog (RESEARCH-ONLY; research/mechanisms/rhai-scripting.md "The progress watchdog"):
// no company may go on for long without building or buying anything while nobody notices why.
//
// Run it as a service on the Scripts page (/scripts/watchdog) or:
//   curl -X POST 127.0.0.1:8800/api/services/start -d '{"name":"watchdog"}'
//
// Every ROUND_S seconds, for every company, from the engine alone (openttd://companies fleet,
// openttd://stations owners, openttd://game/state game_tick):
//   1. Its progress signature: vehicles by type and stations. A change is progress (built, bought,
//      sold or removed something); the game days since the last change are its quiet time.
//   2. The threshold, derived and self-tuned: the quiet times between changes this watchdog has
//      seen, over every company, kept on the blackboard (watchdog.tuning). Until it has seen
//      MIN_GAPS of them the threshold is START_DAYS (owner, 2026-11-22: "zacneme treba tyden");
//      then it is FACTOR times their 90th percentile, kept within MIN_DAYS..MAX_DAYS. A company
//      quiet longer than the threshold is stalled.
//   3. A stalled company gets a diagnosis from what holds it, each read, none guessed: the
//      approvals pending for it (status().pending), replan's hold (replan.<company>.held on the
//      blackboard), its money against the bot's line_budget setting, and every panel service the
//      supervisor reports stalled or given up (api_services(), when the panel has the supervisor).
// Where it goes: the blackboard, one key per company, watchdog.<company> (a rhai.shared event,
// so on the bus and in the DuckDB store), and one chat line when the owner's company (ttd::owner_company(),
// co-played with PrismaticAI) becomes stalled, naming the reasons. A stall is said once, until the
// company makes progress again.
//
// Read-only toward the game: it reads resources, writes the blackboard and chats; it issues no
// command and approves nothing. An approval it finds waiting is settled by the server's deadline
// (research/mechanisms/approval-gate.md "The deadline"), never here. The first line of this file
// declares to the panel's supervisor how long it may stay quiet before it is restarted.
// WATCHDOG_ROUNDS stops it after that many rounds (0 or unset: never).

import "ttd" as ttd;

const ROUND_S = 20;
// The threshold, in game days: the first one, its bounds, and how it follows the data.
const START_DAYS = 7;
const MIN_DAYS = 7;
const MAX_DAYS = 365;
const FACTOR = 1.5;
const MIN_GAPS = 5;
const KEEP_GAPS = 64;
// Ticks per game day (TimerGameTick::DAY_TICKS).
const DAY_TICKS = 74;

fn rounds() { let v = env("WATCHDOG_ROUNDS"); if v == "" { 0 } else { parse_int(v) } }

// The items as one text, separated by "; ".
fn joined(items) {
    let out = "";
    for t in items { out += if out == "" { `${t}` } else { `; ${t}` }; }
    out
}

// The progress signature of a company: its vehicles by type and its stations, as one text.
fn signature(fleet, stations) {
    `train ${fleet.train ?? 0} road ${fleet.road ?? 0} ship ${fleet.ship ?? 0} aircraft ${fleet.aircraft ?? 0} stations ${stations}`
}

// The threshold from the quiet times seen so far (game days): START_DAYS until MIN_GAPS of them,
// then FACTOR times their 90th percentile within MIN_DAYS..MAX_DAYS.
fn threshold_of(gaps) {
    if gaps.len() < global::MIN_GAPS { return global::START_DAYS; }
    let sorted = gaps;
    sorted.sort();
    let at = ((sorted.len() - 1) * 9) / 10;
    let t = (sorted[at] * global::FACTOR).to_int();
    if t < global::MIN_DAYS { global::MIN_DAYS } else if t > global::MAX_DAYS { global::MAX_DAYS } else { t }
}

// What holds a company, from what the panel and the blackboard say: pending approvals for it,
// replan's hold, money against the line budget, services the supervisor reports stalled.
fn reasons(company, pending, replan, money, line_budget, services) {
    let out = [];
    let mine = [];
    for p in pending { if (p.args ?? #{}).company == company { mine.push(p); } }
    if !mine.is_empty() { out.push(`${mine.len()} approval(s) wait for the owner (first #${mine[0].id})`); }
    if type_of(replan) == "map" && type_of(replan.held) != "()" && replan.held != false && replan.held != "" {
        out.push(`replan holds: ${replan.held}`);
    }
    if line_budget > 0 && money < line_budget { out.push(`money ${money} is below the line budget ${line_budget}`); }
    for s in services {
        if s.stalled == true { out.push(`service ${s.name} is stalled`); }
        if s.gave_up == true { out.push(`service ${s.name} was given up`); }
    }
    if out.is_empty() { out.push("nothing that holds it is visible; the bot itself chose to build nothing"); }
    out
}

// One company's round, pure: its previous key (() at first), its signature now, the day now and the
// threshold. Answers the key's new value and the quiet time that just ended (() when none).
fn assess(prev, sig, day, threshold) {
    if type_of(prev) == "()" || prev.signature != sig {
        let ended = if type_of(prev) == "()" { () } else { day - prev.since_day };
        return #{ value: #{ signature: sig, since_day: day, quiet_days: 0, threshold_days: threshold,
                            stalled: false, said: false, reasons: [] },
                  ended: ended };
    }
    let quiet = day - prev.since_day;
    #{ value: #{ signature: sig, since_day: prev.since_day, quiet_days: quiet, threshold_days: threshold,
                 stalled: quiet > threshold, said: prev.said ?? false, reasons: prev.reasons ?? [] },
       ended: () }
}

fn say(text) {
    // The console takes one line; quotes inside the message would end it early.
    text.replace("\"", "'");
    con_say(`"[watchdog] ${text}"`);
}

fn setting_of(bot, name) {
    for s in bot.settings ?? [] { if s.name == name { return s.value; } }
    0
}

let looks = 0;
print(`watchdog: a look every ${ROUND_S} s; a company with no progress for longer than the threshold is stalled`);
loop {
    try {
        let state = read("openttd://game/state");
        let day = state.game_tick / DAY_TICKS;
        let tuning = shared_get("watchdog.tuning") ?? #{ gaps: [] };
        let gaps = tuning.gaps ?? [];
        let threshold = threshold_of(gaps);
        let stations = read("openttd://stations").data.stations ?? [];
        let pending = status().pending ?? [];
        let services = [];
        try { services = api_services().services ?? []; } catch { }
        let bots = #{};
        for b in api_bots().running ?? [] { bots[`${b.id}`] = b; }
        let owner = ttd::owner_company();
        for c in read("openttd://companies").data.companies ?? [] {
            let owned = 0;
            for s in stations { if s.owner == c.id { owned += 1; } }
            let key = `watchdog.${c.id}`;
            let r = assess(shared_get(key), signature(c.fleet ?? #{}, owned), day, threshold);
            if type_of(r.ended) != "()" && r.ended > 0 { gaps.push(r.ended); }
            let v = r.value;
            v.company = c.id;
            v.name = c.name;
            v.date = state.game_date;
            if v.stalled {
                let bot = bots[`${c.id}`];
                let budget = if type_of(bot) == "()" { 0 } else { setting_of(bot, "line_budget") };
                v.reasons = reasons(c.id, pending, shared_get(`replan.${c.id}`), c.money, budget, services);
                if c.id == owner && !v.said {
                    say(`${c.name}: nothing built or bought for ${v.quiet_days} game day(s) (threshold ${threshold}): ${joined(v.reasons)}`);
                    v.said = true;
                }
            }
            shared_set(key, v);
        }
        if gaps.len() > KEEP_GAPS { gaps = gaps.extract(gaps.len() - KEEP_GAPS); }
        shared_set("watchdog.tuning", #{ gaps: gaps, threshold_days: threshold_of(gaps), date: state.game_date });
    } catch (err) {
        print(`watchdog: ${err}`);
    }
    looks += 1;
    if rounds() > 0 && looks >= rounds() { break; }
    sleep(ROUND_S * 1000);
}
