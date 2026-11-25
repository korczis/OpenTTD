// stall: 120
// Strategy digest (RESEARCH-ONLY; research/mechanisms/strategy-digest.md): tells the owner, on a
// fixed cadence of game time, how the strategy is doing and what the system is changing, and
// escalates when it is failing. The owner must not learn that PrismaticAI is losing only by asking.
//
// Run it as a service on the Scripts page (/scripts/digest) or:
//   curl -X POST 127.0.0.1:<panel-port>/api/services/start -d '{"name":"digest"}'
//
// Every ROUND_S seconds it reads the game date. When the date crosses a game week, month or
// quarter it composes ONE digest per running company of the AI, at the highest cadence crossed
// (quarter > month > week), about the period that just ended. Every figure is read, none typed:
//   standing   rank by company value, our share of the leader's value and its trend, vehicles
//              against the largest fleet (openttd://companies);
//   series     the share of the leader by game month, rebuilt from the store (named query
//              company_trend for every company), and its completed quarters;
//   by_type    profit per vehicle per transport type, ours against the leader's
//              (replan.<company>.competition), with the date replan computed it;
//   profit     the last quarter's operating profit and its change (replan.<company>.finances);
//   money      cash, loan, max loan; idle cash and unused credit from gap.<company> when it exists;
//   failed     the watchdog's verdict (watchdog.<company>), replan's hold, refusals and why it did
//              not switch, every panel service stalled or given up, approvals the deadline settled
//              without the owner, and what the gap watch (gap.<company>) and the planner's outcome
//              assessment (outcome.<company>) found;
//   changes    what replan proposed and why, and what became of the approvals of the period;
//   governance who changed governance (auto-approve) in the period, from the panel's governance.*
//              events (kept in digest.governance): the setting, from, to, via, credential, Origin,
//              User-Agent and peer, each as the event carries it;
//   evaluation what service_eval found (eval.poor, eval.coverage): actions persistently poor and
//              services that reacted to few deterioration episodes; associative, not causal;
//   approvals  each pending approval of the company with its countdown to the deadline and the
//              verdict it then gets, read from the approval record (openttd://approvals), never
//              assumed.
// A section whose source is missing or stale is "unknown", never "ok".
//
// Escalation (the constants below; edit them while it runs, the service reloads within a second):
// performance alerts (share_falling, far_behind, behind, fleet_gap, last_place) wait until
// GRACE_QUARTERS quarters are complete; health alerts (stalled, bot_stopped, approvals, idle_cash,
// signals) count at once. The digest's level is the highest of ok < unknown < watch < alert.
//
// Where it goes:
//   blackboard  digest.<company> the latest digest; digest.<company>.<week|month|quarter> a compact
//               row per digest of that cadence; digest.status the heartbeat (GET /api/digest).
//               Every write is a rhai.shared event, so it is on the bus and in the store too;
//   chat        one "[digest]" line for the owner's company: every month and quarter, a week only
//               when its level, rank or alerts changed; never closer than COOLDOWN_S apart, except
//               a new alert, which is said at once; an unresolved alert is said again every REPEAT_S;
//   news        the owner's newspaper: every quarterly digest and every new alert;
//   mesh        every quarterly digest and every new alert as an observation record
//               (task openttd:digest:<company>) when the panel has a mesh session.
// The owner's company is $DIGEST_COMPANY when set (0-based), else the first running company of the
// AI, the one the owner co-plays (as the narrator chooses it).
//
// Read-only toward the game: it issues no command and approves nothing. The first line declares to
// the panel's supervisor how long it may stay quiet before it is restarted. DIGEST_ROUNDS stops it
// after that many rounds (0 or unset: never); DIGEST_COOLDOWN_S overrides COOLDOWN_S.

const AI = "PrismaticAI";
const ROUND_S = 15;
// Ticks per game day (TimerGameTick::DAY_TICKS).
const DAY_TICKS = 74;
// Rows kept per cadence history.
const KEEP_WEEK = 52;
const KEEP_MONTH = 120;
const KEEP_QUARTER = 80;
// Game months of the share series rebuilt from the store.
const SERIES_MONTHS = 36;
// Chat: the least wall-clock time between two scheduled lines, and between two repeats of an
// unresolved alert.
const COOLDOWN_S = 300;
const REPEAT_S = 900;
// Performance thresholds, as shares of the leader's company value.
const SHARE_FAR = 0.25;
const SHARE_FAR_CLEAR = 0.30;
const SHARE_BEHIND = 0.5;
// share_falling: two completed quarters of decline by at least this much in all.
const SHARE_DROP = 0.05;
// fleet_gap: our vehicles against the largest fleet of another company.
const FLEET_GAP = 0.10;
const GRACE_QUARTERS = 2;
// Approvals: a countdown this short (game days) is a watch; without a deadline, an age this long
// is an alert.
const COUNTDOWN_DAYS = 7;
const WAIT_DAYS = 30;
// Idle cash: no progress (watchdog quiet days) for this long while holding money.
const IDLE_QUIET_DAYS = 60;
// A replan key older than this many game months is stale.
const STALE_MONTHS = 2;
// The signal keys other services write per company, read as their writers define them:
//   gap.<company>      gap_watch.rhai (the leader-gap watch): flags {name: reason}, streaks, level
//                      0..3, idle_cash, unused_credit, builds {refused, failed, removed, ...}, held,
//                      proposal, month;
//   outcome.<company>  the planner's outcome assessment (panel_bot.rs, src/outcome.rs): verdict,
//                      why, signals [{kind, severity, why}], proposal, game {seed, date, tick};
//   capital.<company>  capital_watch.rhai (ADR 0042's envelope): flags {under_reinvesting,
//                      hoarding: reason}, trailing {reinvestment}, idle_ratio, vehicles_growth, leader.
// A key that is absent is listed as unknown in the digest, so a writer that stopped is seen.
const SIGNALS = ["gap", "outcome", "capital"];
// A gap flag held this many months in a row is an alert (gap_watch's own TELL).
const GAP_TELL = 2;
// The service evaluation (service_eval): eval.poor (service.action entries persistently poor, with
// their streak) and eval.coverage (per service: deterioration episodes of the newest run and how
// many it reacted to). Its verdicts are associative, not causal, and the digest says so. A service
// that reacted to less than EVAL_REACT_MIN of at least EVAL_MIN_EPISODES episodes is a watch.
const EVAL_REACT_MIN = 0.10;
const EVAL_MIN_EPISODES = 20;
// The paths a mesh record claims (the mesh refuses a claim of none).
const RECORD_SCOPE = ["tools/research/mcp-panel/rhai/digest.rhai"];
// Governance changes (panel events governance.*: who toggled auto-approve, with which credential,
// from where) kept in digest.governance.
const KEEP_GOVERNANCE = 50;

const LEVELS = ["ok", "unknown", "watch", "alert"];

fn rounds() { let v = env("DIGEST_ROUNDS"); if v == "" { 0 } else { parse_int(v) } }
fn cooldown_s() { let v = env("DIGEST_COOLDOWN_S"); if v == "" { global::COOLDOWN_S } else { parse_int(v) } }

fn as_int(x) { if type_of(x) == "f64" { x.to_int() } else if type_of(x) == "i64" { x } else { 0 } }
fn as_float(x) { if type_of(x) == "i64" { x.to_float() } else if type_of(x) == "f64" { x } else { 0.0 } }
fn is_unit(x) { type_of(x) == "()" }

// A share 0..1 as whole percent ("56%"), or "?" when unknown.
fn pct(x) { if is_unit(x) { "?" } else { `${(x * 100.0).round().to_int()}%` } }

// A money amount short: 1234567 -> "1.2M", 45000 -> "45k".
fn short(n) {
    let v = as_float(n);
    let a = v.abs();
    if a >= 1000000.0 { `${(v / 100000.0).round() / 10.0}M` }
    else if a >= 1000.0 { `${(v / 1000.0).round().to_int()}k` }
    else { `${v.round().to_int()}` }
}

// ---- cadence --------------------------------------------------------------------------------

// Where a date and tick fall: day and week index, "YYYY-MM", "YYYY-Qn".
fn period_of(date, tick) {
    let p = `${date}`.split("-");
    let y = if p.len() > 0 { parse_int(p[0]) } else { 0 };
    let m = if p.len() > 1 { parse_int(p[1]) } else { 1 };
    let day = as_int(tick) / global::DAY_TICKS;
    #{ date: `${date}`, tick: as_int(tick), day: day, week: day / 7,
       month: `${date}`.sub_string(0, 7), quarter: `${y}-Q${(m - 1) / 3 + 1}` }
}

// The quarter "YYYY-Qn" of a month "YYYY-MM".
fn quarter_of_month(month) {
    let p = month.split("-");
    if p.len() < 2 { return ""; }
    `${p[0]}-Q${(parse_int(p[1]) - 1) / 3 + 1}`
}

// The cadence crossed from `prev` (the last digest's period, () at first) to `now`: "first",
// "rollback" (the date went back: a new or reloaded game), "quarter", "month", "week", or ().
fn crossed(prev, now) {
    if is_unit(prev) { return "first"; }
    if now.day < prev.day { return "rollback"; }
    if now.quarter != prev.quarter { return "quarter"; }
    if now.month != prev.month { return "month"; }
    if now.week != prev.week { return "week"; }
    ()
}

// The period a digest reports and its label: the one that just ended (a quarterly digest on
// 1954-04-01 is about 1954-Q1). A first digest, or one after a rollback, is "start": where things
// stand today, said at once, and kept in no cadence history (its period is not complete).
fn reported(cadence, prev, now) {
    if cadence == "quarter" { return #{ period: "quarter", label: prev.quarter }; }
    if cadence == "month" { return #{ period: "month", label: prev.month }; }
    if cadence == "week" { return #{ period: "week", label: `week of ${prev.date}` }; }
    #{ period: "start", label: now.date }
}

// Game months between two dates or months ("1954-02-10", "1953-10").
fn months_between(a, b) {
    let pa = `${a}`.split("-"); let pb = `${b}`.split("-");
    if pa.len() < 2 || pb.len() < 2 { return 0; }
    (parse_int(pb[0]) * 12 + parse_int(pb[1])) - (parse_int(pa[0]) * 12 + parse_int(pa[1]))
}

// ---- standing and series --------------------------------------------------------------------

// Our rank, the leader (by company value), our share of its value, and our vehicles against the
// largest fleet of another company. () when we are not among the companies.
fn standing(companies, me) {
    let mine = ();
    for c in companies { if as_int(c.id) == me { mine = c; } }
    if is_unit(mine) { return (); }
    let value = as_float(mine.company_value);
    let rank = 1;
    let leader = (); let runner = ();
    let fleet_max = 0; let fleet_max_name = "";
    for c in companies {
        let v = as_float(c.company_value);
        if v > value { rank += 1; }
        if is_unit(leader) || v > as_float(leader.company_value) { leader = c; }
        if as_int(c.id) != me {
            if is_unit(runner) || v > as_float(runner.company_value) { runner = c; }
            let n = as_int(c.fleet?.total);
            if n > fleet_max { fleet_max = n; fleet_max_name = c.name; }
        }
    }
    let we_lead = as_int(leader.id) == me;
    let lv = as_float(leader.company_value);
    let share = if we_lead { 1.0 } else if lv > 0.0 { value / lv } else { () };
    let margin = if we_lead && !is_unit(runner) && as_float(runner.company_value) > 0.0 { value / as_float(runner.company_value) } else { () };
    let vehicles = as_int(mine.fleet?.total);
    #{ rank: rank, of: companies.len(), name: mine.name, value: value, we_lead: we_lead,
       leader: leader.name, leader_value: lv, share: share,
       runner_up: if is_unit(runner) { () } else { runner.name }, lead_margin: margin,
       vehicles: vehicles, fleet: mine.fleet ?? #{}, largest_fleet: fleet_max, largest_fleet_name: fleet_max_name,
       fleet_share: if fleet_max > 0 { vehicles.to_float() / fleet_max.to_float() } else { () } }
}

// The share of the leader by game month, oldest first, from each company's company_trend rows
// ([month, value, ...], newest first): [{month, share, rank, of, leader, leader_value, value}].
fn series_of(rows_by_company, names, me) {
    let months = #{};
    for id in rows_by_company.keys() {
        for r in rows_by_company[id] {
            let m = `${r[0]}`;
            if !(m in months) { months[m] = #{}; }
            months[m][id] = as_float(r[1]);
        }
    }
    let keys = months.keys();
    keys.sort();
    let out = [];
    let mine = `${me}`;
    for m in keys {
        let vals = months[m];
        if !(mine in vals) { continue; }
        let lid = (); let lv = 0.0; let rank = 1;
        for id in vals.keys() {
            if vals[id] > vals[mine] { rank += 1; }
            if is_unit(lid) || vals[id] > lv { lid = id; lv = vals[id]; }
        }
        out.push(#{ month: m, value: vals[mine], leader: names[lid] ?? lid, leader_value: lv,
                    share: if lid == mine { 1.0 } else if lv > 0.0 { vals[mine] / lv } else { () },
                    rank: rank, of: vals.len() });
    }
    if out.len() > global::SERIES_MONTHS { out = out.extract(out.len() - global::SERIES_MONTHS); }
    out
}

// The completed quarters of a series: the share at the last month of each quarter before
// `current` ("YYYY-Qn", the quarter in progress), oldest first.
fn quarters_of(series, current) {
    let by = #{};
    let order = [];
    for s in series {
        let q = quarter_of_month(s.month);
        if q == current || q == "" { continue; }
        if !(q in by) { order.push(q); }
        by[q] = s;
    }
    let out = [];
    for q in order { out.push(#{ quarter: q, share: by[q].share, rank: by[q].rank, leader: by[q].leader }); }
    out
}

// ---- the other sections ---------------------------------------------------------------------

// Each pending approval of the company with its countdown and the verdict it then gets, from the
// approval record: its own deadline_tick and verdict (default_verdict), else the queue's deadline
// verdict. Also what became of the company's approvals created since `since_tick`.
fn approvals_of(doc, company, tick, since_tick) {
    let queue = doc?.deadline ?? #{};
    let items = [];
    let outcomes = #{ approved: 0, denied: 0, by_deadline: 0, executed: 0 };
    for a in doc?.approvals ?? [] {
        if as_int(a.args?.company) != company || is_unit(a.args?.company) { continue; }
        if a.state == "pending" {
            let dl = as_int(a.deadline_tick);
            let verdict = a.verdict ?? a.default_verdict ?? queue.verdict ?? ();
            items.push(#{ id: a.id, what: what_of(a), age_days: (tick - as_int(a.created_tick)) / global::DAY_TICKS,
                          countdown_days: if dl > 0 { (dl - tick) / global::DAY_TICKS } else { () },
                          verdict: verdict });
        } else if as_int(a.created_tick) >= since_tick {
            let s = `${a.state}`;
            if s in outcomes { outcomes[s] += 1; }
            if `${a.settled_by ?? ""}`.starts_with("deadline") { outcomes.by_deadline += 1; }
        }
    }
    items.sort(|x, y| if is_unit(x.countdown_days) { 1 } else if is_unit(y.countdown_days) { -1 } else { x.countdown_days - y.countdown_days });
    #{ pending: items.len(), items: if items.len() > 8 { items.extract(0, 8) } else { items },
       soonest_days: if items.is_empty() { () } else { items[0].countdown_days },
       oldest_days: items.reduce(|m, x| if x.age_days > m { x.age_days } else { m }, 0),
       deadline: queue, since: outcomes }
}

// One request in a few words: a setting (args.name=value), a plan of settings, else the tool.
fn what_of(a) {
    let tool = `${a.tool ?? a.action ?? "request"}`;
    tool.replace("openttd.", "");
    let args = a.args ?? #{};
    if type_of(args.settings) == "map" { return `${tool} ${args.settings.len()} setting(s)`; }
    if !is_unit(args.name) { return `${tool} ${args.name}=${args.value ?? ""}`; }
    tool
}

// replan's comparison per transport type, its operating profit and its proposal, with its age.
fn replan_part(r, today) {
    if type_of(r) != "map" {
        return #{ source: "unknown", why: "replan has not written replan.<company>", date: (), age_months: (), by_type: [],
                  profit: #{ quarter: (), previous: (), change: () }, adopt: false, held: (), proposed: [], portfolio: [], refused: [] };
    }
    let age = months_between(r.date ?? "", today);
    let q = r.finances?.quarters ?? [];
    let op = if q.len() > 0 && !is_unit(q[0].income) && !is_unit(q[0].expenses) { as_float(q[0].income) + as_float(q[0].expenses) } else { () };
    let op_prev = if q.len() > 1 && !is_unit(q[1].income) && !is_unit(q[1].expenses) { as_float(q[1].income) + as_float(q[1].expenses) } else { () };
    #{ source: if age > global::STALE_MONTHS { "stale" } else { "replan" }, date: r.date, age_months: age,
       by_type: r.competition ?? [],
       profit: #{ quarter: op, previous: op_prev, change: if is_unit(op) || is_unit(op_prev) { () } else { op - op_prev } },
       adopt: r.model?.adopt ?? false, why: r.model?.why ?? "", held: r.held,
       proposed: r.proposed ?? [], portfolio: r.portfolio ?? [], refused: r.refused ?? [] }
}

// What failed or was skipped, each one line, from the keys and the supervisor.
fn failures(wd, rp, services, signals, appr) {
    let out = [];
    if type_of(wd) == "map" && wd.stalled == true {
        out.push(`no progress for ${wd.quiet_days} game day(s) (watchdog threshold ${wd.threshold_days}): ${(wd.reasons ?? []).reduce(|a, b| if a == "" { b } else { a + "; " + b }, "")}`);
    }
    if rp.source != "unknown" {
        if !is_unit(rp.held) && rp.held != false && rp.held != "" { out.push(`replan holds: ${rp.held}`); }
        for x in rp.refused { out.push(`refused: ${x}`); }
        if !rp.adopt && rp.why != "" { out.push(`replan did not switch: ${rp.why}`); }
    } else { out.push(rp.why); }
    for s in services {
        if s.stalled == true { out.push(`service ${s.name} is stalled`); }
        if s.gave_up == true { out.push(`service ${s.name} was given up`); }
    }
    if appr.since.by_deadline > 0 { out.push(`${appr.since.by_deadline} approval(s) were settled by the deadline without you`); }
    let gap = signals.gap;
    if type_of(gap) == "map" {
        let b = gap.builds ?? #{};
        let n = as_int(b.refused) + as_int(b.failed) + as_int(b.removed);
        if n > 0 { out.push(`gap watch ${gap.month ?? ""}: ${as_int(b.refused)} build(s) refused, ${as_int(b.failed)} failed, ${as_int(b.removed)} removed`); }
        if type_of(gap.held) == "string" && gap.held != "" && gap.held != "no lever applies" { out.push(`gap watch held its proposal: ${gap.held}`); }
    }
    let oc = signals.outcome;
    if type_of(oc) == "map" && (oc.verdict == "mismatch" || oc.verdict == "watch") {
        out.push(`planner outcome ${oc.verdict}: ${oc.why ?? ""}`);
    }
    out
}

// The signal keys of SIGNALS that exist for a company: {prefix: value}.
fn signals_of(company) {
    let out = #{};
    for p in global::SIGNALS {
        let v = shared_get(`${p}.${company}`);
        if type_of(v) == "map" { out[p] = v; }
    }
    out
}

// ---- governance ----------------------------------------------------------------------------

// One governance change as a line: "auto_approve false -> true via POST /api/auto-approve
// (credential none, Origin x, UA y, peer z)", from a governance.changed event's data.
fn governance_line(g) {
    let who = [];
    for k in [["credential", "credential"], ["origin", "Origin"], ["user_agent", "UA"], ["peer", "peer"]] {
        let v = g[k[0]];
        if !is_unit(v) && `${v}` != "" { who.push(`${k[1]} ${v}`); }
    }
    let at = if is_unit(g.game_date) { "" } else { " on " + g.game_date };
    let by = if who.is_empty() { "" } else { " (" + who.reduce(|a, b| if a == "" { b } else { a + ", " + b }, "") + ")" };
    `${g.setting ?? g.kind ?? "governance"} ${g.from ?? "?"} -> ${g.to ?? "?"} via ${g.via ?? "?"}${at}${by}`
}

// The governance changes of a period: those at or after `since_tick` (every kept one at first).
fn governance_since(all, since_tick) {
    (all ?? []).filter(|g| is_unit(g.game_tick) || as_int(g.game_tick) >= since_tick)
}

// Whether a change hands the owner's approvals to a machine (auto-approve turned on).
fn governance_grave(g) { `${g.setting ?? ""}` == "auto_approve" && g.to == true }

// ---- service evaluation ----------------------------------------------------------------------

// What service_eval found, from eval.poor and eval.coverage as they are read; () when neither exists.
// A poor entry is "service.action" or a map naming it (key, name or entry) with its streak; a
// coverage entry gives episodes (or deterioration_episodes) and reacted (or reacted_to).
fn evaluation_of(poor, coverage) {
    if type_of(poor) == "()" && type_of(coverage) == "()" { return (); }
    let entries = [];
    for x in poor ?? [] {
        if type_of(x) == "string" { entries.push(#{ entry: x, streak: (), verdict: "poor" }); }
        else if type_of(x) == "map" {
            let name = x.key ?? x.name ?? x.entry ?? (if is_unit(x.service) { "?" } else { `${x.service}.${x.action ?? "?"}` });
            entries.push(#{ entry: `${name}`, streak: x.streak, verdict: x.verdict ?? "poor" });
        }
    }
    let rows = [];
    let cov = if type_of(coverage) == "map" && type_of(coverage.services) == "map" { coverage.services } else { coverage };
    if type_of(cov) == "map" {
        for k in cov.keys() {
            let c = cov[k];
            if type_of(c) != "map" { continue; }
            let episodes = as_int(c.episodes ?? c.deterioration_episodes);
            let reacted = as_int(c.reacted ?? c.reacted_to);
            rows.push(#{ service: k, episodes: episodes, reacted: reacted,
                         share: if episodes > 0 { reacted.to_float() / episodes.to_float() } else { () } });
        }
    }
    #{ poor: entries, coverage: rows, caveat: "associative, not causal: a verdict says what went together, not what caused it" }
}

// The service evaluation's alerts: every persistently poor entry, and every service that reacted
// to too few deterioration episodes. All watches: the verdicts are associative.
fn evaluation_alerts(ev) {
    let out = [];
    if type_of(ev) != "map" { return out; }
    for x in ev.poor {
        let streak = if is_unit(x.streak) { "" } else { " (streak " + x.streak + ")" };
        out.push(alert(`eval.poor.${x.entry}`, "watch", `${x.entry} is persistently ${x.verdict}${streak}; associative, not causal`));
    }
    for c in ev.coverage {
        if c.episodes >= global::EVAL_MIN_EPISODES && !is_unit(c.share) && c.share < global::EVAL_REACT_MIN {
            out.push(alert(`eval.coverage.${c.service}`, "watch",
                           `${c.service} reacted to ${c.reacted} of ${c.episodes} deterioration episodes (${(c.share * 1000.0).round() / 10.0}%)`));
        }
    }
    out
}

// ---- escalation -----------------------------------------------------------------------------

fn alert(key, level, text) { #{ key: key, level: level, text: text } }

// Every alert of a digest. `prev_keys` are the alert keys of the previous digest (hysteresis).
fn alerts_of(d, prev_keys, wd, money, signals, line_budget) {
    let out = [];
    let st = d.standing;
    let qs = d.quarters;
    let grace = qs.len() < global::GRACE_QUARTERS;
    if !is_unit(st) && !grace {
        let n = qs.len();
        if n >= 3 && !is_unit(qs[n - 3].share) && !is_unit(qs[n - 2].share) && !is_unit(qs[n - 1].share)
            && qs[n - 3].share > qs[n - 2].share && qs[n - 2].share > qs[n - 1].share
            && qs[n - 3].share - qs[n - 1].share >= global::SHARE_DROP {
            out.push(alert("share_falling", "alert", `share of the leader fell two quarters running: ${pct(qs[n - 3].share)} -> ${pct(qs[n - 2].share)} -> ${pct(qs[n - 1].share)}`));
        }
        let s = d.series;
        let m = s.len();
        let below = m >= 2 && !is_unit(s[m - 1].share) && !is_unit(s[m - 2].share) && s[m - 1].share < global::SHARE_FAR && s[m - 2].share < global::SHARE_FAR;
        let held = prev_keys.contains("far_behind") && !is_unit(st.share) && st.share < global::SHARE_FAR_CLEAR;
        if below || held {
            out.push(alert("far_behind", "alert", `${pct(st.share)} of ${st.leader}'s value`));
        } else if !is_unit(st.share) && st.share < global::SHARE_BEHIND {
            out.push(alert("behind", "watch", `${pct(st.share)} of ${st.leader}'s value`));
        }
        if !is_unit(st.fleet_share) && st.fleet_share < global::FLEET_GAP {
            out.push(alert("fleet_gap", "alert", `${st.vehicles} vehicles against ${st.largest_fleet_name}'s ${st.largest_fleet}`));
        }
        if st.rank == st.of && st.of > 2 { out.push(alert("last_place", "watch", `rank ${st.rank} of ${st.of}`)); }
    }
    if type_of(wd) == "map" && wd.stalled == true {
        let twice = as_int(wd.quiet_days) > 2 * as_int(wd.threshold_days);
        out.push(alert("stalled", if twice { "alert" } else { "watch" }, `nothing built or bought for ${wd.quiet_days} game day(s)`));
    }
    let a = d.approvals;
    if a.pending > 0 {
        if !is_unit(a.soonest_days) && a.soonest_days <= global::COUNTDOWN_DAYS {
            out.push(alert("approvals_due", "watch", `${a.pending} approval(s) wait; the first gets "${a.items[0].verdict ?? "?"}" in ${a.soonest_days} game day(s)`));
        } else if is_unit(a.soonest_days) && a.oldest_days > global::WAIT_DAYS {
            out.push(alert("approvals_waiting", "alert", `${a.pending} approval(s) wait, the oldest ${a.oldest_days} game day(s), with no deadline`));
        }
    }
    let quiet = if type_of(wd) == "map" { as_int(wd.quiet_days) } else { 0 };
    let gap = signals.gap;
    // With the gap watch's key its idle_capital flag decides (below); without it, derived here.
    let idle = if type_of(gap) == "map" { false }
               else { quiet >= global::IDLE_QUIET_DAYS && (money.cash >= 2.0 * as_float(line_budget) && money.cash > 0.0
                      || money.loan > 0.0 && money.cash >= money.loan_step) };
    if idle { out.push(alert("idle_cash", "watch", `cash ${short(money.cash)}, loan ${short(money.loan)}, no progress for ${quiet} game day(s)`)); }
    let gov = d.governance ?? [];
    if !gov.is_empty() {
        let grave = gov.filter(|g| governance_grave(g));
        out.push(alert("governance", if grave.is_empty() { "watch" } else { "alert" },
                       `${gov.len()} governance change(s): ${governance_line(gov[gov.len() - 1])}`));
    }
    out += signal_alerts(signals);
    out += evaluation_alerts(d.evaluation);
    out
}

// The alerts of the signal keys, as their writers define them: every flag of gap.<company> (an
// alert once it held GAP_TELL months, or when gap_watch proposed a lever), every signal of
// outcome.<company> at its own severity.
fn signal_alerts(signals) {
    let out = [];
    let gap = signals.gap;
    if type_of(gap) == "map" && type_of(gap.flags) == "map" {
        for k in gap.flags.keys() {
            let streak = as_int((gap.streaks ?? #{})[k]);
            let grave = streak >= global::GAP_TELL || as_int(gap.level) >= 3;
            let held = if streak > 1 { " (" + streak + " months)" } else { "" };
            out.push(alert(`gap.${k}`, if grave { "alert" } else { "watch" }, `${gap.flags[k]}${held}`));
        }
    }
    // capital_watch's flags already need persistence (PERSIST quarters, or idle years with a fleet
    // that did not keep up), so each is an alert.
    let cap = signals.capital;
    if type_of(cap) == "map" && type_of(cap.flags) == "map" {
        for k in cap.flags.keys() { out.push(alert(`capital.${k}`, "alert", `${cap.flags[k]}`)); }
    }
    let oc = signals.outcome;
    if type_of(oc) == "map" {
        for x in oc.signals ?? [] {
            out.push(alert(`outcome.${x.kind ?? "signal"}`, if x.severity == "mismatch" { "alert" } else { "watch" }, `${x.why ?? x.kind}`));
        }
    }
    out
}

// The highest level of the alerts; "unknown" over "ok" when a section's source is missing.
fn level_of(alerts, unknown) {
    let at = if unknown.is_empty() { 0 } else { 1 };
    for a in alerts {
        let i = global::LEVELS.index_of(a.level);
        if i > at { at = i; }
    }
    global::LEVELS[at]
}

// ---- the digest -----------------------------------------------------------------------------

// One company's digest from what was read (pure). `prev` is its previous digest (() at first).
fn compose(input) {
    let c = input.company;
    let st = standing(input.companies, c);
    let me = ();
    for x in input.companies { if as_int(x.id) == c { me = x; } }
    let money = if is_unit(me) { #{ cash: 0.0, loan: 0.0, max_loan: 0.0, loan_step: 0.0 } }
                else { #{ cash: as_float(me.money), loan: as_float(me.loan), max_loan: as_float(me.max_loan), loan_step: as_float(me.loan_step) } };
    money.net = money.cash - money.loan;
    let gap = input.signals.gap;
    money.idle_cash = if type_of(gap) == "map" { gap.idle_cash } else { () };
    money.unused_credit = if type_of(gap) == "map" { gap.unused_credit } else { () };
    let cap = input.signals.capital;
    money.reinvestment = if type_of(cap) == "map" { cap.trailing?.reinvestment } else { () };
    money.idle_ratio = if type_of(cap) == "map" { cap.idle_ratio } else { () };
    let series = input.series;
    let rp = replan_part(input.replan, input.now.date);
    let unknown = [];
    if rp.source != "replan" { unknown.push("by_type"); }
    if series.is_empty() { unknown.push("series"); }
    if type_of(input.watchdog) != "map" { unknown.push("watchdog"); }
    for p in global::SIGNALS { if type_of(input.signals[p]) != "map" { unknown.push(p); } }
    if type_of(input.evaluation) != "map" { unknown.push("eval"); }
    let d = #{ company: c, name: if is_unit(st) { `company ${c}` } else { st.name },
               date: input.now.date, period: input.report.period, label: input.report.label,
               standing: st, series: series, quarters: quarters_of(series, input.now.quarter),
               by_type: rp.by_type, by_type_source: rp.source, by_type_date: rp.date,
               profit: rp.profit, money: money,
               changes: #{ adopt: rp.adopt, why: rp.why, proposed: rp.proposed, portfolio: rp.portfolio, date: rp.date },
               approvals: input.approvals, governance: input.governance ?? [], evaluation: input.evaluation,
               unknown: unknown };
    let prev = input.prev;
    let prev_share = if type_of(prev) == "map" && prev.period == d.period && !is_unit(prev.standing) { prev.standing.share } else { () };
    if !is_unit(st) {
        d.standing.share_prev = prev_share;
        d.standing.share_change = if is_unit(prev_share) || is_unit(st.share) { () } else { st.share - prev_share };
        let pl = if type_of(prev) == "map" && !is_unit(prev.standing) { prev.standing.leader } else { () };
        d.standing.leader_changed = !is_unit(pl) && pl != st.leader;
    }
    d.failed = failures(input.watchdog, rp, input.services, input.signals, input.approvals);
    let prev_keys = if type_of(prev) == "map" { (prev.alerts ?? []).map(|a| a.key) } else { [] };
    d.alerts = alerts_of(d, prev_keys, input.watchdog, money, input.signals, input.line_budget);
    d.new_alerts = d.alerts.filter(|a| !prev_keys.contains(a.key)).map(|a| a.key);
    let keys = d.alerts.map(|a| a.key);
    d.cleared = prev_keys.filter(|k| !keys.contains(k));
    d.level = level_of(d.alerts, unknown);
    d
}

// The digest as one chat line.
fn line_of(d) {
    let st = d.standing;
    let parts = [];
    if is_unit(st) { parts.push("not among the companies"); }
    else {
        let change = if is_unit(st.share_change) { "" } else { ` (${if st.share_change >= 0.0 { "+" } else { "" }}${(st.share_change * 100.0).round().to_int()} pt)` };
        parts.push(if st.we_lead { `rank 1/${st.of}, leading ${st.runner_up ?? "nobody"}` }
                   else { `rank ${st.rank}/${st.of}, ${pct(st.share)} of ${st.leader}${change}` });
        parts.push(`${st.vehicles} vs ${st.largest_fleet} vehicles`);
    }
    if !is_unit(d.profit.quarter) { parts.push(`op. profit ${short(d.profit.quarter)}/q`); }
    let a = d.approvals;
    if a.pending > 0 {
        parts.push(if is_unit(a.soonest_days) { `${a.pending} approval(s) wait` }
                   else { `${a.pending} approval(s): ${a.items[0].verdict ?? "?"} in ${a.soonest_days} d` });
    }
    let text = `${d.label}: ${parts.reduce(|x, y| if x == "" { y } else { x + ", " + y }, "")}`;
    let hot = d.alerts.filter(|x| x.level == "alert").map(|x| x.key);
    if !hot.is_empty() { text += `; ALERT ${hot.reduce(|x, y| if x == "" { y } else { x + " " + y }, "")}`; }
    else if d.level != "ok" { text += `; ${d.level}`; }
    text
}

// Whether the owner's company's digest is said in chat now: a new alert at once; a month or a
// quarter after the cooldown; a week only when its level, rank or alerts changed; an unresolved
// alert again after REPEAT_S.
fn should_say(d, prev, since_said_s, since_alert_s, cooldown) {
    if !d.new_alerts.is_empty() { return true; }
    let hot = d.alerts.filter(|x| x.level == "alert");
    if !hot.is_empty() && as_float(since_alert_s) >= as_float(global::REPEAT_S) { return true; }
    if as_float(since_said_s) < as_float(cooldown) { return false; }
    if d.period != "week" { return true; }
    if type_of(prev) != "map" { return true; }
    let rank = if is_unit(d.standing) { 0 } else { d.standing.rank };
    let prank = if is_unit(prev.standing) { 0 } else { prev.standing.rank };
    d.level != prev.level || rank != prank || !d.cleared.is_empty()
}

// The compact row a cadence history keeps.
fn row_of(d) {
    let st = d.standing;
    #{ date: d.date, label: d.label, level: d.level, alerts: d.alerts.map(|a| a.key),
       rank: if is_unit(st) { () } else { st.rank }, of: if is_unit(st) { () } else { st.of },
       share: if is_unit(st) { () } else { st.share }, leader: if is_unit(st) { () } else { st.leader },
       vehicles: if is_unit(st) { () } else { st.vehicles }, largest_fleet: if is_unit(st) { () } else { st.largest_fleet },
       profit: d.profit.quarter, cash: d.money.cash, loan: d.money.loan, pending: d.approvals.pending }
}

fn keep_of(period) {
    if period == "week" { global::KEEP_WEEK } else if period == "quarter" { global::KEEP_QUARTER } else { global::KEEP_MONTH }
}

// ---- host reads (each round) -------------------------------------------------------------------

// One named query of the store (tools/research/mcp-rhai/sql/queries/<name>.sql), its rows.
fn rows_of(name, params) {
    let r = api_dbNamedRun(#{ name: name, params: params });
    if "error" in r { throw `store: ${r.error}`; }
    if "http_error" in r { throw `store: ${r.http_error} ${r.body}`; }
    r.rows ?? []
}

// The share series from the store for company `me`: () when the store cannot answer.
fn store_series(companies, me) {
    let rows = #{};
    let names = #{};
    for c in companies {
        rows[`${c.id}`] = rows_of("company_trend", #{ company: as_int(c.id), months: global::SERIES_MONTHS });
        names[`${c.id}`] = c.name;
    }
    series_of(rows, names, me)
}

// The series rebuilt from the digest's own monthly history when the store has none.
fn history_series(rows) {
    let out = [];
    for r in rows ?? [] {
        out.push(#{ month: `${r.label}`, share: r.share, rank: r.rank, of: r.of, leader: r.leader, value: (), leader_value: () });
    }
    out
}

fn setting_of(bot, name) {
    for s in bot.settings ?? [] { if s.name == name { return s.value; } }
    0
}

fn say(text) {
    // The console takes one line; quotes inside the message would end it early.
    text.replace("\"", "'");
    // con_say is derived from the game's console when the script starts: a service started before
    // the panel read the console has none, and a missing chat must not cost the round its records.
    try { con_say(`"[digest] ${text}"`); } catch (err) { print(`chat not said (${err}): ${text}`); }
}

fn publish(d, title) {
    try {
        mesh_record_write(#{ kind: "observation", task: `openttd:digest:${d.company}`, scope: global::RECORD_SCOPE,
                             title: title, body: to_json(d) });
    } catch (err) {
        print(`mesh: ${err}`);
    }
}

// ---- main
let looks = 0;
let clock = timestamp();
let said_at = -1000000.0;
let alert_said_at = -1000000.0;
print(`digest: a look every ${ROUND_S} s; a digest per game week, month and quarter`);
loop {
    try {
        let state = read("openttd://game/state");
        let tick = as_int(state.game_tick);
        let now = period_of(state.game_date, tick);
        let companies = read("openttd://companies").data.companies ?? [];
        let bots = [];
        for b in api_bots().running ?? [] { if b.ai == AI { bots.push(b); } }
        // The owner's company. In an arena several companies run PrismaticAI and the first is not
        // necessarily the owner's: ttd::owner_company() (fix/owner-company-derived, openttd-51)
        // replaces this choice when it lands (agreed: whichever of the two lands second switches).
        let fixed = env("DIGEST_COMPANY");
        let owner = if fixed != "" { parse_int(fixed) } else if bots.is_empty() { () } else { as_int(bots[0].id) };
        let status = shared_get("digest.status") ?? #{};
        let expected = status.expected ?? [];
        let running = bots.map(|b| as_int(b.id));
        // A bot that had a digest and runs no more is said once: silence is not "ok".
        let stopped = [];
        let said_stopped = status.said_stopped ?? [];
        for id in expected {
            if !running.contains(id) {
                stopped.push(id);
                if !said_stopped.contains(id) {
                    let text = `ALERT bot_stopped: ${AI} company ${id + 1} no longer runs; it gets no digest`;
                    say(text);
                    try { api_newsShow(#{ text: `Strategy alert: ${AI} company ${id + 1} stopped running` }); } catch (err) { print(`news: ${err}`); }
                    publish(#{ company: id, alert: "bot_stopped", date: now.date }, `ALERT company ${id + 1}: bot stopped`);
                    said_stopped.push(id);
                }
            }
        }
        said_stopped = said_stopped.filter(|id| !running.contains(id));
        for id in running { if !expected.contains(id) { expected.push(id); } }
        // Governance changes since the last look, said at once to the owner and kept for the digests.
        let gov_seq = status.gov_seq ?? 0;
        let governance = shared_get("digest.governance") ?? [];
        try {
            let feed = events(gov_seq, "governance");
            for ev in feed.events ?? [] {
                let g = ev.data ?? #{};
                g.kind = ev.kind;
                g.seq = ev.seq;
                governance.push(g);
                let text = `governance: ${governance_line(g)}`;
                say(if governance_grave(g) { `ALERT ${text}` } else { text });
                if governance_grave(g) {
                    try { api_newsShow(#{ text: `Strategy alert: ${governance_line(g)}` }); } catch (err) { print(`news: ${err}`); }
                    try {
                        mesh_record_write(#{ kind: "observation", task: "openttd:digest:governance", scope: RECORD_SCOPE,
                                             title: `ALERT governance: ${governance_line(g)}`, body: to_json(g) });
                    } catch (err) { print(`mesh: ${err}`); }
                }
            }
            gov_seq = feed.last_seq ?? gov_seq;
            if governance.len() > KEEP_GOVERNANCE { governance = governance.extract(governance.len() - KEEP_GOVERNANCE); }
            if !(feed.events ?? []).is_empty() { shared_set("digest.governance", governance); }
        } catch (err) {
            print(`digest: governance: ${err}`);
        }
        let evaluation = evaluation_of(shared_get("eval.poor"), shared_get("eval.coverage"));
        let approvals = ();
        let services = [];
        try { services = api_services().services ?? []; } catch { }
        for b in bots {
            let c = as_int(b.id);
            let prev = shared_get(`digest.${c}`);
            let cadence = crossed(if type_of(prev) == "map" { prev.at } else { () }, now);
            if is_unit(cadence) { continue; }
            if cadence == "rollback" {
                for p in ["week", "month", "quarter"] { shared_set(`digest.${c}.${p}`, []); }
                prev = ();
            }
            let report = reported(cadence, if type_of(prev) == "map" { prev.at } else { now }, now);
            if is_unit(approvals) { approvals = read("openttd://approvals").data; }
            let series = [];
            if report.period != "week" || type_of(prev) != "map" {
                try { series = store_series(companies, c); } catch (err) { print(`digest: ${err}`); }
                if series.is_empty() { series = history_series(shared_get(`digest.${c}.month`)); }
            } else { series = prev.series ?? []; }
            let since_tick = if type_of(prev) == "map" { as_int(prev.at.tick) } else { 0 };
            let d = compose(#{ company: c, companies: companies, now: now, report: report, prev: prev,
                               series: series, replan: shared_get(`replan.${c}`), watchdog: shared_get(`watchdog.${c}`),
                               signals: signals_of(c), services: services,
                               approvals: approvals_of(approvals, c, tick, since_tick),
                               line_budget: setting_of(b, "line_budget"),
                               governance: governance_since(governance, since_tick),
                               evaluation: evaluation });
            if c == owner && !stopped.is_empty() {
                for id in stopped { d.alerts.push(alert("bot_stopped", "alert", `${AI} company ${id + 1} no longer runs`)); }
                d.level = level_of(d.alerts, d.unknown);
            }
            d.at = now;
            d.owner = c == owner;
            shared_set(`digest.${c}`, d);
            if d.period != "start" {
                let hist = shared_get(`digest.${c}.${d.period}`) ?? [];
                hist.push(row_of(d));
                let keep = keep_of(d.period);
                if hist.len() > keep { hist = hist.extract(hist.len() - keep); }
                shared_set(`digest.${c}.${d.period}`, hist);
            }
            let text = line_of(d);
            print(`digest ${c}: ${text}`);
            if d.owner {
                let t = clock.elapsed;
                if should_say(d, prev, t - said_at, t - alert_said_at, cooldown_s()) {
                    say(text);
                    said_at = t;
                    if d.alerts.some(|x| x.level == "alert") { alert_said_at = t; }
                }
                // A closure must not capture the array it filters (Rhai refuses it as a data race).
                let new_keys = d.new_alerts;
                let fresh = d.alerts.filter(|x| new_keys.contains(x.key));
                if d.period == "quarter" || !fresh.is_empty() {
                    let news = if fresh.is_empty() { `Strategy digest ${text}` }
                               else { `Strategy alert: ${fresh.map(|x| x.text).reduce(|x, y| if x == "" { y } else { x + "; " + y }, "")}` };
                    try { api_newsShow(#{ text: news }); } catch (err) { print(`news: ${err}`); }
                }
            }
            if d.period == "quarter" || !d.new_alerts.is_empty() {
                publish(d, `${if d.new_alerts.is_empty() { "digest" } else { "ALERT" }} company ${c + 1} ${d.label}: ${d.level}`);
            }
        }
        shared_set("digest.status", #{ date: now.date, round: looks, owner: owner, expected: expected,
                                       running: running, stopped: stopped, said_stopped: said_stopped, gov_seq: gov_seq });
    } catch (err) {
        print(`digest: ${err}`);
    }
    looks += 1;
    if rounds() > 0 && looks >= rounds() { break; }
    sleep(ROUND_S * 1000);
}
