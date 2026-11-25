// stall: 300
// Leader-gap watch (RESEARCH-ONLY; research/control-loops-audit.md, research/mechanisms/rhai-scripting.md
// "The leader-gap watch"): the feedback loop from "we fall behind the leader" to the owner and to
// one bounded proposal. replan.rhai and watchdog.rhai measure; nothing reacted to what they found.
//
// Run it as a service on the Scripts page (/scripts/gap_watch) or:
//   curl -X POST 127.0.0.1:<panel>/api/services/start -d '{"name":"gap_watch"}'
//
// It re-runs no analysis another service does. It READS:
//   - replan.<company> (replan.rhai): our value, every competitor's value, and per transport type the
//     company earning most a vehicle (competition);
//   - watchdog.<company> (watchdog.rhai): whether the company is stalled and why;
//   - the store's named query company_samples: each month's value, money, loan and vehicles of us
//     and of the leader (already recorded by the panel; no game read);
//   - the panel bus (events after a cursor, at most PAGES pages a look): the bot's own log lines
//     (builds refused, failed or removed, levelling paid for nothing, loan repaid, producers held by
//     others), the approvals filed for the company and the decisions executed from them;
//   - the running bot's declared settings (api_bots, once a game month) and pending approvals.
// Once a game month, for every running company of AI, it classifies the month (flags below), keeps
// how many months in a row each flag held, and acts through the existing channels only:
//   1. the blackboard key gap.<company> (a rhai.shared event: on the bus and in the store), which
//      replan and Battlebrain can read; a blackboard write never acts on a game;
//   2. the owner: one console line and one news item when a flag has held two months, or the level
//      rises; never every month;
//   3. only when the owner started the panel with GAP_WATCH_PROPOSE=1 (observation only otherwise:
//      the levers are computed and recorded, nothing is filed), at most one openttd.ai.set_settings
//      proposal at a time: every lever in ONE approval, applied at once or not at all, expiring
//      after EXPIRE_DAYS. That tool is owner-only: the game server
//      takes its approval at the game's own console, never from the panel, Rhai or auto-approve.
//      The deadline policy that settles it is read from openttd://approvals when it is filed, and
//      named in the owner's line; this script approves nothing and never assumes the verdict.
// A lever only restores a setting the bot declares toward its declared default (or a 0/1 switch to
// 1), clamped to its declared range; it never names a freight-line slot, which is Battlebrain's
// decision type (rule project.decision-authority-boundaries, B2). A proposal's outcome (pending,
// applied, not applied) is checked every month against the bot's live values; the same proposal
// is not filed again once it went unapplied DECLINED times. Missing or stale inputs are the flag
// unknown, never "on track". The design and its review: research/control-loops-audit.md.
//
// Owned, bounded and stoppable (rule project.long-runs-are-owned-bounded-and-stoppable): at start it
// writes gap_watch.owner (who, since when, how to stop it). It adds no compute work to a host, so it
// reads no load or disk; its only host is the panel, whose shared memory holds its stop and pause
// switches (Rhai here has no file access, so there are no PAUSED or STOP files). Any session stops it with the shared key
// gap_watch.stop (the loop ends) or pauses it with gap_watch.pause (it still measures and records,
// but tells and proposes nothing): POST /api/rhai/shared {"key": "gap_watch.pause", "value": true}.
// POST /api/services/stop {"name": "gap_watch"} stops it too. GAP_WATCH_ROUNDS ends it after that
// many looks (0 or unset: never). It prints one line a look, so the supervisor's stall limit above
// is not reached while it works.
import "fr_author" as fa;

// The AI whose companies are watched: every running company of it (/api/bots).
const AI = "PrismaticAI";
// This service's name as a writer (planner.<company>, fr_author.rhai) and its shared keys' prefix.
const WRITER = "gap_watch";
const ROUND_S = 20;
// The bus kinds read, and how many pages of up to 500 events one look takes at most. The panel's
// /api/events envelopes carry no game date, so game.month events ({date: "YYYY-MM-01"}) mark the
// months: every other event counts for the month the last game.month opened.
const KINDS = "ai.log,panel.mcp,decision.recorded,game.month";
const PAGES = 10;
// Game months the trend is judged over, and the thresholds of the flags. SHARE_DROP is the cold
// start only: the threshold is the shared key derived.gap_watch.share_drop when it holds a value in
// 0..1 (the 75th percentile of 6-month drops of value / leader value over the store's runs,
// published by the self-evaluation service), and gap.<company>.thresholds names which one was used.
const WINDOW = 6;
const SHARE_DROP = 0.05;
const DERIVED_SHARE_DROP = "derived.gap_watch.share_drop";
const IDLE_FACTOR = 2;
const MIN_FILED = 10;
const LAND_RATE = 0.5;
const REPLAN_STALE_MONTHS = 2;
// Months a flag must hold before the owner is told (TELL) and before a lever is proposed (PERSIST),
// and the months between two proposals of this service.
const TELL = 2;
const PERSIST = 3;
const COOLDOWN_MONTHS = 3;
// A proposal expires after this many game days (ticks per day: TimerGameTick::DAY_TICKS); the server
// takes at most SET_SETTINGS_MAX settings in one (detail::SET_SETTINGS_MAX).
const EXPIRE_DAYS = 30;
const DAY_TICKS = 74;
const SET_SETTINGS_MAX = 32;
// The same proposal is not filed again after it went unapplied this many times.
const DECLINED = 2;

// The levers: a setting the bot declares, the flag that calls for it, the transport type whose
// competition must show the leader ahead and ours earning (() for none), the target ("default": the
// declared default; "max": the declared maximum, for a 0/1 switch), from_zero (only when the value
// is 0: a positive air_grow_money is a step size, and a higher one buys fewer routes), sites (held
// while this month's levelling was paid for nothing: more air routes would fail the same way), and
// why, with the bot code that reads it. Higher is more expansion for every one of them.
const LEVERS = [
    #{ name: "invest_borrow", flag: "unused_credit", type: (), to: "max", from_zero: false, sites: false,
       why: "InvestSurplus (fleet.nut) counts the unused loan only with invest_borrow=1; the leader plans against balance plus unused loan" },
    #{ name: "air_grow_money", flag: "losing_ground", type: "aircraft", to: "default", from_zero: true, sites: true,
       why: "AutopilotAir (autopilot.nut) buys one more air route per air_grow_money of idle money; 0 never grows past air_routes_max" },
    #{ name: "air_routes_max", flag: "losing_ground", type: "aircraft", to: "default", from_zero: false, sites: true,
       why: "AutopilotAir (autopilot.nut) runs at most air_routes_max own air routes" },
    #{ name: "invest_road_cap", flag: "losing_ground", type: "road", to: "default", from_zero: false, sites: false,
       why: "InvestSurplus (fleet.nut) adds road vehicles to a paying route up to invest_road_cap" },
    #{ name: "max_routes", flag: "losing_ground", type: "road", to: "default", from_zero: false, sites: false,
       why: "the bus routes the company may run (main.nut); raising it while the AI runs makes it expand" },
];

fn rounds() { let v = env("GAP_WATCH_ROUNDS"); if v == "" { 0 } else { parse_int(v) } }

// The owner's switch: proposals are filed only with GAP_WATCH_PROPOSE=1 in the panel's environment.
fn proposing() { env("GAP_WATCH_PROPOSE") == "1" }

// A company id as an integer, whichever number type the JSON gave.
fn id_of(x) { if type_of(x) == "f64" { x.to_int() } else { x } }

// A number as a float, () as 0.
fn num(x) { if type_of(x) == "()" { 0.0 } else if type_of(x) == "i64" { x.to_float() } else { x } }

// "1951-09-10" or "1951-09" -> months since year 0, so two dates subtract.
fn month_of(date) {
    let p = `${date}`.split("-");
    if p.len() < 2 { return 0; }
    parse_int(p[0]) * 12 + parse_int(p[1])
}

// The items as one text, separated by "; ".
fn joined(items) {
    let out = "";
    for t in items { out += if out == "" { `${t}` } else { `; ${t}` }; }
    out
}

// The integer that starts right after `marker` in `text` (digits only), () when there is none.
fn int_after(text, marker) {
    let at = text.index_of(marker);
    if at < 0 { return (); }
    let rest = text.sub_string(at + marker.len());
    let digits = "";
    for ch in rest.chars() { if ch >= '0' && ch <= '9' { digits += ch; } else { break; } }
    if digits == "" { () } else { parse_int(digits) }
}

// The integer that ends right before `marker` in `text`, () when there is none.
fn int_before(text, marker) {
    let at = text.index_of(marker);
    if at <= 0 { return (); }
    let digits = "";
    let i = at - 1;
    while i >= 0 {
        let ch = text.sub_string(i, 1);
        if ch >= "0" && ch <= "9" { digits = ch + digits; i -= 1; } else { break; }
    }
    if digits == "" { () } else { parse_int(digits) }
}

// An empty month of one company's tally.
fn empty_tally() {
    #{ refused: 0, failed: 0, removed: 0, frozen: 0, no_route: 0, wasted_levelling: 0, repaid: 0,
       claimed_by_others: (), filed: [], executed: [] }
}

// "1953-04-01" -> "1953-04", "" when it is no date.
fn month_text(date) { let d = `${date ?? ""}`; if d.len() >= 7 { d.sub_string(0, 7) } else { "" } }

// One bus event into the tallies (company -> month -> tally), pure apart from `t`: the event's own
// game date when it has one (the store's envelopes), else `current`, the month the bus is in. The
// bot's lines are its own wording (PrismaticAI's AILog.Info texts); a line it rewords counts nothing.
fn tally_event(t, ev, current) {
    let d = ev.data ?? #{};
    let month = month_text(ev.game_date);
    if month == "" { month = current; }
    if month.len() >= 7 { month = month.sub_string(0, 7); } else { return t; }
    let company = ();
    let text = "";
    if ev.kind == "ai.log" {
        company = id_of(d.company);
        text = `${d.text ?? ""}`;
        if !text.starts_with("PrismaticAI") { return t; }
    } else if ev.kind == "panel.mcp" {
        text = `${d.text ?? ""}`;
        if !text.contains("openttd.ai.set_setting") || !text.contains("pending approval #") { return t; }
        company = int_after(text, "\"company\": ");
    } else if ev.kind == "decision.recorded" {
        company = "*";
    }
    if type_of(company) == "()" { return t; }
    let key = `${company}`;
    if !(key in t) { t[key] = #{}; }
    let m = t[key];
    if !(month in m) { m[month] = empty_tally(); }
    let s = m[month];
    if ev.kind == "ai.log" {
        if text.contains(" refused") { s.refused += 1; }
        if text.contains(" failed: ") { s.failed += 1; }
        if text.contains("; removing them") { s.removed += 1; }
        if text.contains("construction frozen") { s.frozen += 1; }
        if text.contains("no freight route built") { s.no_route += 1; }
        if text.contains("still refused") { s.wasted_levelling += int_after(text, " for ") ?? 0; }
        if text.contains("repaid ") { s.repaid += int_after(text, "repaid ") ?? 0; }
        let held = int_before(text, " claimed by another company's");
        if type_of(held) != "()" { s.claimed_by_others = held; }
    } else if ev.kind == "panel.mcp" {
        let id = int_after(text, "pending approval #");
        if type_of(id) != "()" { s.filed.push(id); }
    } else {
        let id = id_of(d.approval_id);
        if type_of(id) == "i64" && `${d.command_result ?? ""}` == "succeeded" { s.executed.push(id); }
    }
    m[month] = s;
    t[key] = m;
    t
}

// The plans of a company: each month's filed approvals as one batch, with the executed ones among
// them (executed ids from any month), and whether the batch is settled (none of it still pending).
fn plan_batches(company_months, executed_all, pending_ids) {
    let out = [];
    for month in company_months.keys() {
        let filed = company_months[month].filed;
        if filed.is_empty() { continue; }
        let done = 0; let open = 0;
        for id in filed {
            if executed_all.contains(id) { done += 1; }
            if pending_ids.contains(id) { open += 1; }
        }
        out.push(#{ month: month, filed: filed.len(), executed: done, pending: open, settled: open == 0,
                    partial: open == 0 && done > 0 && done < filed.len() });
    }
    out
}

// The newest run's monthly rows of a company from company_samples (instance, seed, run_ms, tick,
// date, value, money, loan, net, vehicles, ...), the last `months` of them, oldest first.
fn series_of(rows, months) {
    let newest = 0;
    for r in rows { if r[2] > newest { newest = r[2]; } }
    let out = [];
    for r in rows {
        if r[2] == newest {
            out.push(#{ month: `${r[4]}`.sub_string(0, 7), value: num(r[5]), money: num(r[6]),
                        loan: num(r[7]), vehicles: num(r[9]) });
        }
    }
    if out.len() > months { out = out.extract(out.len() - months); }
    out
}

// The leader: the competitor of highest value above ours in replan's record, () when we lead.
fn leader_of(rp) {
    let best = ();
    for c in rp.competitors ?? [] {
        if num(c.value) > num(rp.value) && (best == () || num(c.value) > num(best.value)) { best = c; }
    }
    best
}

// A setting of the running bot by name: its declaration (value, min, max, default, step), or ().
fn setting(bot, name) {
    for s in bot.settings ?? [] { if s.name == name { return s; } }
    ()
}

// The month's measures, pure: replan's and watchdog's keys, our and the leader's monthly series, the
// bot, this month's tally, the plan batches, the month judged.
fn measure(rp, wd, ours, theirs, bot, tally, batches, month, partial_seen) {
    let leader = if type_of(rp) == "map" { leader_of(rp) } else { () };
    let share = [];
    let by_month = #{};
    for r in theirs { by_month[r.month] = r; }
    for r in ours {
        let l = by_month[r.month];
        if type_of(l) != "()" && l.value > 0.0 { share.push(#{ month: r.month, share: r.value / l.value }); }
    }
    let first = if ours.is_empty() { () } else { ours[0] };
    let last = if ours.is_empty() { () } else { ours[ours.len() - 1] };
    let ceiling = 0.0;
    for r in ours { if r.loan > ceiling { ceiling = r.loan; } }
    for r in theirs { if r.loan > ceiling { ceiling = r.loan; } }
    let reserve = num((setting(bot, "cash_reserve") ?? #{}).value);
    let budget = num((setting(bot, "line_budget") ?? #{}).value);
    let idle_cash = if last == () { 0.0 } else { last.money - reserve };
    let credit = if last == () { 0.0 } else { (ceiling - last.loan).max(0.0) };
    let leader_loan = if theirs.is_empty() { 0.0 } else { theirs[theirs.len() - 1].loan };
    let filed = 0; let executed = 0; let partial = [];
    for b in batches {
        if !b.settled { continue; }
        if month_of(month) - month_of(b.month) < global::WINDOW { filed += b.filed; executed += b.executed; }
        if b.partial && !partial_seen.contains(b.month) { partial.push(`${b.month}: ${b.executed} of ${b.filed}`); }
    }
    #{ month: month,
       leader: if leader == () { () } else { #{ company: leader.company, name: leader.name, value: leader.value } },
       value: if type_of(rp) == "map" { rp.value } else { () },
       share: share,
       share_drop: if share.len() < 2 { 0.0 } else { share[0].share - share[share.len() - 1].share },
       vehicles_change: if first == () { 0.0 } else { last.vehicles - first.vehicles },
       leader_vehicles_change: if theirs.len() < 2 { 0.0 } else { theirs[theirs.len() - 1].vehicles - theirs[0].vehicles },
       months: ours.len(),
       money: if last == () { () } else { last.money }, loan: if last == () { () } else { last.loan },
       loan_ceiling_observed: ceiling, leader_loan: leader_loan, cash_reserve: reserve, line_budget: budget,
       idle_cash: idle_cash, unused_credit: credit, leader_months: theirs.len(),
       stall: if type_of(wd) == "map" { #{ stalled: wd.stalled ?? false, quiet_days: wd.quiet_days, threshold_days: wd.threshold_days, reasons: wd.reasons ?? [] } } else { () },
       builds: tally,
       plans: #{ filed: filed, executed: executed, partial: partial, partial_months: batches.filter(|b| b.partial).map(|b| b.month) },
       replan_date: if type_of(rp) == "map" { rp.date } else { () },
       competition: if type_of(rp) == "map" { rp.competition ?? [] } else { [] } }
}

// A ratio as a whole percentage.
fn pct(x) { (x * 100.0).round().to_int() }

// The share-drop threshold, pure: the derived key's value (a number, or a map with value or p75)
// when it lies in 0..1, else SHARE_DROP; with its source.
fn thresholds_of(derived) {
    let v = derived;
    if type_of(v) == "map" { v = v.value ?? v.p75; }
    if type_of(v) == "i64" { v = v.to_float(); }
    if type_of(v) == "f64" && v >= 0.0 && v <= 1.0 { return #{ share_drop: v, source: global::DERIVED_SHARE_DROP }; }
    #{ share_drop: global::SHARE_DROP, source: "SHARE_DROP (cold start)" }
}

// The month's flags, pure, each with its reason.
fn flags_of(m) {
    let out = #{};
    let drop_at = (m.thresholds ?? #{}).share_drop ?? global::SHARE_DROP;
    if m.leader != () && m.months >= 2 && (m.share_drop >= drop_at || (m.leader_vehicles_change > 0.0 && m.vehicles_change <= 0.0)) {
        let now = if m.share.is_empty() { "unknown" } else { `${pct(m.share[m.share.len() - 1].share)} %` };
        out.losing_ground = `${now} of ${m.leader.name}'s value (${if m.share_drop >= 0.0 { "down" } else { "up" }} ${pct(m.share_drop.abs())} points); vehicles ${m.vehicles_change.to_int()} against their ${m.leader_vehicles_change.to_int()} over ${m.months} month(s)`;
    }
    if m.months >= 2 && m.line_budget > 0.0 && m.idle_cash >= global::IDLE_FACTOR.to_float() * m.line_budget && m.vehicles_change <= 0.0 {
        out.idle_capital = `${m.idle_cash.to_int()} cash above the reserve (money ${m.money.to_int()}) is ${global::IDLE_FACTOR}x the line budget ${m.line_budget.to_int()} or more, and the fleet did not grow`;
    }
    if m.months >= 2 && m.line_budget > 0.0 && m.unused_credit >= m.line_budget && m.leader_loan > m.loan && m.vehicles_change <= 0.0 {
        out.unused_credit = `loan ${m.loan.to_int()} of at least ${m.loan_ceiling_observed.to_int()} (the highest seen), while ${if m.leader == () { "the leader" } else { m.leader.name }} borrows ${m.leader_loan.to_int()}; the fleet did not grow`;
    }
    if type_of(m.stall) == "map" && m.stall.stalled == true {
        out.stalled = `watchdog: nothing built for ${m.stall.quiet_days} day(s): ${joined(m.stall.reasons)}`;
    }
    if m.plans.filed >= global::MIN_FILED && m.plans.executed.to_float() < global::LAND_RATE * m.plans.filed.to_float() {
        out.plans_not_landing = `${m.plans.executed} of ${m.plans.filed} proposed setting(s) were executed in ${global::WINDOW} month(s)`;
    }
    let b = m.builds;
    if b.wasted_levelling > 0 || b.removed > 0 || !m.plans.partial.is_empty() {
        let parts = [];
        if b.wasted_levelling > 0 { parts.push(`${b.wasted_levelling} spent levelling sites that were still refused`); }
        if b.removed > 0 { parts.push(`${b.removed} build(s) removed again`); }
        if !m.plans.partial.is_empty() { parts.push(`plans applied in part: ${joined(m.plans.partial)}`); }
        out.collateral = joined(parts);
    }
    if m.replan_date == () || month_of(m.month) - month_of(m.replan_date) >= global::REPLAN_STALE_MONTHS {
        out.replan_silent = `replan.<company> last written ${m.replan_date ?? "never"}`;
    }
    if m.months < 2 || (m.leader != () && m.leader_months < 2) || m.replan_date == () {
        out.unknown = `inputs missing: ${m.months} month(s) of our samples, ${m.leader_months} of the leader's, replan ${m.replan_date ?? "never"}`;
    }
    out
}

// The streaks (flag -> months in a row) after this month, from the previous ones.
fn streaks_of(prev, flags) {
    let out = #{};
    for k in flags.keys() { out[k] = (prev[k] ?? 0) + 1; }
    out
}

// The levers this month calls for, pure: each lever whose flag held PERSIST months, whose transport
// type (if any) the leader wins while ours earn, and whose declared target is above its value now.
fn levers_of(bot, streaks, competition, builds) {
    let out = [];
    for l in global::LEVERS {
        if (streaks[l.flag] ?? 0) < global::PERSIST { continue; }
        if type_of(l.type) != "()" {
            let row = ();
            for c in competition { if c.type == l.type { row = c; } }
            if row == () || row.we_lead == true || num(row.ours_per_vehicle) <= 0.0 { continue; }
        }
        if l.sites && builds.wasted_levelling > 0 { continue; }
        let s = setting(bot, l.name);
        if s == () || s.editable == false { continue; }
        if l.from_zero && s.value != 0 { continue; }
        let target = if l.to == "max" { s.max } else { s["default"] };
        if type_of(target) == "()" { continue; }
        if type_of(s.max) != "()" && target > s.max { target = s.max; }
        if type_of(s.min) != "()" && target < s.min { target = s.min; }
        if s.value >= target { continue; }
        out.push(#{ name: l.name, from: s.value, to: target, why: l.why });
    }
    if out.len() > global::SET_SETTINGS_MAX { out = out.extract(0, global::SET_SETTINGS_MAX); }
    out
}

// The level: 0 on track, 1 a flag, 2 a flag held TELL months, 3 a lever to propose.
fn level_of(streaks, levers) {
    if !levers.is_empty() { return 3; }
    let top = 0;
    for k in streaks.keys() { if streaks[k] > top { top = streaks[k]; } }
    if top >= global::TELL { 2 } else if top > 0 { 1 } else { 0 }
}

// Why a proposal is held now, () when it may be filed: paused, another set_settings pending for the
// company, our last one within COOLDOWN_MONTHS, or another planner holds the company.
fn hold_of(company, month, levers, pending, prev_proposal, paused, refusal, propose_on, declined) {
    if levers.is_empty() { return "no lever applies"; }
    if !propose_on { return "observation only: proposals need GAP_WATCH_PROPOSE=1 in the panel's environment (the owner's switch)"; }
    if paused { return "paused (gap_watch.pause)"; }
    let names = levers.map(|l| l.name);
    for p in pending {
        let a = p.args ?? #{};
        if id_of(a.company) != company { continue; }
        if p.tool == "openttd.ai.set_settings" {
            return `approval #${p.id} (openttd.ai.set_settings) still waits for the owner`;
        }
        if p.tool == "openttd.ai.set_setting" && names.contains(a.name) {
            return `approval #${p.id} (openttd.ai.set_setting ${a.name}) still waits for the owner`;
        }
    }
    if type_of(prev_proposal) == "map" && prev_proposal.signature == signature_of(levers) && declined >= global::DECLINED {
        return `the same proposal went unapplied ${declined} time(s)`;
    }
    if type_of(prev_proposal) == "map" && month_of(month) - month_of(prev_proposal.month) < global::COOLDOWN_MONTHS {
        return `the last proposal (${prev_proposal.month}) is within ${global::COOLDOWN_MONTHS} month(s)`;
    }
    if type_of(refusal) != "()" { return refusal.why; }
    ()
}

// One text naming a set of levers and targets, to tell one proposal from another.
fn signature_of(levers) { joined(levers.map(|l| `${l.name}=${l.to}`)) }

// A filed proposal's outcome now, pure: pending while its approval waits, applied when the bot's
// live values equal every setting it carried, else not_applied.
fn outcome_of(proposal, bot, pending_ids) {
    if type_of(proposal.id) != "()" && pending_ids.contains(proposal.id) { return "pending"; }
    for k in proposal.settings.keys() {
        let s = setting(bot, k);
        if s == () || s.value != proposal.settings[k] { return "not_applied"; }
    }
    "applied"
}

// The owner's line for a month, () when nothing new is worth saying: a flag held TELL months that
// was not said yet, or a higher level than last month.
fn tell_of(name, flags, streaks, level, prev) {
    let said = prev.said ?? [];
    let fresh = [];
    for k in flags.keys() { if streaks[k] >= global::TELL && !said.contains(k) { fresh.push(k); } }
    if fresh.is_empty() && level <= (prev.level ?? 0) { return (); }
    if level < 2 { return (); }
    let parts = [];
    for k in flags.keys() { if streaks[k] >= global::TELL { parts.push(`${k} (${streaks[k]} mo): ${flags[k]}`); } }
    `${name}: ${joined(parts)}`
}

fn say(text) {
    // The console takes one line; quotes inside the message would end it early.
    text.replace("\"", "'");
    con_say(`"[gap-watch] ${text}"`);
}

fn tell(text) {
    say(text);
    try { api_newsShow(#{ text: `[gap-watch] ${text}` }); } catch (err) { print(`gap_watch: news: ${err}`); }
}

// The store's monthly rows of a company (named query company_samples), [] when the store is off.
fn samples(company) {
    let r = api_dbNamedRun(#{ name: "company_samples", params: #{ company: company } });
    if "error" in r || "http_error" in r { print(`gap_watch: store: ${r.error ?? r.http_error}`); return []; }
    r.rows ?? []
}

// The deadline policy in force and the game's tick, read when a proposal is filed
// (openttd://approvals and its envelope): {ticks, days, verdict, game_tick}; the verdict is never
// assumed.
fn deadline_policy() {
    // A try block is a statement, not a value: the answer is assigned inside it.
    let p = #{ verdict: "unknown", game_tick: () };
    try {
        let a = read("openttd://approvals");
        p = (a.data ?? a).deadline ?? p;
        p.game_tick = a.game_tick;
    } catch (err) { p.error = `${err}`; }
    p
}

// File the levers as one openttd.ai.set_settings approval for `company`.
fn propose(company, month, levers) {
    let settings = #{};
    for l in levers { settings[l.name] = l.to; }
    let policy = deadline_policy();
    let tick = policy.game_tick;
    policy.remove("game_tick");
    let expires = if type_of(tick) == "i64" { tick + global::EXPIRE_DAYS * global::DAY_TICKS } else { () };
    let out = #{ month: month, settings: settings, signature: signature_of(levers), expires_tick: expires,
                 deadline: policy, id: (), error: (), outcome: "pending", counted: false };
    try {
        let args = #{ company: company, settings: settings };
        if expires != () { args.expires_tick = expires; }
        let r = api_tool_openttd_ai_set_settings(args);
        // The tool answers {approval_id, status: "pending_approval", message, ...}; an older answer
        // only said "pending approval #N" in its text.
        out.id = if type_of(r) == "map" && "approval_id" in r { id_of(r.approval_id) } else { () };
        if out.id == () { out.id = int_after(to_json(r), "approval #") ?? int_after(to_json(r), "\"approval_id\":"); }
        out.answer = r;
    } catch (err) {
        out.error = `${err}`;
    }
    out
}

// One company's month: measure, classify, record; tell the owner and propose when due.
fn month_of_company(b, month, tallies, executed_all, pending, paused) {
    let key = `gap.${b.id}`;
    let prev = shared_get(key) ?? #{};
    // A month already judged (the bus replayed after a restart) is not judged again.
    if type_of(prev.month) == "string" && month_of(month) <= month_of(prev.month) { return prev; }
    let rp = shared_get(`replan.${b.id}`);
    let wd = shared_get(`watchdog.${b.id}`);
    let ours = series_of(samples(b.id), global::WINDOW);
    let leader = if type_of(rp) == "map" { leader_of(rp) } else { () };
    let theirs = if leader == () { [] } else { series_of(samples(id_of(leader.company)), global::WINDOW) };
    let months = tallies[`${b.id}`] ?? #{};
    let tally = months[month] ?? empty_tally();
    let pending_ids = [];
    for p in pending { pending_ids.push(p.id); }
    let batches = plan_batches(months, executed_all, pending_ids);
    let m = measure(rp, wd, ours, theirs, b, tally, batches, month, prev.partial_seen ?? []);
    m.thresholds = thresholds_of(shared_get(global::DERIVED_SHARE_DROP));
    let flags = flags_of(m);
    let streaks = streaks_of(prev.streaks ?? #{}, flags);
    let levers = levers_of(b, streaks, m.competition, tally);
    let level = level_of(streaks, levers);
    let proposal = prev.proposal;
    let filed_now = false;
    let declined = prev.declined ?? 0;
    if type_of(proposal) == "map" {
        proposal.outcome = outcome_of(proposal, b, pending_ids);
        if proposal.outcome == "not_applied" && proposal.counted != true { declined += 1; proposal.counted = true; }
        if proposal.outcome == "applied" { declined = 0; }
    }
    let held = hold_of(b.id, month, levers, pending, proposal, paused, fa::refusal(b.id, global::WRITER), proposing(), declined);
    if level == 3 && held == () {
        let same = type_of(proposal) == "map" && proposal.signature == signature_of(levers);
        proposal = propose(b.id, month, levers);
        if !same { declined = 0; }
        if proposal.error == () {
            filed_now = true;
            tell(`${b.name}: ${levers.len()} setting(s) proposed as one approval${if proposal.id == () { "" } else { ` #${proposal.id}` }} for the owner at the game's console (settled at its deadline as '${proposal.deadline.verdict}'): ${joined(levers.map(|l| `${l.name} ${l.from} -> ${l.to}`))}`);
        } else {
            held = `the proposal failed: ${proposal.error}`;
        }
    }
    let said = prev.said ?? [];
    // The month a proposal is filed, its line is the owner's line.
    let line = if paused || filed_now { () } else { tell_of(b.name, flags, streaks, level, prev) };
    if type_of(line) != "()" {
        tell(line);
        said = [];
        for k in flags.keys() { if streaks[k] >= global::TELL { said.push(k); } }
    }
    // A flag that cleared may be said again when it returns.
    let kept = [];
    for k in said { if k in flags { kept.push(k); } }
    let value = m;
    value.company = b.id; value.name = b.name; value.flags = flags; value.streaks = streaks;
    value.level = level; value.levers = levers; value.held = held; value.proposal = proposal;
    value.said = kept; value.paused = paused; value.partial_seen = m.plans.partial_months; value.declined = declined; value.proposing = proposing();
    value.reads = [`replan.${b.id}`, `watchdog.${b.id}`, global::DERIVED_SHARE_DROP, "store:company_samples", "bus:" + global::KINDS, "api_bots", "status.pending"];
    shared_set(key, value);
    print(`gap_watch ${month} company ${b.id + 1}: level ${level}, flags ${flags.keys()}, ${if held == () { "proposed" } else { `held: ${held}` }}`);
    value
}

// Judge `month` (just closed) for every running company of AI; answers the tallies kept (WINDOW
// months before `next`).
fn judge(month, next, tallies) {
    let executed_all = [];
    for mo in (tallies["*"] ?? #{}).values() { executed_all += mo.executed; }
    let pending = status().pending ?? [];
    let paused = shared_get("gap_watch.pause") == true;
    for b in api_bots().running ?? [] {
        if b.ai != global::AI { continue; }
        try { month_of_company(b, month, tallies, executed_all, pending, paused); }
        catch (err) { print(`gap_watch: company ${b.id + 1}: ${err}`); }
    }
    for k in tallies.keys() {
        let mo = tallies[k];
        for m in mo.keys() { if month_of(next) - month_of(m) > global::WINDOW { mo.remove(m); } }
        tallies[k] = mo;
    }
    tallies
}

// --- the loop --------------------------------------------------------------------------------------
let looks = 0;
let cursor = 0;
let tallies = #{};
// The game month the bus is in: "" until the first game.month event seen.
let current = "";
shared_set("gap_watch.owner", #{ service: "gap_watch", writer: WRITER, session: env("CLAUDE_CODE_SESSION_ID"),
    majordomus_session: env("MAJORDOMUS_SESSION"), started_look: 0,
    stop: "POST /api/rhai/shared {key: gap_watch.stop, value: true}, or POST /api/services/stop {name: gap_watch}",
    pause: "POST /api/rhai/shared {key: gap_watch.pause, value: true}" });
print(`gap_watch: a look every ${ROUND_S} s; once a game month it judges every running ${AI} company against the leader`);
loop {
    if shared_get("gap_watch.stop") == true { print("gap_watch: stopped by gap_watch.stop"); break; }
    try {
        let pages = 0;
        while pages < PAGES {
            let r = events(cursor, KINDS);
            let evs = r.events ?? [];
            for ev in evs {
                if ev.seq > cursor { cursor = ev.seq; }
                if ev.kind == "game.month" {
                    // The month that just closed is judged when the bus opens the next one.
                    let next = month_text((ev.data ?? #{}).date);
                    if current != "" && next != "" && next != current { tallies = judge(current, next, tallies); }
                    if next != "" { current = next; }
                } else {
                    tallies = tally_event(tallies, ev, current);
                }
            }
            pages += 1;
            if evs.len() < 500 { break; }
        }
        print(`gap_watch: look ${looks + 1}, bus seq ${cursor}, month ${if current == "" { "not seen yet" } else { current }}`);
    } catch (err) {
        print(`gap_watch: ${err}`);
    }
    looks += 1;
    if rounds() > 0 && looks >= rounds() { break; }
    sleep(ROUND_S * 1000);
}
